import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  HANDLE_HIT_PX,
  MIN_COLUMN_PX,
  buildColumns,
  chooseUnit,
  clampBound,
  clampView,
  createView,
  ensureSelectionVisible,
  fitAllScale,
  indexFromX,
  panView,
  placeLabels,
  selectionZoneAtX,
  translateSelection,
  xFromIndex,
  zoomToRange,
  zoomView,
} from '../frontend/src/lib/timelineLayout.js';
import { formatPeriodName, toMonthIndex } from '../frontend/src/lib/timeline.js';

const monthName = (month) =>
  [
    'January',
    'February',
    'March',
    'April',
    'May',
    'June',
    'July',
    'August',
    'September',
    'October',
    'November',
    'December',
  ][month - 1];
const format = { monthName, periodName: (index) => formatPeriodName(index, monthName) };

// A 60-year span with 240 populated months (every third month).
const denseModel = {
  minIndex: 0,
  maxIndex: 60 * 12 - 1,
  length: 60 * 12,
  populated: new Set(),
};
for (let i = 0; i < 60 * 12; i += 3) denseModel.populated.add(i);
const countInRange = (model, a, b) => {
  let count = 0;
  for (let i = a; i <= b; i += 1) if (model.populated.has(i)) count += 1;
  return count;
};
const buildColumnsFor = (unit, view, width) =>
  buildColumns({ unit, view, width, model: denseModel, format, countInRange });

test('fit-all scale shows the whole span and the view cannot escape it', () => {
  const width = 1200;
  const view = createView(width, denseModel);
  assert.equal(view.scale, fitAllScale(width, denseModel));
  assert.equal(view.origin, denseModel.minIndex);
  assert.equal(Math.round((denseModel.maxIndex + 1 - view.origin) * view.scale), width);

  const panned = panView({ view, deltaPx: 500, width, model: denseModel });
  assert.deepEqual(panned, view, 'a fully visible span cannot pan');

  const zoomedOut = clampView({ scale: view.scale / 10, origin: -500 }, width, denseModel);
  assert.deepEqual(zoomedOut, view, 'zooming out past the full span snaps back to it');
});

test('zooming anchors on the pointer and clamps to one month at the closest view', () => {
  const width = 1200;
  const view = createView(width, denseModel);
  const zoomed = zoomView({ view, factor: 4, anchorPx: 600, width, model: denseModel });
  const anchored = indexFromX(600, view);
  assert.ok(
    Math.abs(indexFromX(600, zoomed) - anchored) < 1,
    'anchor index stays under the pointer'
  );

  const maxed = zoomView({ view, factor: 1e6, anchorPx: 0, width, model: denseModel });
  assert.equal(maxed.scale, width, 'closest view spans exactly one month');
  assert.equal(Math.round(width / maxed.scale), 1);

  const pinned = zoomView({ view: maxed, factor: 1e6, anchorPx: 0, width, model: denseModel });
  assert.deepEqual(pinned, maxed, 'input at the zoom limit changes nothing');
});

test('panning clamps to the data span', () => {
  const width = 1200;
  const view = zoomView({
    view: createView(width, denseModel),
    factor: 8,
    anchorPx: 600,
    width,
    model: denseModel,
  });
  const farLeft = panView({ view, deltaPx: 1e6, width, model: denseModel });
  assert.equal(farLeft.origin, denseModel.minIndex);
  const farRight = panView({ view, deltaPx: -1e6, width, model: denseModel });
  assert.equal(farRight.origin, denseModel.maxIndex + 1 - width / farRight.scale);
});

test('granularity follows the pixel budget', () => {
  assert.equal(chooseUnit(0.2), 120, 'a 500-year span can only afford decades');
  assert.equal(chooseUnit(2.5), 12, 'a 40-year span affords years');
  assert.equal(chooseUnit(40), 1, 'a 2-year span affords months');
  assert.ok(12 * 2.5 >= MIN_COLUMN_PX);
});

test('columns tile the viewport, clip to the data and aggregate their counts', () => {
  const width = 1200;
  const view = createView(width, denseModel);
  const unit = chooseUnit(view.scale);
  const columns = buildColumnsFor(unit, view, width);

  assert.ok(columns.length > 0);
  assert.ok(columns.length <= Math.ceil(width / MIN_COLUMN_PX) + 1, 'node count stays bounded');
  assert.equal(columns[0].startIndex, denseModel.minIndex, 'first column clips to the data span');
  assert.equal(columns[columns.length - 1].endIndex, denseModel.maxIndex);
  for (let i = 1; i < columns.length; i += 1) {
    assert.equal(columns[i].gridStart, columns[i - 1].gridStart + unit, 'columns are contiguous');
    assert.equal(
      columns[i].count,
      countInRange(denseModel, columns[i].startIndex, columns[i].endIndex)
    );
  }
  assert.equal(columns[0].label, '0s', 'decade labels come from the grid-aligned start');
});

test('label placement never overlaps, clips or crowds a neighbour', () => {
  const width = 1200;
  const measure = (text) => text.length * 8;
  for (const scale of [0.5, 1, 2.5, 8, 40, 200, 1200]) {
    const view = clampView({ scale, origin: 300 }, width, denseModel);
    const columns = buildColumnsFor(chooseUnit(view.scale), view, width);
    const placed = placeLabels(columns, { width, measure });
    let lastRight = -Infinity;
    for (const column of placed) {
      if (column.labelX === null) continue;
      assert.ok(column.labelX >= 0, 'no label clipped on the left');
      assert.ok(column.labelX + column.labelWidth <= width, 'no label clipped on the right');
      assert.ok(column.labelX >= lastRight, `labels overlap at scale ${view.scale}`);
      lastRight = column.labelX + column.labelWidth + 8;
    }
  }
});

test('a 60-year span at every supported width keeps labels legible and bounded', () => {
  const measure = (text) => text.length * 7.5;
  for (const width of [769, 1024, 1280, 1374, 1920, 2560]) {
    const view = createView(width, denseModel);
    const columns = buildColumnsFor(chooseUnit(view.scale), view, width);
    const placed = placeLabels(columns, { width, measure });
    const labels = placed.filter((c) => c.labelX !== null);
    assert.ok(labels.length > 0, `width ${width} must label something`);
    for (const label of labels) {
      assert.ok(label.labelWidth + label.x >= 0);
      assert.ok(label.labelX + label.labelWidth <= width);
    }
  }
});

test('layout of a 100-year, 1200-month model stays far below the 100 ms budget', () => {
  const model = { minIndex: 0, maxIndex: 100 * 12 - 1, length: 100 * 12 };
  const count = (_, a, b) => Math.max(0, b - a + 1);
  const width = 1920;
  const started = performance.now();
  for (let i = 0; i < 200; i += 1) {
    const view = clampView({ scale: 1 + i * 0.05, origin: i * 7 }, width, model);
    const columns = buildColumns({
      unit: chooseUnit(view.scale),
      view,
      width,
      model,
      format,
      countInRange: count,
    });
    placeLabels(columns, { width, measure: (text) => text.length * 8 });
  }
  const elapsed = performance.now() - started;
  assert.ok(elapsed < 100, `200 layouts took ${elapsed.toFixed(1)} ms`);
});

test('hit-testing maps pixels to months and separates handles from the body', () => {
  const width = 1200;
  const view = { scale: 10, origin: toMonthIndex(2012, 1) };
  const selection = { startIndex: toMonthIndex(2012, 3), endIndex: toMonthIndex(2012, 8) };
  const startX = xFromIndex(selection.startIndex, view);
  const endX = xFromIndex(selection.endIndex + 1, view);

  assert.equal(selectionZoneAtX(startX + HANDLE_HIT_PX - 1, { selection, view, width }), 'start');
  assert.equal(selectionZoneAtX(endX - HANDLE_HIT_PX + 1, { selection, view, width }), 'end');
  assert.equal(selectionZoneAtX((startX + endX) / 2, { selection, view, width }), 'body');
  assert.equal(selectionZoneAtX(startX - 200, { selection, view, width }), null);
  assert.equal(selectionZoneAtX(50, { selection: null, view, width }), null);
  assert.equal(indexFromX(startX, view), selection.startIndex);
});

test('bound clamping and translation keep the range inside the data span', () => {
  const model = denseModel;
  const selection = { startIndex: 300, endIndex: 320 };

  assert.equal(clampBound(100, selection, 'start', model), 100, 'a start bound may move left');
  assert.equal(
    clampBound(400, selection, 'start', model),
    320,
    'a start bound cannot pass the end'
  );
  assert.equal(clampBound(100, selection, 'end', model), 300, 'an end bound cannot pass the start');
  assert.equal(clampBound(1e6, selection, 'end', model), model.maxIndex);

  assert.deepEqual(translateSelection(selection, 5, model), { startIndex: 305, endIndex: 325 });
  assert.deepEqual(translateSelection(selection, 0, model), selection);
  assert.deepEqual(translateSelection({ startIndex: 2, endIndex: 5 }, -10, model), {
    startIndex: 0,
    endIndex: 3,
  });
  assert.deepEqual(translateSelection({ startIndex: 700, endIndex: 719 }, 10, model), {
    startIndex: 719 - 19,
    endIndex: 719,
  });
});

test('selection visibility pans minimally and never zooms in', () => {
  const width = 1200;
  const view = zoomView({
    view: createView(width, denseModel),
    factor: 10,
    anchorPx: 0,
    width,
    model: denseModel,
  });
  const selection = { startIndex: 500, endIndex: 520 };
  const fixed = ensureSelectionVisible(selection, view, width, denseModel);

  assert.ok(fixed.scale <= view.scale, 'zooms out only when needed');
  const startX = xFromIndex(selection.startIndex, fixed);
  const endX = xFromIndex(selection.endIndex + 1, fixed);
  assert.ok(
    startX >= -0.001 && endX <= width + 0.001,
    'the whole selection is inside the viewport'
  );

  const zoomedOut = { scale: fitAllScale(width, denseModel), origin: denseModel.minIndex };
  assert.deepEqual(
    ensureSelectionVisible(selection, zoomedOut, width, denseModel),
    zoomedOut,
    'an already visible selection leaves the view untouched'
  );

  const wide = { startIndex: 100, endIndex: 900 };
  assert.ok(
    ensureSelectionVisible(wide, view, width, denseModel).scale < view.scale,
    'a selection wider than the viewport zooms out to fit'
  );
});

test('zooming to a range fills the viewport with it', () => {
  const width = 1200;
  const range = { startIndex: toMonthIndex(2012, 1), endIndex: toMonthIndex(2012, 12) };
  const view = zoomToRange(range, width, denseModel);
  const startX = xFromIndex(range.startIndex, view);
  const endX = xFromIndex(range.endIndex + 1, view);
  assert.ok(startX >= 0 && endX <= width);
  assert.ok(endX - startX > width * 0.6, 'the range dominates the viewport');
  assert.equal(view.scale, width / (12 * 1.2));
});

test('a zero-width viewport (hidden at the mobile breakpoint) yields an empty layout', () => {
  const view = createView(0, denseModel);
  assert.ok(Number.isFinite(view.scale));
  assert.equal(buildColumnsFor(1, view, 0).length, 0);
});

test('a single-month library has one zoom level', () => {
  const model = { minIndex: 100, maxIndex: 100, length: 1 };
  const view = createView(1200, model);
  assert.equal(view.scale, 1200);
  assert.deepEqual(zoomView({ view, factor: 4, anchorPx: 0, width: 1200, model }), view);
  assert.deepEqual(panView({ view, deltaPx: 400, width: 1200, model }), view);
  const columns = buildColumns({
    unit: chooseUnit(view.scale),
    view,
    width: 1200,
    model,
    format,
    countInRange: () => 1,
  });
  assert.equal(columns.length, 1);
  assert.equal(columns[0].startIndex, 100);
  assert.equal(columns[0].endIndex, 100);
});
