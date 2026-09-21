/**
 * The encoder names the server may report through `x-turbopix-encoder` (and
 * through the transcode status payload) that mean a GPU produced the bytes.
 *
 * These are ffmpeg's own encoder names. Anything outside this list — `libx264`,
 * an unknown name, an empty header, a missing one — is treated as "not
 * hardware": the hint may never claim a GPU it cannot name.
 */
export const HARDWARE_ENCODERS = [
  'h264_nvenc',
  'h264_vaapi',
  'h264_qsv',
  'h264_amf',
  'h264_videotoolbox',
];

/**
 * @param {string|null|undefined} encoder
 * @returns {boolean}
 */
export const isHardwareEncoder = (encoder) =>
  typeof encoder === 'string' && HARDWARE_ENCODERS.includes(encoder);
