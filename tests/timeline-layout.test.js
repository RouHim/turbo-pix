import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  HANDLE_HIT_PX,
  MIN_COLUMN_PX,
  buildColumns,
  canRenderUnit,
  chooseUnit,
  clampBound,
  clampView,
  createView,
  ensureSelectionVisible,
  finerUnit,
  fitAllScale,
  frameUnit,
  indexFromX,
  panView,
  placeLabels,
  selectionZoneAtX,
  translateSelection,
  unitMinSpan,
  unitScaleMax,
  unitScaleMin,
  unitWindow,
  xFromIndex,
  zoomToRange,
  zoomToUnitRange,
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

test('a pinch is one zoom step anchored between the two pointers', () => {
  const width = 1200;
  const view = createView(width, denseModel);
  const leftPx = 400;
  const rightPx = 800;
  const midpoint = (leftPx + rightPx) / 2;

  // Fingers spreading to twice their distance is factor 2 about the midpoint.
  const pinched = zoomView({ view, factor: 2, anchorPx: midpoint, width, model: denseModel });
  assert.equal(pinched.scale, view.scale * 2);
  assert.ok(
    Math.abs(indexFromX(midpoint, pinched) - indexFromX(midpoint, view)) < 1,
    'the pinch midpoint stays under the fingers'
  );

  // The factor is multiplicative, so the pointer-event plumbing can feed one
  // step per move without drifting from a single combined step.
  const stepwise = zoomView({
    view: zoomView({ view, factor: 1.25, anchorPx: midpoint, width, model: denseModel }),
    factor: 1.6,
    anchorPx: midpoint,
    width,
    model: denseModel,
  });
  assert.ok(Math.abs(stepwise.scale - pinched.scale) < 1e-9);
  assert.ok(Math.abs(stepwise.origin - pinched.origin) < 1e-9);
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

test('the level bands sit strictly inside the unit thresholds', () => {
  // Every scale the drill and the level controls clamp to must render exactly
  // the requested unit: a band edge landing on `chooseUnit`'s threshold would
  // silently drill into the wrong granularity (months after a decade click at a
  // 4K width) or fall back to a coarser one on a narrow lane.
  for (const width of [300, 640, 769, 1024, 1200, 1920, 2560, 4000]) {
    for (const unit of [1, 12, 120]) {
      assert.ok(unitScaleMin(unit) <= unitScaleMax(unit), `${unit}: empty band`);
      assert.equal(chooseUnit(unitScaleMin(unit)), unit, `${unit}: band floor`);
      assert.equal(chooseUnit(unitScaleMax(unit)), unit, `${unit}: band ceiling`);
      assert.equal(chooseUnit(width / unitWindow(unit, width)), unit, `${unit}: widest window`);
      assert.equal(chooseUnit(width / unitMinSpan(unit, width)), unit, `${unit}: narrowest span`);
    }
  }
  assert.equal(finerUnit(120), 12);
  assert.equal(finerUnit(12), 1);
  assert.equal(finerUnit(1), null);
});

test('a window holds about width / MIN_COLUMN_PX month columns', () => {
  for (const width of [640, 1200, 1920]) {
    const months = unitWindow(1, width);
    assert.ok(Math.abs(months - width / MIN_COLUMN_PX) < (width / MIN_COLUMN_PX) * 0.01);
    assert.ok(unitWindow(12, width) > months && unitWindow(120, width) > unitWindow(12, width));
  }
  assert.equal(unitMinSpan(1, 1200), 0, 'months are the finest unit: no lower bound');
});

test('a level is renderable exactly when the model reaches into its band', () => {
  const long = { length: 771 };
  const short = { length: 120 };
  const tiny = { length: 20 };
  assert.equal(canRenderUnit(1, 1200, long), true);
  assert.equal(canRenderUnit(12, 1200, long), true);
  assert.equal(canRenderUnit(120, 1200, long), true);
  assert.equal(canRenderUnit(120, 1200, short), false, 'a ten-year library has no decade view');
  assert.equal(canRenderUnit(12, 1200, tiny), false);
  assert.equal(canRenderUnit(1, 1200, { length: 0 }), false, 'an empty model renders nothing');
  assert.equal(canRenderUnit(1, 0, long), false, 'a hidden lane renders nothing');
  assert.equal(canRenderUnit(1, 20, long), false, 'a lane thinner than one month column');
});

// The fixture library: March 1962 … January 2026, like the E2E seeds.
const legacyModel = (() => {
  const minIndex = toMonthIndex(1962, 3);
  const maxIndex = toMonthIndex(2026, 1);
  return { minIndex, maxIndex, length: maxIndex - minIndex + 1 };
})();

test('a drill frames the activated period at the next finer unit', () => {
  const width = 1400;
  const decade = { startIndex: toMonthIndex(1960, 1), endIndex: toMonthIndex(1969, 12) };
  const drilled = zoomToUnitRange(decade, 12, width, legacyModel);
  assert.equal(chooseUnit(drilled.scale), 12, 'a decade shows its year columns');
  // The decade reaches two years left of the data, so its origin clamps to the
  // library's first bucket: the data starts at the lane edge and the decade's
  // remaining 94 months still fit the lane.
  assert.equal(
    Math.round(xFromIndex(legacyModel.minIndex, drilled)),
    0,
    'the data start pins the lane edge'
  );
  assert.ok(
    xFromIndex(decade.endIndex + 1, drilled) <= width,
    'a decade clipped at its start still fits the lane'
  );

  const eighties = { startIndex: toMonthIndex(1980, 1), endIndex: toMonthIndex(1989, 12) };
  const framed = zoomToUnitRange(eighties, 12, width, legacyModel);
  assert.equal(Math.round(xFromIndex(eighties.startIndex, framed)), 0);
  assert.equal(Math.round(xFromIndex(eighties.endIndex + 1, framed)), width);

  const year = { startIndex: toMonthIndex(2012, 1), endIndex: toMonthIndex(2012, 12) };
  const months = zoomToUnitRange(year, 1, width, legacyModel);
  assert.equal(chooseUnit(months.scale), 1, 'a year shows its month columns');
  assert.equal(Math.round(xFromIndex(year.startIndex, months)), 0);
  assert.equal(Math.round(xFromIndex(year.endIndex + 1, months)), width);

  const march = { startIndex: toMonthIndex(2012, 3), endIndex: toMonthIndex(2012, 3) };
  assert.equal(
    zoomToUnitRange(march, 1, width, legacyModel).scale,
    width,
    'one month fills the lane'
  );

  // FR-003 on a lane too narrow for the finer unit: the unit is kept and the
  // period shows partially instead of falling back to a coarser granularity.
  for (const narrow of [200, 300]) {
    assert.equal(chooseUnit(zoomToUnitRange(decade, 12, narrow, legacyModel).scale), 12);
    assert.equal(chooseUnit(zoomToUnitRange(year, 1, narrow, legacyModel).scale), 1);
  }
  // The mirror image at an ultra-wide lane: the drill caps at the finer unit's
  // ceiling instead of over-refining into months.
  assert.equal(chooseUnit(zoomToUnitRange(decade, 12, 4000, legacyModel).scale), 12);
});

test('a level control frames a filter that fits and a window around the view centre otherwise', () => {
  const width = 1200;
  const view = createView(width, legacyModel);
  const year2012 = { startIndex: toMonthIndex(2012, 1), endIndex: toMonthIndex(2012, 12) };

  // Month level: the year fits the month window, so it fills the lane.
  const months = frameUnit(1, { selection: year2012, view, width, model: legacyModel });
  assert.equal(chooseUnit(months.scale), 1);
  assert.equal(Math.round(xFromIndex(year2012.startIndex, months)), 0);
  assert.equal(Math.round(xFromIndex(year2012.endIndex + 1, months)), width);

  // Year level: a one-year filter is narrower than the year band's floor, so the
  // lane keeps year columns and centres the filter inside a wider window.
  const years = frameUnit(12, { selection: year2012, view, width, model: legacyModel });
  assert.equal(chooseUnit(years.scale), 12);
  assert.ok(xFromIndex(year2012.startIndex, years) >= 0);
  assert.ok(xFromIndex(year2012.endIndex + 1, years) <= width);

  // Decade level without a filter: the widest window the level can render, which
  // for a 64-year library is the whole span.
  const decades = frameUnit(120, { selection: null, view, width, model: legacyModel });
  assert.equal(chooseUnit(decades.scale), 120);
  assert.deepEqual(decades, createView(width, legacyModel));

  // A filter wider than the level's window frames around the current view
  // centre instead (FR-007), never around the filter's own centre.
  const wide = { startIndex: toMonthIndex(1960, 1), endIndex: toMonthIndex(1979, 12) };
  const windowed = frameUnit(1, { selection: wide, view, width, model: legacyModel });
  assert.equal(chooseUnit(windowed.scale), 1);
  const viewCentre = view.origin + width / (2 * view.scale);
  const windowCentre = windowed.origin + width / (2 * windowed.scale);
  assert.ok(Math.abs(windowCentre - viewCentre) < 0.001, 'the window stays on the view centre');

  // A level the lane cannot render leaves the view untouched.
  const shortModel = { minIndex: 0, maxIndex: 119, length: 120 };
  assert.deepEqual(frameUnit(120, { selection: null, view, width, model: shortModel }), view);
});
