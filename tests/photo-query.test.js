import { test } from 'node:test';
import assert from 'node:assert/strict';
import { routeToPhotoQuery } from '../frontend/src/lib/photoQuery.js';

/** Mirrors router.svelte.js: the URL's filter fields, parsed to numbers. */
function routeFromSearch(search) {
  const params = new URLSearchParams(search);
  const int = (key) => {
    const raw = params.get(key);
    return raw === null ? null : Number.parseInt(raw, 10);
  };
  return {
    view: params.get('view') || 'all',
    query: params.get('q'),
    sort: params.get('sort') || 'date_desc',
    year: int('year'),
    month: int('month'),
    to_year: int('to_year'),
    to_month: int('to_month'),
    album: int('album'),
  };
}

test('grid keeps the date range end as toYear/toMonth', () => {
  assert.deepEqual(
    routeToPhotoQuery(
      { view: 'all', sort: 'date_desc', year: 2012, month: 3, to_year: 2015, to_month: 9 },
      { view: 'grid' }
    ),
    { sort: 'date', order: 'desc', year: 2012, month: 3, toYear: 2015, toMonth: 9 }
  );
});

test('map keeps the date range end as toYear/toMonth', () => {
  assert.deepEqual(
    routeToPhotoQuery(
      {
        view: 'map',
        query: 'sunset',
        sort: 'date_asc',
        year: 2012,
        month: 3,
        to_year: 2015,
        to_month: 9,
      },
      { view: 'map' }
    ),
    {
      query: 'sunset',
      sort: 'date',
      order: 'asc',
      year: 2012,
      month: 3,
      toYear: 2015,
      toMonth: 9,
    }
  );
});

test('?year=2012&to_year=2015 yields the same range end for grid and map', () => {
  const route = routeFromSearch('?year=2012&to_year=2015');

  for (const view of ['grid', 'map']) {
    const params = routeToPhotoQuery(route, { view });
    assert.equal(params.year, 2012, `${view}: year`);
    assert.equal(params.toYear, 2015, `${view}: toYear`);
  }
});

test('favorites/videos query tokens are grid-only', () => {
  assert.equal(
    routeToPhotoQuery({ view: 'favorites', sort: 'date_desc' }, { view: 'grid' }).query,
    'is_favorite:true'
  );
  assert.equal(
    routeToPhotoQuery({ view: 'videos', sort: 'date_desc' }, { view: 'grid' }).query,
    'type:video'
  );

  const favoritesOnMap = routeToPhotoQuery(
    { view: 'favorites', sort: 'date_desc' },
    { view: 'map' }
  );
  assert.equal('query' in favoritesOnMap, false);
  const videosOnMap = routeToPhotoQuery({ view: 'videos', sort: 'date_desc' }, { view: 'map' });
  assert.equal('query' in videosOnMap, false);
});

test('map forwards the route query verbatim; the grid leaves it to the search pipeline', () => {
  assert.equal(
    routeToPhotoQuery({ view: 'map', query: 'location:Berlin' }, { view: 'map' }).query,
    'location:Berlin'
  );
  const grid = routeToPhotoQuery({ view: 'all', query: 'location:Berlin' }, { view: 'grid' });
  assert.equal('query' in grid, false);
});

test('absent filter fields are omitted', () => {
  assert.deepEqual(routeToPhotoQuery({}), {});
  assert.deepEqual(routeToPhotoQuery({ view: 'all', sort: 'date_desc' }), {
    sort: 'date',
    order: 'desc',
  });
  assert.deepEqual(
    routeToPhotoQuery({ view: 'all', year: null, month: null }, { view: 'map' }),
    {}
  );
});

test('sort field/order split matches the grid contract', () => {
  assert.deepEqual(routeToPhotoQuery({ sort: 'size_desc' }), { sort: 'size', order: 'desc' });
  assert.deepEqual(routeToPhotoQuery({ sort: 'name_asc' }), { sort: 'name', order: 'asc' });
  assert.deepEqual(routeToPhotoQuery({ sort: 'date' }), { sort: 'date', order: 'desc' });
});

test('album scope passes through for the map only', () => {
  // The grid's album detail sends `album` on its own route; the map's request
  // shape is the only one that carries it in the shared params.
  assert.equal(routeToPhotoQuery({ album: 7 }, { view: 'grid' }).album, undefined);
  assert.equal(routeToPhotoQuery({ album: 7 }, { view: 'map' }).album, 7);
});

test('grid is the default view', () => {
  assert.deepEqual(routeToPhotoQuery({ view: 'videos', sort: 'date_desc' }), {
    query: 'type:video',
    sort: 'date',
    order: 'desc',
  });
});
