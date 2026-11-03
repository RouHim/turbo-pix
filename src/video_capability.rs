//! Shared video-playback capability helpers: the container families, the
//! client's declared codec set (video *and* audio), and the single playback
//! decision that maps resolved capabilities + a client declaration onto a
//! delivery mode.

use serde_json::Value;

use crate::db::Photo;

/// Bumped whenever the set of persisted capability facts changes; its presence
/// is the "record complete, never probe again" marker.
pub const CAPABILITY_VERSION: u64 = 1;

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
    /// `Some(true)` / `Some(false)` when a pass established the layout (the
    /// probe's own verdict, or the fact the record already stored), `None` when
    /// none ever did: the record carries no `moov_at_start` key and the moov
    /// pass yielded no verdict. Never invented — `plan` reads `None` as "not
    /// known to be progressive" and takes the remux rung for an MP4-family
    /// source instead of the whole-file download `Direct` costs.
    pub moov_at_start: Option<bool>,
    pub duration_secs: Option<f64>,
    /// True when this call had to probe the file (incomplete record).
    pub probed: bool,
}

impl ResolvedCapabilities {
    /// Whether a delivery may name an encoder. A source with no video stream
    /// (or an unknown codec) was not video-encoded, so silence is the only
    /// honest hint.
    pub fn may_claim_encoder(&self) -> bool {
        !self.codec.is_empty()
    }

    /// Whether the container carries no mappable video stream, using the
    /// stored snapshot and this call's probe together: `probed` is set only by
    /// a successful probe, so completeness is `probed ||
    /// record_is_complete(photo)`.
    pub fn has_no_mappable_stream(&self, photo: &Photo) -> bool {
        (self.probed || record_is_complete(photo))
            && self.codec.is_empty()
            && self.audio_codec.is_none()
    }

    /// The facts the stored record already holds. `probed: false`: no file was
    /// consulted for this answer.
    pub fn from_record(photo: &Photo) -> Self {
        Self {
            codec: photo.video_codec().unwrap_or_default().to_string(),
            container: photo.container().map(str::to_string),
            family: ContainerFamily::from_record(photo.container(), &photo.filename),
            bit_depth: photo.bit_depth(),
            audio_codec: photo.audio_codec().map(str::to_string),
            moov_at_start: stored_moov_at_start(photo),
            duration_secs: photo.duration.filter(|d| *d > 0.0),
            probed: false,
        }
    }

    /// The facts a probe established, merged over the stored record.
    pub(crate) fn merged(
        photo: &Photo,
        patch: &CapabilityPatch,
        moov_at_start: Option<bool>,
    ) -> Self {
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

/// The layout the record itself stores: `Some(true)` / `Some(false)` when the
/// key is there, `None` when it is absent — the state [`record_is_complete`]
/// documents as "never probed", NOT "true". The scanner's own fallback for a
/// failed layout pass is not a record: absence stays absence.
pub(crate) fn stored_moov_at_start(photo: &Photo) -> Option<bool> {
    photo
        .metadata
        .get("video")
        .and_then(|video| video.get("moov_at_start"))
        .and_then(Value::as_bool)
}

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

/// Audio codecs a copied MP4 track can carry *and* the client can decode from
/// it. `None`/`""` is a source with no audio track at all — nothing to carry.
///
/// Deliberately narrower than the client's declaration, which is a *decode*
/// claim: the shipped client derives its `vorbis` token from an
/// `audio/webm; codecs="vorbis"` probe, and a WebM probe licenses nothing about
/// MP4. Vorbis can be muxed into MP4 (ffmpeg writes it into an `mp4a`/ESDS
/// sample entry) but no Chromium MP4 path decodes it, so copying it hands the
/// client an init segment whose audio track contradicts the MIME it created its
/// SourceBuffer for. Such a track goes to [`Delivery::StreamAudio`] instead,
/// which copies the video and re-encodes the audio to AAC.
fn copyable_audio_into_mp4(codec: Option<&str>) -> bool {
    matches!(
        codec,
        None | Some("")
            | Some("aac")
            | Some("opus")
            | Some("mp3")
            | Some("ac3")
            | Some("eac3")
            | Some("dts")
            | Some("flac")
    )
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
    // A copy muxes both tracks into MP4, so the audio track has to be carryable
    // there independently of the client declaring it. Both copy branches below
    // are gated on this: the client's `vorbis` token comes from a WebM probe, so
    // a declared Vorbis track is exactly the case where a copy would promise an
    // MP4 sample entry the client cannot decode.
    let audio_copyable = copyable_audio_into_mp4(caps.audio_codec.as_deref());

    if video_ok && audio_ok {
        // Same container class the browser understands, and — for MP4-family —
        // a layout that is KNOWN to be progressive. An unknown layout is not
        // one: the record that carries no `moov_at_start` key is exactly the
        // legacy row whose `-v trace` pass could fail or be killed (see
        // `video_probe::resolve`), and handing it to the element as `Direct`
        // makes the browser download the whole file before the first frame when
        // it turns out to be moov-at-end. The remux rung is a `-c copy`
        // faststart sidecar — cheap, correct, and already faststart when the
        // source was.
        let layout_ok = !caps.family.has_moov_layout() || caps.moov_at_start == Some(true);
        if direct_container_ok(caps.family) && layout_ok {
            return Delivery::Direct;
        }
        if copyable_into_mp4(&caps.codec) && audio_copyable {
            return Delivery::StreamRemux;
        }
    }
    if video_ok && copyable_into_mp4(&caps.codec) {
        // Video is fine; the container or the audio track is not.
        return if audio_ok && audio_copyable {
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
            moov_at_start: Some(moov),
            duration_secs: Some(10.0),
            probed: true,
        }
    }

    /// The same source with NO established layout: what `plan` sees for an
    /// MP4-family record that carries no `moov_at_start` key whose moov pass
    /// yielded no verdict (`video_probe`'s `None`), where the layout is simply
    /// unknown rather than known to be progressive.
    fn caps_without_layout(
        codec: &str,
        container: &str,
        audio: Option<&str>,
    ) -> ResolvedCapabilities {
        let mut caps = caps(codec, container, Some(8), audio, true);
        caps.moov_at_start = None;
        caps
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
    fn an_unknown_mp4_layout_takes_the_remux_rung() {
        // An MP4-family source whose layout no pass ever established — the
        // legacy record with no `moov_at_start` key whose `-v trace` pass
        // failed or was killed (see `video_probe::resolve_within`) — must not
        // be handed to the element as `Direct`: if the file turns out to be
        // moov-at-end, the browser downloads all of it before the first frame,
        // permanently, because a completed record is never probed again. The
        // lossless `-c copy` remux is the cheap and correct rung instead.
        let unknown = caps_without_layout("h264", "mp4", Some("aac"));
        assert_eq!(unknown.moov_at_start, None, "the premise: layout unknown");
        assert_eq!(plan(&unknown, &web_client()), Delivery::StreamRemux);

        // The layout is only consulted for MP4-family containers: a Matroska
        // source has no moov to be at either end of, so an unknown flag changes
        // nothing about its (already remuxed) delivery.
        assert_eq!(
            plan(
                &caps_without_layout("h264", "matroska", Some("aac")),
                &web_client()
            ),
            Delivery::StreamRemux
        );
        let webm = ResolvedCapabilities {
            family: ContainerFamily::Webm,
            ..caps_without_layout("vp9", "webm", Some("opus"))
        };
        assert_eq!(plan(&webm, &client_all()), Delivery::Direct);

        // A known-progressive source still plays directly: the conservative
        // reading is about the unknown layout, not a blanket refusal.
        assert_eq!(
            plan(
                &caps("h264", "mp4", Some(8), Some("aac"), true),
                &web_client()
            ),
            Delivery::Direct
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
    fn a_vorbis_track_is_reencoded_instead_of_copied_into_mp4() {
        // GIVEN the classic h264 + Vorbis Matroska and a client whose `vorbis`
        // token comes from an `audio/webm` probe (the shipped declaration)
        let vorbis = caps("h264", "matroska", Some(8), Some("vorbis"), true);
        assert!(
            web_client().audio.vorbis,
            "the premise: a WebM probe does declare Vorbis"
        );

        // THEN the audio track is NOT copied into MP4 — no Chromium MP4 path
        // decodes a Vorbis sample entry — while the video is still passed
        // through instead of being re-encoded
        assert_eq!(plan(&vorbis, &web_client()), Delivery::StreamAudio);
        assert_eq!(plan(&vorbis, &client_all()), Delivery::StreamAudio);

        // AND the same file with an MP4-carryable track still copies losslessly
        assert_eq!(
            plan(
                &caps("h264", "matroska", Some(8), Some("aac"), true),
                &web_client()
            ),
            Delivery::StreamRemux
        );
        assert_eq!(
            plan(
                &caps("h264", "matroska", Some(8), Some("ac3"), true),
                &web_client()
            ),
            Delivery::StreamAudio,
            "an undeclared AC-3 track re-encodes for the same reason"
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
