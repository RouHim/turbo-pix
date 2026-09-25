// Pure view geometry for the desktop timeline selector: month indices in,
// pixels out. No Svelte imports and no DOM access: label measurement is
// injected, so the collision rules are unit-testable.
import { fromMonthIndex, MONTHS_PER_DECADE, MONTHS_PER_YEAR } from './timeline.js';

/** Pointer/keyboard target floor and axe `target-size` minimum. */
export const MIN_COLUMN_PX = 28;
/** Minimum spacing between two rendered labels (FR-005). */
const MIN_LABEL_GAP_PX = 8;
/** Measured text is a hair narrower than rendered text; reserve a margin. */
const LABEL_SAFETY_PX = 4;
/**
 * Pointer grab zone around a selection edge: the *lane's* hit test, deliberately
 * narrower than the DOM handle. `.timeline-handle` is anchored inside its
 * selection edge and is 24px wide (the axe `target-size` floor, WCAG 2.5.8), so
 * a press that lands on the handle moves that bound even where this zone would
 * not claim it. The zone still decides everything else, and at the finest
 * granularity the arithmetic is tight: a 28px month column leaves a 2-month
 * range only an 8px translate band between its two handles (56px − 2 × 24px).
 * A single period is not a range (FR-009), so its lane body brushes rather
 * than translates.
 */
export const HANDLE_HIT_PX = 12;

/** Multiplicative step of the zoom controls. */
export const ZOOM_STEP = 1.6;
/** Candidate column units, finest first: month, year, decade. */
const UNITS = [1, MONTHS_PER_YEAR, MONTHS_PER_DECADE];
/** Padding drawn around a drilled-into range, as a fraction of its span. */
const ZOOM_TO_RANGE_PADDING = 0.2;

export const fitAllScale = (width, model) =>
  width <= 0 || model.length === 0 ? 1 : width / model.length;

/** Closest view: exactly one month fills the viewport. */
const maxScale = (width, model) => Math.max(width, fitAllScale(width, model));

const clampScale = (scale, width, model) =>
  Math.min(Math.max(scale, fitAllScale(width, model)), maxScale(width, model));

const clampOrigin = (origin, { width, scale, model }) => {
  if (width <= 0 || model.length === 0) return model.minIndex;
  const visible = width / scale;
  const maxOrigin = model.maxIndex + 1 - visible;
  if (maxOrigin <= model.minIndex) return model.minIndex;
  return Math.min(Math.max(origin, model.minIndex), maxOrigin);
};

export const createView = (width, model) => ({
  scale: fitAllScale(width, model),
  origin: model.minIndex,
});

export const clampView = (view, width, model) => {
  const scale = clampScale(view.scale, width, model);
  return { scale, origin: clampOrigin(view.origin, { width, scale, model }) };
};

export const zoomView = ({ view, factor, anchorPx, width, model }) => {
  const anchorIndex = view.origin + anchorPx / view.scale;
  const scale = clampScale(view.scale * factor, width, model);
  return { scale, origin: clampOrigin(anchorIndex - anchorPx / scale, { width, scale, model }) };
};

/** Positive `deltaPx` means the content was dragged that many pixels to the right. */
export const panView = ({ view, deltaPx, width, model }) => ({
  scale: view.scale,
  origin: clampOrigin(view.origin - deltaPx / view.scale, { width, scale: view.scale, model }),
});

/** Frame `range` with `paddingRatio` of its span left as breathing room. */
export const zoomToRange = (range, width, model, paddingRatio = ZOOM_TO_RANGE_PADDING) => {
  const span = range.endIndex - range.startIndex + 1;
  const scale = clampScale(width / (span * (1 + paddingRatio)), width, model);
  const center = (range.startIndex + range.endIndex + 1) / 2;
  const origin = center - width / (2 * scale);
  // A range outside the modelled span has nothing to clamp towards: pinning the
  // view to the span would push the range the caller asked for off-screen.
  const outside = range.endIndex < model.minIndex || range.startIndex > model.maxIndex;
  return { scale, origin: outside ? origin : clampOrigin(origin, { width, scale, model }) };
};

export const xFromIndex = (index, view) => (index - view.origin) * view.scale;
export const indexFromX = (x, view) => Math.floor(view.origin + x / view.scale);

/** The finest unit whose columns still meet the pointer-target floor. */
export const chooseUnit = (scale) =>
  UNITS.find((unit) => unit * scale >= MIN_COLUMN_PX) ?? UNITS[UNITS.length - 1];

/**
 * Relative margin keeping a clamped scale off both edges of its unit's band.
 * `chooseUnit` compares with `>=`, so a band edge that landed exactly on a
 * threshold would render the next finer unit; the margin makes
 * `chooseUnit(unitScaleMin(u)) === u` and `chooseUnit(unitScaleMax(u)) === u`
 * hold even after floating-point rounding (0.1% is ~0.028px on a 28px column).
 */
const BAND_MARGIN = 1e-3;

/** The next finer column unit, or null for the finest (a single month). */
export const finerUnit = (unit) =>
  unit === MONTHS_PER_DECADE ? MONTHS_PER_YEAR : unit === MONTHS_PER_YEAR ? 1 : null;

/** Narrowest scale that renders `unit`, just above `chooseUnit`'s threshold. */
export const unitScaleMin = (unit) => (MIN_COLUMN_PX * (1 + BAND_MARGIN)) / unit;

/** Widest scale that renders `unit`, just below the finer unit's threshold. */
export const unitScaleMax = (unit) => {
  const finer = finerUnit(unit);
  return finer === null ? Number.POSITIVE_INFINITY : (MIN_COLUMN_PX * (1 - BAND_MARGIN)) / finer;
};

/** Widest span (months) the lane can show at `unit`. */
export const unitWindow = (unit, width) => width / unitScaleMin(unit);

/** Narrowest span (months) that still renders `unit`; 0 for months. */
export const unitMinSpan = (unit, width) => width / unitScaleMax(unit);

/**
 * Whether the lane can render `unit` at all: a model span must fall inside the
 * unit's band. A ten-year library has no decade view, a lane thinner than one
 * month column has no month view, and both are no-ops rather than clamps.
 */
export const canRenderUnit = (unit, width, model) => {
  if (width <= 0 || model.length === 0) return false;
  const window = Math.min(unitWindow(unit, width), model.length);
  return window >= 1 && window > unitMinSpan(unit, width);
};

const formatColumnLabel = (unit, gridStart, format) => {
  const { year } = fromMonthIndex(gridStart);
  if (unit === MONTHS_PER_DECADE) return `${year - (year % 10)}s`;
  if (unit === MONTHS_PER_YEAR) return String(year);
  return format.periodName(gridStart);
};

export const buildColumns = ({ unit, view, width, model, format, countInRange }) => {
  const columns = [];
  if (width <= 0 || model.length === 0) return columns;

  const firstGridStart = Math.floor(view.origin / unit) * unit;
  const lastIndex = view.origin + width / view.scale;

  for (let gridStart = firstGridStart; gridStart < lastIndex; gridStart += unit) {
    const startIndex = Math.max(gridStart, model.minIndex);
    const endIndex = Math.min(gridStart + unit - 1, model.maxIndex);
    if (endIndex < startIndex) continue;
    columns.push({
      gridStart,
      startIndex,
      endIndex,
      x: xFromIndex(startIndex, view),
      width: (endIndex - startIndex + 1) * view.scale,
      count: countInRange(model, startIndex, endIndex),
      label: formatColumnLabel(unit, gridStart, format),
    });
  }
  return columns;
};

/**
 * Distribute labels left to right, dropping any that would be clipped by the
 * viewport or crowd the previously placed one. Returning `labelX: null` is the
 * whole collision strategy: no overlap, no clipping, no fake spacing maths.
 */
export const placeLabels = (columns, { width, measure }) => {
  let lastRight = -Infinity;
  return columns.map((column) => {
    const labelWidth = measure(column.label) + LABEL_SAFETY_PX;
    const labelX = Math.round(column.x + column.width / 2 - labelWidth / 2);
    if (labelX < 0 || labelX + labelWidth > width || labelX < lastRight + MIN_LABEL_GAP_PX) {
      return { ...column, labelX: null, labelWidth: null };
    }
    lastRight = labelX + labelWidth;
    return { ...column, labelX, labelWidth };
  });
};

export const selectionZoneAtX = (x, { selection, view, width, handlePx = HANDLE_HIT_PX }) => {
  if (!selection || width <= 0) return null;
  const startX = xFromIndex(selection.startIndex, view);
  const endX = xFromIndex(selection.endIndex + 1, view);
  if (x >= startX - handlePx && x <= startX + handlePx) return 'start';
  if (x >= endX - handlePx && x <= endX + handlePx) return 'end';
  if (x > startX && x < endX) return 'body';
  return null;
};

/** Clamp a dragged bound: it may not pass the opposite bound (no inversion). */
export const clampBound = (index, selection, bound, model) => {
  const clamped = Math.min(Math.max(index, model.minIndex), model.maxIndex);
  if (!selection) return clamped;
  return bound === 'start'
    ? Math.min(clamped, selection.endIndex)
    : Math.max(clamped, selection.startIndex);
};

export const translateSelection = (selection, deltaMonths, model) => {
  if (!selection || deltaMonths === 0) return selection;
  const span = selection.endIndex - selection.startIndex;
  const startIndex = Math.min(
    Math.max(selection.startIndex + deltaMonths, model.minIndex),
    model.maxIndex - span
  );
  return { startIndex, endIndex: startIndex + span };
};

/** FR-010: pan minimally, zoom out only when the selection cannot fit. */
export const ensureSelectionVisible = (selection, view, width, model) => {
  if (!selection || width <= 0) return view;
  const span = selection.endIndex - selection.startIndex + 1;
  const scale = clampScale(Math.min(view.scale, width / span), width, model);
  const startX = xFromIndex(selection.startIndex, { ...view, scale });
  const endX = xFromIndex(selection.endIndex + 1, { ...view, scale });
  let origin = view.origin;
  if (startX < 0) origin = selection.startIndex;
  else if (endX > width) origin = selection.endIndex + 1 - width / scale;
  return { scale, origin: clampOrigin(origin, { width, scale, model }) };
};

/** Clamp a period's span into the band that renders `unit`. */
const clampSpanToUnit = (span, unit, width) =>
  Math.min(Math.max(span, unitMinSpan(unit, width)), unitWindow(unit, width));

/** A view holding `span` months centred on `centre`, pinned to the model. */
const viewAround = (span, centre, width, model) => {
  const scale = clampScale(width / span, width, model);
  return { scale, origin: clampOrigin(centre - width / (2 * scale), { width, scale, model }) };
};

/**
 * FR-002/FR-003: frame `range` at `unit` so the new view renders exactly that
 * unit — the activation drill, one granularity level into the activated period.
 * The span is clamped into the unit's band, so a lane too narrow for the period
 * keeps the unit (showing part of it) and an ultra-wide lane does not refine
 * past it.
 */
export const zoomToUnitRange = (range, unit, width, model) =>
  viewAround(
    clampSpanToUnit(range.endIndex - range.startIndex + 1, unit, width),
    (range.startIndex + range.endIndex + 1) / 2,
    width,
    model
  );

/**
 * FR-007: a granularity control frames the active filter when that filter fits
 * the level's window, and otherwise a window around the current view centre —
 * it never writes a filter, so it takes no `onchange`. Returns `view` unchanged
 * when the lane cannot render the level at all (a no-op, like every other
 * unrenderable level).
 */
export const frameUnit = (unit, { selection, view, width, model }) => {
  if (!canRenderUnit(unit, width, model)) return view;
  const window = Math.min(unitWindow(unit, width), model.length);
  if (selection !== null && selection.endIndex - selection.startIndex + 1 <= window) {
    return viewAround(
      clampSpanToUnit(selection.endIndex - selection.startIndex + 1, unit, width),
      (selection.startIndex + selection.endIndex + 1) / 2,
      width,
      model
    );
  }
  return viewAround(window, view.origin + width / (2 * view.scale), width, model);
};
