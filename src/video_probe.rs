//! Serve-time derivation and persistence of a video's playback capability
//! record.
//!
//! Legacy rows (indexed before the capability extractor existed) carry no
//! container / bit-depth / moov facts. Deciding from those records routes every
//! video into conversion — the dominant defect the spec targets. This module
//! probes the file once, persists what it learns into `photos.metadata.video`,
//! and returns the resolved facts.

use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use serde_json::{json, Value};
use sqlx::SqlitePool;
use tokio::sync::Semaphore;

use crate::db::Photo;
use crate::metadata_extractor::container_from_format_name;
use crate::video_capability::{parse_pix_fmt_bit_depth, ContainerFamily};
use crate::video_processor::{get_ffprobe_path, has_moov_at_start_within, run_bounded};

/// Bumped whenever the set of persisted capability facts changes; its presence
/// is the "record complete, never probe again" marker.
pub const CAPABILITY_VERSION: u64 = 1;

/// How many serve-time capability probes may run at once. Four is the bound the
/// remux path has always used (`video_processor`'s remux semaphore), and the
/// probe is the same class of work: a short-lived blocking child process that
/// must not be spawned per request without limit.
const PROBE_BOUND: usize = 4;

/// Bounds concurrent capability probes. Each probe spawns one blocking ffprobe
/// plus, for MP4-family sources, the `-v trace` moov pass whose entire stderr
/// `Command::output()` buffers in memory. The dominant case is a legacy library
/// (records indexed before the capability extractor existed) whose first visit
/// asks for every video at once: unbounded, that fans one ffprobe pair per
/// request into tokio's blocking pool — 512 threads by default — where the
/// bound admits [`PROBE_BOUND`]. Distinct from the remux and transcode
/// semaphores for the same reason those two are distinct from each other: a
/// probe never waits behind a slow re-encode.
static PROBE_SEMAPHORE: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(PROBE_BOUND));

/// How long ONE ffprobe pass of a capability probe may run before the child is
/// killed. ffprobe answers from local storage in well under a second, so this
/// only ever fires for a probe that would not finish at all: a corrupt or
/// truncated file, a path on a stale network mount, a FIFO. A probe that never
/// returns is the one failure this module cannot degrade from — its permit is
/// held for the whole probe, and once all [`PROBE_BOUND`] permits are held every
/// later request for an incomplete record waits forever with no response — so
/// both passes of a probe (the facts pass and the MP4-family moov pass) run
/// under this deadline and are killed on expiry.
const PROBE_PASS_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a request waits for one of the [`PROBE_BOUND`] permits before
/// probing anyway, without one.
///
/// The wait is bounded for the same reason the passes are: a wedged pool must
/// not hang every later request. The fallback is "probe unbounded" rather than
/// "answer from the stored record", because this module's contract is that a
/// request which cannot get a permit still SUCCEEDS — degrading here would route
/// a playable file into conversion, the exact defect the probe exists to
/// prevent, and the fan-out it risks is bounded in time by
/// [`PROBE_PASS_TIMEOUT`]. A live pool therefore always hands over a permit
/// inside this wait (every permit is released when its bounded passes end), so
/// it expires only when the pool itself is stuck — permits held by probes that
/// never started, e.g. behind a saturated blocking pool.
const PROBE_PERMIT_WAIT: Duration = Duration::from_secs(30);

#[derive(Debug, Default, PartialEq)]
pub struct CapabilityPatch {
    /// False for audio-only / cover-art-only containers: the probe succeeded but
    /// found no non-attached video stream.
    pub has_video_stream: bool,
    /// False when the probe found no non-attached audio stream. Without this
    /// fact a `None` [`Self::audio_codec`] cannot be told apart from "the first
    /// audio stream reported no `codec_name`", and `merged` would then keep a
    /// stored codec for a file the probe just proved carries no audio at all.
    pub has_audio_stream: bool,
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
        has_audio_stream: first_stream(parsed, "audio").is_some(),
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
/// probe, each killed once `timeout` elapses. `None` when ffprobe cannot run,
/// has to be killed, or emits no JSON — the caller then decides from the stored
/// record, exactly as it does for a non-zero exit. The layout verdict is
/// `None` when the moov pass yielded none: a pass that failed or was killed
/// proves nothing about the file, so the caller keeps the stored fact instead
/// of inventing one.
fn probe_file(path: &Path, timeout: Duration) -> Option<(CapabilityPatch, Option<bool>)> {
    let output = run_bounded(
        std::process::Command::new(get_ffprobe_path())
            .args([
                "-v",
                "error",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
            ])
            .arg(path),
        timeout,
    )?;
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
    // The facts pass already proved the file is readable, so a layout question
    // the moov pass could not answer must not refuse the whole probe — but it
    // must not fabricate a verdict either. That pass reads the entire input
    // (`ffprobe -v trace` buffers a line per packet: hundreds of KB for a 20 s
    // clip, megabytes for minutes), so a multi-GB non-progressive source on
    // cold storage outlives this deadline on its first play, and a transient
    // ffprobe failure lands in the same arm. "At start" is the one answer that
    // can turn an accurate stored `moov_at_start: false` into a permanent lie;
    // `resolve_within` falls back to that stored fact instead.
    let moov_at_start = if family.has_moov_layout() {
        has_moov_at_start_within(path, timeout).ok()
    } else {
        // No moov layout to probe for: "at start" is a fact for these families,
        // not a guess, and `plan` never consults the flag for them.
        Some(true)
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
            // A probe that established there is no non-attached video stream
            // resolves to NO video codec: the stored record may still hold the
            // cover picture's codec (dropped above, but the in-memory `photo`
            // snapshot cannot see that write), and answering with it would
            // contradict the probe's own finding. A probe that DID find a video
            // stream but reported no `codec_name` keeps the fallback — the
            // record then carries no codec key at all, so it stays incomplete
            // and is probed again rather than being trusted.
            codec: if patch.has_video_stream {
                patch
                    .codec
                    .clone()
                    .unwrap_or_else(|| photo.video_codec().unwrap_or_default().to_string())
            } else {
                String::new()
            },
            family: ContainerFamily::from_record(container.as_deref(), &photo.filename),
            container,
            bit_depth: patch.bit_depth.or_else(|| photo.bit_depth()),
            // Same rule as `codec` above, for the same reason: a probe that
            // established there is no non-attached audio stream resolves to NO
            // audio codec, because the stored record may still hold one from an
            // earlier scan of this path (an in-place replacement, before the
            // rescan updates the row) and answering with it would contradict the
            // probe. The stored value is only a fallback for a probe that DID
            // find an audio stream but reported no `codec_name` — there the
            // record carries no codec key, so it stays incomplete and is probed
            // again rather than trusted. A stale `Some(…)` here is not cosmetic:
            // `has_no_mappable_stream` keys on an empty codec AND
            // `audio_codec.is_none()`, so it would claim a conversion slot for a
            // run whose maps select nothing, and `output_mime` would advertise
            // audio the copied init segment does not carry.
            audio_codec: if patch.has_audio_stream {
                patch
                    .audio_codec
                    .clone()
                    .or_else(|| photo.audio_codec().map(str::to_string))
            } else {
                None
            },
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
    resolve_within(pool, photo, PROBE_PASS_TIMEOUT).await
}

/// [`resolve`] with an injectable probe deadline, so tests can pin the timeout
/// behaviour in milliseconds instead of waiting out [`PROBE_PASS_TIMEOUT`].
async fn resolve_within(
    pool: &SqlitePool,
    photo: &Photo,
    probe_timeout: Duration,
) -> ResolvedCapabilities {
    if record_is_complete(photo) {
        return ResolvedCapabilities::from_record(photo);
    }

    // Acquired only for an incomplete record — a complete one must never queue
    // behind other requests' probes — and moved INTO the blocking probe, so it
    // is held for the whole probe (both the facts pass and the moov pass) no
    // matter what happens to this request future, and at most [`PROBE_BOUND`]
    // ffprobe pairs exist at once no matter how many first hits arrive
    // together. The permit belongs to the probe, not to the request: a handler
    // future is dropped mid-probe whenever a client hangs up or the player
    // aborts a superseded fetch (every seek), and releasing the slot there
    // would let the replacement request start a second probe of the same file
    // while the first still runs — `spawn_blocking` work cannot be aborted and
    // the probe's own deadline is what ends its ffprobe children — re-creating
    // the unbounded fan-out this bound exists to prevent. It is therefore
    // released when the blocking probe finishes, not when this function returns.
    //
    // The wait is bounded (see [`PROBE_PERMIT_WAIT`]): a pool whose permits
    // never come back — every one held by a probe that has not even started,
    // say behind a saturated blocking pool — must degrade to running this probe
    // without a permit, not hang the request forever. Acquisition also fails if
    // the semaphore is closed, which nothing does.
    let permit = tokio::time::timeout(PROBE_PERMIT_WAIT, PROBE_SEMAPHORE.acquire())
        .await
        .ok()
        .and_then(Result::ok);
    let path = Path::new(&photo.file_path).to_path_buf();
    let probed = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        probe_file(&path, probe_timeout)
    })
    .await
    .ok()
    .flatten();

    let Some((patch, moov_verdict)) = probed else {
        // A FAILED probe — non-zero ffprobe exit, unparseable JSON, or a pass
        // that had to be killed at its deadline — deliberately persists nothing
        // and leaves the record incomplete: a file that is unreadable (or
        // unreadably slow) now may become readable later, and poisoning the
        // record with a "no video stream" marker would be worse than re-probing.
        log::warn!(
            "Capability probe failed; deciding from the stored record only: {}",
            photo.filename
        );
        return ResolvedCapabilities::from_record(photo);
    };

    // A moov pass that yielded no verdict proved nothing, so the record's own
    // layout is what stands: falling back to "at start" here would overwrite an
    // accurate stored `moov_at_start: false` with `true` AND complete the
    // record, after which `record_is_complete` short-circuits `resolve`, `plan`
    // reads a progressive-looking MP4 and returns `Delivery::Direct`, and
    // `get_video_file`'s `Direct` arm serves the original moov-at-end file
    // without re-checking the layout — the browser downloads the whole file
    // before the first frame instead of taking the remux rung, permanently.
    // Absent means "never wrote a false value" (`Photo::moov_at_start`), which
    // is the same default a record fresh from the scanner carries.
    let moov_at_start = moov_verdict.unwrap_or_else(|| photo.moov_at_start());

    let mut video = serde_json::Map::new();
    video.insert("capability_version".to_string(), json!(CAPABILITY_VERSION));
    if !patch.has_video_stream {
        // The probe succeeded and found no non-attached video stream (audio-only
        // or cover-art-only container). Persist that as a complete fact so the
        // record is not re-probed on every request, and DELETE any stale `codec`
        // with a null member (RFC 7396, exactly like `audio_codec` below): a row
        // indexed before the attached-pic filter existed carries the cover
        // picture's codec (`mjpeg`/`png`) in `metadata.video.codec`, and a patch
        // that omits the key leaves it in place — `merged` would then prefer
        // that stale string over the probe's finding and `record_is_complete`
        // would keep accepting it, making the wrong fact permanent (the
        // no-mappable-stream guard never fires and a client is promised a video
        // codec for a run that maps no video stream).
        video.insert("no_video_stream".to_string(), json!(true));
        video.insert("codec".to_string(), Value::Null);
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
    // A null member is RFC 7396's "remove this key", not a stored null (see
    // `Photo::persist_capability_and_duration`): sending it drops any stale
    // `audio_codec` instead of leaving a value the file no longer has. "Asked
    // the file, there is no audio track" stays readable as a COMPLETE record
    // (the version marker) whose `audio_codec` key is absent — a different fact
    // from "never probed", which is the record without the marker.
    video.insert("audio_codec".to_string(), json!(patch.audio_codec));
    // The probe's verdict when it has one, the record's own fact when the moov
    // pass yielded none: either way the record is completed with a layout it
    // can be trusted for, which is what keeps the failed pass from wedging the
    // probe (the request is answered and later ones need no probe at all).
    video.insert("moov_at_start".to_string(), json!(moov_at_start));

    // One transaction for both writes: a capability record is either fully
    // written or not written at all. Written as two statements, a failed
    // duration write left the `capability_version` marker behind — which
    // `record_is_complete` accepts — so `resolve` would short-circuit on the
    // next request and the missing duration would be permanent.
    if let Err(e) = Photo::persist_capability_and_duration(
        pool,
        &photo.hash_sha256,
        &json!({ "video": video }),
        patch.duration_secs,
    )
    .await
    {
        log::warn!("Persisting derived video capabilities failed: {e}");
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

    /// A row indexed before the extractor skipped attached cover pictures can
    /// carry the cover's codec (`mjpeg`/`png`) as `metadata.video.codec`. A
    /// probe that finds no non-attached video stream must DROP that stale key:
    /// the merge is RFC 7396, so omitting it would leave the wrong fact in
    /// place, `merged` would prefer it over the probe's own finding, and
    /// `record_is_complete` accepts it — the source would keep a video codec it
    /// does not have, for every later request, forever.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stale_cover_codec_is_cleared_when_the_probe_finds_no_video_stream() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        // Audio-only matroska whose only "video" stream is the attached cover
        // picture: the probe sees no non-attached video stream at all.
        let script = fake_ffprobe(
            temp_dir.path(),
            &temp_dir.path().join("probe_count"),
            r#"{"format":{"format_name":"matroska,webm","duration":"3.0"},"streams":[{"codec_type":"video","codec_name":"mjpeg","disposition":{"attached_pic":1}},{"codec_type":"audio","codec_name":"aac","disposition":{"attached_pic":0}}]}"#,
        );
        let _guard = crate::video_processor::tests::TestEnvGuard::set(
            "FFPROBE_PATH",
            script.to_str().unwrap(),
        );

        let pool = crate::db::create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-stale-cover-codec");
        let mut photo = test_photo(&hash);
        photo.file_path = temp_dir
            .path()
            .join("cover-only.mkv")
            .to_string_lossy()
            .into_owned();
        photo.filename = "cover-only.mkv".to_string();
        // The stale fact an older indexer wrote (the cover's codec) and no
        // capability marker, so the record is probed.
        photo.metadata = json!({ "video": { "codec": "mjpeg" } });
        photo.create(&pool).await.expect("create");

        let resolved = resolve(&pool, &photo).await;

        assert!(resolved.probed, "an incomplete record must probe the file");
        assert_eq!(
            resolved.codec, "",
            "the probe established there is no video stream, so the stale stored \
             cover codec must not be preferred over that finding"
        );

        let stored = Photo::find_by_hash(&pool, &hash).await.unwrap().unwrap();
        assert!(
            stored.metadata["video"].get("codec").is_none(),
            "the stale cover codec must be removed, not kept: {}",
            stored.metadata["video"]
        );
        assert_eq!(stored.metadata["video"]["no_video_stream"], true);
        assert!(
            record_is_complete(&stored),
            "the no-video-stream marker must complete the record without a codec"
        );
        // The chain downstream reads the same fact: the record resolves to an
        // empty codec, which is what `handlers_video`'s no-mappable-stream
        // guard and `output_mime` act on.
        assert_eq!(ResolvedCapabilities::from_record(&stored).codec, "");
    }

    /// The mirror case for audio: a row from an earlier scan of the same path
    /// can carry an `audio_codec` for a file that now has no audio stream at
    /// all (an in-place replacement, before the rescan updates the row).
    /// `has_audio_stream` is what tells "the probe found no audio track" apart
    /// from "the first audio stream reported no `codec_name`", so the probe's
    /// finding must win. Keeping the stored codec is not cosmetic: it makes
    /// `handlers_video`'s `has_no_mappable_stream` (empty codec AND no audio
    /// codec) miss, so a doomed conversion claims a slot and its 15-minute
    /// cooldown blocks a later legitimate retry, and `output_mime` advertises
    /// audio the copied init segment does not carry.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stale_audio_codec_is_cleared_when_the_probe_finds_no_audio_stream() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        // Video-only matroska: the probe finds no audio stream at all, and
        // matroska has no moov pass, so this is one ffprobe call.
        let script = fake_ffprobe(
            temp_dir.path(),
            &temp_dir.path().join("probe_count"),
            r#"{"format":{"format_name":"matroska,webm","duration":"3.0"},"streams":[{"codec_type":"video","codec_name":"h264","pix_fmt":"yuv420p","disposition":{"attached_pic":0}}]}"#,
        );
        let _guard = crate::video_processor::tests::TestEnvGuard::set(
            "FFPROBE_PATH",
            script.to_str().unwrap(),
        );

        let pool = crate::db::create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-stale-audio-codec");
        let mut photo = test_photo(&hash);
        photo.file_path = temp_dir
            .path()
            .join("silent.mkv")
            .to_string_lossy()
            .into_owned();
        photo.filename = "silent.mkv".to_string();
        // The stale fact an earlier scan of this path wrote, and no capability
        // marker, so the record is probed.
        photo.metadata = json!({ "video": { "codec": "h264", "audio_codec": "ac3" } });
        photo.create(&pool).await.expect("create");

        let resolved = resolve(&pool, &photo).await;

        assert!(resolved.probed, "an incomplete record must probe the file");
        assert_eq!(
            resolved.audio_codec, None,
            "the probe found no audio stream, so the stored `ac3` must not be \
             preferred over that finding"
        );
        assert_eq!(
            resolved.codec, "h264",
            "the probe did find a video stream, so its codec still resolves"
        );

        let stored = Photo::find_by_hash(&pool, &hash).await.unwrap().unwrap();
        assert!(
            stored.metadata["video"].get("audio_codec").is_none(),
            "the stale audio codec must be removed from the record too: {}",
            stored.metadata["video"]
        );
        assert_eq!(
            ResolvedCapabilities::from_record(&stored).audio_codec,
            None,
            "every later request must read the same finding from the record"
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

    /// Fake ffprobe that parks one `marker.<pid>` file while it runs and holds
    /// the call open for `hold` seconds, so a test can watch how many probes
    /// are in flight at the same time. Prints an audio-only matroska record, so
    /// each probe is a single ffprobe call (no moov pass).
    #[cfg(unix)]
    fn slow_fake_ffprobe(dir: &Path, hold_secs: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let script = dir.join("slow_ffprobe.sh");
        // The marker name must expand `$$` to the child's own pid, so the paths
        // are double-quoted (single quotes would make every probe touch the
        // literal `marker.$$` and hide the concurrency this test measures).
        std::fs::write(
            &script,
            format!(
                "#!/usr/bin/env sh\n\
                 touch \"{dir}/marker.$$\"\n\
                 sleep {hold_secs}\n\
                 rm -f \"{dir}/marker.$$\"\n\
                 printf '%s' '{{\"format\":{{\"format_name\":\"matroska,webm\"}},\
                 \"streams\":[{{\"codec_type\":\"audio\",\"codec_name\":\"opus\",\
                 \"disposition\":{{\"attached_pic\":0}}}}]}}'\n",
                dir = dir.display(),
            ),
        )
        .expect("write slow fake ffprobe");
        let mut perms = std::fs::metadata(&script)
            .expect("stat slow fake ffprobe")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).expect("chmod slow fake ffprobe");
        script
    }

    /// How many probes are running right now, counted by their marker files.
    #[cfg(unix)]
    fn probes_in_flight(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .expect("read marker dir")
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("marker."))
            })
            .count()
    }

    /// A legacy library's first visit asks for every video at once, and each
    /// incomplete record costs an ffprobe (plus the moov pass for MP4-family
    /// sources) on the blocking pool. The probe semaphore is what keeps that
    /// burst from fanning out one child process per request: with more requests
    /// than permits, only `PROBE_BOUND` probes may be in flight simultaneously.
    #[cfg(unix)]
    #[tokio::test]
    async fn concurrent_probes_stay_within_the_probe_bound() {
        const REQUESTS: usize = 8;
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        // Long enough that the queued requests pile up while the first batch is
        // parked, so the observed peak is the bound itself and not a race.
        let script = slow_fake_ffprobe(temp_dir.path(), "1");
        let _guard = crate::video_processor::tests::TestEnvGuard::set(
            "FFPROBE_PATH",
            script.to_str().unwrap(),
        );

        let pool = crate::db::create_in_memory_pool().await.expect("pool");
        let mut tasks = tokio::task::JoinSet::new();
        for index in 0..REQUESTS {
            let hash = format!("{:0<64}", format!("hash-probe-bound-{index}"));
            let mut photo = test_photo(&hash);
            photo.file_path = temp_dir
                .path()
                .join(format!("video-{index}.mkv"))
                .to_string_lossy()
                .into_owned();
            photo.filename = format!("video-{index}.mkv");
            photo.create(&pool).await.expect("create");
            let pool = pool.clone();
            tasks.spawn(async move { resolve(&pool, &photo).await });
        }

        // WHEN the burst is in flight, sample how many probes run at once
        let markers_dir = temp_dir.path().to_path_buf();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while probes_in_flight(&markers_dir) == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the first probes must start");
        let mut peak = 0;
        for _ in 0..20 {
            peak = peak.max(probes_in_flight(&markers_dir));
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }

        // THEN the bound held (and was actually exercised — a probe path that
        // serialized or fanned out would peak at 1 or at all 8 requests)
        assert_eq!(
            peak, PROBE_BOUND,
            "{REQUESTS} concurrent requests must run exactly PROBE_BOUND \
             ({PROBE_BOUND}) probes at once, not {peak}"
        );

        // AND every request still completes and is probed
        let resolved = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let mut probed = 0;
            while let Some(joined) = tasks.join_next().await {
                assert!(joined.expect("probe task must not panic").probed);
                probed += 1;
            }
            probed
        })
        .await
        .expect("every probe must finish");
        assert_eq!(resolved, REQUESTS);
        assert_eq!(
            probes_in_flight(&markers_dir),
            0,
            "every probe released its marker (and its permit) when it finished"
        );
    }

    /// The permit belongs to the BLOCKING probe, not to the request future. A
    /// handler future dropped mid-probe — the player aborts a superseded fetch
    /// on every seek, and a client that hangs up cancels the handler — must not
    /// hand the slot back while its ffprobe pair still runs: `spawn_blocking`
    /// work cannot be aborted and nothing kills the ffprobe children, so
    /// freeing the slot there lets the replacement request start a second probe
    /// of the same file and re-creates the unbounded fan-out the bound exists
    /// to prevent.
    ///
    /// Deterministic: the fake ffprobe signals that it has started and then
    /// parks until this test releases it, so nothing here races the blocking
    /// pool or guesses a duration. The shared env lock keeps every other
    /// probing test off the semaphore while the counts are read.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_dropped_request_holds_its_permit_until_the_probe_finishes() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let started = temp_dir.path().join("probe_started");
        let release = temp_dir.path().join("probe_release");
        let script = temp_dir.path().join("blocking_ffprobe.sh");
        // The wait is bounded so a failure can never leave a child spinning
        // forever; audio-only matroska keeps it to one ffprobe call (no moov
        // pass).
        std::fs::write(
            &script,
            format!(
                "#!/usr/bin/env sh\n\
                 touch '{started}'\n\
                 i=0\n\
                 while [ ! -e '{release}' ] && [ \"$i\" -lt 500 ]; do\n\
                   sleep 0.02\n\
                   i=$((i + 1))\n\
                 done\n\
                 printf '%s' '{{\"format\":{{\"format_name\":\"matroska,webm\"}},\
                 \"streams\":[{{\"codec_type\":\"audio\",\"codec_name\":\"opus\",\
                 \"disposition\":{{\"attached_pic\":0}}}}]}}'\n",
                started = started.display(),
                release = release.display(),
            ),
        )
        .expect("write blocking fake ffprobe");
        let mut perms = std::fs::metadata(&script)
            .expect("stat blocking fake ffprobe")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).expect("chmod blocking fake ffprobe");

        let _guard = crate::video_processor::tests::TestEnvGuard::set(
            "FFPROBE_PATH",
            script.to_str().unwrap(),
        );

        let pool = crate::db::create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-probe-permit");
        let mut photo = test_photo(&hash);
        photo.file_path = temp_dir
            .path()
            .join("audio-only.mkv")
            .to_string_lossy()
            .into_owned();
        photo.filename = "audio-only.mkv".to_string();
        photo.create(&pool).await.expect("create");

        let before = PROBE_SEMAPHORE.available_permits();
        let pool_for_task = pool.clone();
        let photo_for_task = photo.clone();
        let task = tokio::spawn(async move { resolve(&pool_for_task, &photo_for_task).await });

        // GIVEN a request whose probe is in flight
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !started.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the probe must start");

        // WHEN the request future is dropped mid-probe
        task.abort();
        let _ = task.await;

        // THEN its permit is still held by the probe that is still running
        assert_eq!(
            PROBE_SEMAPHORE.available_permits(),
            before - 1,
            "a dropped request must not release the permit its probe still holds"
        );

        // AND the permit returns once the probe finishes — exactly once
        std::fs::write(&release, b"go").expect("release the probe");
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while PROBE_SEMAPHORE.available_permits() < before {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the finished probe must return its permit");
        assert_eq!(PROBE_SEMAPHORE.available_permits(), before);
    }

    /// Fake ffprobe whose shell body is `body`, so a test can make one pass
    /// sleep/hang while another answers.
    #[cfg(unix)]
    fn write_fake_ffprobe(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let script = dir.join(name);
        std::fs::write(&script, format!("#!/usr/bin/env sh\n{body}")).expect("write fake ffprobe");
        let mut perms = std::fs::metadata(&script)
            .expect("stat fake ffprobe")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).expect("chmod fake ffprobe");
        script
    }

    /// A probe whose ffprobe never returns must not wedge the module: the pass is
    /// killed at its deadline, the request decides from the stored record instead
    /// of hanging, and the permit the probe held comes back. The fake `exec`s the
    /// sleep so the killed process is the sleeper itself (a shell that merely
    /// spawns it would keep the probe's pipes open and the drain would inherit
    /// the hang the deadline is meant to end).
    #[cfg(unix)]
    #[tokio::test]
    async fn a_probe_that_outlives_its_deadline_degrades_and_releases_its_permit() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let script = write_fake_ffprobe(temp_dir.path(), "sleeping_ffprobe.sh", "exec sleep 30\n");
        let _guard = crate::video_processor::tests::TestEnvGuard::set(
            "FFPROBE_PATH",
            script.to_str().unwrap(),
        );

        let pool = crate::db::create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-probe-deadline");
        let mut photo = test_photo(&hash);
        photo.file_path = temp_dir
            .path()
            .join("video.mp4")
            .to_string_lossy()
            .into_owned();
        photo.filename = "video.mp4".to_string();
        photo.create(&pool).await.expect("create");

        let before = PROBE_SEMAPHORE.available_permits();
        // WHEN the probe outlives its deadline
        let started = std::time::Instant::now();
        let resolved = resolve_within(&pool, &photo, Duration::from_millis(150)).await;

        // THEN the request degrades to the stored record instead of hanging on
        // the ffprobe (the fake sleeps 30s)
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "a stuck ffprobe must be killed at the deadline, took {:?}",
            started.elapsed()
        );
        assert!(
            !resolved.probed,
            "a killed probe must decide from the stored record"
        );
        assert_eq!(resolved, ResolvedCapabilities::from_record(&photo));

        // AND nothing was persisted, so the record stays retryable
        let stored = Photo::find_by_hash(&pool, &hash).await.unwrap().unwrap();
        assert!(
            stored.metadata.get("video").is_none(),
            "a killed probe must not persist a record"
        );

        // AND the permit the probe held is back, so the bound cannot be wedged
        assert_eq!(PROBE_SEMAPHORE.available_permits(), before);
    }

    /// The moov pass is bounded the same way: a `-v trace` pass that never
    /// returns is killed at the deadline, the facts the probe already has are
    /// kept, and the permit comes back. The layout is one of those facts: a
    /// record that already carried `moov_at_start: false` (an incomplete one —
    /// the row is re-probed because it has no capability marker) must still say
    /// `false` when the pass yields no verdict. Reading the killed pass as "at
    /// start" would instead complete the record with the opposite of the truth,
    /// permanently: `plan` would route this moov-at-end MP4 into `Direct` and
    /// `get_video_file`'s `Direct` arm never re-checks the layout, so the
    /// browser would download the whole file before the first frame.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_moov_pass_that_outlives_its_deadline_does_not_wedge_the_probe() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let counter_path = temp_dir.path().join("probe_count");
        let counter = counter_path.display().to_string();
        // MP4-family facts on the first pass (so the moov pass runs at all) and a
        // parked moov pass on all later ones.
        let json = r#"{"format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"5.0"},"streams":[{"codec_type":"video","codec_name":"h264","pix_fmt":"yuv420p","disposition":{"attached_pic":0}}]}"#;
        let script = write_fake_ffprobe(
            temp_dir.path(),
            "hanging_moov_ffprobe.sh",
            &format!(
                "printf 'x\\n' >> '{counter}'\n\
                 for arg in \"$@\"; do\n\
                 if [ \"$arg\" = trace ]; then exec sleep 30; fi\n\
                 done\n\
                 printf '%s' '{json}'\n"
            ),
        );
        let _guard = crate::video_processor::tests::TestEnvGuard::set(
            "FFPROBE_PATH",
            script.to_str().unwrap(),
        );

        let pool = crate::db::create_in_memory_pool().await.expect("pool");
        let hash = format!("{:0<64}", "hash-moov-deadline");
        let mut photo = test_photo(&hash);
        photo.file_path = temp_dir
            .path()
            .join("video.mp4")
            .to_string_lossy()
            .into_owned();
        photo.filename = "video.mp4".to_string();
        // The accurate fact a previous probe wrote before the file's moov was
        // moved to the end of the file, and no capability marker, so this record
        // is still probed.
        photo.metadata = json!({ "video": { "moov_at_start": false } });
        assert!(!record_is_complete(&photo), "the row must still be probed");
        photo.create(&pool).await.expect("create");

        let before = PROBE_SEMAPHORE.available_permits();
        // WHEN the moov pass outlives its deadline
        let started = std::time::Instant::now();
        let resolved = resolve_within(&pool, &photo, Duration::from_millis(150)).await;

        // THEN the probe keeps its facts instead of hanging on the trace pass
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "a stuck moov pass must be killed at the deadline, took {:?}",
            started.elapsed()
        );
        assert!(resolved.probed, "the facts pass succeeded");
        assert_eq!(resolved.codec, "h264");
        assert!(
            !resolved.moov_at_start,
            "a killed trace pass must keep the stored layout, not invent 'at start'"
        );
        // AND the decision that reads that fact still sends this file to the
        // remux rung: the browser cannot seek a moov-at-end MP4, so `Direct`
        // would make it download the whole file before the first frame.
        assert_eq!(
            crate::video_capability::plan(
                &resolved,
                &crate::video_capability::ClientCodecs::parse(Some("h264-8,aac"))
            ),
            crate::video_capability::Delivery::StreamRemux,
            "a moov-at-end MP4 must never be decided as direct"
        );
        assert_eq!(
            std::fs::read_to_string(&counter_path)
                .unwrap()
                .lines()
                .count(),
            2,
            "the facts pass and the moov pass both ran"
        );

        // AND the record is complete, so the file is never probed again — with
        // the stored layout, not the guess, so that completeness is not a lie
        let stored = Photo::find_by_hash(&pool, &hash).await.unwrap().unwrap();
        assert_eq!(stored.metadata["video"]["capability_version"], 1);
        assert_eq!(
            stored.metadata["video"]["moov_at_start"], false,
            "the stored layout must survive the failed pass"
        );

        // AND the permit came back
        assert_eq!(PROBE_SEMAPHORE.available_permits(), before);
    }
}
