//! Fragmented-MP4 streaming out of a live ffmpeg process.
//!
//! `ffmpeg -ss <start> -i <in> … -movflags frag_keyframe+empty_moov+default_base_moof
//! +omit_tfhd_offset+delay_moov -frag_duration 1000000 -f mp4 pipe:1` writes
//! `ftyp`+`moov` before the first fragment and never rewinds, so the client can
//! hand the chunks straight to a `SourceBuffer` (`timestampOffset = start`)
//! instead of waiting for a complete file. ffmpeg rebases output timestamps to
//! zero, which is exactly why the seek mapping lives in the client's
//! `timestampOffset` and not in ffmpeg timestamp flags.

use std::io;
use std::path::Path;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, ReadBuf};
use tokio::process::{Child, ChildStderr, ChildStdout, Command};
use tokio::sync::SemaphorePermit;
use tokio::task::JoinHandle;

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

/// How often [`supervise`]'s watchdog looks at the progress stamp. Small enough
/// that a run which has gone quiet is killed within a fraction of a second of
/// the configured timeout, cheap enough to keep watching for the hours a long
/// conversion legitimately runs.
const STALL_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Longest a killed run's stderr drain may hold up the watchdog's answer. The
/// drain only feeds the error message; a descendant that outlives the killed
/// child (possible when a run was started through a shell) can keep the pipe
/// open long past the kill, and that must not read as "the run is still
/// running" — the stalled encoder is already dead.
const STDERR_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

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
/// apart. `-map 0:a:0?` tolerates sources with no audio track. `video_codec` is
/// the source's first video track as the capability record resolved it, and only
/// matters for the copy modes — see the `-tag:v` note below.
pub fn build_args(
    mode: StreamMode,
    input: &Path,
    start_secs: f64,
    video_codec: &str,
) -> Vec<String> {
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
    // A copied HEVC track is tagged `hev1` by the MP4 muxer (unless the source
    // happened to carry `hvc1` already), while `output_mime` advertises
    // `hvc1.*` to the client: Chromium rejects an append whose init segment
    // contradicts the buffer's declared codec string, so the copy has to carry
    // the tag the client was promised. Only a copy can be HEVC — a Transcode run
    // emits H.264, for which the muxer rejects `hvc1` outright.
    if mode != StreamMode::Transcode && video_codec == "hevc" {
        args.extend(["-tag:v", "hvc1"].iter().map(|s| s.to_string()));
    }
    args.extend(
        [
            // `delay_moov` is what makes an AC-3/E-AC-3 audio copy muxable at
            // all: the MP4 muxer otherwise refuses to write the header
            // ("Cannot write moov atom before AC3 packets") and the run ends
            // before any init segment exists. It still writes the moov ahead of
            // every fragment, so the client can create its SourceBuffer up front.
            "-movflags",
            "frag_keyframe+empty_moov+default_base_moof+omit_tfhd_offset+delay_moov",
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

/// Process-wide monotonic epoch for [`ProgressStamp`]: an [`Instant`] cannot be
/// stored in an atomic, "milliseconds since this epoch" can.
static MONOTONIC_EPOCH: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);

fn monotonic_millis() -> u64 {
    MONOTONIC_EPOCH.elapsed().as_millis() as u64
}

/// "This run is still producing output" marker, shared between the response
/// body and [`supervise`]'s watchdog: [`ProgressReader`] touches it on every
/// chunk it hands to the client, the watchdog polls it.
///
/// A fixed wall-clock deadline cannot tell a stuck encoder from a healthy
/// conversion that is simply long — an hour-long 1080p HEVC source outlives any
/// sane configured cap, and killing it mid-playback is what the client reports
/// as a stream that ended early. Only a run that goes quiet is stuck.
#[derive(Debug, Clone)]
pub struct ProgressStamp(Arc<AtomicU64>);

impl ProgressStamp {
    /// A stamp that counts as "just produced output", so a slow ffmpeg
    /// start-up is not mistaken for a stall.
    fn new() -> Self {
        Self(Arc::new(AtomicU64::new(monotonic_millis())))
    }

    fn touch(&self) {
        self.0.store(monotonic_millis(), Ordering::Relaxed);
    }

    /// How long the run has been quiet.
    fn idle_for(&self) -> Duration {
        Duration::from_millis(monotonic_millis().saturating_sub(self.0.load(Ordering::Relaxed)))
    }
}

/// The child's stdout, stamping the shared [`ProgressStamp`] with every chunk
/// the response body reads. The body is the only place that sees bytes actually
/// flow, so it is the only honest witness of "still making progress".
#[derive(Debug)]
pub struct ProgressReader {
    inner: ChildStdout,
    stamp: ProgressStamp,
}

impl AsyncRead for ProgressReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.inner).poll_read(cx, buf);
        if matches!(polled, Poll::Ready(Ok(()))) {
            this.stamp.touch();
        }
        polled
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
    pub stdout: ProgressReader,
    pub stderr: ChildStderr,
    pub child: Child,
    pub permit: SemaphorePermit<'static>,
    /// The watchdog's view of this run's progress; hand it to [`supervise`].
    pub progress: ProgressStamp,
}

/// Start one stream run. `video_codec` is the source's first video track as the
/// capability record resolved it (see [`build_args`]).
pub async fn start_stream(
    mode: StreamMode,
    input: &Path,
    start_secs: f64,
    video_codec: &str,
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
        .args(build_args(mode, input, start_secs, video_codec))
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

    let progress = ProgressStamp::new();
    Ok(StreamHandle {
        mode,
        stdout: ProgressReader {
            inner: stdout,
            stamp: progress.clone(),
        },
        stderr,
        child,
        permit,
        progress,
    })
}

/// Join the stderr drain, but never let it delay the outcome: the drained text
/// only feeds the error message (or the cached `ffmpeg exited with status …`
/// tail), so a pipe that outlives the child is not a reason to keep waiting.
async fn drain_stderr(task: JoinHandle<String>) -> Option<String> {
    match tokio::time::timeout(STDERR_DRAIN_TIMEOUT, task).await {
        Ok(Ok(text)) => Some(text),
        _ => None,
    }
}

/// Watch one ffmpeg child to completion.
///
/// The watchdog is progress-based, not deadline-based: `progress` is stamped by
/// the response body as it reads the child's stdout, and a run that keeps
/// producing output is never killed — a healthy conversion of a long source
/// legitimately runs far past the configured timeout, and ending it there
/// truncated every such video mid-playback (the client reports "the delivered
/// stream ended early", and re-opening re-converts from scratch every time). A
/// run that produces no output for `TURBO_PIX_TRANSCODE_TIMEOUT_SECS` — a stuck
/// encoder, a source ffmpeg cannot get past, or a client that stopped reading so
/// the pipe backs up — is killed, which is what keeps it from pinning a
/// conversion slot forever. Dropping the response body closes stdout, which
/// makes ffmpeg exit on EPIPE: that is what bounds client disconnects.
pub async fn supervise(
    mut child: Child,
    stderr: ChildStderr,
    progress: ProgressStamp,
) -> Result<(), String> {
    use tokio::io::AsyncReadExt;

    let stderr_task = tokio::spawn(async move {
        let mut reader = stderr;
        let mut buf = String::new();
        let _ = reader.read_to_string(&mut buf).await;
        buf
    });

    let stall = Duration::from_secs(transcode_timeout_secs());
    let status = loop {
        // Re-arming the wait every poll is safe: `Child::wait` keeps the
        // process's exit future inside the handle, so a cancelled wait resumes
        // where it left off instead of losing the child's exit status. The
        // temporary is dropped at the end of this statement, which is what lets
        // the arms below kill the child.
        let polled = tokio::time::timeout(STALL_POLL_INTERVAL, child.wait()).await;
        match polled {
            Ok(Ok(status)) => break status,
            Ok(Err(e)) => {
                let _ = child.kill().await;
                let _ = drain_stderr(stderr_task).await;
                return Err(format!("ffmpeg wait failed: {e}"));
            }
            Err(_) => {
                let idle = progress.idle_for();
                if idle >= stall {
                    let _ = child.kill().await;
                    let _ = drain_stderr(stderr_task).await;
                    return Err(format!(
                        "stream stalled: no output for {}s (timeout {}s)",
                        idle.as_secs(),
                        stall.as_secs()
                    ));
                }
            }
        }
    };
    let stderr_text = drain_stderr(stderr_task).await.unwrap_or_default();
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
    async fn read_head(stdout: &mut ProgressReader, needle: &[u8]) -> Vec<u8> {
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

    /// Read a whole stream body like the client does (touching the progress
    /// stamp on the way), for fixtures small enough to hold in memory.
    async fn drain_stream(stdout: &mut ProgressReader) -> Vec<u8> {
        use tokio::io::AsyncReadExt;

        let mut bytes = Vec::new();
        stdout
            .read_to_end(&mut bytes)
            .await
            .expect("read stream body");
        bytes
    }

    #[test]
    fn transcode_args_reencode_into_fragmented_mp4() {
        let args = build_args(StreamMode::Transcode, Path::new("/in.mp4"), 0.0, "hevc");
        let joined = args.join(" ");
        assert!(joined.contains("-c:v libx264"));
        assert!(joined.contains("-preset veryfast"));
        assert!(joined.contains("-g 48"));
        assert!(joined.contains("-c:a aac"));
        assert!(joined.contains("-map 0:v:0 -map 0:a:0?"));
        assert!(joined.contains("-frag_duration 1000000"));
        assert!(joined.ends_with("-f mp4 pipe:1"));
        assert!(!joined.contains("-ss"), "start 0 must not emit a seek flag");
        // The output is re-encoded H.264: an `hvc1` tag on it is rejected by the
        // muxer, so a re-encode must never carry the copy-mode tag.
        assert!(!joined.contains("-tag:v"), "{joined}");
    }

    #[test]
    fn audio_mode_copies_video_and_reencodes_audio() {
        let joined = build_args(StreamMode::Audio, Path::new("/in.mp4"), 12.5, "h264").join(" ");
        assert!(joined.contains("-ss 12.500 -i /in.mp4"));
        assert!(joined.contains("-c:v copy"));
        assert!(joined.contains("-c:a aac"));
        assert!(!joined.contains("libx264"));
    }

    #[test]
    fn remux_mode_copies_everything() {
        let joined = build_args(StreamMode::Remux, Path::new("/in.mkv"), 3.0, "h264").join(" ");
        assert!(joined.contains("-c copy"));
        assert!(!joined.contains("libx264"));
    }

    #[test]
    fn copied_hevc_tracks_are_tagged_hvc1_and_other_codecs_are_left_alone() {
        // The muxer's default tag for a copied HEVC track is `hev1`, but the
        // client was promised `hvc1.*` in the MIME: the init segment has to
        // carry the tag the SourceBuffer was created for.
        for mode in [StreamMode::Remux, StreamMode::Audio] {
            let args = build_args(mode, Path::new("/in.mkv"), 0.0, "hevc");
            assert!(
                args.iter().any(|arg| arg == "-tag:v") && args.iter().any(|arg| arg == "hvc1"),
                "{mode:?} must tag a copied HEVC track hvc1: {}",
                args.join(" ")
            );
        }
        for codec in ["h264", "av1", "vp9", "vp8", ""] {
            let args = build_args(StreamMode::Remux, Path::new("/in.mkv"), 0.0, codec);
            assert!(
                !args.iter().any(|arg| arg == "-tag:v"),
                "a {codec} copy must not carry hvc1: {}",
                args.join(" ")
            );
        }
    }

    #[test]
    fn copy_modes_delay_the_moov_so_ac3_audio_can_be_muxed() {
        // ffmpeg's MP4 muxer refuses to write a fragmented header that copies
        // AC-3/E-AC-3 audio ("Cannot write moov atom before AC3 packets. Set the
        // delay_moov flag to fix this.") and exits before the init segment
        // exists; with `delay_moov` it writes the moov once the first packets
        // have been parsed, still ahead of every fragment.
        for mode in [StreamMode::Remux, StreamMode::Audio, StreamMode::Transcode] {
            let joined = build_args(mode, Path::new("/in.mp4"), 0.0, "h264").join(" ");
            assert!(
                joined.contains(
                    "-movflags frag_keyframe+empty_moov+default_base_moof+omit_tfhd_offset+delay_moov"
                ),
                "{mode:?} must request delay_moov: {joined}"
            );
        }
    }

    /// Top-level box header walk over the bytes ffmpeg managed to write: the
    /// size of the first box of `kind`, or 0 when it is absent or malformed.
    /// A refused header (`Cannot write moov atom before AC3 packets`) leaves the
    /// `moov` fourcc behind with a zeroed size, so an assertion on the fourcc
    /// alone would pass for a run that produced no usable init segment.
    fn box_size(head: &[u8], kind: &[u8; 4]) -> usize {
        let mut offset = 0;
        while offset + 8 <= head.len() {
            let size = u32::from_be_bytes(head[offset..offset + 4].try_into().unwrap()) as usize;
            if &head[offset + 4..offset + 8] == kind {
                return size;
            }
            if size < 8 {
                break;
            }
            offset += size;
        }
        0
    }

    #[tokio::test]
    async fn remux_of_an_ac3_source_writes_a_complete_init_segment() {
        let _env_lock = crate::video_processor::tests::acquire_test_env_lock();
        let fixture = Path::new("test-data/test_video_ac3.mp4");
        if !fixture.exists() || !crate::video_processor::ffmpeg_available() {
            eprintln!("skipping: fixture or ffmpeg unavailable");
            return;
        }

        let mut handle = start_stream(StreamMode::Remux, fixture, 0.0, "h264")
            .await
            .expect("remux stream must start");
        // Read the whole body like the client does: the run must reach a clean
        // exit, not just write a header and die (which is what the muxer's
        // refusal produced — exit 234 and a zeroed `moov`).
        let bytes = drain_stream(&mut handle.stdout).await;
        assert!(
            box_size(&bytes, b"ftyp") >= 8,
            "the stream must start with a complete ftyp box"
        );
        assert!(
            box_size(&bytes, b"moov") >= 8,
            "an AC-3 remux must still emit a real init segment, got {:?}",
            String::from_utf8_lossy(&bytes[..bytes.len().min(64)])
        );
        assert!(
            supervise(handle.child, handle.stderr, handle.progress)
                .await
                .is_ok(),
            "the AC-3 remux must run to completion"
        );
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

        let mut handle = start_stream(StreamMode::Transcode, fixture, 0.0, "hevc")
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
        let _ = supervise(handle.child, handle.stderr, handle.progress).await;
    }

    #[tokio::test]
    async fn remux_of_a_hev1_source_carries_the_hvc1_sample_entry() {
        let _env_lock = crate::video_processor::tests::acquire_test_env_lock();
        let fixture = Path::new("test-data/test_video_hevc.mp4");
        if !fixture.exists() || !crate::video_processor::ffmpeg_available() {
            eprintln!("skipping: fixture or ffmpeg unavailable");
            return;
        }

        // Matroska carries no sample-entry tags, so copying the fixture's HEVC
        // into an MP4 tags the track `hev1` — the exact shape a library source
        // with an untagged HEVC track has. Without `-tag:v hvc1` the remux
        // reproduces that `hev1`, contradicting the `hvc1.*` MIME the client was
        // handed when it created its SourceBuffer.
        let temp_dir = tempfile::TempDir::new().unwrap();
        let source = temp_dir.path().join("hev1_source.mkv");
        let status = std::process::Command::new(crate::video_processor::get_ffmpeg_path())
            .args(["-v", "error", "-y", "-i"])
            .arg(fixture)
            .args(["-map", "0:v:0", "-c", "copy", "-f", "matroska"])
            .arg(&source)
            .status()
            .expect("ffmpeg must run for the test source");
        assert!(status.success(), "building the test source must succeed");
        // Sanity: the muxer's default for this source really is `hev1`.
        let tagged = temp_dir.path().join("default_tag.mp4");
        let status = std::process::Command::new(crate::video_processor::get_ffmpeg_path())
            .args(["-v", "error", "-y", "-i"])
            .arg(&source)
            .args(["-map", "0:v:0", "-c", "copy", "-f", "mp4"])
            .arg(&tagged)
            .status()
            .expect("ffmpeg must run for the default tag");
        assert!(status.success());
        let probe = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_tag_string",
                "-of",
                "csv=p=0",
            ])
            .arg(&tagged)
            .output()
            .expect("ffprobe must run");
        assert!(
            String::from_utf8_lossy(&probe.stdout).contains("hev1"),
            "fixture source must default to hev1, got {}",
            String::from_utf8_lossy(&probe.stdout)
        );

        let mut handle = start_stream(StreamMode::Remux, &source, 0.0, "hevc")
            .await
            .expect("remux stream must start");
        let head = read_head(&mut handle.stdout, b"moov").await;
        assert!(
            head.windows(4).any(|w| w == b"hvc1"),
            "a copied HEVC track must carry the advertised hvc1 sample entry"
        );
        assert!(
            !head.windows(4).any(|w| w == b"hev1"),
            "the hev1 default must not survive into the init segment"
        );
        drop(handle.stdout);
        let _ = supervise(handle.child, handle.stderr, handle.progress).await;
    }

    #[tokio::test]
    async fn boundary_seek_on_a_sub_second_fixture_still_produces_bytes() {
        let _env_lock = crate::video_processor::tests::acquire_test_env_lock();
        let fixture = Path::new("test-data/test_video.mp4"); // 0.3 s
        if !fixture.exists() || !crate::video_processor::ffmpeg_available() {
            return;
        }

        let mut handle = start_stream(StreamMode::Transcode, fixture, 0.2, "h264")
            .await
            .expect("a seek near the end must still start");
        let head = read_head(&mut handle.stdout, b"ftyp").await;
        assert!(
            !head.is_empty(),
            "boundary seeks must produce a valid stream head"
        );
        drop(handle.stdout);
        let _ = supervise(handle.child, handle.stderr, handle.progress).await;
    }

    #[tokio::test]
    async fn remux_stream_from_matroska_produces_mp4_fragments() {
        let _env_lock = crate::video_processor::tests::acquire_test_env_lock();
        let fixture = Path::new("test-data/test_video_long.mkv");
        if !fixture.exists() || !crate::video_processor::ffmpeg_available() {
            return;
        }

        let mut handle = start_stream(StreamMode::Remux, fixture, 5.0, "h264")
            .await
            .expect("remux stream must start");
        let head = read_head(&mut handle.stdout, b"moov").await;
        assert!(head.windows(4).any(|w| w == b"ftyp"));
        assert!(head.windows(4).any(|w| w == b"moov"));
        // An H.264 copy keeps its own sample entry: the HEVC tag must not leak
        // into every copy.
        assert!(head.windows(4).any(|w| w == b"avc1"));
        assert!(!head.windows(4).any(|w| w == b"hvc1"));
        drop(handle.stdout);
        let _ = supervise(handle.child, handle.stderr, handle.progress).await;
    }

    /// A stand-in stream process: the watchdog only cares about bytes on stdout
    /// and the exit status, so `sh` is a faithful (and instant) fake.
    #[cfg(unix)]
    fn fake_stream(body: &str) -> Child {
        Command::new("sh")
            .arg("-c")
            .arg(body)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("fake stream process must spawn")
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_stream_that_keeps_producing_output_survives_the_timeout() {
        let _timeout = crate::video_processor::tests::TestEnvGuard::set(
            "TURBO_PIX_TRANSCODE_TIMEOUT_SECS",
            "1",
        );
        // ~3 s of output at 4 chunks/s: three times the configured cap, which a
        // wall-clock deadline would kill mid-stream.
        let mut child = fake_stream(
            "i=0; while [ $i -lt 12 ]; do printf 'chunk'; sleep 0.25; i=$((i+1)); done",
        );
        let stderr = child.stderr.take().unwrap();
        let progress = ProgressStamp::new();
        let mut reader = ProgressReader {
            inner: child.stdout.take().unwrap(),
            stamp: progress.clone(),
        };
        // The body is what stamps progress, exactly like the real response.
        let drain = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut sink = Vec::new();
            reader.read_to_end(&mut sink).await.expect("drain stdout");
            sink.len()
        });

        let started = Instant::now();
        let outcome = supervise(child, stderr, progress).await;
        let elapsed = started.elapsed();
        let drained = drain.await.unwrap();

        assert!(
            outcome.is_ok(),
            "a run that keeps producing output must never be killed: {outcome:?} after {elapsed:?}"
        );
        assert!(
            drained > 0 && elapsed > Duration::from_secs(1),
            "the run must really have outlived the 1s cap (drained {drained} bytes in {elapsed:?})"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_stream_that_goes_silent_past_the_timeout_is_killed() {
        let _timeout = crate::video_processor::tests::TestEnvGuard::set(
            "TURBO_PIX_TRANSCODE_TIMEOUT_SECS",
            "1",
        );
        // Nothing on stdout and nobody reading it: a stuck encoder (or a client
        // that stopped reading) must still be bounded by the configured timeout.
        // `exec` so the sleep IS the child: through a shell that forks, the
        // orphan would keep the stderr pipe open and the kill would only be
        // observably complete once it exited on its own.
        let mut child = fake_stream("exec sleep 30");
        let _stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let progress = ProgressStamp::new();

        let started = Instant::now();
        let outcome = supervise(child, stderr, progress).await;
        let elapsed = started.elapsed();

        assert!(
            outcome.is_err(),
            "a silent run must be killed, not waited out"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "the kill must land near the 1s cap, took {elapsed:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_killed_run_returns_even_when_its_pipe_outlives_it() {
        let _timeout = crate::video_processor::tests::TestEnvGuard::set(
            "TURBO_PIX_TRANSCODE_TIMEOUT_SECS",
            "1",
        );
        // The shell stays alive (`wait`) while a forked descendant holds the
        // stderr write end: killing the child closes nothing, so a run whose
        // answer waited for the pipe's EOF would report the stall only once the
        // orphan exited on its own — the watchdog has to answer anyway.
        let mut child = fake_stream("sleep 30 & wait");
        let _stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let progress = ProgressStamp::new();

        let started = Instant::now();
        let outcome = supervise(child, stderr, progress).await;
        let elapsed = started.elapsed();

        assert!(outcome.is_err(), "a silent run must be killed");
        assert!(
            elapsed < Duration::from_secs(10),
            "an outliving pipe must not delay the kill's answer, took {elapsed:?}"
        );
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
            let err = start_stream(
                StreamMode::Transcode,
                Path::new("/nonexistent.mp4"),
                0.0,
                "h264",
            )
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
