/**
 * The browser's real video codec capability declaration.
 *
 * The set is the browser's own capability answers plus what an actual playback
 * proved — never a user-agent verdict. The declaration is serialized into a
 * comma-joined string (`h264-8,h264-10,hevc,av1,vp9,vp8,aac,…`) and travels to
 * the server as the `client` query parameter (`getVideoUrl`,
 * `api.getVideoDecision`) so the serve-time decision (Direct Play / remux /
 * transcode) is authoritative on the backend rather than guessed client-side.
 * The server still honours an `X-TurboPix-Codecs` request header for clients
 * that can set one — a media element's own request cannot, which is why this
 * app uses the query param.
 *
 * Probing follows Jellyfin's convention: `canPlayType` results of `probably`
 * OR `maybe` count as supported (accepting only `probably` under-reports —
 * Firefox routinely answers `maybe` for codecs it can actually decode).
 */

import { markCodecVerified, verifiedCodecs } from './playbackVerification.js';

export const videoCodecSupport = {
  _clientString: null,

  /**
   * Jellyfin-style `canPlayType` probe: any result other than `no` is taken
   * as "supported" (`probably` and `maybe` both count).
   * @param {string} mimeType - e.g. 'video/mp4; codecs="avc1.42E01E, mp4a.40.2"'
   * @param {HTMLVideoElement} [video] - optional element to use
   * @returns {boolean}
   */
  canPlayType(mimeType, video = undefined) {
    if (typeof document === 'undefined' || !document.createElement) return false;
    const el = video || document.createElement('video');
    if (typeof el.canPlayType !== 'function') return false;
    return !!el.canPlayType(mimeType).replace(/no/, '');
  },

  /**
   * H.264 8-bit (Baseline/Main/High profiles — virtually every browser).
   * Matches Jellyfin's `canPlayH264` probe.
   * @returns {boolean}
   */
  canPlayH264() {
    return this.canPlayType('video/mp4; codecs="avc1.42E01E, mp4a.40.2"');
  },

  /**
   * H.264 High-10 profile (10-bit). Rarely supported in browsers; only sent
   * to the server when genuinely supported so 10-bit H.264 can Direct Play.
   * Profile idc 0x6E = High 10, level 40 (0x28).
   * @returns {boolean}
   */
  canPlayH264High10() {
    return this.canPlayType('video/mp4; codecs="avc1.6E0028, mp4a.40.2"');
  },

  /**
   * HEVC support straight from the browser's decoder answer. The user agent is
   * never consulted: a browser that decodes HEVC declares it whatever it says
   * about itself.
   * @returns {boolean}
   */
  canPlayHEVC() {
    const hevcCodecs = ['hvc1.1.6.L93.B0', 'hvc1.1.6.L120.B0', 'hev1.1.6.L93.B0'];
    return hevcCodecs.some((codec) => this.canPlayType(`video/mp4; codecs="${codec}"`));
  },

  /**
   * AV1 in MP4 (`av01.0.15M.08` = Main 8-bit). AV1 'maybe' is accepted.
   * @returns {boolean}
   */
  canPlayAV1() {
    return this.canPlayType('video/mp4; codecs="av01.0.15M.08"');
  },

  /**
   * VP9 in WebM.
   * @returns {boolean}
   */
  canPlayVP9() {
    return this.canPlayType('video/webm; codecs="vp9"');
  },

  /**
   * VP8 in WebM.
   * @returns {boolean}
   */
  canPlayVP8() {
    return this.canPlayType('video/webm; codecs="vp8"');
  },

  /**
   * Audio-codec probes; tokens mirror the server's ClientCodecs audio set.
   * @returns {string[]}
   */
  audioProbeTokens() {
    const mp4 = (codec) => `audio/mp4; codecs="${codec}"`;
    const webm = (codec) => `audio/webm; codecs="${codec}"`;
    const probes = [
      ['aac', mp4('mp4a.40.2')],
      ['opus', webm('opus')],
      ['mp3', 'audio/mpeg'],
      ['flac', 'audio/flac'],
      ['ac3', mp4('ac-3')],
      ['eac3', mp4('ec-3')],
      ['dts', mp4('dts')],
      ['vorbis', webm('vorbis')],
    ];
    return probes.filter(([, mime]) => this.canPlayType(mime)).map(([token]) => token);
  },

  /**
   * Whether the browser can play an audio-codec token, answered from the same
   * probes the declaration uses. A missing track (`null`/`''`) needs no codec
   * and counts as playable.
   * @param {string|null|undefined} token
   * @returns {boolean}
   */
  canPlayAudioCodec(token) {
    if (!token) return true;
    return this.audioProbeTokens().includes(token);
  },

  /**
   * The client's supported codec set as a comma-joined capability string,
   * handed to the server as the `client` query parameter (server's
   * ClientCodecs::parse format: `h264-8,h264-10,hevc,av1,vp9,vp8,aac,…`;
   * only supported tokens are emitted). Probed tokens come first, followed by
   * the codecs an actual playback proved, read from the verified store, each
   * added only when not already present. Memoized; call `clearCache()` to
   * recompute.
   * @returns {string}
   */
  getClientCodecsString() {
    if (this._clientString !== null) return this._clientString;
    const parts = [];
    if (this.canPlayH264()) parts.push('h264-8');
    if (this.canPlayH264High10()) parts.push('h264-10');
    if (this.canPlayHEVC()) parts.push('hevc');
    if (this.canPlayAV1()) parts.push('av1');
    if (this.canPlayVP9()) parts.push('vp9');
    if (this.canPlayVP8()) parts.push('vp8');
    parts.push(...this.audioProbeTokens());
    for (const token of verifiedCodecs()) {
      if (!parts.includes(token)) parts.push(token);
    }
    this._clientString = parts.join(',');
    return this._clientString;
  },

  /**
   * Record that an actual playback proved `token`, so the next declaration
   * carries it. Unknown tokens are refused by the store.
   * @param {string|null|undefined} token
   */
  recordVerifiedCodec(token) {
    markCodecVerified(token);
    this.clearCache();
  },

  /**
   * Clear the memoized capability string, forcing the next call to re-probe.
   */
  clearCache() {
    this._clientString = null;
  },
};
