import { test } from 'node:test';
import assert from 'node:assert/strict';

import { HARDWARE_ENCODERS, isHardwareEncoder } from '../frontend/src/lib/video/encoderHint.js';

test('every encoder the server may report as hardware classifies as hardware', () => {
  for (const encoder of HARDWARE_ENCODERS) {
    assert.equal(isHardwareEncoder(encoder), true, encoder);
  }
});

test('software, empty and unknown values never classify as hardware', () => {
  for (const value of ['libx264', 'hevc_vaapi', '', '  ', null, undefined, 0]) {
    assert.equal(isHardwareEncoder(value), false, String(value));
  }
});
