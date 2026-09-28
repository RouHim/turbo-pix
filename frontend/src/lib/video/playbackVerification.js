/**
 * Remembers the codecs an actual playback proved this browser can decode.
 *
 * A capability query (`canPlayType`) is a hint that can be wrong; the only
 * trustworthy witness is a decoded first frame. The declaration the client
 * sends the server folds these tokens in, so a codec that really played is
 * declared even when the query said no.
 *
 * Only tokens the declaration can express are stored. Storage is a JSON array
 * of strings and every access is guarded: an unreadable, malformed or denied
 * store reads as empty and never throws, because a playback verdict is already
 * in hand and must survive a hostile `localStorage`.
 */

export const VERIFIED_CODECS_STORAGE_KEY = 'turbopix_verified_video_codecs';

/**
 * The codec vocabulary the capability declaration speaks, in the order
 * verification is expected to accumulate.
 */
const CODEC_TOKENS = ['h264-8', 'h264-10', 'hevc', 'av1', 'vp9', 'vp8'];

/**
 * The declaration token a codec and its bit depth map to. H.264 is the only
 * codec the declaration splits by depth: 10-bit is a different decoder than
 * 8-bit. Everything else is declared by codec alone, and anything the
 * declaration cannot name maps to `null`.
 *
 * @param {string|null|undefined} codec
 * @param {number|null|undefined} bitDepth
 * @returns {string|null}
 */
export const codecTokenFor = (codec, bitDepth) => {
  if (codec === 'h264') {
    return bitDepth > 8 ? 'h264-10' : 'h264-8';
  }
  return CODEC_TOKENS.includes(codec) ? codec : null;
};

/**
 * Keeps only declaration tokens, first occurrence wins.
 *
 * @param {unknown} value
 * @returns {string[]}
 */
const normalizeTokens = (value) => {
  if (!Array.isArray(value)) return [];
  const seen = new Set();
  const tokens = [];
  for (const token of value) {
    if (typeof token === 'string' && CODEC_TOKENS.includes(token) && !seen.has(token)) {
      seen.add(token);
      tokens.push(token);
    }
  }
  return tokens;
};

/**
 * The persisted tokens, deduplicated, in insertion order. Any storage failure
 * reads as empty rather than breaking the caller.
 *
 * @returns {string[]}
 */
export const verifiedCodecs = () => {
  if (typeof localStorage === 'undefined') return [];
  try {
    const stored = localStorage.getItem(VERIFIED_CODECS_STORAGE_KEY);
    return stored === null ? [] : normalizeTokens(JSON.parse(stored));
  } catch {
    return [];
  }
};

/**
 * Records that a real playback proved `token`, then returns the resulting
 * list. Unknown tokens are refused: the declaration cannot express them. A
 * denied write is swallowed so the caller still learns what was verified.
 *
 * @param {string|null|undefined} token
 * @returns {string[]}
 */
export const markCodecVerified = (token) => {
  const current = verifiedCodecs();
  if (!CODEC_TOKENS.includes(token) || current.includes(token)) return current;
  const next = [...current, token];
  if (typeof localStorage !== 'undefined') {
    try {
      localStorage.setItem(VERIFIED_CODECS_STORAGE_KEY, JSON.stringify(next));
    } catch {
      // A denied write must not discard the verdict the caller just observed.
    }
  }
  return next;
};
