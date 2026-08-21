/// Shared video-playback capability helpers.
///
/// Task 2 lands the serve-time decision engine (Direct Play / faststart remux /
/// transcode) in this same module.
///
/// Maps a ffprobe pixel format string to its bit depth.
///
/// 8-bit formats are listed explicitly; higher-depth formats follow the
/// `NAME<BITS><le|be>` naming convention (e.g. `yuv420p10le`,
/// `yuv444p12be`). Unknown formats yield `None` (caller treats as "unsupported
/// for native playback").
pub fn parse_pix_fmt_bit_depth(pix_fmt: Option<&str>) -> Option<u32> {
    let pix_fmt = pix_fmt?;

    match pix_fmt {
        "yuv420p" | "yuv422p" | "yuv444p" | "nv12" | "nv21" | "yuvj420p" | "yuvj422p"
        | "yuvj444p" => Some(8),
        _ if pix_fmt.ends_with("10le") || pix_fmt.ends_with("10be") => Some(10),
        _ if pix_fmt.ends_with("12le") || pix_fmt.ends_with("12be") => Some(12),
        _ if pix_fmt.ends_with("16le") => Some(16),
        _ => None,
    }
}
