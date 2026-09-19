//! Fragmented-MP4 streaming out of a live ffmpeg process.
//!
//! `ffmpeg -ss <start> -i <in> … -movflags frag_keyframe+empty_moov+default_base_moof
//! -frag_duration 1000000 -f mp4 pipe:1` writes `ftyp`+`moov` immediately and
//! never rewinds, so the client can hand the chunks straight to a
//! `SourceBuffer` (`timestampOffset = start`) instead of waiting for a complete
//! file. ffmpeg rebases output timestamps to zero, which is exactly why the
//! seek mapping lives in the client's `timestampOffset` and not in ffmpeg
//! timestamp flags.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::{Child, ChildStderr, ChildStdout, Command};
use tokio::sync::SemaphorePermit;

use crate::video_processor::{
    acquire_transcode_permit, format_binary_error, get_ffmpeg_path, transcode_timeout_secs,
};

/// Seconds a stream request waits for a conversion slot before answering
/// 503 + Retry-After.
pub const STREAM_QUEUE_WAIT_SECS_DEFAULT: u64 = 20;

/// Longest `start` still counted as a run from the head of the source.
///
/// The player restarts the stream at `start=<seconds>` on every seek, and a
/// seek run converts only the tail of the file — finishing one says nothing
/// about the rest of the source, so only a run at (or within half a second of)
/// the head counts as a full playthrough. Handlers use this to decide whether a
/// finished run may fill the whole-file cache; it never gates playback, because
/// the cache is only ever a fast path (FR-010).
pub const FULL_RUN_MAX_START_SECS: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamMode {
    Remux,
    Audio,
    Transcode,
}

impl StreamMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Remux => "remux",
            Self::Audio => "audio",
            Self::Transcode => "transcode",
        }
    }

    pub fn from_query(value: &str) -> Option<Self> {
        match value {
            "remux" => Some(Self::Remux),
            "audio" => Some(Self::Audio),
            "transcode" => Some(Self::Transcode),
            _ => None,
        }
    }
}

/// How long a stream request waits for a conversion slot before answering
/// 503 + Retry-After (the client keeps its waiting state visible and retries).
/// Slots are `TURBO_PIX_MAX_TRANSCODES` permits, so waiters can never become
/// unbounded ffmpeg processes.
pub fn stream_queue_wait_secs() -> u64 {
    match std::env::var("TURBO_PIX_STREAM_QUEUE_WAIT_SECS") {
        Ok(raw) => raw.trim().parse::<u64>().unwrap_or_else(|_| {
            log::warn!("Invalid TURBO_PIX_STREAM_QUEUE_WAIT_SECS '{raw}', using default");
            STREAM_QUEUE_WAIT_SECS_DEFAULT
        }),
        Err(_) => STREAM_QUEUE_WAIT_SECS_DEFAULT,
    }
}

/// ffmpeg arguments for one stream run.
///
/// `-ss` is an *input* option (fast seek); ffmpeg rebases the output timeline
/// to zero and the client maps it back with `SourceBuffer.timestampOffset`.
/// Fragmented output flushes `ftyp`+`moov` before the first frame, so the
/// client can create its SourceBuffer immediately; `-frag_duration 1000000`
/// keeps fragments at ≤1 s even for stream copies whose keyframes are far
/// apart. `-map 0:a:0?` tolerates sources with no audio track.
pub fn build_args(mode: StreamMode, input: &Path, start_secs: f64) -> Vec<String> {
    let mut args: Vec<String> = vec!["-v".into(), "error".into(), "-nostdin".into()];
    if mode == StreamMode::Transcode {
        args.extend(["-hwaccel".into(), "auto".into()]);
    }
    if start_secs > 0.0 {
        args.extend(["-ss".into(), format!("{start_secs:.3}")]);
    }
    args.extend(["-i".into(), input.to_string_lossy().into_owned()]);
    args.extend([
        "-map".into(),
        "0:v:0".into(),
        "-map".into(),
        "0:a:0?".into(),
    ]);
    match mode {
        StreamMode::Transcode => args.extend(
            [
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "23",
                "-pix_fmt",
                "yuv420p",
                "-profile:v",
                "main",
                "-g",
                "48",
                "-keyint_min",
                "48",
                "-sc_threshold",
                "0",
                "-c:a",
                "aac",
                "-b:a",
                "160k",
                "-ac",
                "2",
            ]
            .iter()
            .map(|s| s.to_string()),
        ),
        StreamMode::Audio => args.extend(
            ["-c:v", "copy", "-c:a", "aac", "-b:a", "160k", "-ac", "2"]
                .iter()
                .map(|s| s.to_string()),
        ),
        StreamMode::Remux => args.extend(["-c", "copy"].iter().map(|s| s.to_string())),
    }
    args.extend(
        [
            "-movflags",
            "frag_keyframe+empty_moov+default_base_moof+omit_tfhd_offset",
            "-frag_duration",
            "1000000",
            "-f",
            "mp4",
            "pipe:1",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    args
}

/// Output MIME for the SourceBuffer the client must create, derived from the
/// codecs the run actually emits. Transcode always emits H.264 + AAC; Audio and
/// Remux copy the source video codec and (Remux) the source audio codec, so
/// declaring a hard-coded `avc1,mp4a.40.2` there would make the client's
/// SourceBuffer drop exactly the track it declared support for. A silent source
/// (`-map 0:a:0?` emits no audio track) yields a video-only init segment, so
/// the MIME must not promise an audio codec the segment does not carry —
/// Chromium rejects that append with `CHUNK_DEMUXER_ERROR_APPEND_FAILED`.
pub fn output_mime(mode: StreamMode, video_codec: &str, audio_codec: Option<&str>) -> String {
    let video = match (mode, video_codec) {
        (StreamMode::Transcode, _) => "avc1.42E01E",
        (_, "hevc") => "hvc1.1.6.L93.B0",
        (_, "av1") => "av01.0.15M.08",
        (_, "vp9") => "vp09.00.10.08",
        (_, "vp8") => "vp08.00.10.08",
        _ => "avc1.42E01E",
    };
    let audio = match (mode, audio_codec) {
        (_, None | Some("")) => None,
        (StreamMode::Transcode | StreamMode::Audio, _) | (_, Some("aac")) => Some("mp4a.40.2"),
        (_, Some("opus")) => Some("opus"),
        (_, Some("mp3")) => Some("mp4a.6B"),
        (_, Some("ac3")) => Some("ac-3"),
        (_, Some("eac3")) => Some("ec-3"),
        (_, Some("dts")) => Some("dts"),
        (_, Some("flac")) => Some("flac"),
        (_, Some(_)) => Some("mp4a.40.2"),
    };
    match audio {
        Some(audio) => format!("video/mp4; codecs=\"{video},{audio}\""),
        None => format!("video/mp4; codecs=\"{video}\""),
    }
}

#[derive(Debug)]
pub enum StreamStartError {
    /// No conversion slot within the queue wait.
    Busy,
    /// `TURBO_PIX_MAX_TRANSCODES=0`.
    Disabled,
    Spawn(String),
}

#[derive(Debug)]
pub struct StreamHandle {
    pub mode: StreamMode,
    pub stdout: ChildStdout,
    pub stderr: ChildStderr,
    pub child: Child,
    pub permit: SemaphorePermit<'static>,
}

pub async fn start_stream(
    mode: StreamMode,
    input: &Path,
    start_secs: f64,
) -> Result<StreamHandle, StreamStartError> {
    let permit = match tokio::time::timeout(
        Duration::from_secs(stream_queue_wait_secs()),
        acquire_transcode_permit(),
    )
    .await
    {
        Ok(Ok(permit)) => permit,
        Ok(Err(_disabled)) => return Err(StreamStartError::Disabled),
        Err(_) => return Err(StreamStartError::Busy),
    };

    let ffmpeg = get_ffmpeg_path();
    let mut child = Command::new(&ffmpeg)
        .args(build_args(mode, input, start_secs))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // A client that hangs up mid-seek cancels the handler future before
        // `supervise` takes ownership of the child; without this the transient
        // ffmpeg would keep a conversion slot while streaming to nobody.
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| StreamStartError::Spawn(format_binary_error("ffmpeg", &ffmpeg, &e)))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| StreamStartError::Spawn("ffmpeg stdout pipe unavailable".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| StreamStartError::Spawn("ffmpeg stderr pipe unavailable".to_string()))?;

    Ok(StreamHandle {
        mode,
        stdout,
        stderr,
        child,
        permit,
    })
}

/// Watch one ffmpeg child to completion. Killing at the deadline keeps a stuck
/// encoder from pinning a slot forever. Dropping the response body closes
/// stdout, which makes ffmpeg exit on EPIPE — that is what bounds client
/// disconnects.
pub async fn supervise(mut child: Child, stderr: ChildStderr) -> Result<(), String> {
    use tokio::io::AsyncReadExt;

    let stderr_task = tokio::spawn(async move {
        let mut reader = stderr;
        let mut buf = String::new();
        let _ = reader.read_to_string(&mut buf).await;
        buf
    });

    let deadline = Duration::from_secs(transcode_timeout_secs());
    let status = match tokio::time::timeout(deadline, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => {
            let _ = child.kill().await;
            let _ = stderr_task.await;
            return Err(format!("ffmpeg wait failed: {e}"));
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = stderr_task.await;
            return Err(format!("stream timed out after {}s", deadline.as_secs()));
        }
    };
    let stderr_text = stderr_task.await.unwrap_or_default();
    if status.success() {
        return Ok(());
    }
    let tail = stderr_text
        .lines()
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ");
    Err(format!("ffmpeg exited with status {status}: {tail}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Upper bound on how many bytes a head assertion reads from the stream.
    const HEAD_BYTES: usize = 64 * 1024;

    /// Read until `needle` shows up, the head fills, or the stream ends. A
    /// single `read` races ffmpeg's writes: `ftyp` and `moov` are separate
    /// `write()` calls, so the first poll can legitimately return `ftyp` alone.
    async fn read_head(stdout: &mut ChildStdout, needle: &[u8]) -> Vec<u8> {
        use tokio::io::AsyncReadExt;

        let mut head = Vec::new();
        let mut chunk = vec![0u8; 8 * 1024];
        while head.len() < HEAD_BYTES {
            let read = stdout.read(&mut chunk).await.expect("read stream head");
            if read == 0 {
                break;
            }
            head.extend_from_slice(&chunk[..read]);
            if head.windows(needle.len()).any(|w| w == needle) {
                break;
            }
        }
        head
    }

    #[test]
    fn transcode_args_reencode_into_fragmented_mp4() {
        let joined = build_args(StreamMode::Transcode, Path::new("/in.mp4"), 0.0).join(" ");
        assert!(joined.contains("-c:v libx264"));
        assert!(joined.contains("-preset veryfast"));
        assert!(joined.contains("-g 48"));
        assert!(joined.contains("-c:a aac"));
        assert!(joined.contains("-map 0:v:0 -map 0:a:0?"));
        assert!(joined.contains("-movflags frag_keyframe+empty_moov+default_base_moof"));
        assert!(joined.contains("-frag_duration 1000000"));
        assert!(joined.ends_with("-f mp4 pipe:1"));
        assert!(!joined.contains("-ss"), "start 0 must not emit a seek flag");
    }

    #[test]
    fn audio_mode_copies_video_and_reencodes_audio() {
        let joined = build_args(StreamMode::Audio, Path::new("/in.mp4"), 12.5).join(" ");
        assert!(joined.contains("-ss 12.500 -i /in.mp4"));
        assert!(joined.contains("-c:v copy"));
        assert!(joined.contains("-c:a aac"));
        assert!(!joined.contains("libx264"));
    }

    #[test]
    fn remux_mode_copies_everything() {
        let joined = build_args(StreamMode::Remux, Path::new("/in.mkv"), 3.0).join(" ");
        assert!(joined.contains("-c copy"));
        assert!(!joined.contains("libx264"));
    }

    #[test]
    fn mode_parsing_rejects_unknown_tokens() {
        assert_eq!(StreamMode::from_query("remux"), Some(StreamMode::Remux));
        assert_eq!(StreamMode::from_query("audio"), Some(StreamMode::Audio));
        assert_eq!(
            StreamMode::from_query("transcode"),
            Some(StreamMode::Transcode)
        );
        assert_eq!(StreamMode::from_query("direct"), None);
        assert_eq!(StreamMode::from_query(""), None);
    }

    #[test]
    fn output_mime_matches_the_mode() {
        assert_eq!(
            output_mime(StreamMode::Transcode, "hevc", Some("ac3")),
            "video/mp4; codecs=\"avc1.42E01E,mp4a.40.2\""
        );
        assert_eq!(
            output_mime(StreamMode::Remux, "hevc", Some("aac")),
            "video/mp4; codecs=\"hvc1.1.6.L93.B0,mp4a.40.2\""
        );
        assert_eq!(
            output_mime(StreamMode::Audio, "hevc", Some("ac3")),
            "video/mp4; codecs=\"hvc1.1.6.L93.B0,mp4a.40.2\""
        );
        assert_eq!(
            output_mime(StreamMode::Remux, "h264", Some("ac3")),
            "video/mp4; codecs=\"avc1.42E01E,ac-3\""
        );
        // A silent source produces a video-only init segment: promising an
        // audio codec makes the browser reject the append outright.
        assert_eq!(
            output_mime(StreamMode::Transcode, "hevc", None),
            "video/mp4; codecs=\"avc1.42E01E\""
        );
        assert_eq!(
            output_mime(StreamMode::Remux, "h264", Some("")),
            "video/mp4; codecs=\"avc1.42E01E\""
        );
    }

    #[tokio::test]
    async fn streams_hevc_fixture_as_fragmented_mp4() {
        let _env_lock = crate::video_processor::tests::acquire_test_env_lock();
        let fixture = Path::new("test-data/test_video_hevc.mp4");
        if !fixture.exists() || !crate::video_processor::ffmpeg_available() {
            eprintln!("skipping: fixture or ffmpeg unavailable");
            return;
        }

        let mut handle = start_stream(StreamMode::Transcode, fixture, 0.0)
            .await
            .expect("stream must start");
        let head = read_head(&mut handle.stdout, b"moov").await;
        assert!(
            head.windows(4).any(|w| w == b"ftyp"),
            "fMP4 must start with ftyp"
        );
        assert!(
            head.windows(4).any(|w| w == b"moov"),
            "empty_moov must be written up front"
        );
        // Hang up like a client that got its first fragments: closing the read
        // end gives ffmpeg EPIPE, which is what bounds a real disconnect. A
        // 20 s source never fits in the pipe buffer, so leaving stdout open
        // would block ffmpeg until the transcode deadline.
        drop(handle.stdout);
        let _ = supervise(handle.child, handle.stderr).await;
    }

    #[tokio::test]
    async fn boundary_seek_on_a_sub_second_fixture_still_produces_bytes() {
        let _env_lock = crate::video_processor::tests::acquire_test_env_lock();
        let fixture = Path::new("test-data/test_video.mp4"); // 0.3 s
        if !fixture.exists() || !crate::video_processor::ffmpeg_available() {
            return;
        }

        let mut handle = start_stream(StreamMode::Transcode, fixture, 0.2)
            .await
            .expect("a seek near the end must still start");
        let head = read_head(&mut handle.stdout, b"ftyp").await;
        assert!(
            !head.is_empty(),
            "boundary seeks must produce a valid stream head"
        );
        drop(handle.stdout);
        let _ = supervise(handle.child, handle.stderr).await;
    }

    #[tokio::test]
    async fn remux_stream_from_matroska_produces_mp4_fragments() {
        let _env_lock = crate::video_processor::tests::acquire_test_env_lock();
        let fixture = Path::new("test-data/test_video_long.mkv");
        if !fixture.exists() || !crate::video_processor::ffmpeg_available() {
            return;
        }

        let mut handle = start_stream(StreamMode::Remux, fixture, 5.0)
            .await
            .expect("remux stream must start");
        let head = read_head(&mut handle.stdout, b"moov").await;
        assert!(head.windows(4).any(|w| w == b"ftyp"));
        assert!(head.windows(4).any(|w| w == b"moov"));
        drop(handle.stdout);
        let _ = supervise(handle.child, handle.stderr).await;
    }

    #[tokio::test]
    async fn repeated_refused_requests_spawn_no_processes() {
        // The queue wait is what turns a saturated pool into a bounded answer:
        // with every permit held and a zero wait, N attempts must all come back
        // Busy — no encoder spawned, no permit consumed, pool unchanged.
        let _wait = crate::video_processor::tests::TestEnvGuard::set(
            "TURBO_PIX_STREAM_QUEUE_WAIT_SECS",
            "0",
        );
        // Keep the pool out of the "transcoding disabled" path (a different,
        // equally bounded answer) whatever the ambient environment holds.
        let _pool =
            crate::video_processor::tests::TestEnvGuard::set("TURBO_PIX_MAX_TRANSCODES", "2");

        let semaphore = crate::video_processor::transcode_semaphore();
        let capacity = semaphore.available_permits();
        let mut held = Vec::new();
        while let Ok(permit) = semaphore.try_acquire() {
            held.push(permit);
        }

        for _ in 0..5 {
            let err = start_stream(StreamMode::Transcode, Path::new("/nonexistent.mp4"), 0.0)
                .await
                .unwrap_err();
            assert!(
                matches!(err, StreamStartError::Busy),
                "a saturated pool must answer Busy, got {err:?}"
            );
        }

        assert_eq!(
            semaphore.available_permits(),
            0,
            "refused requests must neither consume nor create permits"
        );
        drop(held);
        assert_eq!(
            semaphore.available_permits(),
            capacity,
            "the permits handed back must be exactly the pool that was held"
        );
    }
}
