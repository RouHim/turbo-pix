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

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum DirectPlay {
    Yes,
    NeedsRemux,
    No,
}

pub struct ClientCodecs {
    pub h264_8: bool,
    pub h264_10: bool,
    pub hevc: bool,
    pub av1: bool,
    pub vp9: bool,
    pub vp8: bool,
}

impl ClientCodecs {
    pub fn conservative() -> Self {
        Self {
            h264_8: true,
            ..Self::none()
        }
    }
    fn none() -> Self {
        Self {
            h264_8: false,
            h264_10: false,
            hevc: false,
            av1: false,
            vp9: false,
            vp8: false,
        }
    }
    pub fn parse(header: Option<&str>) -> Self {
        // A client that sends no capability header gets the conservative
        // baseline (h264-8 only), matching the test contract.
        let Some(raw) = header else {
            return Self::conservative();
        };
        let mut c = Self::none();
        for tok in raw.split(',').map(str::trim) {
            match tok {
                "h264-8" => c.h264_8 = true,
                "h264-10" => c.h264_10 = true,
                "hevc" => c.hevc = true,
                "av1" => c.av1 = true,
                "vp9" => c.vp9 = true,
                "vp8" => c.vp8 = true,
                _ => {}
            }
        }
        c
    }
}

pub fn decide(
    codec: &str,
    container: &str,
    bit_depth: Option<u32>,
    moov_at_start: bool,
    file_size: i64,
    client: &ClientCodecs,
) -> DirectPlay {
    if file_size <= 0 {
        return DirectPlay::No;
    }
    match codec {
        "h264" => {
            if container != "mp4" && container != "mov" && container != "m4v" {
                return DirectPlay::No;
            }
            let supported = if bit_depth.unwrap_or(8) <= 8 {
                client.h264_8
            } else {
                client.h264_10
            };
            if !supported {
                return DirectPlay::No;
            }
            if moov_at_start {
                DirectPlay::Yes
            } else {
                DirectPlay::NeedsRemux
            }
        }
        "av1" | "vp8" | "vp9" => {
            let client_support = if codec == "av1" {
                client.av1
            } else if codec == "vp9" {
                client.vp9
            } else {
                client.vp8
            };
            if (container == "webm" || container == "mp4") && client_support {
                if moov_at_start {
                    DirectPlay::Yes
                } else {
                    DirectPlay::NeedsRemux
                }
            } else {
                DirectPlay::No
            }
        }
        _ => DirectPlay::No, // hevc, mpeg4, fraps, indeo5, msmpeg4*, mjpeg, ...
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client_all() -> ClientCodecs {
        ClientCodecs {
            h264_8: true,
            h264_10: true,
            hevc: true,
            av1: true,
            vp9: true,
            vp8: true,
        }
    }
    fn client_h264_8() -> ClientCodecs {
        ClientCodecs::conservative()
    }

    #[test]
    fn h264_8_mp4_moov_at_start_direct_plays() {
        let c = client_all();
        assert_eq!(
            decide("h264", "mp4", Some(8), true, 1000, &c),
            DirectPlay::Yes
        );
        assert_eq!(
            decide("h264", "mov", Some(8), true, 1000, &c),
            DirectPlay::Yes
        );
        // missing bit_depth defaults to 8-bit (safe default for nearly all library h264)
        assert_eq!(decide("h264", "mp4", None, true, 1000, &c), DirectPlay::Yes);
    }

    #[test]
    fn h264_8_with_moov_at_end_needs_remux() {
        let c = client_all();
        assert_eq!(
            decide("h264", "mp4", Some(8), false, 1000, &c),
            DirectPlay::NeedsRemux
        );
    }

    #[test]
    fn h264_10bit_requires_client_h264_10() {
        let all = client_all();
        assert_eq!(
            decide("h264", "mp4", Some(10), true, 1000, &all),
            DirectPlay::Yes
        );
        let c8 = client_h264_8();
        assert_eq!(
            decide("h264", "mp4", Some(10), true, 1000, &c8),
            DirectPlay::No
        );
    }

    #[test]
    fn conservative_client_h264_10_needs_transcode() {
        let c = client_h264_8();
        assert_eq!(
            decide("h264", "mp4", Some(10), true, 1000, &c),
            DirectPlay::No
        );
    }

    #[test]
    fn hevc_and_empty_never_direct_playable() {
        let c = client_all();
        assert_eq!(
            decide("hevc", "mp4", Some(8), true, 1000, &c),
            DirectPlay::No
        );
        assert_eq!(decide("h264", "mp4", Some(8), true, 0, &c), DirectPlay::No);
        // empty file
    }

    #[test]
    fn av1_and_vp_in_webm_direct_playable_when_client_supports() {
        let c = client_all();
        assert_eq!(
            decide("av1", "webm", Some(8), true, 1000, &c),
            DirectPlay::Yes
        );
        assert_eq!(
            decide("vp9", "webm", Some(8), true, 1000, &c),
            DirectPlay::Yes
        );
        let c8 = client_h264_8();
        assert_eq!(
            decide("av1", "webm", Some(8), true, 1000, &c8),
            DirectPlay::No
        );
    }

    #[test]
    fn legacy_codecs_never_direct_playable() {
        let c = client_all();
        for codec in [
            "mpeg4",
            "fraps",
            "indeo5",
            "msmpeg4v2",
            "msmpeg4v1",
            "mjpeg",
        ] {
            assert_eq!(
                decide(codec, "avi", Some(8), true, 1000, &c),
                DirectPlay::No
            );
        }
    }

    #[test]
    fn parse_client_header() {
        let c = ClientCodecs::parse(Some("h264-8,hevc,av1"));
        assert!(c.h264_8 && c.hevc && c.av1 && !c.h264_10);
        let d = ClientCodecs::parse(None);
        assert!(d.h264_8 && !d.hevc);
    }
}
