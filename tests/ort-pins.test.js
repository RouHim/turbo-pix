import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const TARGETS = [
  'x86_64-unknown-linux-gnu',
  'x86_64-unknown-linux-musl',
  'aarch64-unknown-linux-musl',
];
const PIN_DIR = new URL('../scripts/ort/', import.meta.url);

const readPin = (target) => {
  const text = readFileSync(new URL(`${target}.env`, PIN_DIR), 'utf8');
  return {
    tag: text.match(/^ORT_TAG=(v\d+\.\d+\.\d+)$/m)?.[1],
    sha: text.match(/^ORT_SHA=([a-f0-9]{64})$/m)?.[1],
  };
};

test('ORT pins share one tag and carry full SHA-256 digests', () => {
  const pins = TARGETS.map((target) => ({ target, ...readPin(target) }));

  for (const pin of pins) {
    assert.ok(pin.tag, `${pin.target}: missing ORT_TAG`);
    assert.ok(pin.sha, `${pin.target}: missing 64-hex ORT_SHA`);
  }

  const tags = new Set(pins.map((pin) => pin.tag));
  assert.equal(
    tags.size,
    1,
    `ORT_TAG differs across targets: ${pins.map((pin) => `${pin.target}=${pin.tag}`).join(', ')}`
  );
});
