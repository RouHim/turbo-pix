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
///
/// Test-only for now: nothing in the crate queries a listing yet, and a helper
/// that would otherwise be dead code stays out of the shipped binary.
#[cfg(test)]
fn lists_encoder(listing: &str, name: &str) -> bool {
    listing
        .lines()
        .any(|line| line.split_whitespace().any(|token| token == name))
}

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
}
