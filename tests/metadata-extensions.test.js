/**
 * Writable-container parity guard.
 *
 * The viewer's edit button is enabled from the frontend's
 * METADATA_VIDEO_EXTENSIONS (`frontend/src/lib/constants.js`), while the PATCH
 * endpoint accepts a container from the backend's WRITABLE_EXTENSIONS
 * (`src/mp4_metadata.rs`). Nothing else connects the two: drop an extension on
 * either side and the editor either offers a save the server refuses
 * (`unsupported_container`) or hides a save the server would have taken, with
 * no failing test anywhere. The backend list is parsed out of the Rust source
 * rather than restated here, so a new writable container fails this test until
 * the frontend catches up.
 *
 * The assertions are on the constant `isMetadataEditable` consults
 * (`utils.js`) rather than on `isMetadataEditable` itself: `utils.js` imports
 * `i18n.js`, which imports `en.json` without a JSON import attribute, so
 * `node --test` dies at link time before any of its code runs. The container
 * walk the editor drives end to end is covered in
 * `tests/e2e/specs/video-metadata.e2e.spec.js`.
 *
 * Usage: node --test tests/metadata-extensions.test.js  (part of npm run test:unit)
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { APP_CONSTANTS } from '../frontend/src/lib/constants.js';

const MP4_METADATA = new URL('../src/mp4_metadata.rs', import.meta.url);

/**
 * Parse the extensions the backend will rewrite, straight out of its own
 * constant. The declaration carries an explicit array length, so the parsed
 * entries are counted against it: a regex that stops matching — a type change
 * to `[&[&str]]`, an entry written some other way — then fails loudly instead
 * of yielding an empty (and thus vacuously equal) set.
 */
function parseBackendWritableExtensions() {
  const source = readFileSync(MP4_METADATA, 'utf8');
  const declaration = /pub const WRITABLE_EXTENSIONS: \[&str; (\d+)\] = \[([^\]]*)\]/.exec(source);
  assert.ok(
    declaration,
    'could not find `pub const WRITABLE_EXTENSIONS: [&str; N] = [...]` in src/mp4_metadata.rs'
  );
  const extensions = [...declaration[2].matchAll(/"([a-z0-9]+)"/g)].map((match) => match[1]);
  assert.equal(
    extensions.length,
    Number(declaration[1]),
    'the declared length of WRITABLE_EXTENSIONS must match the entries parsed out of it'
  );
  return extensions;
}

/** The frontend's list, in the dotted lowercase shape both sides compare in. */
const frontendWritable = APP_CONSTANTS.METADATA_VIDEO_EXTENSIONS.map((ext) =>
  ext.replace(/^\./, '').toLowerCase()
);

test('the editor is offered for exactly the containers the endpoint can rewrite', () => {
  assert.deepEqual(frontendWritable, parseBackendWritableExtensions());
});

test('MOV and M4V are writable, not just MP4', () => {
  // Named one by one: the spec, the localized `unsupported_container` message
  // and the backend's own table all promise these three, so losing one to a
  // list edit has to fail here rather than quietly narrow the feature.
  for (const extension of ['.mp4', '.mov', '.m4v']) {
    assert.ok(
      APP_CONSTANTS.METADATA_VIDEO_EXTENSIONS.includes(extension),
      `${extension} must stay editable`
    );
  }
  // The containers the viewer plays but the endpoint cannot rewrite. Offering
  // the editor for one of them would be a save that is always refused.
  for (const extension of ['.mkv', '.webm', '.avi']) {
    assert.ok(
      !APP_CONSTANTS.METADATA_VIDEO_EXTENSIONS.includes(extension),
      `${extension} must stay read-only`
    );
  }
});

test('every writable container is one the viewer already treats as a video', () => {
  // `isMetadataEditable` answers from the FILENAME, so a container missing
  // from VIDEO_EXTENSIONS would be offered an editor on a record the viewer
  // does not even render as a video — and a RAW extension in the writable list
  // would offer the container editor on a photo.
  for (const extension of frontendWritable) {
    const dotted = `.${extension}`;
    assert.ok(
      APP_CONSTANTS.VIDEO_EXTENSIONS.includes(dotted),
      `${dotted} is editable but not a known video container`
    );
    assert.ok(
      !APP_CONSTANTS.RAW_EXTENSIONS.includes(dotted),
      `${dotted} is both a RAW format and an editable video container`
    );
  }
});
