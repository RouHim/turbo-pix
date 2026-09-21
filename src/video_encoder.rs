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

use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

/// Longest a single ffmpeg probe (encoder listing or encode) may take. A
/// working encode answers in milliseconds; a broken vendor runtime can hang,
/// and a false "unusable" verdict is cheaper than a stalled boot.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Synthetic probe input: instant to encode, comfortably above every vendor's
/// minimum frame size, and available without a file on disk.
const PROBE_INPUT: &str = "color=c=black:s=320x240:r=30:d=0.1";

/// Software fallback. Every hardware decision degrades to this, and it is the
/// only encoder the CPU path uses.
pub const SOFTWARE_ENCODER: &str = "libx264";

/// System-memory pixel format the software path emits; the hardware backends use
/// it too unless they only accept hardware surfaces.
const SOFTWARE_PIX_FMT: &str = "yuv420p";

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

    /// Backends driven through a DRM render node. Linux-only: the same encoders
    /// take their device implicitly on other platforms.
    fn needs_render_node(self) -> bool {
        cfg!(target_os = "linux") && matches!(self, Self::Vaapi | Self::Qsv | Self::Amf)
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
        args.extend(
            self.encoder
                .preset_args()
                .iter()
                .map(|arg| (*arg).to_string()),
        );
        args.extend(
            self.encoder
                .quality_args()
                .iter()
                .map(|arg| (*arg).to_string()),
        );
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
        let plan = detect(&crate::video_processor::get_ffmpeg_path(), &render_nodes()).await;
        // A lost race changes nothing: detection is a pure function of this
        // machine, so both callers computed the same verdict.
        let _ = ACTIVE_PLAN.set(plan);
        match active() {
            Some(plan) => log::info!("Hardware video encoder: {}", plan.label()),
            None => {
                log::info!("No usable hardware video encoder; transcoding uses {SOFTWARE_ENCODER}")
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video_processor::tests::{make_executable, TestEnvGuard};
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn every_backend_declares_a_distinct_ffmpeg_encoder_name() {
        // GIVEN the preference list
        let names: Vec<&str> = HwEncoder::ALL.iter().map(|e| e.name()).collect();

        // THEN every name is unique and looks like an ffmpeg H.264 encoder
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            names.len(),
            "duplicate encoder names: {names:?}"
        );
        for name in names {
            assert!(name.starts_with("h264_"), "{name} is not an H.264 encoder");
        }
    }

    #[test]
    fn quality_knob_keeps_the_software_crf_value_on_every_qp_scale_backend() {
        // GIVEN the backends whose quality knob is a quantiser parameter on the
        // same 0-51 scale as libx264's CRF
        // THEN each one passes the software path's value (23) as its own target,
        // so "parity by default" is not a per-backend accident
        //
        // VideoToolbox is excluded on purpose: its constant-quality scale is
        // 1-100, so the number cannot be shared — its own test pins the scale.
        for encoder in [
            HwEncoder::Nvenc,
            HwEncoder::Vaapi,
            HwEncoder::Qsv,
            HwEncoder::Amf,
        ] {
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
        let b_index = args
            .iter()
            .position(|arg| arg == "-b:v")
            .expect("-b:v missing");
        assert_eq!(args[b_index + 1], "0", "NVENC needs -b:v 0: {args:?}");
    }

    #[test]
    fn vaapi_plan_selects_the_device_and_uploads_frames() {
        // GIVEN a VAAPI plan on a specific render node
        let plan = HwPlan::new(HwEncoder::Vaapi, Some("/dev/dri/renderD129".to_string()));

        // WHEN its argument pieces are built
        // THEN the device sits before `-i`, the encoder is VAAPI's, and the
        // filter graph uploads into hardware surfaces
        assert_eq!(
            plan.input_args(),
            vec!["-vaapi_device", "/dev/dri/renderD129"]
        );
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
            vec![
                "-init_hw_device",
                "qsv=hw:/dev/dri/renderD129",
                "-filter_hw_device",
                "hw"
            ]
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
            assert!(
                plan.video_args().contains(&"yuv420p".to_string()),
                "{encoder:?}"
            );
        }
    }

    #[test]
    fn videotoolbox_uses_the_one_to_hundred_quality_scale() {
        // GIVEN a VideoToolbox plan
        // WHEN its arguments are built
        // THEN quality is expressed on its own 1-100 scale, not as a QP
        let args = HwPlan::new(HwEncoder::VideoToolbox, None).video_args();
        let q_index = args
            .iter()
            .position(|arg| arg == "-q:v")
            .expect("-q:v missing");
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
        let listing =
            " V....D hevc_vaapi           H.265/HEVC (VAAPI)\n V....D libx264              H.264\n";

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
        assert_eq!(
            plan.device(),
            Some("/dev/dri/renderD129"),
            "log: {:?}",
            args_log(&log)
        );
        // AND both nodes were actually tried, not just the first
        assert_eq!(
            args_log(&log)
                .iter()
                .filter(|line| line.contains("renderD128"))
                .count(),
            1
        );
        assert_eq!(
            args_log(&log)
                .iter()
                .filter(|line| line.contains("renderD129"))
                .count(),
            1
        );
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
                args_log(&log)
                    .iter()
                    .filter(|line| line.contains(other))
                    .count(),
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
            !args_log(&log)
                .iter()
                .any(|line| line.contains("h264_vaapi")),
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
            assert!(
                probe_line.contains(expected),
                "missing {expected}: {probe_line}"
            );
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
}
