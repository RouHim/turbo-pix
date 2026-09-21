# GPU-Accelerated ffmpeg Transcoding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When the host has a usable ffmpeg hardware H.264 encoder, run both the whole-file conversion and the live streaming transcode rung on it; when it does not (or the hardware attempt fails), behave exactly like today's `libx264` build.

**Architecture:** A new `src/video_encoder.rs` owns one process-wide verdict. It lists ffmpeg's encoders, then *proves* the first candidate in preference order by running a throwaway encode through the same argument helpers a real job uses. Both transcoding paths take the verdict as an explicit `Option<&HwPlan>` parameter, so the argument builders stay pure and unit-testable, and the CPU path only ever sees `None`. Each job keeps a single in-job fallback: whole-file conversions retry once in software after a hardware failure, and a hardware-planned stream run that dies before its first byte is respawned in software inside the same request.

**Tech Stack:** Rust 2021 (tokio, warp, serde), ffmpeg CLI as the encoder backend, Playwright E2E.

**Spec:** `.spec/ffmpeg-gpu-transcoding.md`

## Global Constraints

- **No new configuration.** No new environment variable, no new config key, no change to existing ones. The only operator-visible surface change is the `encoder` field added to the transcode status payload.
- **Encoder preference order (fixed):** `h264_nvenc` → `h264_vaapi` → `h264_qsv` → `h264_amf` → `h264_videotoolbox`. VAAPI outranks QSV because QSV sits on the same driver VAAPI drives while needing an extra runtime (libmfx/oneVPL) that is frequently absent; probing QSV first costs ~2 s to fail on such hosts.
- **Quality knob per backend, all anchored to the software `-crf 23`:** `h264_nvenc` → `-rc vbr -cq 23 -b:v 0 -tune hq` (`-b:v 0` is mandatory; without it NVENC clamps to its 2 Mbps default), `h264_vaapi` → `-qp 23`, `h264_qsv` → `-global_quality 23`, `h264_amf` → `-rc cqp -qp_i 23 -qp_p 23`, `h264_videotoolbox` → `-q:v 55` (its constant-quality scale is 1–100).
- **Pixel formats:** `-pix_fmt yuv420p` everywhere except QSV, which consumes `nv12` surfaces. Upload filters: VAAPI `-vf format=nv12,hwupload`, QSV `-vf hwupload=extra_hw_frames=64,format=qsv`. NVENC, AMF and VideoToolbox take system-memory frames and get no filter.
- **Decode side is untouched:** the existing `-hwaccel auto` stays exactly as it is in both paths. Only the *encoder* decision changes, which is why the probe only has to prove the encode shape.
- **The shared flags are empirically compatible with `h264_vaapi`** (verified with this project's ffmpeg 9.0.1 against a real 1080p HEVC source): `-profile:v main -g 48 -keyint_min 48 -sc_threshold 0` together with `-vf format=nv12,hwupload -c:v h264_vaapi -qp 23 -pix_fmt yuv420p` and the fragmented-output `-movflags`/`-frag_duration` produce a valid fMP4 (H.264 Main + AAC) with `rc=0`. Do not "defensively" drop them: a backend that refuses one of them fails the probe and is never selected, which is the designed behaviour.
- **Timings:** probe timeout 2 s per ffmpeg call; stream first-byte gate timeout 10 s (a silent-but-alive run is left to the existing stall watchdog, never killed by the gate).
- **`None` plan means byte-identical behaviour to today** — same ffmpeg argument vector, same logs, same status transitions (spec FR-009). This is asserted by a regression test in both argument builders.
- **Zero warnings:** `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` must pass before each commit. No dead code: helpers that only tests use are `#[cfg(test)]`.
- **No test may call `video_encoder::init()`** except the one caching test that resolves it to `None` — `ACTIVE_PLAN` is process-global and would leak into every other test in the binary.
- Measured on this host (Intel renderD129 via VAAPI, 5 s of 1080p30 HEVC source): software `-preset fast -crf 23` = 2.3 s / 3.1 MB / SSIM 0.9583; VAAPI `-qp 23` = 0.7 s / 4.4 MB / SSIM 0.9580. Hardware is ~3× faster at equal SSIM and ~40 % larger files — the size growth is the accepted cost, the quality is not negotiable.

## Review Focus

Ordered by how likely each is to bite a real user; the test that pins it is named in each line and is added in that task's steps.

1. **A listed-but-broken encoder on the machine's *second* render node** — a host whose first render node belongs to a GPU with no H.264 encode entrypoint must still find the working node instead of reporting "no hardware encoder" or failing every job. Pinned by `probe_finds_the_render_node_that_can_encode` (Task 2) and `probe_rejects_a_listed_encoder_that_cannot_encode` (Task 2).
2. **Hardware encoder refuses the actual source** (device busy, session limit, unsupported pixel format, driver reset) — the user must still get a playable video, not an error, and must not lose a conversion slot waiting for a second timeout. Pinned by `hardware_failure_falls_back_to_the_software_encoder` (Task 3) and `immediate_hardware_failure_respawns_in_software` (Task 4).
3. **Progress going backwards when the software retry restarts the encode from frame zero** — the poller would show the bar jumping back to a low percent. Pinned by `progress_never_goes_backwards_across_a_fallback` (Task 3).
4. **The first bytes of a hardware-planned stream being lost or duplicated by the gate** — the client would fail to build its SourceBuffer (`VP9`/`ftyp`/`moov` box missing from the front of the stream). Pinned by `first_bytes_are_preserved_for_the_body` (Task 4).
5. **A probed-and-passing encoder that a *later* job cannot open, on the streaming path, in the middle of a response** — after bytes have been sent the run cannot be replaced, so the gate must trigger only before the first byte and must never swallow a genuine spawn error. Pinned by `planless_stream_returns_the_childs_bytes_verbatim` and `spawn_error_still_maps_to_start_error` (Task 4).

## File Structure

| File | Responsibility after this plan |
| --- | --- |
| `src/video_encoder.rs` *(new)* | `HwEncoder`/`HwPlan` model, per-backend ffmpeg argument helpers, render-node discovery, the one-shot probe, the process-wide verdict (`init`/`active`). Knows nothing about conversions or streams. |
| `src/lib.rs` | Module registration (`pub mod video_encoder;`). |
| `src/main.rs` | Warms the verdict at startup, once, before the server binds. |
| `src/video_processor.rs` | `build_conversion_args` gains the plan; the conversion attempt gains a typed outcome and the software fallback; `TranscodeStatus` gains `encoder`; `ConversionOutcome` is defined here (it is reported next to the status it feeds). |
| `src/video_stream.rs` | `build_args` gains the plan; `start_stream` gains the plan seam and the first-byte gate; `ProgressReader` gains prefix replay. |
| `src/handlers_video.rs` | Reports the encoder that produced a finished conversion through `TranscodeStatus`. |
| `tests/e2e/specs/transcoding.e2e.spec.js` | Asserts the reported encoder is a known value and that playback is unaffected. |
| `AGENTS.md` | Learnings entry 7 extended (no new entry — the list is capped at 10). |

---

## Task 1: Hardware encoder model and per-backend ffmpeg argument vectors

**Files:**
- Create: `src/video_encoder.rs`
- Modify: `src/lib.rs:37` (module list, keep alphabetical order: `video_capability`, `video_encoder`, `video_probe`)
- Test: `src/video_encoder.rs` (`#[cfg(test)] mod tests` at the bottom)

**Interfaces:**
- Consumes: nothing (pure module; no ffmpeg process is started in this task).
- Produces:
  - `pub const SOFTWARE_ENCODER: &str = "libx264"`
  - `pub enum HwEncoder { Nvenc, Vaapi, Qsv, Amf, VideoToolbox }` with `pub const ALL: [HwEncoder; 5]`, `pub fn name(self) -> &'static str`
  - `pub struct HwPlan` with `pub fn new(encoder: HwEncoder, device: Option<String>) -> Self`, `pub fn encoder(&self) -> HwEncoder`, `pub fn device(&self) -> Option<&str>`, `pub fn input_args(&self) -> Vec<String>`, `pub fn upload_filter_args(&self) -> Option<[String; 2]>`, `pub fn video_args(&self) -> Vec<String>`, `pub fn label(&self) -> String`
  - `fn lists_encoder(listing: &str, name: &str) -> bool` (private, used by Task 2)

- [ ] **Step 1: Write the failing tests**

Create `src/video_encoder.rs` with only the module docs and a test module, so the tests fail to compile against names that do not exist yet:

```rust
//! Hardware H.264 encoder selection.
//!
//! ffmpeg ships a hardware encoder for every major GPU vendor, but a *listed*
//! encoder is not a *usable* one: the encoder can be compiled in while the
//! device node, the driver or the vendor runtime is missing, and that only
//! shows up when a job actually opens it. Each candidate therefore has to
//! encode something once before it is trusted.
//!
//! Both transcoding paths share this module: the whole-file conversion in
//! `video_processor` and the live MSE stream in `video_stream` ask for the same
//! [`HwPlan`] and derive their ffmpeg arguments from the same helpers. That is
//! the point of the sharing — the startup probe runs the very argument shape a
//! real job runs, so a passing probe means the job's encoder command works.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_backend_declares_a_distinct_ffmpeg_encoder_name() {
        // GIVEN the preference list
        let names: Vec<&str> = HwEncoder::ALL.iter().map(|e| e.name()).collect();

        // THEN every name is unique and looks like an ffmpeg H.264 encoder
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "duplicate encoder names: {names:?}");
        for name in names {
            assert!(name.starts_with("h264_"), "{name} is not an H.264 encoder");
        }
    }

    #[test]
    fn quality_knob_keeps_the_software_crf_value_on_every_backend() {
        // GIVEN every backend
        // THEN each one passes the software path's CRF value (23) as its own
        // quality target, so "parity by default" is not a per-backend accident
        for encoder in HwEncoder::ALL {
            let plan = HwPlan::new(encoder, None);
            let args = plan.video_args();
            assert!(
                args.iter().any(|arg| arg == "23"),
                "{:?} must carry the 23 quality target: {args:?}",
                encoder
            );
        }
    }

    #[test]
    fn nvenc_lifts_the_bitrate_cap_for_constant_quality() {
        // GIVEN an NVENC plan
        // WHEN the video arguments are built
        let args = HwPlan::new(HwEncoder::Nvenc, None).video_args();

        // THEN constant-quality mode is requested with the cap lifted (`-b:v 0`),
        // because NVENC otherwise clamps quality to its 2 Mbps default
        assert!(args.contains(&"h264_nvenc".to_string()), "{args:?}");
        assert!(args.contains(&"-cq".to_string()), "{args:?}");
        let b_index = args.iter().position(|arg| arg == "-b:v").expect("-b:v missing");
        assert_eq!(args[b_index + 1], "0", "NVENC needs -b:v 0: {args:?}");
    }

    #[test]
    fn vaapi_plan_selects_the_device_and_uploads_frames() {
        // GIVEN a VAAPI plan on a specific render node
        let plan = HwPlan::new(HwEncoder::Vaapi, Some("/dev/dri/renderD129".to_string()));

        // WHEN its argument pieces are built
        // THEN the device sits before `-i`, the encoder is VAAPI's, and the
        // filter graph uploads into hardware surfaces
        assert_eq!(plan.input_args(), vec!["-vaapi_device", "/dev/dri/renderD129"]);
        assert_eq!(
            plan.upload_filter_args(),
            Some(["-vf".to_string(), "format=nv12,hwupload".to_string()])
        );
        assert!(plan.video_args().contains(&"h264_vaapi".to_string()));
        assert!(plan.video_args().contains(&"-qp".to_string()));
        assert!(plan.video_args().contains(&"yuv420p".to_string()));
    }

    #[test]
    fn qsv_plan_initialises_the_filter_device_and_uses_surfaces() {
        // GIVEN a QSV plan on a render node
        let plan = HwPlan::new(HwEncoder::Qsv, Some("/dev/dri/renderD129".to_string()));

        // WHEN its argument pieces are built
        // THEN the filter device is initialised explicitly (the `hwupload`
        // filter needs its own device, unlike VAAPI's shorthand) and the pixel
        // format is the surface format QSV accepts
        assert_eq!(
            plan.input_args(),
            vec!["-init_hw_device", "qsv=hw:/dev/dri/renderD129", "-filter_hw_device", "hw"]
        );
        assert_eq!(
            plan.upload_filter_args(),
            Some([
                "-vf".to_string(),
                "hwupload=extra_hw_frames=64,format=qsv".to_string()
            ])
        );
        assert!(plan.video_args().contains(&"nv12".to_string()));
        assert!(plan.video_args().contains(&"-global_quality".to_string()));
    }

    #[test]
    fn system_memory_backends_need_no_filter_and_use_yuv420p() {
        // GIVEN the backends that accept system-memory frames
        // THEN they add no upload filter and emit the software path's format
        for encoder in [HwEncoder::Nvenc, HwEncoder::Amf, HwEncoder::VideoToolbox] {
            let plan = HwPlan::new(encoder, None);
            assert_eq!(plan.upload_filter_args(), None, "{encoder:?}");
            assert!(plan.input_args().is_empty(), "{encoder:?}");
            assert!(plan.video_args().contains(&"yuv420p".to_string()), "{encoder:?}");
        }
    }

    #[test]
    fn videotoolbox_uses_the_one_to_hundred_quality_scale() {
        // GIVEN a VideoToolbox plan
        // WHEN its arguments are built
        // THEN quality is expressed on its own 1-100 scale, not as a QP
        let args = HwPlan::new(HwEncoder::VideoToolbox, None).video_args();
        let q_index = args.iter().position(|arg| arg == "-q:v").expect("-q:v missing");
        let value: u32 = args[q_index + 1].parse().expect("quality must be numeric");
        assert!((1..=100).contains(&value), "out of range: {value}");
    }

    #[test]
    fn a_plan_without_a_device_emits_no_device_arguments() {
        // GIVEN a plan the probe could not pin to a node
        // WHEN its arguments are built
        // THEN no half-formed device option is emitted (the job then fails and
        // the software fallback takes over, which is the safe direction)
        for encoder in HwEncoder::ALL {
            let plan = HwPlan::new(encoder, None);
            assert!(
                !plan.input_args().iter().any(|arg| arg.contains("/dev/dri")),
                "{encoder:?}: {:?}",
                plan.input_args()
            );
        }
    }

    #[test]
    fn encoder_listing_match_requires_a_whole_token() {
        // GIVEN an ffmpeg build that only has the HEVC VAAPI encoder
        let listing = " V....D hevc_vaapi           H.265/HEVC (VAAPI)\n V....D libx264              H.264\n";

        // THEN an H.264 search must not be satisfied by it
        assert!(!lists_encoder(listing, "h264_vaapi"));
        assert!(lists_encoder(listing, "libx264"));
    }

    #[test]
    fn label_names_the_device_when_there_is_one() {
        // GIVEN plans with and without a device
        // THEN the log label distinguishes them
        assert_eq!(
            HwPlan::new(HwEncoder::Vaapi, Some("/dev/dri/renderD129".to_string())).label(),
            "h264_vaapi (/dev/dri/renderD129)"
        );
        assert_eq!(HwPlan::new(HwEncoder::Nvenc, None).label(), "h264_nvenc");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib video_encoder`
Expected: compile error — `cannot find type HwEncoder`, `cannot find function lists_encoder`.

- [ ] **Step 3: Implement the module**

Prepend to `src/video_encoder.rs` (above `#[cfg(test)]`):

```rust
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

/// Software fallback. Every hardware decision degrades to this, and it is the
/// only encoder the CPU path uses.
pub const SOFTWARE_ENCODER: &str = "libx264";

/// System-memory pixel format the software path emits; the hardware backends use
/// it too unless they only accept hardware surfaces.
const SOFTWARE_PIX_FMT: &str = "yuv420p";

/// Longest a single ffmpeg probe (encoder listing or encode) may take. A
/// working encode answers in milliseconds; a broken vendor runtime can hang,
/// and a false "unusable" verdict is cheaper than a stalled boot.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Synthetic probe input: instant to encode, comfortably above every vendor's
/// minimum frame size, and available without a file on disk.
const PROBE_INPUT: &str = "color=c=black:s=320x240:r=30:d=0.1";

/// Hardware H.264 encoder the process may use.
///
/// [`HwEncoder::ALL`] is the preference order: NVIDIA first (nothing else
/// reaches NVENC), then VAAPI ahead of QSV — QSV sits on top of the very driver
/// VAAPI drives, so VAAPI covers every device QSV does while needing no
/// libmfx/oneVPL runtime, and probing QSV first costs seconds on hosts where
/// that runtime is missing. AMF and VideoToolbox close the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwEncoder {
    Nvenc,
    Vaapi,
    Qsv,
    Amf,
    VideoToolbox,
}

impl HwEncoder {
    /// Probe order; the first encoder that survives its probe wins.
    pub const ALL: [HwEncoder; 5] = [
        Self::Nvenc,
        Self::Vaapi,
        Self::Qsv,
        Self::Amf,
        Self::VideoToolbox,
    ];

    /// ffmpeg encoder name, i.e. the `-c:v` value.
    pub fn name(self) -> &'static str {
        match self {
            Self::Nvenc => "h264_nvenc",
            Self::Vaapi => "h264_vaapi",
            Self::Qsv => "h264_qsv",
            Self::Amf => "h264_amf",
            Self::VideoToolbox => "h264_videotoolbox",
        }
    }

    /// Backends whose encoder only accepts hardware surfaces, so the filter
    /// graph has to upload frames first.
    fn needs_upload_filter(self) -> bool {
        matches!(self, Self::Vaapi | Self::Qsv)
    }

    /// Backends driven through a DRM render node. Linux-only: the same encoders
    /// take their device implicitly on other platforms.
    fn needs_render_node(self) -> bool {
        cfg!(target_os = "linux") && matches!(self, Self::Vaapi | Self::Qsv | Self::Amf)
    }

    /// Speed/quality preset, where the backend has one. NVENC's modern presets
    /// run p1 (fastest) to p7 (slowest); QSV accepts libx264's preset names.
    fn preset_args(self) -> &'static [&'static str] {
        match self {
            Self::Nvenc => &["-preset", "p5"],
            Self::Qsv => &["-preset", "veryfast"],
            _ => &[],
        }
    }

    /// Constant-quality rate control, anchored to the software path's `-crf 23`.
    ///
    /// There is no exact translation — x264's CRF adapts per frame and per
    /// block, every one of these knobs is coarser — so the number is matched
    /// rather than the bitrate: measured against `-crf 23` on this project's
    /// fixtures, VAAPI `-qp 23` lands within 0.0005 SSIM of the software output.
    fn quality_args(self) -> &'static [&'static str] {
        match self {
            // `-cq` only reaches its target when the bitrate cap is lifted:
            // left alone, NVENC clamps quality to its 2 Mbps default.
            Self::Nvenc => &["-rc", "vbr", "-cq", "23", "-b:v", "0", "-tune", "hq"],
            // VAAPI has no CRF-like mode; CQP is its constant-quality mode.
            Self::Vaapi => &["-qp", "23"],
            // ICQ: QSV's CRF equivalent.
            Self::Qsv => &["-global_quality", "23"],
            // AMF exposes no constant-quality-plus-adaptation mode; equal I/P
            // quantizers are its closest equivalent.
            Self::Amf => &["-rc", "cqp", "-qp_i", "23", "-qp_p", "23"],
            // VideoToolbox's constant-quality scale is 1-100; the usable band
            // starts around 50, so 55 is the conservative mid-high default.
            Self::VideoToolbox => &["-q:v", "55"],
        }
    }

    /// Pixel format the encoder accepts. QSV is the odd one out: it consumes
    /// `nv12` surfaces rather than `yuv420p` frames.
    fn pix_fmt(self) -> &'static str {
        match self {
            Self::Qsv => "nv12",
            _ => SOFTWARE_PIX_FMT,
        }
    }
}

/// `Vec<String>` from string pieces, the shape the ffmpeg builders use.
fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_string()).collect()
}

/// A hardware encoder that passed its probe, plus the device it passed on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HwPlan {
    encoder: HwEncoder,
    /// DRM render node the probe succeeded on (for the backends that take one).
    device: Option<String>,
}

impl HwPlan {
    pub fn new(encoder: HwEncoder, device: Option<String>) -> Self {
        Self { encoder, device }
    }

    pub fn encoder(&self) -> HwEncoder {
        self.encoder
    }

    pub fn device(&self) -> Option<&str> {
        self.device.as_deref()
    }

    /// Arguments that must sit before `-i`: device selection for the backends
    /// that need one.
    pub fn input_args(&self) -> Vec<String> {
        match (self.encoder, self.device()) {
            // `-vaapi_device` is shorthand for `-init_hw_device vaapi=…` plus
            // `-filter_hw_device …`, so it arms the encoder and the upload
            // filter with one option.
            (HwEncoder::Vaapi, Some(node)) => strings(&["-vaapi_device", node]),
            // QSV needs the device named for the filter graph as well, which
            // the shorthand does not cover.
            (HwEncoder::Qsv, Some(node)) => strings(&[
                "-init_hw_device",
                &format!("qsv=hw:{node}"),
                "-filter_hw_device",
                "hw",
            ]),
            _ => Vec::new(),
        }
    }

    /// `-vf …` pair for the backends that can only encode hardware surfaces.
    /// `None` means the encoder takes the decoder's frames as they are.
    pub fn upload_filter_args(&self) -> Option<[String; 2]> {
        match self.encoder {
            HwEncoder::Vaapi => Some(["-vf".to_string(), "format=nv12,hwupload".to_string()]),
            // `extra_hw_frames` gives the upload pool room to keep the encoder
            // fed; a shallow pool stalls on bursts.
            HwEncoder::Qsv => Some([
                "-vf".to_string(),
                "hwupload=extra_hw_frames=64,format=qsv".to_string(),
            ]),
            _ => None,
        }
    }

    /// Replaces the software `-c:v libx264 -preset … -crf 23` triple.
    pub fn video_args(&self) -> Vec<String> {
        let mut args = strings(&["-c:v", self.encoder.name()]);
        args.extend(self.encoder.preset_args().iter().map(|arg| (*arg).to_string()));
        args.extend(self.encoder.quality_args().iter().map(|arg| (*arg).to_string()));
        args.extend(strings(&["-pix_fmt", self.encoder.pix_fmt()]));
        args
    }

    /// Human-readable target for logs, e.g. `h264_vaapi (/dev/dri/renderD129)`.
    pub fn label(&self) -> String {
        match self.device() {
            Some(node) => format!("{} ({node})", self.encoder.name()),
            None => self.encoder.name().to_string(),
        }
    }
}

/// True when ffmpeg's encoder table contains exactly this encoder. The table is
/// one encoder per line (` V....D h264_vaapi   H.264/AVC (VAAPI)`), so the name
/// has to match a whole token: `hevc_vaapi` must never satisfy `h264_vaapi`.
fn lists_encoder(listing: &str, name: &str) -> bool {
    listing
        .lines()
        .any(|line| line.split_whitespace().any(|token| token == name))
}
```

Add `pub mod video_encoder;` to `src/lib.rs` between `pub mod video_capability;` and `pub mod video_probe;`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib video_encoder`
Expected: 10 tests pass.

- [ ] **Step 5: Lint, format, commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --lib video_encoder
git add src/video_encoder.rs src/lib.rs
git commit -m "feat(video): add hardware encoder model and ffmpeg argument builders"
```

---

## Task 2: Probe the encoders once and remember the verdict

**Files:**
- Modify: `src/video_encoder.rs` (add detection + the process-wide verdict)
- Modify: `src/main.rs` (warm the verdict at startup)
- Modify: `src/video_processor.rs:2025` (`make_executable` → `pub(crate)` so the new test module can reuse it)
- Test: `src/video_encoder.rs` (`mod tests`)

**Interfaces:**
- Consumes: `HwEncoder`, `HwPlan`, `lists_encoder`, plus `crate::video_processor::{get_ffmpeg_path, transcode_max_pool}` and `crate::video_processor::tests::make_executable`.
- Produces:
  - `pub async fn init() -> Option<HwPlan>` — probes once, logs the verdict, safe to call again
  - `pub fn active() -> Option<HwPlan>` — the cached verdict, `None` when `init` never ran or found nothing
  - `pub async fn detect(ffmpeg: &str, nodes: &[String]) -> Option<HwPlan>` — the testable seam (nodes injected)
  - `fn render_nodes() -> Vec<String>`, `async fn encoder_listing(ffmpeg: &str) -> Option<String>`, `async fn probe(ffmpeg: &str, plan: &HwPlan) -> bool`

- [ ] **Step 1: Write the failing tests**

Append to `src/video_encoder.rs`'s `mod tests` (add `use crate::video_processor::tests::{make_executable, TestEnvGuard};`, `use std::path::Path;` and `use tempfile::TempDir;` to the test module's imports):

```rust
    #[cfg(unix)]
    fn write_fake_ffmpeg(dir: &Path, body: &str) -> String {
        let path = dir.join("fake-ffmpeg.sh");
        std::fs::write(&path, format!("#!/usr/bin/env sh\n{body}")).expect("fake ffmpeg written");
        make_executable(&path);
        path.to_string_lossy().into_owned()
    }

    #[cfg(unix)]
    fn args_log(lines: &std::path::Path) -> Vec<String> {
        std::fs::read_to_string(lines)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_finds_the_render_node_that_can_encode() {
        // GIVEN an ffmpeg that lists h264_vaapi but can only encode on the
        // second render node (the first belongs to a GPU without an encode
        // entrypoint, which is what an AMD Mars + Intel iGPU host looks like)
        let temp = TempDir::new().unwrap();
        let log = temp.path().join("args.log");
        let ffmpeg = write_fake_ffmpeg(
            temp.path(),
            &format!(
                "printf '%s\\n' \"$*\" >> '{}'\n\
                 case \"$*\" in\n\
                 *-encoders*) printf '%s\\n' ' V....D h264_vaapi           H.264/AVC (VAAPI)'; exit 0 ;;\n\
                 *renderD128*) exit 1 ;;\n\
                 esac\n\
                 exit 0\n",
                log.display()
            ),
        );
        let nodes = [
            "/dev/dri/renderD128".to_string(),
            "/dev/dri/renderD129".to_string(),
        ];
        let _guard = TestEnvGuard::set("FFMPEG_PATH", &ffmpeg);

        // WHEN the candidates are probed
        let plan = detect(&ffmpeg, &nodes).await;

        // THEN the node that can encode is the one that is used
        let plan = plan.expect("a usable encoder must be found");
        assert_eq!(plan.encoder(), HwEncoder::Vaapi);
        assert_eq!(plan.device(), Some("/dev/dri/renderD129"), "log: {:?}", args_log(&log));
        // AND both nodes were actually tried, not just the first
        assert_eq!(args_log(&log).iter().filter(|line| line.contains("renderD128")).count(), 1);
        assert_eq!(args_log(&log).iter().filter(|line| line.contains("renderD129")).count(), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_rejects_a_listed_encoder_that_cannot_encode() {
        // GIVEN an ffmpeg that lists h264_nvenc but has no working device
        let temp = TempDir::new().unwrap();
        let ffmpeg = write_fake_ffmpeg(
            temp.path(),
            "case \"$*\" in\n\
             *-encoders*) printf '%s\\n' ' V....D h264_nvenc           H.264/AVC (NVENC)'; exit 0 ;;\n\
             esac\n\
             exit 1\n",
        );

        // WHEN the candidates are probed
        let plan = detect(&ffmpeg, &[]).await;

        // THEN a listed encoder that cannot encode is not trusted
        assert_eq!(plan, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_skips_encoders_that_are_not_listed() {
        // GIVEN an ffmpeg that only lists h264_vaapi
        let temp = TempDir::new().unwrap();
        let log = temp.path().join("args.log");
        let ffmpeg = write_fake_ffmpeg(
            temp.path(),
            &format!(
                "printf '%s\\n' \"$*\" >> '{}'\n\
                 printf '%s\\n' ' V....D h264_vaapi           H.264/AVC (VAAPI)'\n\
                 exit 0\n",
                log.display()
            ),
        );
        let nodes = ["/dev/dri/renderD129".to_string()];

        // WHEN the candidates are probed
        let plan = detect(&ffmpeg, &nodes).await;

        // THEN no other backend was ever launched
        assert_eq!(plan.map(|p| p.encoder()), Some(HwEncoder::Vaapi));
        for other in ["h264_nvenc", "h264_qsv", "h264_amf", "h264_videotoolbox"] {
            assert_eq!(
                args_log(&log).iter().filter(|line| line.contains(other)).count(),
                0,
                "{other} must not be probed when ffmpeg does not list it"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_times_out_on_a_hanging_encoder() {
        // GIVEN an ffmpeg whose probe never returns
        let temp = TempDir::new().unwrap();
        let ffmpeg = write_fake_ffmpeg(
            temp.path(),
            "case \"$*\" in\n\
             *-encoders*) printf '%s\\n' ' V....D h264_vaapi           H.264/AVC (VAAPI)'; exit 0 ;;\n\
             esac\n\
             sleep 30\n",
        );
        let nodes = ["/dev/dri/renderD129".to_string()];

        // WHEN the candidates are probed
        let started = std::time::Instant::now();
        let plan = detect(&ffmpeg, &nodes).await;

        // THEN the probe is abandoned instead of hanging the caller
        assert_eq!(plan, None);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "a hanging probe must be cut off, took {:?}",
            started.elapsed()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn preference_order_picks_the_first_usable_encoder() {
        // GIVEN an ffmpeg that lists NVENC and VAAPI, where only VAAPI encodes
        let temp = TempDir::new().unwrap();
        let ffmpeg = write_fake_ffmpeg(
            temp.path(),
            "case \"$*\" in\n\
             *-encoders*) printf '%s\\n' ' V....D h264_nvenc           H.264/AVC (NVENC)' ' V....D h264_vaapi           H.264/AVC (VAAPI)'; exit 0 ;;\n\
             *h264_nvenc*) exit 1 ;;\n\
             esac\n\
             exit 0\n",
        );
        let nodes = ["/dev/dri/renderD129".to_string()];

        // WHEN the candidates are probed
        let plan = detect(&ffmpeg, &nodes).await;

        // THEN the first *usable* backend is chosen, not the first listed one
        assert_eq!(plan.map(|p| p.encoder()), Some(HwEncoder::Vaapi));
    }

    #[tokio::test]
    async fn without_a_render_node_the_node_backends_are_never_probed() {
        // GIVEN an ffmpeg that lists VAAPI and a machine with no render nodes
        let temp = TempDir::new().unwrap();
        let log = temp.path().join("args.log");
        let ffmpeg = write_fake_ffmpeg(
            temp.path(),
            &format!(
                "printf '%s\\n' \"$*\" >> '{}'\n\
                 printf '%s\\n' ' V....D h264_vaapi           H.264/AVC (VAAPI)'\n\
                 exit 0\n",
                log.display()
            ),
        );

        // WHEN the candidates are probed
        let plan = detect(&ffmpeg, &[]).await;

        // THEN the backend is skipped without launching a doomed probe
        assert_eq!(plan, None);
        assert!(
            !args_log(&log).iter().any(|line| line.contains("h264_vaapi")),
            "VAAPI must not be probed without a render node: {:?}",
            args_log(&log)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probe_runs_the_same_argument_shape_a_job_uses() {
        // GIVEN a working VAAPI host
        let temp = TempDir::new().unwrap();
        let log = temp.path().join("args.log");
        let ffmpeg = write_fake_ffmpeg(
            temp.path(),
            &format!(
                "printf '%s\\n' \"$*\" >> '{}'\n\
                 printf '%s\\n' ' V....D h264_vaapi           H.264/AVC (VAAPI)'\n\
                 exit 0\n",
                log.display()
            ),
        );
        let nodes = ["/dev/dri/renderD129".to_string()];

        // WHEN the probe runs
        assert!(detect(&ffmpeg, &nodes).await.is_some());

        // THEN it carried the device, the upload filter and the encoder flags,
        // i.e. the probe proves the shape the job will actually run
        let probe_line = args_log(&log)
            .into_iter()
            .find(|line| line.contains("h264_vaapi"))
            .expect("a probe must have run");
        for expected in [
            "-vaapi_device /dev/dri/renderD129",
            "-vf format=nv12,hwupload",
            "-c:v h264_vaapi",
            "-f lavfi",
            "color=c=black",
            "-f null",
        ] {
            assert!(probe_line.contains(expected), "missing {expected}: {probe_line}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn initialisation_probes_at_most_once() {
        // GIVEN an ffmpeg that lists nothing usable, so the verdict is None and
        // the process-wide cache cannot leak a hardware plan into other tests
        let temp = TempDir::new().unwrap();
        let log = temp.path().join("args.log");
        let ffmpeg = write_fake_ffmpeg(
            temp.path(),
            &format!(
                "printf '%s\\n' \"$*\" >> '{}'\n\
                 printf '%s\\n' ' V....D libx264              H.264 (libx264)'\n\
                 exit 0\n",
                log.display()
            ),
        );
        let _guard = TestEnvGuard::set("FFMPEG_PATH", &ffmpeg);

        // WHEN the process initialises twice
        let first = init().await;
        let second = init().await;

        // THEN the verdict is stable and ffmpeg ran exactly once
        assert_eq!(first, None);
        assert_eq!(second, None);
        assert_eq!(args_log(&log).len(), 1, "init must probe at most once");
        assert_eq!(active(), None);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib video_encoder`
Expected: compile errors — `cannot find function detect`, `cannot find function init`, `cannot find function active`; and `make_executable` is private.

- [ ] **Step 3: Implement detection and the verdict**

In `src/video_processor.rs`, widen the test helper (line ~2025) so the new module's tests can reuse it instead of copying it:

```rust
    #[cfg(unix)]
    pub(crate) fn make_executable(path: &Path) {
```

Add to `src/video_encoder.rs` (above `#[cfg(test)]`):

```rust
/// The process's verdict, resolved once. `None` is a real verdict ("no usable
/// hardware encoder"), which is why the cell holds an `Option<HwPlan>` rather
/// than a plan only.
static ACTIVE_PLAN: OnceLock<Option<HwPlan>> = OnceLock::new();

/// Resolve the hardware encoder once per process and remember the verdict.
///
/// Called from `main` before the server binds, so the verdict is in the log
/// before the first job and no job pays for the probe. Calling it again is
/// cheap and never re-probes. Not for tests: the verdict is process-global, so
/// tests inject a plan explicitly wherever one is needed.
pub async fn init() -> Option<HwPlan> {
    if ACTIVE_PLAN.get().is_none() {
        let plan = detect(
            &crate::video_processor::get_ffmpeg_path(),
            &render_nodes(),
        )
        .await;
        // A lost race changes nothing: detection is a pure function of this
        // machine, so both callers computed the same verdict.
        let _ = ACTIVE_PLAN.set(plan);
        match active() {
            Some(plan) => log::info!("Hardware video encoder: {}", plan.label()),
            None => log::info!(
                "No usable hardware video encoder; transcoding uses {SOFTWARE_ENCODER}"
            ),
        }
    }
    active()
}

/// The verdict from [`init`], or `None` when it never ran or found nothing
/// usable. Both transcoding paths fall back to the software encoder on `None`.
pub fn active() -> Option<HwPlan> {
    ACTIVE_PLAN.get().cloned().flatten()
}

/// Probe every candidate in preference order and return the first that encodes.
///
/// `nodes` is the render-node list to probe (injected so tests do not depend on
/// the host's `/dev/dri`).
pub async fn detect(ffmpeg: &str, nodes: &[String]) -> Option<HwPlan> {
    let Some(listing) = encoder_listing(ffmpeg).await else {
        log::debug!("Could not list ffmpeg encoders; using {SOFTWARE_ENCODER}");
        return None;
    };

    for encoder in HwEncoder::ALL {
        if !lists_encoder(&listing, encoder.name()) {
            log::debug!("{} is not compiled into ffmpeg; skipping", encoder.name());
            continue;
        }
        if encoder.needs_render_node() && nodes.is_empty() {
            log::debug!("{} needs a DRM render node; none present", encoder.name());
            continue;
        }
        let devices: Vec<Option<String>> = if encoder.needs_render_node() {
            nodes.iter().cloned().map(Some).collect()
        } else {
            vec![None]
        };
        for device in devices {
            let plan = HwPlan::new(encoder, device);
            if probe(ffmpeg, &plan).await {
                return Some(plan);
            }
        }
    }
    None
}

/// ffmpeg's encoder table, or `None` when ffmpeg cannot be run at all.
async fn encoder_listing(ffmpeg: &str) -> Option<String> {
    let output = timeout(
        PROBE_TIMEOUT,
        Command::new(ffmpeg)
            .args(["-hide_banner", "-encoders"])
            .stdin(Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    String::from_utf8(output.stdout).ok()
}

/// DRM render nodes, ascending. A machine can expose several (an Intel iGPU and
/// a discrete card), the usable one is not necessarily the first, and a node
/// that has no encode entrypoint must not disqualify the backend.
fn render_nodes() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/dev/dri") else {
        return Vec::new();
    };
    let mut nodes: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("renderD"))
        .map(|name| format!("/dev/dri/{name}"))
        .collect();
    nodes.sort();
    nodes
}

/// Run one throwaway encode through the exact argument shape a real job uses.
async fn probe(ffmpeg: &str, plan: &HwPlan) -> bool {
    let mut command = Command::new(ffmpeg);
    command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
        .args(plan.input_args())
        .args(["-f", "lavfi", "-i", PROBE_INPUT]);
    if let Some(filter) = plan.upload_filter_args() {
        command.args(filter);
    }
    command
        .args(plan.video_args())
        .args(["-frames:v", "3", "-y", "-f", "null", "-"]);

    let child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            log::debug!("Could not run ffmpeg for the {} probe: {e}", plan.label());
            return false;
        }
    };

    match timeout(PROBE_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) if output.status.success() => true,
        Ok(Ok(output)) => {
            log::debug!(
                "{} failed its encode probe: {}",
                plan.label(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
            false
        }
        Ok(Err(e)) => {
            log::debug!("{} probe could not be waited on: {e}", plan.label());
            false
        }
        Err(_) => {
            log::debug!("{} probe timed out after {PROBE_TIMEOUT:?}", plan.label());
            false
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib video_encoder`
Expected: 18 tests pass (10 from Task 1 + 8 here).

- [ ] **Step 5: Wire the verdict into startup**

In `src/main.rs`, add `use turbo_pix::video_encoder;` to the `use turbo_pix::…` block (keep alphabetical order — after `video_processor`), and insert this block after the `TURBO_PIX_TRANSCODE_TIMEOUT_SECS` defaulting and before the non-loopback warning:

```rust
    // Decide the hardware H.264 encoder once, before any job runs. The probe is
    // a real encode rather than ffmpeg's encoder listing, which is what keeps a
    // listed-but-unusable encoder from burning a conversion attempt later.
    // Skipped when transcoding is disabled: nothing would ever ask for it.
    if video_processor::transcode_max_pool() > 0 {
        video_encoder::init().await;
    }
```

- [ ] **Step 6: Verify the server still starts and reports the verdict**

```bash
cargo build --bin turbo-pix
RUST_LOG=info,ffmpeg=debug ./target/debug/turbo-pix 2>&1 &
sleep 5 && curl -s http://localhost:18473/health && kill %1
```

Expected: the log contains exactly one `Hardware video encoder: …` or `No usable hardware video encoder; …` line. On this workstation it must read `Hardware video encoder: h264_vaapi (/dev/dri/renderD129)` (the Intel render node; `renderD128` is the AMD node, which is listed by ffmpeg but has no H.264 encode entrypoint). If the data directory has no models yet, pre-seed with `./target/debug/turbo-pix --download-models` first.

- [ ] **Step 7: Lint, format, commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --lib video_encoder
git add src/video_encoder.rs src/video_processor.rs src/main.rs
git commit -m "feat(video): probe hardware encoders once at startup and cache the verdict"
```

---

## Task 3: Whole-file conversions use the probed encoder, with a software fallback

**Files:**
- Modify: `src/video_processor.rs` (`TranscodeStatus`, `build_conversion_args`, `convert_video_with_progress`, `convert_video_with_timeout_and_path` → `convert_attempt` + `convert_with_fallback`, `ConversionOutcome`, tests)
- Modify: `src/handlers_video.rs` (status construction in `spawn_whole_file_transcode`)
- Test: `src/video_processor.rs` (`mod tests`)

**Interfaces:**
- Consumes: `video_encoder::{active, HwPlan, SOFTWARE_ENCODER}`, existing `transcode_timeout_secs`, `get_ffmpeg_path`, `acquire_transcode_permit`, `ProgressParser`.
- Produces:
  - `pub struct ConversionOutcome { pub encoder: String, pub fell_back: bool }`
  - `pub async fn convert_video_with_progress(…, on_progress: Arc<dyn Fn(Option<u8>) + Send + Sync>) -> CacheResult<ConversionOutcome>` (return type changes)
  - `pub fn build_conversion_args(input, output, conversion, codecs, with_progress, plan: Option<&HwPlan>) -> Vec<String>`
  - `async fn convert_with_fallback(…, plan: Option<HwPlan>) -> CacheResult<ConversionOutcome>` (private; the tests' entry point)
  - `TranscodeStatus` gains `pub encoder: Option<String>`

- [ ] **Step 1: Write the failing tests**

Add to `src/video_processor.rs`'s `mod tests` (add `use crate::video_encoder::HwEncoder;` to that module's imports — `HwPlan` and `SOFTWARE_ENCODER` already arrive through `use super::*`):

```rust
    #[test]
    fn reencode_args_are_unchanged_without_a_plan() {
        // GIVEN the software path (no plan, exactly what a GPU-less host uses)
        let args = build_conversion_args(
            Path::new("/in.mp4"),
            Path::new("/out.mp4"),
            FileConversion::Reencode,
            SourceCodecs { video: Some("hevc"), audio: Some("aac") },
            false,
            None,
        )
        .join(" ");

        // THEN the ffmpeg invocation is byte-for-byte the historical one
        assert_eq!(
            args,
            "-hwaccel auto -i /in.mp4 -map 0:v:0 -map 0:a:0? -c:v libx264 -preset fast -crf 23 \
             -c:a copy -movflags +faststart -y -f mp4 /out.mp4",
        );
    }

    #[test]
    fn reencode_args_swap_in_the_hardware_encoder() {
        // GIVEN a VAAPI plan on a render node
        let plan = HwPlan::new(HwEncoder::Vaapi, Some("/dev/dri/renderD129".to_string()));

        // WHEN the conversion arguments are built
        let args = build_conversion_args(
            Path::new("/in.mp4"),
            Path::new("/out.mp4"),
            FileConversion::Reencode,
            SourceCodecs { video: Some("hevc"), audio: Some("aac") },
            false,
            Some(&plan),
        )
        .join(" ");

        // THEN the device precedes the input, frames are uploaded, the encoder
        // is VAAPI's and no libx264 flag survives
        let device_index = args.find("-vaapi_device").expect("device missing");
        let input_index = args.find("-i /in.mp4").expect("input missing");
        assert!(device_index < input_index, "device must precede -i: {args}");
        assert!(args.contains("-vf format=nv12,hwupload"), "{args}");
        assert!(args.contains("-c:v h264_vaapi"), "{args}");
        assert!(args.contains("-qp 23"), "{args}");
        assert!(!args.contains("libx264"), "{args}");
        assert!(!args.contains("-crf"), "{args}");
        // AND the container and audio handling are untouched
        assert!(args.contains("-movflags +faststart"), "{args}");
        assert!(args.contains("-f mp4"), "{args}");
    }

    #[test]
    fn video_copy_args_ignore_the_plan() {
        // GIVEN a plan and a conversion that only remuxes the video track
        let plan = HwPlan::new(HwEncoder::Nvenc, None);

        // WHEN the arguments are built
        let args = build_conversion_args(
            Path::new("/in.mp4"),
            Path::new("/out.mp4"),
            FileConversion::VideoCopy,
            SourceCodecs { video: Some("hevc"), audio: Some("ac3") },
            false,
            Some(&plan),
        )
        .join(" ");

        // THEN nothing about the encoder changes: a copy never encodes
        assert!(args.contains("-c:v copy"), "{args}");
        assert!(!args.contains("h264_nvenc"), "{args}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn hardware_failure_falls_back_to_the_software_encoder() {
        // GIVEN an ffmpeg whose hardware encoder refuses every source
        let temp = TempDir::new().unwrap();
        let log = temp.path().join("args.log");
        let ffmpeg = temp.path().join("fake-ffmpeg.sh");
        std::fs::write(
            &ffmpeg,
            format!(
                "#!/usr/bin/env sh\n\
                 printf '%s\\n' \"$*\" >> '{}'\n\
                 for last; do :; done\n\
                 case \"$*\" in\n\
                 *h264_vaapi*) exit 1 ;;\n\
                 esac\n\
                 touch \"$last\"\n\
                 exit 0\n",
                log.display()
            ),
        )
        .unwrap();
        crate::video_processor::tests::make_executable(&ffmpeg);
        let plan = HwPlan::new(HwEncoder::Vaapi, Some("/dev/dri/renderD129".to_string()));
        let output = temp.path().join("out.mp4");

        // WHEN the conversion runs
        let outcome = convert_with_fallback(
            Path::new("test-data/test_video_hevc.mp4"),
            &output,
            FileConversion::Reencode,
            SourceCodecs { video: Some("hevc"), audio: None },
            Duration::from_secs(30),
            ffmpeg.to_string_lossy().into_owned(),
            None,
            Some(plan),
        )
        .await
        .expect("the software retry must finish the job");

        // THEN the artifact exists, the outcome names the software encoder, and
        // both attempts were made against the same output
        assert!(output.exists(), "the fallback artifact must exist");
        assert_eq!(outcome.encoder, "libx264");
        assert!(outcome.fell_back);
        let lines = std::fs::read_to_string(&log).unwrap();
        assert!(lines.lines().any(|line| line.contains("h264_vaapi")), "{lines}");
        assert!(lines.lines().any(|line| line.contains("libx264")), "{lines}");
        // AND no temp file is left behind
        assert!(!output.with_extension("mp4.tmp").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_successful_hardware_conversion_reports_the_hardware_encoder() {
        // GIVEN an ffmpeg whose hardware encoder works
        let temp = TempDir::new().unwrap();
        let ffmpeg = temp.path().join("fake-ffmpeg.sh");
        std::fs::write(
            &ffmpeg,
            "#!/usr/bin/env sh\nfor last; do :; done\ntouch \"$last\"\nexit 0\n",
        )
        .unwrap();
        crate::video_processor::tests::make_executable(&ffmpeg);
        let plan = HwPlan::new(HwEncoder::Vaapi, Some("/dev/dri/renderD129".to_string()));
        let output = temp.path().join("out.mp4");

        // WHEN the conversion runs
        let outcome = convert_with_fallback(
            Path::new("test-data/test_video_hevc.mp4"),
            &output,
            FileConversion::Reencode,
            SourceCodecs { video: Some("hevc"), audio: None },
            Duration::from_secs(30),
            ffmpeg.to_string_lossy().into_owned(),
            None,
            Some(plan),
        )
        .await
        .expect("the hardware conversion must succeed");

        // THEN the outcome names it and reports no fallback
        assert_eq!(outcome.encoder, "h264_vaapi");
        assert!(!outcome.fell_back);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failed_fallback_reports_the_software_error_and_leaves_no_debris() {
        // GIVEN an ffmpeg that fails for every encoder
        let temp = TempDir::new().unwrap();
        let ffmpeg = temp.path().join("fake-ffmpeg.sh");
        std::fs::write(
            &ffmpeg,
            "#!/usr/bin/env sh\nprintf '%s\\n' 'boom' >&2\nexit 1\n",
        )
        .unwrap();
        crate::video_processor::tests::make_executable(&ffmpeg);
        let plan = HwPlan::new(HwEncoder::Vaapi, Some("/dev/dri/renderD129".to_string()));
        let output = temp.path().join("out.mp4");

        // WHEN the conversion runs
        let result = convert_with_fallback(
            Path::new("test-data/test_video_hevc.mp4"),
            &output,
            FileConversion::Reencode,
            SourceCodecs { video: Some("hevc"), audio: None },
            Duration::from_secs(30),
            ffmpeg.to_string_lossy().into_owned(),
            None,
            Some(plan),
        )
        .await;

        // THEN the failure surfaces and nothing is left at the output path
        assert!(result.is_err(), "both attempts failed, so the job fails");
        assert!(!output.exists(), "no partial artifact may survive");
        assert!(!output.with_extension("mp4.tmp").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_timed_out_hardware_attempt_is_not_retried_in_software() {
        // GIVEN an ffmpeg whose hardware attempt hangs past the deadline
        let temp = TempDir::new().unwrap();
        let log = temp.path().join("args.log");
        let ffmpeg = temp.path().join("fake-ffmpeg.sh");
        std::fs::write(
            &ffmpeg,
            format!(
                "#!/usr/bin/env sh\n\
                 printf '%s\\n' \"$*\" >> '{}'\n\
                 case \"$*\" in\n\
                 *h264_vaapi*) sleep 30 ;;\n\
                 esac\n\
                 for last; do :; done\n\
                 touch \"$last\"\n",
                log.display()
            ),
        )
        .unwrap();
        crate::video_processor::tests::make_executable(&ffmpeg);
        let plan = HwPlan::new(HwEncoder::Vaapi, Some("/dev/dri/renderD129".to_string()));
        let output = temp.path().join("out.mp4");

        // WHEN the conversion runs out of time
        let result = convert_with_fallback(
            Path::new("test-data/test_video_hevc.mp4"),
            &output,
            FileConversion::Reencode,
            SourceCodecs { video: Some("hevc"), audio: None },
            Duration::from_secs(1),
            ffmpeg.to_string_lossy().into_owned(),
            None,
            Some(plan),
        )
        .await;

        // THEN it fails without spending a second budget: the hardware attempt
        // already consumed the whole per-transcode deadline
        assert!(result.is_err());
        let lines = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            lines.lines().filter(|line| !line.is_empty()).count(),
            1,
            "a timeout must not trigger a second attempt: {lines}"
        );
    }

    #[test]
    fn progress_never_goes_backwards_across_a_fallback() {
        // GIVEN a high-water wrapper shared by two attempts
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_clone = Arc::clone(&seen);
        let callback: Arc<dyn Fn(Option<u8>) + Send + Sync> =
            Arc::new(move |percent| seen_clone.lock().unwrap().push(percent));
        let wrapper = ProgressHighWater::default().wrap(Some(callback));

        // WHEN the hardware attempt reports 30 and the software retry restarts
        // from zero and climbs to 40
        for percent in [Some(30), Some(10), Some(20), Some(40)] {
            wrapper.as_ref().unwrap()(percent);
        }

        // THEN only the forward-moving values reach the client
        assert_eq!(*seen.lock().unwrap(), vec![Some(30), Some(40)]);
    }
```

Also update the three existing tests that call `convert_video_with_timeout_and_path` (lines ~2141, ~2188, ~2229) to call `convert_with_fallback(…, None)` with the same arguments plus the trailing `None` plan, and update the `build_conversion_args` test helper closure (~2278) to pass `None` as the last argument. Update `test_transcode_status_json` (~2375) to construct the status with `encoder: Some("h264_vaapi".to_string())` and assert `json.contains("\"encoder\":\"h264_vaapi\"")`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib video_processor`
Expected: compile errors — `build_conversion_args` takes 5 arguments, `convert_with_fallback` and `ProgressHighWater` not found, `TranscodeStatus` missing field `encoder`.

- [ ] **Step 3: Implement the outcome, the plan parameter and the fallback**

In `src/video_processor.rs`:

1. Import the new module types: add `use crate::video_encoder::{self, HwPlan, SOFTWARE_ENCODER};` to the local-import block, and `use std::sync::atomic::{AtomicU8, Ordering};` to the std block (atomic is already imported for `AtomicU64` — extend that line).

2. Extend `TranscodeStatus`:

```rust
#[derive(Serialize, Clone, Debug)]
pub struct TranscodeStatus {
    pub state: TranscodeState,
    pub hash: String,
    pub started_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub percent: Option<u8>,
    /// ffmpeg encoder that produced the artifact (`libx264` or a hardware
    /// encoder), set on `Completed`. `None` while unknown.
    pub encoder: Option<String>,
}
```

and add `encoder: None` to the two other `TranscodeStatus` literals in this file (`in_progress_status`, `test_transcode_status_json`).

3. Add the outcome type above `convert_video_with_progress`:

```rust
/// What a finished conversion actually did, so the caller can report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionOutcome {
    /// The ffmpeg encoder that produced the artifact.
    pub encoder: String,
    /// True when a hardware attempt failed and the software encoder finished.
    pub fell_back: bool,
}
```

4. Replace `convert_video_with_progress`'s body with the plan-aware entry point:

```rust
pub async fn convert_video_with_progress(
    input_path: &Path,
    output_path: &Path,
    conversion: FileConversion,
    codecs: SourceCodecs<'_>,
    on_progress: Arc<dyn Fn(Option<u8>) + Send + Sync>,
) -> CacheResult<ConversionOutcome> {
    // A copy never encodes, so it never asks for a hardware encoder.
    let plan = match conversion {
        FileConversion::Reencode => video_encoder::active(),
        FileConversion::VideoCopy => None,
    };
    convert_with_fallback(
        input_path,
        output_path,
        conversion,
        codecs,
        Duration::from_secs(transcode_timeout_secs()),
        get_ffmpeg_path(),
        Some(on_progress),
        plan,
    )
    .await
}
```

5. Rename `convert_video_with_timeout_and_path` to `convert_attempt`, give it a trailing `plan: Option<&HwPlan>` parameter, and return the attempt outcome instead of `CacheResult<()>`:

```rust
/// One conversion attempt, with its failure modes kept apart: the caller
/// retries an ordinary failure in software, but a timeout has already spent the
/// whole per-transcode budget and must not spend a second one.
enum Attempt {
    Done,
    Failed(String),
    TimedOut(String),
}
```

Replace the whole function with this (the body is the existing one, with every `CacheError` return turned into the matching `Attempt` and the `plan` threaded into `build_conversion_args`):

```rust
async fn convert_attempt(
    input_path: &Path,
    output_path: &Path,
    conversion: FileConversion,
    codecs: SourceCodecs<'_>,
    timeout_duration: Duration,
    ffmpeg_path: String,
    on_progress: Option<Arc<dyn Fn(Option<u8>) + Send + Sync>>,
    plan: Option<&HwPlan>,
) -> Attempt {
    // Write to a temp file in the SAME directory as the final path so the
    // completed file can be atomically renamed into place. A failed or
    // timed-out transcode must never leave a partial file at `output_path`,
    // which callers would otherwise treat as valid video.
    let temp_output_path = output_path.with_extension("mp4.tmp");
    let output_path_owned = output_path.to_path_buf();

    let inner = async {
        let _permit = match acquire_transcode_permit().await {
            Ok(permit) => permit,
            Err(e) => return Attempt::Failed(e.to_string()),
        };

        // Create output directory if it doesn't exist
        if let Some(parent) = temp_output_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Attempt::Failed(format!("Failed to create output directory: {e}"));
            }
        }

        // Probe the input duration once so progress can be expressed as a
        // percentage. A best-effort probe: if it fails, percent is unknown and
        // the client is told "working" with no number.
        let with_progress = on_progress.is_some();
        let duration = if with_progress {
            extract_video_metadata(input_path)
                .await
                .ok()
                .map(|m| m.duration)
        } else {
            None
        };
        let mut progress = ProgressParser {
            duration,
            last_percent: 0,
            on_progress,
        };
        if duration.is_none() {
            progress.signal_unknown();
        }

        // `build_conversion_args` decides how the video and audio tracks are
        // handled; `-progress pipe:1` (when someone consumes it) streams
        // key=value progress lines to stdout, which we read incrementally to
        // report percent. `plan` picks the encoder: `Some` replaces the libx264
        // flags with a hardware encoder, `None` is the software path.
        let ffmpeg_path_for_err = ffmpeg_path.clone();
        let mut command = TokioCommand::new(ffmpeg_path);
        command
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .args(build_conversion_args(
                input_path,
                &temp_output_path,
                conversion,
                codecs,
                with_progress,
                plan,
            ));

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                return Attempt::Failed(format_binary_error("ffmpeg", &ffmpeg_path_for_err, &e))
            }
        };

        // Drain stdout (progress) incrementally; collect stderr for the error
        // message so a long ffmpeg run cannot deadlock on a full pipe.
        let Some(stdout) = child.stdout.take() else {
            return Attempt::Failed("ffmpeg stdout pipe unavailable".to_string());
        };
        let Some(stderr) = child.stderr.take() else {
            return Attempt::Failed("ffmpeg stderr pipe unavailable".to_string());
        };

        let mut stderr_reader = BufReader::new(stderr);
        let stderr_handle = tokio::spawn(async move {
            let mut buf = String::new();
            let _ = stderr_reader.read_to_string(&mut buf).await;
            buf
        });

        let mut stdout_reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            match stdout_reader.read_line(&mut line).await {
                Ok(0) => break, // EOF: ffmpeg closed stdout
                Ok(_) => progress.handle_line(&line),
                Err(e) => {
                    // A read error on the progress pipe is non-fatal: the
                    // transcode's real outcome comes from the exit status.
                    log::debug!("ffmpeg progress pipe read error: {e}");
                    break;
                }
            }
        }

        // Wait for ffmpeg to finish, then join the stderr collector.
        let status = match child.wait().await {
            Ok(status) => status,
            Err(e) => {
                return Attempt::Failed(format_binary_error("ffmpeg", &ffmpeg_path_for_err, &e))
            }
        };
        let stderr = stderr_handle.await.unwrap_or_default();

        if !status.success() {
            log::error!("FFmpeg transcoding failed!");
            log::error!("FFmpeg stderr: {}", stderr);
            let _ = std::fs::remove_file(&temp_output_path);
            return Attempt::Failed(format!(
                "ffmpeg transcode exited with status {}. stderr: {}",
                status, stderr
            ));
        }

        // Move the completed temp file into place (atomic on the same filesystem).
        if let Err(e) = std::fs::rename(&temp_output_path, &output_path_owned) {
            let _ = std::fs::remove_file(&temp_output_path);
            return Attempt::Failed(format!(
                "Failed to move transcoded video into place: {}",
                e
            ));
        }

        Attempt::Done
    };

    match timeout(timeout_duration, inner).await {
        Ok(attempt) => attempt,
        Err(_) => {
            // The inner future (and with it the ffmpeg child, via kill_on_drop)
            // has been dropped; remove whatever partial output it wrote.
            let _ = std::fs::remove_file(&temp_output_path);
            Attempt::TimedOut(format!(
                "Transcoding timed out after {}s",
                timeout_duration.as_secs()
            ))
        }
    }
}
```

6. Add the fallback wrapper right after `convert_attempt`:

```rust
/// Progress callback shared by every attempt of one job, so the software retry
/// cannot lower the percentage the client already saw: each attempt's parser
/// counts from zero, the high-water mark does not.
#[derive(Clone, Default)]
struct ProgressHighWater {
    last: Arc<AtomicU8>,
}

impl ProgressHighWater {
    fn wrap(
        &self,
        callback: Option<Arc<dyn Fn(Option<u8>) + Send + Sync>>,
    ) -> Option<Arc<dyn Fn(Option<u8>) + Send + Sync>> {
        let last = Arc::clone(&self.last);
        callback.map(move |callback| {
            Arc::new(move |percent: Option<u8>| match percent {
                Some(percent) => {
                    if last.fetch_max(percent, Ordering::Relaxed) < percent {
                        callback(Some(percent));
                    }
                }
                None => callback(None),
            }) as Arc<dyn Fn(Option<u8>) + Send + Sync>
        })
    }
}

/// Convert with the hardware plan first, finishing in software when that
/// attempt fails.
///
/// The probe can prove that a hardware encoder opens and encodes, but not that
/// it will accept *this* source on *this* day: the device can be busy, gone, or
/// refusing the pixel format. The user must still get their video, so an
/// ordinary failure is retried once without the plan. A timeout is not retried
/// — the budget is already spent.
async fn convert_with_fallback(
    input_path: &Path,
    output_path: &Path,
    conversion: FileConversion,
    codecs: SourceCodecs<'_>,
    timeout_duration: Duration,
    ffmpeg_path: String,
    on_progress: Option<Arc<dyn Fn(Option<u8>) + Send + Sync>>,
    plan: Option<HwPlan>,
) -> CacheResult<ConversionOutcome> {
    let progress = ProgressHighWater::default();
    let Some(plan) = plan else {
        return match convert_attempt(
            input_path,
            output_path,
            conversion,
            codecs,
            timeout_duration,
            ffmpeg_path,
            progress.wrap(on_progress),
            None,
        )
        .await
        {
            Attempt::Done => Ok(ConversionOutcome {
                encoder: SOFTWARE_ENCODER.to_string(),
                fell_back: false,
            }),
            Attempt::Failed(message) | Attempt::TimedOut(message) => {
                Err(CacheError::VideoProcessingError(message))
            }
        };
    };

    let first = convert_attempt(
        input_path,
        output_path,
        conversion,
        codecs,
        timeout_duration,
        ffmpeg_path.clone(),
        progress.wrap(on_progress.clone()),
        Some(&plan),
    )
    .await;

    match first {
        Attempt::Done => Ok(ConversionOutcome {
            encoder: plan.encoder().name().to_string(),
            fell_back: false,
        }),
        Attempt::TimedOut(message) => Err(CacheError::VideoProcessingError(message)),
        Attempt::Failed(message) => {
            log::warn!(
                "Hardware encoder {} failed ({}); retrying with {}",
                plan.label(),
                message,
                SOFTWARE_ENCODER
            );
            match convert_attempt(
                input_path,
                output_path,
                conversion,
                codecs,
                timeout_duration,
                ffmpeg_path,
                progress.wrap(on_progress),
                None,
            )
            .await
            {
                Attempt::Done => Ok(ConversionOutcome {
                    encoder: SOFTWARE_ENCODER.to_string(),
                    fell_back: true,
                }),
                Attempt::Failed(message) | Attempt::TimedOut(message) => {
                    Err(CacheError::VideoProcessingError(message))
                }
            }
        }
    }
}
```

7. Delete `convert_video_with_timeout_and_path` (its behaviour now lives in `convert_attempt` + the wrapper; keeping an unused adapter would be dead code).

8. Give `build_conversion_args` the plan and use it in the `Reencode` arm:

```rust
pub fn build_conversion_args(
    input: &Path,
    output: &Path,
    conversion: FileConversion,
    codecs: SourceCodecs<'_>,
    with_progress: bool,
    plan: Option<&HwPlan>,
) -> Vec<String> {
```

```rust
        FileConversion::Reencode => {
            // Hardware-accelerated decoding is worth asking for only when the
            // video is actually decoded (a copy never is). `auto` delegates
            // when the source is decodable in hardware and stays in software
            // otherwise, so the encoder decision is the one that needs probing.
            args.splice(0..0, ["-hwaccel", "auto"].iter().map(|arg| arg.to_string()));
            match plan {
                Some(plan) => {
                    // Device selection precedes the input; the upload filter and
                    // the encoder options belong to the output.
                    args.splice(0..0, plan.input_args());
                    if let Some(filter) = plan.upload_filter_args() {
                        args.extend(filter);
                    }
                    args.extend(plan.video_args());
                    log::info!("Converting with hardware encoder {}", plan.label());
                }
                None => args.extend(
                    [
                        "-c:v", "libx264", // More widely available than libopenh264
                        "-preset", "fast", // Good for real-time transcoding
                        "-crf", "23", // 18-28, lower = better quality
                    ]
                    .iter()
                    .map(|arg| arg.to_string()),
                ),
            }
        }
```

In `src/handlers_video.rs`, the `Ok(_)` arm of the spawn becomes `Ok(outcome)` and the Completed status carries the encoder:

```rust
                    TranscodeStatus {
                        state: TranscodeState::Completed,
                        hash: hash.clone(),
                        started_at: Some(started_at),
                        error: None,
                        percent: Some(100),
                        encoder: Some(outcome.encoder.clone()),
                    },
```

and the `InProgress` callback plus the `Err(e)` arm get `encoder: None`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib video_processor`
Expected: all pass, including the 8 new tests and the updated existing ones. The fake-ffmpeg tests need no real ffmpeg; the `#[cfg(unix)]` ones skip nothing on Linux.

- [ ] **Step 5: Run the whole library suite and the handler tests**

Run: `cargo test --lib`
Expected: green. Any test that asserted the old `TranscodeStatus` JSON or the old `convert_video_with_progress` return type must be updated in this task — no test may be deleted to make this pass.

- [ ] **Step 6: Lint, format, commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add src/video_processor.rs src/handlers_video.rs
git commit -m "feat(video): run whole-file conversions on the probed encoder with libx264 fallback"
```

---

## Task 4: The live transcode rung uses the probed encoder, with a first-byte gate

**Files:**
- Modify: `src/video_stream.rs` (`build_args`, `start_stream`, `ProgressReader`, imports)
- Modify: `src/handlers_video.rs` (only if `start_stream`'s signature changes at its call site — it does not, but re-run its tests)
- Test: `src/video_stream.rs` (`mod tests`)

**Interfaces:**
- Consumes: `video_encoder::{active, HwPlan}`.
- Produces:
  - `pub fn build_args(mode, input, start_secs, video_codec, plan: Option<&HwPlan>) -> Vec<String>`
  - `pub async fn start_stream(mode, input, start_secs, video_codec) -> Result<StreamHandle, StreamStartError>` (unchanged signature; resolves the plan itself for `StreamMode::Transcode`)
  - `async fn start_stream_with_plan(mode, input, start_secs, video_codec, plan: Option<HwPlan>) -> Result<StreamHandle, StreamStartError>` (the test seam)
  - `const FIRST_BYTES_TIMEOUT: Duration`

- [ ] **Step 1: Write the failing tests**

Add to `src/video_stream.rs`'s `mod tests` (add `use crate::video_encoder::{HwEncoder, HwPlan};`, `use crate::video_processor::tests::{make_executable, TestEnvGuard};`, `use tempfile::TempDir;` and `use crate::video_processor::clear_transcode_status;` where the module does not already have them):

```rust
    #[test]
    fn transcode_args_are_unchanged_without_a_plan() {
        // GIVEN the software path
        let args = build_args(StreamMode::Transcode, Path::new("/in.mp4"), 0.0, "hevc", None);

        // THEN the historical argument vector is intact
        assert!(args.contains(&"libx264".to_string()));
        assert!(args.contains(&"veryfast".to_string()));
        assert!(!args.iter().any(|arg| arg.starts_with("h264_")));
        assert!(!args.contains(&"-vaapi_device".to_string()));
    }

    #[test]
    fn transcode_args_swap_in_the_hardware_encoder() {
        // GIVEN a VAAPI plan
        let plan = HwPlan::new(HwEncoder::Vaapi, Some("/dev/dri/renderD129".to_string()));

        // WHEN the stream arguments are built
        let args = build_args(
            StreamMode::Transcode,
            Path::new("/in.mp4"),
            0.0,
            "hevc",
            Some(&plan),
        );
        let joined = args.join(" ");

        // THEN the device precedes the input, frames are uploaded, and the
        // client-visible profile and fragmented-output contract are untouched
        assert!(joined.find("-vaapi_device").unwrap() < joined.find("-i /in.mp4").unwrap());
        assert!(joined.contains("-vf format=nv12,hwupload"), "{joined}");
        assert!(joined.contains("-c:v h264_vaapi"), "{joined}");
        assert!(!joined.contains("libx264"), "{joined}");
        assert!(joined.contains("-profile:v main"), "{joined}");
        assert!(joined.contains("-pix_fmt yuv420p"), "{joined}");
        assert!(
            joined.contains("-movflags frag_keyframe+empty_moov+default_base_moof+omit_tfhd_offset+delay_moov"),
            "{joined}"
        );
    }

    #[test]
    fn copy_modes_ignore_the_plan() {
        // GIVEN a plan and the two modes that do not encode video
        let plan = HwPlan::new(HwEncoder::Nvenc, None);

        // THEN neither picks up a hardware encoder
        for mode in [StreamMode::Remux, StreamMode::Audio] {
            let joined = build_args(mode, Path::new("/in.mkv"), 0.0, "hevc", Some(&plan)).join(" ");
            assert!(!joined.contains("h264_nvenc"), "{mode:?}: {joined}");
            assert!(!joined.contains("-vaapi_device"), "{mode:?}: {joined}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn immediate_hardware_failure_respawns_in_software() {
        // GIVEN an ffmpeg whose hardware encoder cannot open the source but
        // whose software encoder streams fine
        let temp = TempDir::new().unwrap();
        let log = temp.path().join("args.log");
        let ffmpeg = temp.path().join("fake-ffmpeg.sh");
        std::fs::write(
            &ffmpeg,
            format!(
                "#!/usr/bin/env sh\n\
                 printf '%s\\n' \"$*\" >> '{}'\n\
                 case \"$*\" in\n\
                 *h264_vaapi*) printf '%s\\n' 'encoder refused' >&2; exit 1 ;;\n\
                 esac\n\
                 printf 'ftypsoftware-run'\n\
                 exit 0\n",
                log.display()
            ),
        )
        .unwrap();
        crate::video_processor::tests::make_executable(&ffmpeg);
        let _ffmpeg_guard = TestEnvGuard::set("FFMPEG_PATH", ffmpeg.to_str().unwrap());
        let fixture = Path::new("test-data/test_video_hevc.mp4");
        let plan = HwPlan::new(HwEncoder::Vaapi, Some("/dev/dri/renderD129".to_string()));

        // WHEN the stream starts with that plan
        let mut handle = start_stream_with_plan(StreamMode::Transcode, fixture, 0.0, "hevc", Some(plan))
            .await
            .expect("the software respawn must deliver a stream");

        // THEN the client gets the software run's bytes, from a run that was
        // launched without the hardware flags
        let mut head = vec![0u8; 16];
        tokio::io::AsyncReadExt::read_exact(&mut handle.stdout, &mut head)
            .await
            .expect("body must carry bytes");
        assert_eq!(&head, b"ftypsoftware-run");
        let lines: Vec<String> = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(lines.len(), 2, "expected a hardware attempt then a respawn: {lines:?}");
        assert!(lines[0].contains("h264_vaapi"), "{lines:?}");
        assert!(!lines[1].contains("h264_vaapi"), "{lines:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn first_bytes_are_preserved_for_the_body() {
        // GIVEN an ffmpeg that emits its init segment and then a marker
        let temp = TempDir::new().unwrap();
        let ffmpeg = temp.path().join("fake-ffmpeg.sh");
        std::fs::write(&ffmpeg, "#!/usr/bin/env sh\nprintf 'ftypmoovpayload'\nexit 0\n").unwrap();
        crate::video_processor::tests::make_executable(&ffmpeg);
        let _ffmpeg_guard = TestEnvGuard::set("FFMPEG_PATH", ffmpeg.to_str().unwrap());
        let fixture = Path::new("test-data/test_video_hevc.mp4");
        let plan = HwPlan::new(HwEncoder::Vaapi, Some("/dev/dri/renderD129".to_string()));

        // WHEN a hardware-planned stream starts (the gate therefore read the
        // first bytes before the body existed)
        let mut handle = start_stream_with_plan(StreamMode::Transcode, fixture, 0.0, "hevc", Some(plan))
            .await
            .expect("stream must start");

        // THEN the body still starts at the first byte, exactly once
        let mut all = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut handle.stdout, &mut all)
            .await
            .expect("body must be readable");
        assert_eq!(all, b"ftypmoovpayload");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn planless_stream_returns_the_childs_bytes_verbatim() {
        // GIVEN the software path (no plan, no gate)
        let temp = TempDir::new().unwrap();
        let ffmpeg = temp.path().join("fake-ffmpeg.sh");
        std::fs::write(&ffmpeg, "#!/usr/bin/env sh\nprintf 'ftypmoovpayload'\nexit 0\n").unwrap();
        crate::video_processor::tests::make_executable(&ffmpeg);
        let _ffmpeg_guard = TestEnvGuard::set("FFMPEG_PATH", ffmpeg.to_str().unwrap());

        // WHEN the stream starts
        let mut handle = start_stream_with_plan(
            StreamMode::Transcode,
            Path::new("test-data/test_video_hevc.mp4"),
            0.0,
            "hevc",
            None,
        )
        .await
        .expect("stream must start");

        // THEN the bytes are exactly the child's output
        let mut all = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut handle.stdout, &mut all)
            .await
            .expect("body must be readable");
        assert_eq!(all, b"ftypmoovpayload");
    }

    #[tokio::test]
    async fn spawn_error_still_maps_to_start_error() {
        // GIVEN a binary that does not exist
        let _ffmpeg_guard = TestEnvGuard::set("FFMPEG_PATH", "/nonexistent/ffmpeg");

        // WHEN a stream is started
        let result = start_stream_with_plan(
            StreamMode::Transcode,
            Path::new("test-data/test_video_hevc.mp4"),
            0.0,
            "hevc",
            None,
        )
        .await;

        // THEN it is reported as a spawn failure, not swallowed by the gate
        assert!(
            matches!(result, Err(StreamStartError::Spawn(_))),
            "expected a spawn error"
        );
    }
```

Note: the two fake-ffmpeg stream tests above exercise the gate without real ffmpeg. The six existing `build_args` call sites in this test module (`transcode_args_reencode_into_fragmented_mp4` at :486, `audio_mode_copies_video_and_reencodes_audio` at :503, `remux_mode_copies_everything` at :512, `copied_hevc_tracks_are_tagged_hvc1` at :523 and :531, `copy_modes_delay_the_moov` at :548) each gain a trailing `None` argument — their assertions stay exactly as they are, because they pin the software path this change must not alter. The existing `start_stream` tests need no change: `start_stream` resolves `active()`, which stays `None` in the test binary because no test initialises the process-wide verdict with a hardware plan.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib video_stream`
Expected: compile errors — `build_args` takes 4 arguments, `start_stream_with_plan` not found.

- [ ] **Step 3: Implement the plan-aware arguments and the gate**

In `src/video_stream.rs`:

1. Extend the imports: `use crate::video_encoder::{self, HwPlan};` and add `AsyncReadExt` to the existing `use tokio::io::{AsyncRead, ReadBuf};` line.

2. Add the constant next to `STALL_POLL_INTERVAL`:

```rust
/// Longest a hardware-planned run may take to hand over its first bytes before
/// it is treated as merely slow instead of broken. ffmpeg writes the fragmented
/// header as soon as the encoder opens, so a hardware encoder that cannot start
/// produces nothing and exits; a silent-but-alive run is left to the stall
/// watchdog, which knows how to tell "stuck" from "long".
const FIRST_BYTES_TIMEOUT: Duration = Duration::from_secs(10);
```

3. Give `build_args` the plan:

```rust
pub fn build_args(
    mode: StreamMode,
    input: &Path,
    start_secs: f64,
    video_codec: &str,
    plan: Option<&HwPlan>,
) -> Vec<String> {
    let mut args: Vec<String> = vec!["-v".into(), "error".into(), "-nostdin".into()];
    if mode == StreamMode::Transcode {
        args.extend(["-hwaccel".into(), "auto".into()]);
        // Device selection is an input option, like `-hwaccel`.
        if let Some(plan) = plan {
            args.splice(0..0, plan.input_args());
        }
    }
```

and the `Transcode` arm:

```rust
        StreamMode::Transcode => match plan {
            Some(plan) => {
                if let Some(filter) = plan.upload_filter_args() {
                    args.extend(filter);
                }
                args.extend(plan.video_args());
                args.extend(
                    [
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
                );
            }
            None => args.extend(
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
        },
```

4. Add prefix replay to `ProgressReader`:

```rust
#[derive(Debug)]
pub struct ProgressReader {
    /// Bytes read from the child before the response body existed (a
    /// hardware-planned run's first chunk, kept so the gate does not consume
    /// the head of the stream). Drained before `inner`.
    prefix: Vec<u8>,
    prefix_pos: usize,
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
        if this.prefix_pos < this.prefix.len() {
            let remaining = this.prefix.len() - this.prefix_pos;
            let take = remaining.min(buf.remaining());
            let end = this.prefix_pos + take;
            buf.put_slice(&this.prefix[this.prefix_pos..end]);
            this.prefix_pos = end;
            if this.prefix_pos == this.prefix.len() {
                this.prefix.clear();
                this.prefix_pos = 0;
            }
            this.stamp.touch();
            return Poll::Ready(Ok(()));
        }
        let polled = Pin::new(&mut this.inner).poll_read(cx, buf);
        if matches!(polled, Poll::Ready(Ok(()))) {
            this.stamp.touch();
        }
        polled
    }
}
```

5. Split `start_stream` into the seam and the production entry point, and add the gate:

```rust
/// Start one stream run. `video_codec` is the source's first video track as the
/// capability record resolved it (see [`build_args`]).
pub async fn start_stream(
    mode: StreamMode,
    input: &Path,
    start_secs: f64,
    video_codec: &str,
) -> Result<StreamHandle, StreamStartError> {
    // Only the transcode rung encodes, so only it can use a hardware encoder.
    let plan = match mode {
        StreamMode::Transcode => video_encoder::active(),
        _ => None,
    };
    start_stream_with_plan(mode, input, start_secs, video_codec, plan).await
}

/// [`start_stream`] with the encoder decision injected, so tests can exercise
/// the hardware path without initialising the process-wide verdict.
pub async fn start_stream_with_plan(
    mode: StreamMode,
    input: &Path,
    start_secs: f64,
    video_codec: &str,
    plan: Option<HwPlan>,
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
    let mut child = spawn_stream_child(&ffmpeg, mode, input, start_secs, video_codec, plan.as_ref())?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| StreamStartError::Spawn("ffmpeg stdout pipe unavailable".to_string()))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| StreamStartError::Spawn("ffmpeg stderr pipe unavailable".to_string()))?;

    let mut prefix = Vec::new();
    if plan.is_some() {
        // A hardware encoder that passed its probe can still refuse the real
        // source. That shows up as "died before the init segment", and it has
        // to be replaced here — once bytes have reached the body the run can no
        // longer be swapped, and the client would spend a ladder step on a
        // failure it cannot see coming.
        match take_first_bytes(&mut child, &mut stdout).await {
            Ok(first) => prefix = first,
            Err(()) => {
                log::warn!(
                    "Hardware encoder produced nothing for {}; retrying with the software encoder",
                    input.display()
                );
                drop(child);
                child = spawn_stream_child(&ffmpeg, mode, input, start_secs, video_codec, None)?;
                stdout = child.stdout.take().ok_or_else(|| {
                    StreamStartError::Spawn("ffmpeg stdout pipe unavailable".to_string())
                })?;
                stderr = child.stderr.take().ok_or_else(|| {
                    StreamStartError::Spawn("ffmpeg stderr pipe unavailable".to_string())
                })?;
            }
        }
    }

    let progress = ProgressStamp::new();
    Ok(StreamHandle {
        mode,
        stdout: ProgressReader {
            prefix,
            prefix_pos: 0,
            inner: stdout,
            stamp: progress.clone(),
        },
        stderr,
        child,
        permit,
        progress,
    })
}

/// Spawn one stream run, with the plan's device arguments when there is one.
fn spawn_stream_child(
    ffmpeg: &str,
    mode: StreamMode,
    input: &Path,
    start_secs: f64,
    video_codec: &str,
    plan: Option<&HwPlan>,
) -> Result<Child, StreamStartError> {
    Command::new(ffmpeg)
        .args(build_args(mode, input, start_secs, video_codec, plan))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // A client that hangs up mid-seek cancels the handler future before
        // `supervise` takes ownership of the child; without this the transient
        // ffmpeg would keep a conversion slot while streaming to nobody.
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| StreamStartError::Spawn(format_binary_error("ffmpeg", ffmpeg, &e)))
}

/// Wait for a hardware-planned run to hand over its first bytes.
///
/// `Err(())` means the run produced nothing and is not worth keeping (it either
/// exited or closed stdout); `Ok(vec![])` means it is alive but slow, which the
/// stall watchdog owns, not this gate. Whatever was read is returned so the
/// response body can start at the first byte.
async fn take_first_bytes(child: &mut Child, stdout: &mut ChildStdout) -> Result<Vec<u8>, ()> {
    let mut buf = vec![0u8; 64 * 1024];
    let gate = async {
        tokio::select! {
            status = child.wait() => Err(status.map(|_| ())),
            read = stdout.read(&mut buf) => match read {
                Ok(n) if n > 0 => Ok(n),
                _ => Err(()),
            },
        }
    };
    match tokio::time::timeout(FIRST_BYTES_TIMEOUT, gate).await {
        Ok(Ok(n)) => Ok(buf[..n].to_vec()),
        Ok(Err(_)) => {
            if let Some(mut stderr) = child.stderr.take() {
                let mut text = String::new();
                let _ = tokio::time::timeout(
                    STDERR_DRAIN_TIMEOUT,
                    tokio::io::AsyncReadExt::read_to_string(&mut stderr, &mut text),
                )
                .await;
                log::debug!("Hardware stream run produced no output: {}", text.trim());
            }
            Err(())
        }
        Err(_) => Ok(Vec::new()),
    }
}
```

Two invariants to preserve while editing (call them out in a code comment):

- **The permit is acquired exactly once, before either spawn, and the respawn never re-acquires it.** `spawn_stream_child` deliberately does not touch the semaphore: replacing the process must not consume a second slot, or a hardware failure would shrink the pool until restarts. The pool bound stays the existing E2E `stream_endpoint_returns_503` coverage (that spec saturates the pool and asserts the 503), so no new test is added for it — a permit-count assertion in a unit test would race every other test in the binary.
- **The gate never runs for a plan-less run.** With `plan == None` the function must behave exactly like today's `start_stream`: spawn, take the pipes, return immediately. `planless_stream_returns_the_childs_bytes_verbatim` pins the observable half of that.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib video_stream`
Expected: all pass — the 3 argument tests, the 4 fake-ffmpeg/gate tests, and every pre-existing stream test (they call `start_stream`, whose plan resolves to `None` in the test binary).

- [ ] **Step 5: Run the transcoding E2E spec against the real backend**

```bash
npm run build
cargo build --bin turbo-pix
npx playwright test tests/e2e/specs/transcoding.e2e.spec.js tests/e2e/specs/video-streaming.e2e.spec.js
```

Expected: both specs pass on this workstation (which reports `h264_vaapi`), i.e. hardware-encoded fragmented output still plays through MSE, seeks, and clears the conversion notice exactly as with `libx264`.

- [ ] **Step 6: Lint, format, commit**

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
git add src/video_stream.rs src/handlers_video.rs
git commit -m "feat(video): stream the transcode rung on the probed encoder with a first-byte software gate"
```

---

## Task 5: E2E coverage for the reported encoder and full-suite verification

**Files:**
- Modify: `tests/e2e/specs/transcoding.e2e.spec.js`
- Test: that spec (Playwright against the real backend)

**Interfaces:**
- Consumes: `GET /api/photos/{hash}/video/status` → `{state, hash, started_at, error, percent, encoder}`; the `encoder` value is `libx264` or one of the five hardware names.
- Produces: no production code.

- [ ] **Step 1: Add the failing E2E test**

In `tests/e2e/specs/transcoding.e2e.spec.js`, extend the existing `Transcoding` describe block:

```js
  test('should report the encoder that produced the conversion', async ({ page }) => {
    test.setTimeout(120_000);

    // GIVEN an AVI/mpeg4 source that always converts (Chromium cannot play it)
    const photo = await findVideoByFilename(page, 'test_video_legacy.avi');
    await TestHelpers.clearCachedConversions(photo.hash_sha256);

    // WHEN the conversion is requested and completes
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);
    await page.request.get(`/api/photos/${photo.hash_sha256}/video`);
    await expect
      .poll(
        async () => {
          const response = await page.request.get(
            `/api/photos/${photo.hash_sha256}/video/status`
          );
          if (!response.ok()) return 'missing-status';
          const status = await response.json();
          return status.state;
        },
        { timeout: 90_000, intervals: [1000] }
      )
      .toBe('Completed');

    // THEN the status names a known encoder and never an alias or a flag blob
    const status = await (
      await page.request.get(`/api/photos/${photo.hash_sha256}/video/status`)
    ).json();
    expect([
      'libx264',
      'h264_nvenc',
      'h264_vaapi',
      'h264_qsv',
      'h264_amf',
      'h264_videotoolbox',
    ]).toContain(status.encoder);

    // AND playback of the produced artifact is unaffected
    await TestHelpers.navigateToView(page, 'videos');
    await TestHelpers.waitForPhotosToLoad(page);
    await page.locator(TestHelpers.selectors.photoCard(photo.hash_sha256)).click();
    await TestHelpers.verifyViewerOpen(page);
    await expect(page.locator(TestHelpers.selectors.viewerVideo)).toBeVisible();
  });
```

- [ ] **Step 2: Run it to verify it passes (it would fail with `undefined` before Task 3)**

```bash
npx playwright test tests/e2e/specs/transcoding.e2e.spec.js
```

Expected: 3 passing. If the machine has no seeded model cache, run `./target/debug/turbo-pix --download-models` first, and let a previous Playwright run's teardown settle before re-running (the known port race).

- [ ] **Step 3: Run the full project gates**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
npm run lint
npm run test:unit
npm run test:e2e
```

Expected: all green. `cargo test` count must be the pre-change count plus the 30 new tests (10 + 8 + 8 + 7 minus the ones merged into existing tests) — record the exact numbers in the commit message.

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/specs/transcoding.e2e.spec.js
git commit -m "test(e2e): assert the reported transcode encoder and keep playback coverage green"
```

---

## Task 6: Prove the GPU path on real hardware, then record the learnings

**Files:**
- Modify: `AGENTS.md` (Learnings entry 7)
- No production code.

**Interfaces:**
- Consumes: everything above; a host with a working hardware encoder (this workstation: Intel renderD129 via VAAPI).
- Produces: recorded evidence for spec SC-001, SC-003 and SC-004.

- [ ] **Step 1: Prove the real GPU path end to end**

```bash
npm run build && cargo build --bin turbo-pix
rm -rf data/cache/transcoded
RUST_LOG=info ./target/debug/turbo-pix &
sleep 5
curl -s http://localhost:18473/health
# open the HEVC fixture through the API so the server converts it
HASH=$(curl -s 'http://localhost:18473/api/photos?q=type:video&limit=200' | jq -r '.photos[] | select(.filename=="test_video_hevc.mp4") | .hash_sha256')
curl -s "http://localhost:18473/api/photos/$HASH/video/status" | jq .
```

Expected: the startup log names `h264_vaapi (/dev/dri/renderD129)` and the completed status reports `"encoder": "h264_vaapi"`. If the status reports `libx264`, the probe rejected the encoder: `RUST_LOG=debug` shows which candidate failed and why.

- [ ] **Step 2: Measure the speedup and the quality on the same host**

```bash
ffmpeg -hide_banner -loglevel error -y -f lavfi -i testsrc2=size=1920x1080:rate=30:duration=12 \
  -f lavfi -i sine=frequency=440:duration=12 -c:v libx265 -preset ultrafast -tag:v hvc1 \
  -c:a aac -shortest -f mp4 /tmp/gpu-bench-src.mp4
# software baseline
/usr/bin/time -f '%es %MkB' ffmpeg -v error -y -t 5 -i /tmp/gpu-bench-src.mp4 -map 0:v:0 \
  -c:v libx264 -preset fast -crf 23 -f mp4 /tmp/gpu-bench-cpu.mp4
# hardware
/usr/bin/time -f '%es %MkB' ffmpeg -v error -y -hwaccel auto -vaapi_device /dev/dri/renderD129 \
  -t 5 -i /tmp/gpu-bench-src.mp4 -map 0:v:0 -vf format=nv12,hwupload \
  -c:v h264_vaapi -qp 23 -f mp4 /tmp/gpu-bench-gpu.mp4
# quality parity
ffmpeg -hide_banner -i /tmp/gpu-bench-cpu.mp4 -i /tmp/gpu-bench-src.mp4 -lavfi ssim -f null - 2>&1 | grep Parsed_ssim
ffmpeg -hide_banner -i /tmp/gpu-bench-gpu.mp4 -i /tmp/gpu-bench-src.mp4 -lavfi ssim -f null - 2>&1 | grep Parsed_ssim
```

Expected (measured on 2026-09-21 before this plan was written, i7-6820HQ + Intel HD 530): CPU 2.3 s / 3.1 MB / SSIM 0.9583; VAAPI 0.7 s / 4.4 MB / SSIM 0.9580 — at most 50 % of the software wall-clock (SC-004) at equal perceptual quality. Record the fresh numbers in the commit message.

- [ ] **Step 3: Prove the failure path (SC-003)**

```bash
# Simulate a hardware encoder that passes the probe but fails the job by
# pointing the probe at the working node and the jobs at a busy one:
# temporarily edit /dev/dri permissions is NOT needed — instead run the server
# with a wrapper that fails only when the input is the real file:
cat > /tmp/ffmpeg-fails-on-job <<'EOF'
#!/usr/bin/env sh
case "$*" in
  *color=c=black*) exec /usr/bin/ffmpeg "$@" ;;
  *test_video_hevc*) printf '%s\n' 'simulated hardware failure' >&2; exit 1 ;;
esac
exec /usr/bin/ffmpeg "$@"
EOF
chmod +x /tmp/ffmpeg-fails-on-job
FFMPEG_PATH=/tmp/ffmpeg-fails-on-job RUST_LOG=info ./target/debug/turbo-pix &
sleep 5
curl -s "http://localhost:18473/api/photos/$HASH/video/status" | jq .state,.encoder
```

Expected: the job still reaches `Completed`, `encoder` reads `libx264`, and the server log contains one `Hardware encoder h264_vaapi (…) failed (…)` warning followed by the successful software attempt.

- [ ] **Step 4: Fold the learnings into AGENTS.md**

Extend Learnings entry 7 (do not add an entry — the section is capped at 10) with, at minimum:
- hardware encoder selection lives in `src/video_encoder.rs`, is probed once per process from `main`, and is injected into both paths as an `Option<&HwPlan>` so the argument builders stay pure;
- the probe must run the *job's* argument shape (`-vaapi_device` + `-vf format=nv12,hwupload` + `-c:v h264_vaapi -qp 23`) because ffmpeg listing an encoder proves nothing — on this host `h264_qsv`/`h264_amf` are listed but unusable, and `/dev/dri/renderD128` (AMD) is listed but has no H.264 encode entrypoint while `renderD129` (Intel) works;
- VAAPI ranks above QSV, and a broken candidate can cost seconds (QSV's MFX session failure took 2.06 s) — the probe is bounded by `PROBE_TIMEOUT` (2 s) for that reason;
- parity is by number, not by bitrate: `-qp 23` / `-cq 23 -b:v 0` / `-global_quality 23` land next to `-crf 23` in SSIM, and hardware output is ~40 % larger at that quality;
- a hardware failure retries once in software *within the same job* (whole-file) or inside the same request before the first byte (stream), and a timeout is never retried; progress is guarded by a shared high-water mark so the retry cannot drag the client's percentage backwards.

Verify every other entry in the Learnings section is still accurate before committing (the repo rule requires this).

- [ ] **Step 5: Commit**

```bash
git add AGENTS.md
git commit -m "docs(agents): record hardware encoder probing and fallback learnings"
```

---

## Spec Coverage

| Spec item | Where it is implemented |
| --- | --- |
| FR-001 probe with a real encode | Task 2 `probe`/`detect` |
| FR-002 process-wide, computed once | Task 2 `ACTIVE_PLAN`, `initialisation_probes_at_most_once` |
| FR-003 five backends, fixed order | Task 1 `HwEncoder::ALL`, Task 2 `preference_order_picks_the_first_usable_encoder` |
| FR-004 both paths use it | Task 3 (`convert_video_with_progress`), Task 4 (`start_stream`) |
| FR-005 in-job software fallback | Task 3 `convert_with_fallback`, Task 4 first-byte gate |
| FR-006 no new configuration | no env/config changes anywhere; Task 6 verifies a plain `cargo run` picks it up |
| FR-007 delivered media unchanged | Task 1 pix-format/profile rules, Task 4 E2E run |
| FR-008 existing gating preserved | Task 3 keeps `acquire_transcode_permit` and the timeout; Task 4 keeps the permit across the respawn |
| FR-009 identical without hardware | `reencode_args_are_unchanged_without_a_plan`, `transcode_args_are_unchanged_without_a_plan`, `planless_stream_returns_the_childs_bytes_verbatim` |
| FR-010 encoder observable | Task 3 `TranscodeStatus.encoder` + job-level `info!`/`warn!`, Task 5 E2E assertion |
| SC-001 (GPU host reports hardware) | Task 6 Step 1 |
| SC-002 (GPU-less host unchanged) | Task 5 full-suite run on the CI host |
| SC-003 (failure still delivers) | Task 3 fallback tests, Task 6 Step 3 |
| SC-004 (≤50 % wall-clock) | Task 6 Step 2 |
| SC-005 (no client-visible regression) | Task 4 Step 5, Task 5 Step 3 |
