//! Serve-time derivation and persistence of a video's playback capability
//! record.
//!
//! Legacy rows (indexed before the capability extractor existed) carry no
//! container / bit-depth / moov facts. Deciding from those records routes every
//! video into conversion — the dominant defect the spec targets. This module
//! probes the file once, persists what it learns into `photos.metadata.video`,
//! and returns the resolved facts.

use std::path::Path;

use serde_json::{json, Value};
use sqlx::SqlitePool;

use crate::db::Photo;
use crate::metadata_extractor::container_from_format_name;
use crate::video_capability::{parse_pix_fmt_bit_depth, ContainerFamily};
use crate::video_processor::{get_ffprobe_path, has_moov_at_start};

/// Bumped whenever the set of persisted capability facts changes; its presence
/// is the "record complete, never probe again" marker.
pub const CAPABILITY_VERSION: u64 = 1;

#[derive(Debug, Default, PartialEq)]
pub struct CapabilityPatch {
    /// False for audio-only / cover-art-only containers: the probe succeeded but
    /// found no non-attached video stream.
    pub has_video_stream: bool,
    pub codec: Option<String>,
    pub container: Option<String>,
    pub bit_depth: Option<u32>,
    pub audio_codec: Option<String>,
    pub duration_secs: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCapabilities {
    pub codec: String,
    pub container: Option<String>,
    pub family: ContainerFamily,
    pub bit_depth: Option<u32>,
    pub audio_codec: Option<String>,
    pub moov_at_start: bool,
    pub duration_secs: Option<f64>,
    /// True when this call had to probe the file (incomplete record).
    pub probed: bool,
}

/// Complete = a previous probe/scan wrote the version marker plus either a codec
/// or the explicit "probed, no video stream" marker.
/// An absent `moov_at_start` key is "never probed", NOT "true".
pub fn record_is_complete(photo: &Photo) -> bool {
    let Some(video) = photo.metadata.get("video") else {
        return false;
    };
    if video.get("capability_version").and_then(Value::as_u64) != Some(CAPABILITY_VERSION) {
        return false;
    }
    video
        .get("codec")
        .and_then(Value::as_str)
        .is_some_and(|c| !c.is_empty())
        || video.get("no_video_stream").and_then(Value::as_bool) == Some(true)
}

/// First stream of `kind` that is not an attached cover picture.
fn first_stream<'a>(parsed: &'a Value, kind: &str) -> Option<&'a Value> {
    parsed["streams"].as_array()?.iter().find(|s| {
        s["codec_type"].as_str() == Some(kind)
            && s["disposition"]["attached_pic"].as_i64() != Some(1)
    })
}

/// First non-attached video stream and first non-attached audio stream — the
/// only streams the playback decision may consider (spec: first/default audio
/// track only).
pub(crate) fn parse_capabilities_from_ffprobe(parsed: &Value) -> CapabilityPatch {
    let stream_field = |kind: &str, field: &str| {
        first_stream(parsed, kind)
            .and_then(|s| s[field].as_str())
            .map(str::to_string)
    };

    CapabilityPatch {
        has_video_stream: first_stream(parsed, "video").is_some(),
        codec: stream_field("video", "codec_name"),
        container: container_from_format_name(parsed),
        bit_depth: parse_pix_fmt_bit_depth(stream_field("video", "pix_fmt").as_deref()),
        audio_codec: stream_field("audio", "codec_name"),
        duration_secs: parsed["format"]["duration"]
            .as_str()
            .and_then(|d| d.parse::<f64>().ok())
            .filter(|d| d.is_finite() && *d > 0.0),
    }
}

/// Blocking probe: one ffprobe pass plus (MP4-family only) the moov layout
/// probe. `None` when ffprobe cannot run or emits no JSON.
fn probe_file(path: &Path) -> Option<(CapabilityPatch, bool)> {
    let output = std::process::Command::new(get_ffprobe_path())
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let parsed: Value = serde_json::from_slice(&output.stdout).ok()?;
    let patch = parse_capabilities_from_ffprobe(&parsed);
    let family = ContainerFamily::from_record(
        patch.container.as_deref(),
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default(),
    );
    let moov_at_start = if family.has_moov_layout() {
        has_moov_at_start(path).unwrap_or(true)
    } else {
        true
    };
    Some((patch, moov_at_start))
}

impl ResolvedCapabilities {
    pub fn from_record(photo: &Photo) -> Self {
        Self {
            codec: photo.video_codec().unwrap_or_default().to_string(),
            container: photo.container().map(str::to_string),
            family: ContainerFamily::from_record(photo.container(), &photo.filename),
            bit_depth: photo.bit_depth(),
            audio_codec: photo.audio_codec().map(str::to_string),
            moov_at_start: photo.moov_at_start(),
            duration_secs: photo.duration.filter(|d| *d > 0.0),
            probed: false,
        }
    }

    fn merged(photo: &Photo, patch: &CapabilityPatch, moov_at_start: bool) -> Self {
        let container = patch
            .container
            .clone()
            .or_else(|| photo.container().map(str::to_string));
        Self {
            codec: patch
                .codec
                .clone()
                .unwrap_or_else(|| photo.video_codec().unwrap_or_default().to_string()),
            family: ContainerFamily::from_record(container.as_deref(), &photo.filename),
            container,
            bit_depth: patch.bit_depth.or_else(|| photo.bit_depth()),
            audio_codec: patch
                .audio_codec
                .clone()
                .or_else(|| photo.audio_codec().map(str::to_string)),
            moov_at_start,
            duration_secs: patch
                .duration_secs
                .or_else(|| photo.duration.filter(|d| *d > 0.0)),
            probed: true,
        }
    }
}

/// Resolve the capability facts for one request. Complete records are used
/// as-is (no filesystem access); incomplete records are probed once and the
/// result is persisted, so later decisions are cheap.
pub async fn resolve(pool: &SqlitePool, photo: &Photo) -> ResolvedCapabilities {
    if record_is_complete(photo) {
        return ResolvedCapabilities::from_record(photo);
    }

    let path = Path::new(&photo.file_path).to_path_buf();
    let probed = tokio::task::spawn_blocking(move || probe_file(&path))
        .await
        .ok()
        .flatten();

    let Some((patch, moov_at_start)) = probed else {
        // A FAILED probe (non-zero ffprobe exit / unparseable JSON) deliberately
        // persists nothing and leaves the record incomplete: a file that is
        // unreadable now may become readable later, and poisoning the record
        // with a "no video stream" marker would be worse than re-probing.
        log::warn!(
            "Capability probe failed; deciding from the stored record only: {}",
            photo.filename
        );
        return ResolvedCapabilities::from_record(photo);
    };

    let mut video = serde_json::Map::new();
    video.insert("capability_version".to_string(), json!(CAPABILITY_VERSION));
    if !patch.has_video_stream {
        // The probe succeeded and found no non-attached video stream (audio-only
        // or cover-art-only container). Persist that as a complete fact so the
        // record is not re-probed on every request.
        video.insert("no_video_stream".to_string(), json!(true));
    }
    if let Some(codec) = &patch.codec {
        video.insert("codec".to_string(), json!(codec));
    }
    if let Some(container) = &patch.container {
        video.insert("container".to_string(), json!(container));
    }
    if let Some(bit_depth) = patch.bit_depth {
        video.insert("bit_depth".to_string(), json!(bit_depth));
    }
    // Explicit null documents "asked the file, there is no audio track" — which
    // is different from "never probed".
    video.insert("audio_codec".to_string(), json!(patch.audio_codec));
    video.insert("moov_at_start".to_string(), json!(moov_at_start));

    if let Err(e) =
        Photo::persist_metadata_patch(pool, &photo.hash_sha256, &json!({ "video": video })).await
    {
        log::warn!("Persisting derived video capabilities failed: {e}");
    }
    if let Some(duration) = patch.duration_secs {
        if let Err(e) = Photo::persist_duration_if_missing(pool, &photo.hash_sha256, duration).await
        {
            log::warn!("Persisting derived video duration failed: {e}");
        }
    }

    ResolvedCapabilities::merged(photo, &patch, moov_at_start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ffprobe_json() -> Value {
        json!({
            "format": { "format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "12.5" },
            "streams": [
                { "codec_type": "video", "codec_name": "h264", "pix_fmt": "yuv420p",
                  "disposition": { "attached_pic": 0 } },
                { "codec_type": "audio", "codec_name": "ac3",
                  "disposition": { "attached_pic": 0 } }
            ]
        })
    }

    #[test]
    fn parses_container_codec_bitdepth_audio_and_duration() {
        let patch = parse_capabilities_from_ffprobe(&ffprobe_json());
        assert_eq!(patch.codec.as_deref(), Some("h264"));
        assert_eq!(patch.container.as_deref(), Some("mov"));
        assert_eq!(patch.bit_depth, Some(8));
        assert_eq!(patch.audio_codec.as_deref(), Some("ac3"));
        assert_eq!(patch.duration_secs, Some(12.5));
    }

    #[test]
    fn skips_attached_cover_pictures_and_higher_bit_depths() {
        let parsed = json!({
            "format": { "format_name": "matroska,webm" },
            "streams": [
                { "codec_type": "video", "codec_name": "mjpeg",
                  "disposition": { "attached_pic": 1 } },
                { "codec_type": "video", "codec_name": "h264", "pix_fmt": "yuv420p10le",
                  "disposition": { "attached_pic": 0 } }
            ]
        });
        let patch = parse_capabilities_from_ffprobe(&parsed);
        assert_eq!(patch.codec.as_deref(), Some("h264"));
        assert_eq!(patch.bit_depth, Some(10));
        assert_eq!(patch.audio_codec, None);
        assert_eq!(patch.duration_secs, None);
    }

    #[test]
    fn container_family_discriminates_mkv_from_webm() {
        assert_eq!(
            ContainerFamily::from_record(Some("matroska"), "a.mkv"),
            ContainerFamily::Matroska
        );
        assert_eq!(
            ContainerFamily::from_record(Some("matroska"), "a.webm"),
            ContainerFamily::Webm
        );
        assert_eq!(
            ContainerFamily::from_record(Some("mov"), "a.mov"),
            ContainerFamily::Mp4
        );
        assert_eq!(
            ContainerFamily::from_record(None, "a.mp4"),
            ContainerFamily::Mp4
        );
        assert_eq!(
            ContainerFamily::from_record(None, "a.avi"),
            ContainerFamily::Avi
        );
        assert_eq!(
            ContainerFamily::from_record(Some("weird"), "a.bin"),
            ContainerFamily::Other
        );
    }

    /// Minimal `Photo` row for resolver tests; mirrors the literal used by the
    /// `handlers_video` test module (`src/handlers_video.rs:800-836`).
    fn test_photo(hash: &str) -> Photo {
        Photo {
            hash_sha256: hash.to_string(),
            file_path: String::new(),
            filename: "video.mp4".to_string(),
            file_size: 0,
            mime_type: Some("video/mp4".to_string()),
            taken_at: None,
            width: None,
            height: None,
            orientation: None,
            duration: None,
            thumbnail_path: None,
            has_thumbnail: Some(false),
            blurhash: None,
            is_favorite: Some(false),
            semantic_vector_indexed: Some(false),
            metadata: json!({}),
            date_modified: chrono::Utc::now(),
            date_indexed: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn resolve_derives_and_persists_legacy_records_once() {
        // Hold the shared test env lock: this test shells out to real ffprobe
        // via `probe_file`/`has_moov_at_start`, while other test modules point
        // `FFPROBE_PATH` at fake scripts while holding the same lock.
        let _env_lock = crate::video_processor::tests::acquire_test_env_lock();
        let fixture = Path::new("test-data/test_video.mp4");
        if !fixture.exists() || !crate::video_processor::ffmpeg_available() {
            eprintln!("skipping: fixture or ffmpeg unavailable");
            return;
        }
        let pool = crate::db::create_in_memory_pool().await.expect("pool");
        // `photos.hash_sha256` carries a `length(...) = 64` CHECK constraint, so
        // the brief's short literal is padded to a valid hash.
        let hash = format!("{:0<64}", "hash-resolve");
        let mut photo = test_photo(&hash);
        photo.file_path = fixture.to_string_lossy().into_owned();
        photo.filename = "test_video.mp4".to_string();
        photo.metadata = json!({});
        photo.create(&pool).await.expect("create");

        let first = resolve(&pool, &photo).await;
        assert!(first.probed, "an incomplete record must probe the file");
        assert_eq!(first.codec, "h264");
        assert_eq!(first.container.as_deref(), Some("mov"));
        assert_eq!(first.bit_depth, Some(8));
        assert!(first.moov_at_start);
        assert!(first.duration_secs.is_some_and(|d| d > 0.0));

        let stored = Photo::find_by_hash(&pool, &hash).await.unwrap().unwrap();
        assert_eq!(stored.metadata["video"]["capability_version"], 1);
        assert!(stored.duration.is_some_and(|d| d > 0.0));

        let second = resolve(&pool, &stored).await;
        assert!(!second.probed, "a complete record must not probe again");
        assert_eq!(second.codec, "h264");
    }

    #[test]
    fn record_is_complete_needs_version_plus_codec_or_no_video_marker() {
        let mut photo = test_photo("hash");
        photo.metadata = json!({ "video": { "capability_version": 1, "codec": "h264" } });
        assert!(record_is_complete(&photo));

        photo.metadata = json!({ "video": { "capability_version": 1, "no_video_stream": true } });
        assert!(
            record_is_complete(&photo),
            "a probed file with no video stream is a complete fact"
        );

        // A bare version marker with neither fact stays incomplete.
        photo.metadata = json!({ "video": { "capability_version": 1 } });
        assert!(!record_is_complete(&photo));

        // The marker without the version is not a probe result.
        photo.metadata = json!({ "video": { "no_video_stream": true } });
        assert!(!record_is_complete(&photo));
    }

    /// Fake ffprobe that records one line per invocation in `counter` and prints
    /// `json` on stdout, so a test can count probe passes without real ffmpeg.
    #[cfg(unix)]
    fn fake_ffprobe(dir: &Path, counter: &Path, json: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let script = dir.join("ffprobe.sh");
        std::fs::write(
            &script,
            format!(
                "#!/usr/bin/env sh\nprintf 'x\\n' >> '{counter}'\nprintf '%s' '{json}'\n",
                counter = counter.display(),
            ),
        )
        .expect("write fake ffprobe");
        let mut perms = std::fs::metadata(&script)
            .expect("stat fake ffprobe")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).expect("chmod fake ffprobe");
        script
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_without_video_stream_persists_marker_and_is_complete_once() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let counter = temp_dir.path().join("probe_count");
        // Audio-only matroska: no non-attached video stream, and no moov pass.
        let script = fake_ffprobe(
            temp_dir.path(),
            &counter,
            r#"{"format":{"format_name":"matroska,webm","duration":"3.0"},"streams":[{"codec_type":"audio","codec_name":"opus","disposition":{"attached_pic":0}}]}"#,
        );
        // `TestEnvGuard` holds the shared env lock while FFPROBE_PATH points at
        // the fake, mirroring the other env-dependent test modules.
        let _guard = crate::video_processor::tests::TestEnvGuard::set(
            "FFPROBE_PATH",
            script.to_str().unwrap(),
        );

        let pool = crate::db::create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-no-video");
        let mut photo = test_photo(&hash);
        photo.file_path = temp_dir
            .path()
            .join("audio-only.mkv")
            .to_string_lossy()
            .into_owned();
        photo.filename = "audio-only.mkv".to_string();
        photo.create(&pool).await.expect("create");

        let first = resolve(&pool, &photo).await;
        assert!(first.probed, "an incomplete record must probe the file");
        assert_eq!(first.codec, "");

        let stored = Photo::find_by_hash(&pool, &hash).await.unwrap().unwrap();
        assert_eq!(stored.metadata["video"]["capability_version"], 1);
        assert_eq!(stored.metadata["video"]["no_video_stream"], true);
        assert_eq!(stored.metadata["video"]["audio_codec"], "opus");
        assert!(
            record_is_complete(&stored),
            "the no-video-stream marker must complete the record"
        );

        let probes = || std::fs::read_to_string(&counter).unwrap().lines().count();
        assert_eq!(
            probes(),
            1,
            "matroska has no moov pass: exactly one ffprobe call"
        );

        let second = resolve(&pool, &stored).await;
        assert!(!second.probed, "a complete record must not probe again");
        assert_eq!(
            probes(),
            1,
            "the second resolve must not shell out to ffprobe"
        );
    }

    #[tokio::test]
    async fn failed_probe_persists_nothing_and_stays_incomplete() {
        let _guard = crate::video_processor::tests::TestEnvGuard::set(
            "FFPROBE_PATH",
            "/nonexistent/ffprobe",
        );

        let pool = crate::db::create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-failed-probe");
        let mut photo = test_photo(&hash);
        photo.file_path = "/nonexistent/video.mp4".to_string();
        photo.create(&pool).await.expect("create");

        let resolved = resolve(&pool, &photo).await;
        assert!(!resolved.probed);

        let stored = Photo::find_by_hash(&pool, &hash).await.unwrap().unwrap();
        assert!(
            stored.metadata.get("video").is_none(),
            "a failed probe must persist nothing"
        );
        assert!(
            !record_is_complete(&stored),
            "a failed probe must stay incomplete so it is retried later"
        );
    }
}
