import { test } from 'node:test';
import assert from 'node:assert/strict';

import { HARDWARE_ENCODERS, isHardwareEncoder } from '../frontend/src/lib/video/encoderHint.js';

/**
 * The encoder names the server's probe can select, spelled out here on purpose:
 * `src/video_encoder.rs` (`HwEncoder::ALL`, in preference order) reports exactly
 * these through `x-turbopix-encoder` and the transcode status. Asserting the
 * list against itself would pass vacuously for an empty list and for a
 * misspelled or renamed entry, which is precisely what this test exists to
 * catch — a name the server sends but the classifier does not know silently
 * downgrades a GPU conversion to a CPU claim.
 */
const SERVER_ENCODERS = ['h264_nvenc', 'h264_vaapi', 'h264_qsv', 'h264_amf', 'h264_videotoolbox'];

test('every encoder the server may report as hardware classifies as hardware', () => {
  assert.deepEqual(
    HARDWARE_ENCODERS,
    SERVER_ENCODERS,
    'the classifier list drifted from the server'
  );
  for (const encoder of SERVER_ENCODERS) {
    assert.equal(isHardwareEncoder(encoder), true, encoder);
  }
});

test('software, empty and unknown values never classify as hardware', () => {
  for (const value of ['libx264', 'hevc_vaapi', '', '  ', null, undefined, 0]) {
    assert.equal(isHardwareEncoder(value), false, String(value));
  }
});
