//! Shared video-playback capability helpers: the container families, the
//! client's declared codec set (video *and* audio), and the single playback
//! decision that maps resolved capabilities + a client declaration onto a
//! delivery mode.

use crate::video_probe::ResolvedCapabilities;

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

/// Container family after normalising the ffprobe `format_name` token and the
/// file extension. ffprobe reports `"matroska,webm"` for BOTH `.mkv` and
/// `.webm`, so the extension is the only reliable discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerFamily {
    Mp4,
    Matroska,
    Webm,
    Avi,
    MpegTs,
    Ogg,
    Other,
}

impl ContainerFamily {
    pub fn from_record(container: Option<&str>, file_name: &str) -> Self {
        let ext = file_name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        match container.unwrap_or_default().to_ascii_lowercase().as_str() {
            "mp4" | "mov" | "m4v" | "m4a" | "3gp" | "3g2" | "mj2" => Self::Mp4,
            "webm" => Self::Webm,
            "matroska" | "mkv" => {
                if ext == "webm" {
                    Self::Webm
                } else {
                    Self::Matroska
                }
            }
            "avi" => Self::Avi,
            "mpegts" => Self::MpegTs,
            "ogg" => Self::Ogg,
            _ => match ext.as_str() {
                "mp4" | "mov" | "m4v" => Self::Mp4,
                "mkv" => Self::Matroska,
                "webm" => Self::Webm,
                "avi" => Self::Avi,
                "ts" | "m2ts" => Self::MpegTs,
                "ogv" | "ogg" => Self::Ogg,
                _ => Self::Other,
            },
        }
    }

    /// Matroska-family containers carry no `moov` atom; only MP4-family files
    /// have a progressive-playback layout that can be wrong.
    pub fn has_moov_layout(self) -> bool {
        matches!(self, Self::Mp4)
    }
}

/// How one request should be delivered. The server owns this decision; the
/// client may only consume it (and escalate to conversion when the delivered
/// bytes turn out to be undecodable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Hand the original file to the media element as-is.
    Direct,
    /// Stream a fragmented-MP4 remux (container/layout only, no re-encode).
    StreamRemux,
    /// Stream the video track copied and only the audio converted.
    StreamAudio,
    /// Stream a full transcode (video and audio re-encoded).
    StreamTranscode,
}

/// Containers a media element may be handed as-is.
fn direct_container_ok(family: ContainerFamily) -> bool {
    matches!(family, ContainerFamily::Mp4 | ContainerFamily::Webm)
}

/// Video codecs that can be copied (never re-encoded) into an MP4 output.
fn copyable_into_mp4(codec: &str) -> bool {
    matches!(codec, "h264" | "hevc" | "av1" | "vp9")
}

fn video_supported(codec: &str, bit_depth: Option<u32>, client: &ClientCodecs) -> bool {
    match codec {
        "h264" => {
            if bit_depth.unwrap_or(8) <= 8 {
                client.h264_8
            } else {
                client.h264_10
            }
        }
        "hevc" => client.hevc,
        "av1" => client.av1,
        "vp9" => client.vp9,
        "vp8" => client.vp8,
        _ => false,
    }
}

fn audio_supported(codec: Option<&str>, client: &ClientCodecs) -> bool {
    match codec {
        None | Some("") => true,
        Some("aac") => client.audio.aac,
        Some("opus") => client.audio.opus,
        Some("mp3") => client.audio.mp3,
        Some("flac") => client.audio.flac,
        Some("ac3") => client.audio.ac3,
        Some("eac3") => client.audio.eac3,
        Some("dts") => client.audio.dts,
        Some("vorbis") => client.audio.vorbis,
        Some(_) => false,
    }
}

/// The single playback decision (spec FR-001..FR-004, FR-007).
pub fn plan(caps: &ResolvedCapabilities, client: &ClientCodecs) -> Delivery {
    let video_ok = video_supported(&caps.codec, caps.bit_depth, client);
    let audio_ok = audio_supported(caps.audio_codec.as_deref(), client);

    if video_ok && audio_ok {
        // Same container class the browser understands, and — for MP4-family —
        // a progressive layout.
        let layout_ok = !caps.family.has_moov_layout() || caps.moov_at_start;
        if direct_container_ok(caps.family) && layout_ok {
            return Delivery::Direct;
        }
        if copyable_into_mp4(&caps.codec) {
            return Delivery::StreamRemux;
        }
    }
    if video_ok && copyable_into_mp4(&caps.codec) {
        // Video is fine; the container or the audio track is not.
        return if audio_ok {
            Delivery::StreamRemux
        } else {
            Delivery::StreamAudio
        };
    }
    Delivery::StreamTranscode
}

/// Audio codecs a client can decode. Mirrors the tokens `ClientCodecs::parse`
/// accepts. Audio is a separate decision dimension: browsers disagree on AC-3
/// and DTS support, so "the video plays" says nothing about the audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AudioCodecs {
    pub aac: bool,
    pub opus: bool,
    pub mp3: bool,
    pub flac: bool,
    pub ac3: bool,
    pub eac3: bool,
    pub dts: bool,
    pub vorbis: bool,
}

/// The client's declared decoding capabilities, parsed from the
/// `X-TurboPix-Codecs` header or the `?client=` query param.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientCodecs {
    pub h264_8: bool,
    pub h264_10: bool,
    pub hevc: bool,
    pub av1: bool,
    pub vp9: bool,
    pub vp8: bool,
    pub audio: AudioCodecs,
}

impl ClientCodecs {
    pub fn none() -> Self {
        Self {
            h264_8: false,
            h264_10: false,
            hevc: false,
            av1: false,
            vp9: false,
            vp8: false,
            audio: AudioCodecs::default(),
        }
    }

    pub fn conservative() -> Self {
        Self {
            h264_8: true,
            ..Self::none()
        }
    }

    /// Unknown tokens are ignored. A missing OR blank declaration yields the
    /// conservative baseline; a declaration that parses to nothing at all
    /// yields an empty capability set (the client explicitly claimed nothing).
    pub fn parse(header: Option<&str>) -> Self {
        let Some(raw) = header else {
            return Self::conservative();
        };
        if raw.trim().is_empty() {
            return Self::conservative();
        }
        let mut c = Self::none();
        for tok in raw.split(',').map(str::trim) {
            match tok {
                "h264-8" => c.h264_8 = true,
                "h264-10" => c.h264_10 = true,
                "hevc" => c.hevc = true,
                "av1" => c.av1 = true,
                "vp9" => c.vp9 = true,
                "vp8" => c.vp8 = true,
                "aac" => c.audio.aac = true,
                "opus" => c.audio.opus = true,
                "mp3" => c.audio.mp3 = true,
                "flac" => c.audio.flac = true,
                "ac3" => c.audio.ac3 = true,
                "eac3" => c.audio.eac3 = true,
                "dts" => c.audio.dts = true,
                "vorbis" => c.audio.vorbis = true,
                _ => {}
            }
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video_probe::ResolvedCapabilities;

    fn caps(
        codec: &str,
        container: &str,
        bit_depth: Option<u32>,
        audio: Option<&str>,
        moov: bool,
    ) -> ResolvedCapabilities {
        ResolvedCapabilities {
            codec: codec.to_string(),
            container: Some(container.to_string()),
            family: ContainerFamily::from_record(Some(container), "video.mp4"),
            bit_depth,
            audio_codec: audio.map(str::to_string),
            moov_at_start: moov,
            duration_secs: Some(10.0),
            probed: true,
        }
    }

    fn client_all() -> ClientCodecs {
        ClientCodecs::parse(Some(
            "h264-8,h264-10,hevc,av1,vp9,vp8,aac,opus,mp3,flac,ac3,eac3,dts,vorbis",
        ))
    }

    /// What a plain Chrome/Firefox desktop really declares.
    fn web_client() -> ClientCodecs {
        ClientCodecs::parse(Some("h264-8,aac,opus,mp3,vorbis"))
    }

    #[test]
    fn h264_mp4_with_audio_plays_directly() {
        assert_eq!(
            plan(
                &caps("h264", "mov", Some(8), Some("aac"), true),
                &web_client()
            ),
            Delivery::Direct
        );
        assert_eq!(
            plan(&caps("h264", "mp4", None, None, true), &web_client()),
            Delivery::Direct
        );
    }

    #[test]
    fn moov_at_end_remuxes_losslessly() {
        assert_eq!(
            plan(
                &caps("h264", "mov", Some(8), Some("aac"), false),
                &web_client()
            ),
            Delivery::StreamRemux
        );
    }

    #[test]
    fn h264_in_matroska_remuxes_to_mp4() {
        assert_eq!(
            plan(
                &caps("h264", "matroska", Some(8), Some("aac"), true),
                &web_client()
            ),
            Delivery::StreamRemux
        );
    }

    #[test]
    fn webm_vp9_plays_directly_only_when_declared() {
        let webm = ResolvedCapabilities {
            family: ContainerFamily::Webm,
            ..caps("vp9", "webm", Some(8), Some("opus"), true)
        };
        // "directly only when declared": with vp9 declared it plays as-is ...
        assert_eq!(plan(&webm, &client_all()), Delivery::Direct);
        // ... and without it the source is converted.
        let no_vp9 = ClientCodecs::parse(Some("h264-8,aac"));
        assert_eq!(plan(&webm, &no_vp9), Delivery::StreamTranscode);
    }

    #[test]
    fn hevc_direct_plays_only_when_declared() {
        let hevc = caps("hevc", "mov", Some(8), Some("aac"), true);
        assert_eq!(plan(&hevc, &client_all()), Delivery::Direct);
        assert_eq!(plan(&hevc, &web_client()), Delivery::StreamTranscode);
    }

    #[test]
    fn undeclared_audio_converts_audio_only() {
        let ac3 = caps("h264", "mov", Some(8), Some("ac3"), true);
        assert_eq!(plan(&ac3, &web_client()), Delivery::StreamAudio);
        assert_eq!(plan(&ac3, &client_all()), Delivery::Direct);
    }

    #[test]
    fn ten_bit_h264_needs_declared_support() {
        let ten_bit = caps("h264", "mov", Some(10), Some("aac"), true);
        assert_eq!(plan(&ten_bit, &web_client()), Delivery::StreamTranscode);
        assert_eq!(plan(&ten_bit, &client_all()), Delivery::Direct);
    }

    #[test]
    fn legacy_and_unknown_sources_convert() {
        assert_eq!(
            plan(
                &caps("mpeg4", "avi", Some(8), Some("mp3"), true),
                &client_all()
            ),
            Delivery::StreamTranscode
        );
        assert_eq!(
            plan(&caps("", "", None, None, true), &client_all()),
            Delivery::StreamTranscode
        );
        assert_eq!(
            plan(
                &caps("hevc", "matroska", Some(8), Some("aac"), true),
                &web_client()
            ),
            Delivery::StreamTranscode
        );
    }

    #[test]
    fn empty_or_missing_declaration_falls_back_to_conservative() {
        assert_eq!(ClientCodecs::parse(Some("")), ClientCodecs::conservative());
        assert_eq!(ClientCodecs::parse(None), ClientCodecs::conservative());
        assert_eq!(
            ClientCodecs::parse(Some("bogus-token")),
            ClientCodecs::none(),
            "a declaration with no recognized token is an empty capability set, not h264-8"
        );
    }
}
