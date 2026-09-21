import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  buildMapFilters,
  fetchSemanticPhotoSet,
  formatCoordinates,
  getLocationLabel,
  getPhotoCoordinates,
  groupPhotosByLocation,
  isSemanticQuery,
  wrapLongitudeForView,
} from '../frontend/src/lib/map.js';

const photo = (hash, latitude, longitude, extra = {}) => ({
  hash_sha256: hash,
  metadata: { location: { latitude, longitude, ...extra } },
});

test('getPhotoCoordinates accepts a complete numeric pair', () => {
  assert.deepEqual(getPhotoCoordinates(photo('a', 48.1, 11.5)), {
    latitude: 48.1,
    longitude: 11.5,
  });
});

test('getPhotoCoordinates rejects missing, non-numeric, and out-of-range values', () => {
  assert.equal(getPhotoCoordinates(photo('a', undefined, 11.5)), null);
  assert.equal(getPhotoCoordinates(photo('a', 48.1, undefined)), null);
  assert.equal(getPhotoCoordinates(photo('a', '48.1', '11.5')), null);
  assert.equal(getPhotoCoordinates(photo('a', Number.NaN, 11.5)), null);
  assert.equal(getPhotoCoordinates(photo('a', 91, 11.5)), null);
  assert.equal(getPhotoCoordinates(photo('a', 48.1, 181)), null);
  assert.equal(getPhotoCoordinates({ hash_sha256: 'a', metadata: {} }), null);
  assert.equal(getPhotoCoordinates(null), null);
});

test('groupPhotosByLocation collapses identical coordinates and preserves order', () => {
  const berlin = photo('a', 52.52, 13.405, { city: 'Berlin' });
  const berlin2 = photo('b', 52.52, 13.405);
  const munich = photo('c', 48.137, 11.575);
  const unlocated = { hash_sha256: 'd', metadata: {} };

  const locations = groupPhotosByLocation([berlin, unlocated, berlin2, munich]);

  assert.equal(locations.length, 2);
  assert.equal(locations[0].key, '52.52,13.405');
  assert.deepEqual(
    locations[0].photos.map((entry) => entry.hash_sha256),
    ['a', 'b']
  );
  assert.equal(locations[1].key, '48.137,11.575');
});

test('getLocationLabel prefers the first resolved city and returns null otherwise', () => {
  const locations = groupPhotosByLocation([photo('a', 1, 2), photo('b', 1, 2, { city: 'Berlin' })]);
  assert.equal(getLocationLabel(locations[0]), 'Berlin');

  const withoutCity = groupPhotosByLocation([photo('c', 3, 4)]);
  assert.equal(getLocationLabel(withoutCity[0]), null);
});

test('formatCoordinates prints six decimals like the metadata panel', () => {
  assert.equal(
    formatCoordinates({ latitude: 48.1372, longitude: 11.5755 }),
    '48.137200, 11.575500'
  );
});

test('wrapLongitudeForView leaves a longitude already inside the window untouched', () => {
  assert.equal(wrapLongitudeForView(13.4, -155.35, 182.15), 13.4);
  assert.equal(wrapLongitudeForView(-74, -155.35, 182.15), -74);
  assert.equal(wrapLongitudeForView(0, 0, 10), 0);
  assert.equal(wrapLongitudeForView(10, 0, 10), 10);
});

test('wrapLongitudeForView moves a longitude onto its visible ±360 copy', () => {
  // A window panned east across the seam shows the +360 copy.
  assert.equal(wrapLongitudeForView(-175, 170, 200), 185);
  assert.equal(wrapLongitudeForView(-170, 21.25, 358.75), 190);
  // The mirror: a window panned west across the seam shows the -360 copy.
  assert.equal(wrapLongitudeForView(175, -198.75, 138.75), -185);
  // Panning is unbounded, so the window can sit a whole world further east
  // than the probe above: the +720 copy is the visible one.
  assert.equal(wrapLongitudeForView(13.4, 720, 1058.8), 733.4);
});

test('wrapLongitudeForView keeps a longitude with no visible copy as-is', () => {
  assert.equal(wrapLongitudeForView(-100, 10, 20), -100);
  assert.equal(wrapLongitudeForView(100, 10, 20), 100);
});

test('wrapLongitudeForView never pushes an on-screen longitude out of view', () => {
  // 1920px at zoom 3 centred on Berlin (13.4°): window [-155.35, 182.15].
  // The old edge-only check shifted every negative longitude by +360 and drew
  // this photo at 286°, 104° beyond the right edge.
  assert.equal(wrapLongitudeForView(-74, -155.35, 182.15), -74);
  // Mirror window centred on -30° (west -198.75, east 138.75): a positive
  // longitude inside it must not become L - 360.
  assert.equal(wrapLongitudeForView(120, -198.75, 138.75), 120);
  assert.equal(wrapLongitudeForView(13.4, -198.75, 138.75), 13.4);
});

test('buildMapFilters mirrors the grid filter construction', () => {
  assert.deepEqual(
    buildMapFilters({ query: 'location:Berlin', sort: 'date_asc', year: 2024, month: 5 }),
    { query: 'location:Berlin', sort: 'date', order: 'asc', year: 2024, month: 5 }
  );
  assert.deepEqual(buildMapFilters({ query: null, sort: 'size_desc', year: null, month: null }), {
    query: null,
    sort: 'size',
    order: 'desc',
    year: undefined,
    month: undefined,
  });
});

test('isSemanticQuery treats prefix queries as regular searches', () => {
  assert.equal(isSemanticQuery('location:Berlin'), false);
  assert.equal(isSemanticQuery('type:video'), false);
  assert.equal(isSemanticQuery('is_favorite:true'), false);
  assert.equal(isSemanticQuery('sunset over the lake'), true);
  assert.equal(isSemanticQuery(null), false);
});

test('fetchSemanticPhotoSet pages all results, keeps order, and skips broken photos', async () => {
  const calls = [];
  const client = {
    semanticSearch: async (query, limit, offset) => {
      calls.push({ query, limit, offset });
      if (offset === 0) {
        // Simulate a full page so the loop must continue with the next offset.
        return { results: Array.from({ length: 200 }, (_, i) => ({ hash: `h${i}` })) };
      }
      return { results: [{ hash: 'h200' }] };
    },
    getPhoto: async (hash) => {
      if (hash === 'h5') throw new Error('gone');
      return { hash_sha256: hash, metadata: { location: { latitude: 1, longitude: 2 } } };
    },
  };

  const photos = await fetchSemanticPhotoSet(client, 'dogs');
  assert.deepEqual(calls, [
    { query: 'dogs', limit: 200, offset: 0 },
    { query: 'dogs', limit: 200, offset: 200 },
  ]);
  assert.equal(photos.length, 200);
  assert.equal(photos[0].hash_sha256, 'h0');
  assert.equal(photos.at(-1).hash_sha256, 'h200');
  assert.ok(!photos.some((entry) => entry.hash_sha256 === 'h5'));
});
