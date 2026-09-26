# Timeline Selector Year/Month Drill-Down Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make one activation on any rendered period — decade, year or month — select exactly that period and zoom one level into it, and add Decade / Year / Month granularity controls that reach month columns in a single action, without changing the filter.

**Architecture:** The lane's granularity is *derived* (`unit = chooseUnit(view.scale)`, `frontend/src/lib/timelineLayout.js`) as the spec's Key Entities require. This plan adds the inverse mapping — the scale band that renders a given unit (`unitScaleMin`/`unitScaleMax`, spans `unitMinSpan`/`unitWindow`) — plus two framing functions: `zoomToUnitRange` (activation drill: the period's own span, clamped into the target unit's band, so the view can never land on a coarser unit) and `frameUnit` (granularity controls: frame the active filter when it fits the level's window, else a window around the current view centre, never writing a filter). A grid-aligned decade becomes a first-class *period* in `timelineRoute.js` (it keeps its own January–December bounds like a bare year), and decade labels become localizable ("1960s" / "1960er").

**Tech Stack:** Svelte 5 runes, plain JS modules unit-tested with `node --test`, Playwright E2E against the real Rust backend, sqlx/SQLite (untouched — no schema change).

**Spec:** `.spec/timeline-selector-year-month-drilldown.md`

## Global Constraints

- Scope is the desktop selector only (`frontend/src/components/TimelineSelector.svelte`, hosted by `TimelineSlider.svelte`); below the 768px breakpoint the mobile year/month dropdowns stay unchanged.
- `MIN_COLUMN_PX = 28` (axe `target-size` floor) stays the pointer-target floor; every clamped scale must keep the unit's columns at or above it.
- No new filter semantics: a decade, a year and a month are all inclusive month ranges in the existing route shape (`year`/`month`/`to_year`/`to_month`).
- Every new user-visible string lands in **both** `frontend/src/i18n/en.json` and `frontend/src/i18n/de.json` with identical structure; `npm run test:i18n` must pass (`tests/i18n-integrity.test.js` is wired into CI).
- Zero lint/format issues: `npm run lint`, `npm run format`. Rust untouched, so `cargo clippy --all-targets -D warnings` only has to stay green.
- Frontend build order is load-bearing: `npm run build` before `cargo build --bin turbo-pix` (`build.rs` panics without `dist/`).
- Breaking behavior changes are allowed; every existing test that pinned the old decade behavior is updated in the same task, never left red.
- Source files are never mutated by this feature; no new runtime dependencies.

## Review Focus

Failure modes the spec implies but no single acceptance criterion names — each one gets its own test in the task that owns the code:

1. **Narrow desktop lanes (769–1024px viewports).** A drill or a level control must still land on the requested granularity and keep 28px columns; the clamp arithmetic is the only thing preventing a coarser fallback. Tested in Tasks 1/2 at widths 300/640/769/1024/1200/1920/2560/4000 and by the axe `target-size` scan.
2. **Library edges clipped inside the activated period.** The oldest bucket is March 1962, so the 1960s decade reaches left of the data and the newest decade reaches right of it: the label must still name the decade, the URL must round-trip unchanged, the overlay/handles must stay in the lane and the grid must hold exactly that decade's photos. Tested in Tasks 4/5.
3. **Empty periods inside a populated band** (the 1990s decade, April 2012) reached by activation, by arrow navigation and by a restored filter (`?year=1990&to_year=1999`): no filter may be applied, the view must not move, and a restored empty period must be cleared by the existing rule without the level controls resurrecting it. Tested in Tasks 4/5/6.
4. **Hand-edited / non-canonical URLs** (`?year=1962&month=1&to_year=1969&to_month=12`, reversed bounds, ranges with no overlap): the new decade branch must not swallow them — only a grid-aligned decade keeps its own bounds, every other range keeps clamping. Tested in Task 4.
5. **Writes that must not happen.** A granularity control must never write a filter; a commit whose canonical filter equals the active one must not add a history entry (otherwise Back appears to do nothing). Tested in Tasks 6/7.

## Decisions taken where the spec is silent

1. **The level is the rendered column unit**, not separate state: `unit = chooseUnit(view.scale)`, so the pressed control (`aria-pressed`) always equals what the lane renders — spec Key Entities ("derived from the view scale and the library's span").
2. **A control whose level is already rendered is a no-op**, including when the filter would fit a different framing (Scenario 2 acceptance 3).
3. **A grid-aligned decade filter is a period**: it keeps its own January-to-December-ten-year bounds (like a bare year), so the header label reads "1960s", the round trip through `filterFromSelection` is the identity, and the handles stay grabbable at the true bound.
4. **The decade suffix is localizable**: `ui.timeline_decade_label` = `"{start}s"` (en) / `"{start}er"` (de), replacing the hardcoded `s` in the ruler labels (today German readers see "1960s").
5. **A commit that writes the same canonical filter is skipped** — no duplicate history entry on re-activation (FR-010).
6. **The mobile year dropdown gains an option for a filtered year the library has no bucket in** (a decade filter names 1960 while the oldest bucket is March 1962); without it the select would silently fall back to "All Years" while the grid stays filtered.
7. **`BAND_MARGIN = 1e-3`** keeps every clamped scale strictly inside its unit's band: `chooseUnit(width / unitWindow(u, width)) === u` and `chooseUnit(width / unitMinSpan(u, width)) === u` hold exactly, so floating-point rounding can never flip a drill into the wrong granularity.
8. A lane narrower than `MIN_COLUMN_PX * span` (a decade needs ~280px at year level) shows part of the period but keeps the level: FR-003 (never coarsen) wins. Unreachable at supported desktop widths (the lane is > 600px from 769px upwards), asserted anyway in Task 2.

## File Structure

| File | Responsibility after this plan |
| --- | --- |
| `frontend/src/lib/timelineLayout.js` | Geometry only. Gains the unit band (`finerUnit`, `unitScaleMin`, `unitScaleMax`, `unitWindow`, `unitMinSpan`, `canRenderUnit`) and the two framings (`zoomToUnitRange`, `frameUnit`); `formatColumnLabel` now takes the decade label from the injected `format`. |
| `frontend/src/lib/timeline.js` | Pure date model. `formatSelectionLabel` gains the grid-aligned decade case. |
| `frontend/src/lib/timelineRoute.js` | Route ⇄ selection. Gains `isDecadeFilter` and the decade-is-a-period branch in `selectionFromFilter`. |
| `frontend/src/components/TimelineSelector.svelte` | The lane. `activateColumn` commits every period and drills one level in; new `LEVELS` list + `activateLevel` + level control markup. |
| `frontend/src/components/TimelineSlider.svelte` | Host. Passes `decadeLabel`, skips identical-filter commits, lists a decade filter's year in the mobile dropdown. |
| `frontend/src/i18n/en.json`, `de.json` | Five new keys: `ui.timeline_decade_label`, `ui.timeline_granularity`, `ui.timeline_level_decade`, `ui.timeline_level_year`, `ui.timeline_level_month`. |
| `tests/timeline-layout.test.js` | Unit tests for the bands and both framings (runs via `npm run test:unit`). |
| `tests/timeline-model.test.js` | Unit test for the decade label. |
| `tests/timeline-route.test.js` | Unit tests for the decade-is-a-period rule. |
| `tests/i18n-integrity.test.js` | The map-key scan is extended to `TimelineSelector.svelte`, so the `LEVELS` keys are verified. |
| `tests/e2e/specs/timeline.e2e.spec.js` | Updated decade expectations + the new acceptance tests. |
| `AGENTS.md` | Learnings entry folded in (Task 8). |

---

### Task 1: The unit band — the scale range that renders a granularity level

**Files:**
- Modify: `frontend/src/lib/timelineLayout.js` (after `chooseUnit`, before `formatColumnLabel`)
- Test: `tests/timeline-layout.test.js`

**Interfaces:**
- Consumes: `MIN_COLUMN_PX`, `MONTHS_PER_YEAR`, `MONTHS_PER_DECADE`, `chooseUnit` (all already in `timelineLayout.js`).
- Produces:
  - `finerUnit(unit: number): number | null` — `120 → 12 → 1 → null`.
  - `unitScaleMin(unit: number): number` — narrowest scale that certainly renders `unit`.
  - `unitScaleMax(unit: number): number` — widest scale that certainly renders `unit` (`Infinity` for months).
  - `unitWindow(unit: number, width: number): number` — widest span in months the lane shows at `unit`.
  - `unitMinSpan(unit: number, width: number): number` — narrowest span that still renders `unit` (`0` for months).
  - `canRenderUnit(unit: number, width: number, model: { length: number }): boolean`.
  - These names/types are used verbatim by Tasks 2, 5 and 6.

- [ ] **Step 1: Write the failing tests**

Append to `tests/timeline-layout.test.js` (and extend the import list at the top of the file with `canRenderUnit`, `finerUnit`, `unitMinSpan`, `unitScaleMax`, `unitScaleMin`, `unitWindow`):

```js
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `node --test tests/timeline-layout.test.js`
Expected: FAIL — `canRenderUnit is not a function` (the imports resolve to `undefined`).

- [ ] **Step 3: Implement the band**

Insert into `frontend/src/lib/timelineLayout.js` directly after `chooseUnit`:

```js
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `node --test tests/timeline-layout.test.js`
Expected: PASS — all tests, including the pre-existing ones (`chooseUnit` itself is untouched).

- [ ] **Step 5: Run the full unit suite, lint and format**

```bash
npm run test:unit
npm run lint
npm run format
```

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/timelineLayout.js tests/timeline-layout.test.js
git commit -m "feat(timeline): add the granularity band arithmetic"
```

---

### Task 2: The two framings — activation drill and level control

**Files:**
- Modify: `frontend/src/lib/timelineLayout.js` (after `ensureSelectionVisible`)
- Test: `tests/timeline-layout.test.js`

**Interfaces:**
- Consumes: Task 1's `canRenderUnit`, `unitWindow`, `unitMinSpan`; existing `clampScale`, `clampOrigin`.
- Produces:
  - `zoomToUnitRange(range: {startIndex,endIndex}, unit: number, width: number, model: any): {scale, origin}` — used by Task 5.
  - `frameUnit(unit: number, { selection, view, width, model }): {scale, origin}` — used by Task 6; returns `view` unchanged when the level cannot be rendered.

- [ ] **Step 1: Write the failing tests**

Append to `tests/timeline-layout.test.js` (add `frameUnit`, `zoomToUnitRange` to the imports):

```js
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
  assert.equal(Math.round(xFromIndex(legacyModel.minIndex, drilled)), 0, 'the data start pins the lane edge');
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
  assert.equal(zoomToUnitRange(march, 1, width, legacyModel).scale, width, 'one month fills the lane');

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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `node --test tests/timeline-layout.test.js`
Expected: FAIL — `zoomToUnitRange is not a function`.

- [ ] **Step 3: Implement the framings**

Append to `frontend/src/lib/timelineLayout.js`:

```js
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `node --test tests/timeline-layout.test.js`
Expected: PASS.

- [ ] **Step 5: Run the full unit suite, lint and format**

```bash
npm run test:unit
npm run lint
npm run format
```

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/timelineLayout.js tests/timeline-layout.test.js
git commit -m "feat(timeline): frame a period or a filter at a granularity level"
```

---

### Task 3: The decade reads as a decade, in both languages

**Files:**
- Modify: `frontend/src/lib/timeline.js` (`formatSelectionLabel`), `frontend/src/lib/timelineLayout.js` (`formatColumnLabel`), `frontend/src/components/TimelineSelector.svelte` (the `format` derived), `frontend/src/components/TimelineSlider.svelte` (`labelText`'s format)
- Modify: `frontend/src/i18n/en.json`, `frontend/src/i18n/de.json`
- Test: `tests/timeline-model.test.js`, `tests/timeline-layout.test.js`

**Interfaces:**
- Consumes: nothing new.
- Produces: the `format` object passed into `formatSelectionLabel`/`buildColumns` gains a required member `decadeLabel(year: number): string`; `formatColumnLabel` (Task 5 relies on the column labels staying "1960s") calls it.
- Why this is one task: `formatSelectionLabel` calling `format.decadeLabel` without both components providing it crashes at runtime, and the key without the dictionaries fails `npm run test:i18n` — the label, the dictionaries and both call sites must land together.

- [ ] **Step 1: Write the failing tests**

In `tests/timeline-model.test.js`, add `decadeLabel: (year) => \`${year}s\`` to the `format` constant and append:

```js
test('a grid-aligned decade is labelled as the decade', () => {
  const sixties = { startIndex: toMonthIndex(1960, 1), endIndex: toMonthIndex(1969, 12) };
  assert.equal(formatSelectionLabel(sixties, format), '1960s');

  // Clipped at the library's first bucket (March 1962) it is no longer a
  // decade-shaped selection and keeps the explicit range wording.
  const clipped = { startIndex: toMonthIndex(1962, 3), endIndex: toMonthIndex(1969, 12) };
  assert.equal(formatSelectionLabel(clipped, format), 'March 1962 – December 1969');

  // A ten-year span that is not grid-aligned is an ordinary range.
  const shifted = { startIndex: toMonthIndex(1963, 1), endIndex: toMonthIndex(1972, 12) };
  assert.equal(formatSelectionLabel(shifted, format), '1963 – 1972');

  // Single periods and whole years are unaffected.
  assert.equal(
    formatSelectionLabel(
      { startIndex: toMonthIndex(2012, 1), endIndex: toMonthIndex(2012, 12) },
      format
    ),
    '2012'
  );
  assert.equal(
    formatSelectionLabel(
      { startIndex: toMonthIndex(2012, 3), endIndex: toMonthIndex(2012, 3) },
      format
    ),
    'March 2012'
  );
});

```

Also add `decadeLabel: (year) => \`${year}s\`` to the `format` constant in `tests/timeline-layout.test.js` (the existing assertion `assert.equal(columns[0].label, '0s')` must keep passing — `formatColumnLabel` now asks the format for the label) and append this test there, where `denseModel`/`buildColumns`/`countInRange` already exist:

```js
test('a decade column label comes from the injected decade template', () => {
  // The ruler's decade labels must localise ("1960s" / "1960er"), so the
  // hardcoded `s` suffix is gone: `formatColumnLabel` asks the format object.
  const german = { ...format, decadeLabel: (year) => `${year}er` };
  const view = createView(1200, denseModel);
  const columns = buildColumns({
    unit: 120,
    view,
    width: 1200,
    model: denseModel,
    format: german,
    countInRange,
  });
  assert.equal(columns[0].label, '0er');
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `node --test tests/timeline-model.test.js tests/timeline-layout.test.js`
Expected: FAIL — `formatSelectionLabel` returns `1960 – 1969` for the decade, and `formatColumnLabel` ignores `decadeLabel`.

- [ ] **Step 3: Implement the label**

In `frontend/src/lib/timeline.js`, inside `formatSelectionLabel`, after `const wholeYears = start.month === 1 && end.month === 12;`:

```js
  // A grid-aligned ten-year span *is* the decade the ruler labels "1960s", and
  // the route writes it back unchanged: read it as the period it represents,
  // never as its clipped bounds.
  if (
    wholeYears &&
    selection.startIndex % MONTHS_PER_DECADE === 0 &&
    selection.endIndex - selection.startIndex === MONTHS_PER_DECADE - 1
  ) {
    return format.decadeLabel(start.year);
  }
```

Update the doc comment above the function to list `1960s` among the shapes.

In `frontend/src/lib/timelineLayout.js`, in `formatColumnLabel`:

```js
  if (unit === MONTHS_PER_DECADE) return format.decadeLabel(year - (year % 10));
```

In `frontend/src/components/TimelineSelector.svelte`, add to the `format` derived:

```js
    decadeLabel: (year) =>
      $t('ui.timeline_decade_label', { values: { start: String(year) }, default: '{start}s' }),
```

In `frontend/src/components/TimelineSlider.svelte`, add the same member to the object passed to `formatSelectionLabel` inside `labelText`.

- [ ] **Step 4: Add the dictionary entries**

`frontend/src/i18n/en.json`, directly after `"timeline_fit_all": "Fit all",`:

```json
    "timeline_decade_label": "{start}s",
```

`frontend/src/i18n/de.json`, directly after `"timeline_fit_all": "Alle anzeigen",`:

```json
    "timeline_decade_label": "{start}er",
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
node --test tests/timeline-model.test.js tests/timeline-layout.test.js
npm run test:i18n
npm run test:unit
```

Expected: PASS, `i18n integrity: N keys checked, 0 unresolved, en/de parity OK`.

- [ ] **Step 6: Lint, format and build the frontend**

```bash
npm run lint
npm run format
npm run build
```

- [ ] **Step 7: Commit**

```bash
git add frontend/src/lib/timeline.js frontend/src/lib/timelineLayout.js \
  frontend/src/components/TimelineSelector.svelte frontend/src/components/TimelineSlider.svelte \
  frontend/src/i18n/en.json frontend/src/i18n/de.json \
  tests/timeline-model.test.js tests/timeline-layout.test.js
git commit -m "feat(timeline): label a grid-aligned decade as the decade"
```

---
### Task 4: A grid-aligned decade is a period in the route

**Files:**
- Modify: `frontend/src/lib/timelineRoute.js`
- Test: `tests/timeline-route.test.js`

**Interfaces:**
- Consumes: `toMonthIndex`, `clampSelectionToModel` (already imported).
- Produces: `isDecadeFilter(filter): boolean` — true for the canonical shape `{ year: y*10, month: null, to_year: y*10+9, to_month: null }`; `selectionFromFilter` keeps a decade's own bounds instead of clamping.

- [ ] **Step 1: Write the failing test**

Append to `tests/timeline-route.test.js` (add `isDecadeFilter` to the import list):

```js
test('a grid-aligned decade is a period and keeps its own bounds', () => {
  // March 1962 … September 1974: the 1960s reach left of the library.
  const legacyModel = buildTimelineModel([
    { year: 1962, month: 3, count: 1 },
    { year: 1969, month: 12, count: 1 },
    { year: 1974, month: 9, count: 1 },
  ]);

  const decade = normalizeDateFilter({ year: 1960, month: null, to_year: 1969, to_month: null });
  assert.ok(isDecadeFilter(decade));
  assert.deepEqual(selectionFromFilter(decade, legacyModel), {
    startIndex: toMonthIndex(1960, 1),
    endIndex: toMonthIndex(1969, 12),
  });
  assert.deepEqual(
    filterFromSelection(selectionFromFilter(decade, legacyModel)),
    decade,
    'the round trip is the identity, so the clamp effect leaves the URL alone'
  );

  // Only a *grid-aligned* ten-year range is a period; a shifted one keeps
  // clamping to the library.
  const shifted = normalizeDateFilter({ year: 1963, month: null, to_year: 1972, to_month: null });
  assert.ok(!isDecadeFilter(shifted));
  assert.deepEqual(selectionFromFilter(shifted, legacyModel), {
    startIndex: toMonthIndex(1963, 1),
    endIndex: toMonthIndex(1972, 12),
  });

  // …and a range reaching left of the library still clamps to its oldest bucket.
  const early = normalizeDateFilter({ year: 1955, month: 3, to_year: 1972, to_month: 12 });
  assert.ok(!isDecadeFilter(early));
  assert.deepEqual(selectionFromFilter(early, legacyModel), {
    startIndex: legacyModel.minIndex,
    endIndex: toMonthIndex(1972, 12),
  });

  // A decade the library has nothing in still clears the filter.
  const nineties = normalizeDateFilter({ year: 1990, month: null, to_year: 1999, to_month: null });
  assert.ok(isDecadeFilter(nineties));
  assert.equal(selectionFromFilter(nineties, legacyModel), null);

  // A decade-shaped *non* range (an explicit end month) is not a decade period.
  assert.ok(
    !isDecadeFilter(normalizeDateFilter({ year: 1960, month: 3, to_year: 1969, to_month: 12 }))
  );
  assert.ok(!isDecadeFilter(EMPTY_DATE_FILTER));
  assert.ok(!isDecadeFilter({ year: 1961, month: null, to_year: 1970, to_month: null }));
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `node --test tests/timeline-route.test.js`
Expected: FAIL — `isDecadeFilter is not a function`, and the decade selection comes back clamped to `{ minIndex, endIndex: 1969-12 }`.

- [ ] **Step 3: Implement the decade period**

In `frontend/src/lib/timelineRoute.js`, after `EMPTY_DATE_FILTER`:

```js
/**
 * A filter naming a whole grid-aligned decade (`1960-01 … 1969-12`), the shape
 * an activation of a decade column writes. Normalised filters use `null` for
 * the January start and the December end, so the check is on that canonical
 * form only.
 */
export const isDecadeFilter = (filter) =>
  Boolean(filter) &&
  filter.year !== null &&
  filter.month === null &&
  filter.year % 10 === 0 &&
  filter.to_year === filter.year + 9 &&
  filter.to_month === null;
```

In `selectionFromFilter`, replace the range branch:

```js
  if (toYear !== null) {
    // A decade is a *period*, not a range: keep its own boundaries so the label
    // names the decade and the round trip through `filterFromSelection` is
    // stable, exactly as the bare-period branch below does. Only a period the
    // library has nothing in at all clears the filter.
    if (isDecadeFilter(filter)) {
      return endIndex < model.minIndex || startIndex > model.maxIndex
        ? null
        : { startIndex, endIndex };
    }
    return clampSelectionToModel({ startIndex, endIndex }, model);
  }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `node --test tests/timeline-route.test.js`
Expected: PASS (the pre-existing clamp tests still hold).

- [ ] **Step 5: Full unit suite, lint, format, commit**

```bash
npm run test:unit
npm run lint
npm run format
git add frontend/src/lib/timelineRoute.js tests/timeline-route.test.js
git commit -m "feat(timeline): treat a grid-aligned decade as a period"
```

---

### Task 5: One activation selects the period and drills one level in

**Files:**
- Modify: `frontend/src/components/TimelineSelector.svelte` (`activateColumn`, imports, and its comment block)
- Test: `tests/e2e/specs/timeline.e2e.spec.js`

**Interfaces:**
- Consumes: `finerUnit`, `zoomToUnitRange` (Tasks 1–2), `isDecadeFilter`/decade periods (Task 4), `format.decadeLabel` (Task 3).
- Produces: every period activation writes exactly one filter (decade → `?year=Y&to_year=Y+9`, year → `?year=Y`, month → `?year=Y&month=M`) and then sets `view = zoomToUnitRange(period, finerUnit(unit) ?? unit, width, model)` with `reframeSuppressed = true`.

- [ ] **Step 1: Update the E2E expectations and add the new tests**

In `tests/e2e/specs/timeline.e2e.spec.js`, replace the decade step of `'should select any month in three interactions from the full span'`:

```js
    // WHEN: drilling into the 1960s by activating the decade column
    await page.locator('.timeline-column[data-period-start="23520"]').click();

    // THEN: years appear, and the decade is the filter now — activating a period
    // selects that period (FR-001), and a grid-aligned decade commits the whole
    // ten years even though the library starts in March 1962
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '12');
    await TestHelpers.waitForUrlParam(page, 'to_year', '1969');
    expect(TestHelpers.getUrlState(page)).toMatchObject({
      year: 1960,
      month: null,
      toYear: 1969,
      toMonth: null,
    });
```

Append these tests inside the existing `test.describe('Timeline', …)` block:

```js
  test('should select a grid-aligned decade, drill into its years and keep the drill', async ({
    page,
  }) => {
    // GIVEN: the cleared full span, where the columns are decades
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '120');

    // WHEN: the 1960s column is activated (23520 === 1960 * 12). The library's
    // oldest bucket is March 1962, so the decade reaches left of the data.
    await page.locator('.timeline-column[data-period-start="23520"]').click();
    await TestHelpers.waitForUrlParam(page, 'to_year', '1969');

    // THEN: exactly that decade is the filter, in the canonical range form
    expect(TestHelpers.getUrlState(page)).toMatchObject({
      year: 1960,
      month: null,
      toYear: 1969,
      toMonth: null,
    });

    // AND: the label names the decade, not its clipped March 1962 bounds
    await expect(page.locator('.timeline-header .timeline-label')).toHaveText('1960s');

    // AND: the view drilled one level in — the lane renders the decade's year
    // columns, starting at the library's first year and reaching to 1972 for a
    // 120-month span whose origin is pinned to the data start
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '12');
    await expect(page.locator('.timeline-column').first()).toHaveAttribute(
      'data-period-start',
      '23544'
    );
    await expect(page.locator('.timeline-column').last()).toHaveAttribute(
      'data-period-start',
      '23664'
    );

    // AND: FR-013 — nothing re-frames the drill away. Re-assert after a full
    // animation frame budget: a revert would land on fit-all's decades.
    await page.waitForTimeout(250);
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '12');

    // AND: the grid holds the decade's photos only — legacy_01, March 1962
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.photo-card')).toHaveCount(1);
  });

  test('should refuse an empty decade and announce it', async ({ page }) => {
    // GIVEN: the decade view, whose 1990s column is empty (the fixture skips the
    // decade: legacy_03 is 1985, legacy_04 is 2004)
    const nineties = page.locator('.timeline-column[data-period-start="23880"]');
    await expect(nineties).toHaveAttribute('aria-disabled', 'true');
    await expect(nineties).toHaveAttribute('aria-label', /^1990s, No photos$/);

    // AND: hovering announces the period and its emptiness before activation
    await nineties.hover();
    await expect(page.locator('.timeline-status')).toHaveText('1990s, No photos');

    // WHEN: the period is activated — a raw input click, because `aria-disabled`
    // columns are intentionally not DOM-disabled and Playwright's actionability
    // gate would refuse `locator.click()`
    const box = await nineties.boundingBox();
    const before = await page.locator('.timeline-column').first().getAttribute('data-period-start');
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);

    // THEN: neither the filter nor the view moved
    await expect(page).not.toHaveURL(/year=/);
    await expect(page.locator('.timeline-column').first()).toHaveAttribute(
      'data-period-start',
      before
    );
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '120');
  });

  test('should keep an activated month filling the lane and stop at the deepest zoom', async ({
    page,
  }) => {
    // GIVEN: the 2012 year view, where the columns are months
    await page.goto('/?year=2012');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '1');

    // WHEN: March 2012 (24146) is activated
    await page.locator('.timeline-column[data-period-start="24146"]').click();
    await TestHelpers.waitForUrlParam(page, 'month', '3');

    // THEN: one month fills the lane — the finest level (FR-002/FR-009)
    const month = await page.locator('.timeline-column[data-period-start="24146"]').boundingBox();
    const lane = await page.locator('.timeline-lane').boundingBox();
    expect(month.width).toBeGreaterThanOrEqual(lane.width - 1);

    // AND: further zoom input has no effect on the view
    await page.click('.timeline-zoom-in');
    const after = await page.locator('.timeline-column[data-period-start="24146"]').boundingBox();
    expect(Math.round(after.width)).toBe(Math.round(month.width));
    expect(Math.round(after.x)).toBe(Math.round(month.x));

    // AND: the filter is still that single month, not an end-bounded range
    expect(TestHelpers.getUrlState(page)).toMatchObject({
      year: 2012,
      month: 3,
      toYear: null,
      toMonth: null,
    });
  });

  test('should not activate the column beneath a drag that starts on the selected decade', async ({
    page,
  }) => {
    // GIVEN: the 1960s selected and drilled, so its selection covers the lane
    await page.locator('.timeline-column[data-period-start="23520"]').click();
    await TestHelpers.waitForUrlParam(page, 'to_year', '1969');
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '12');

    // WHEN: a drag starts inside the selection (the 1964 column) and ends three
    // whole year columns later (the 1967 column) — a range translation, not a
    // press on a period
    const lane = await page.locator('.timeline-lane').boundingBox();
    const from = await page.locator('.timeline-column[data-period-start="23568"]').boundingBox();
    const to = await page.locator('.timeline-column[data-period-start="23604"]').boundingBox();
    await page.mouse.move(from.x + from.width / 2, lane.y + lane.height / 2);
    await page.mouse.down();
    await page.mouse.move(to.x + to.width / 2, lane.y + lane.height / 2, { steps: 8 });
    await page.mouse.up();

    // THEN: only the drag's own result stands — the decade translated by three
    // years — and 1964 was never activated (that would have written ?year=1964)
    const state = TestHelpers.getUrlState(page);
    expect(state.year).toBe(1963);
    expect(state.month).toBeNull();
    expect(state.toYear).toBe(1972);
    expect(state.toMonth).toBeNull();
  });

  test('should restore a decade filter and its year view after a reload and history navigation', async ({
    page,
  }) => {
    // GIVEN: a decade deep link
    await page.goto('/?year=1960&to_year=1969');
    await TestHelpers.waitForPhotosToLoad(page);

    // THEN: the filter survives untouched (no canonicalisation rewrite) and the
    // view frames the decade at year granularity
    await expect(page.locator('.timeline-header .timeline-label')).toHaveText('1960s');
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '12');
    await expect(page).toHaveURL(/[?&]year=1960&to_year=1969(&|$)/);

    // WHEN: the user drills into 1962 and then goes back
    await page.locator('.timeline-column[data-period-start="23544"]').click();
    await TestHelpers.waitForUrlParam(page, 'year', '1962');
    await expect(page.locator('.timeline-header .timeline-label')).toHaveText('1962');
    await page.goBack();

    // THEN: the decade filter and its year view are back
    await expect(page).toHaveURL(/[?&]year=1960&to_year=1969(&|$)/);
    await expect(page.locator('.timeline-header .timeline-label')).toHaveText('1960s');
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '12');

    // AND: forward restores the year
    await page.goForward();
    await TestHelpers.waitForUrlParam(page, 'year', '1962');
    await expect(page.locator('.timeline-header .timeline-label')).toHaveText('1962');
  });
```

- [ ] **Step 2: Run the new tests to verify they fail**

Run: `npx playwright test tests/e2e/specs/timeline.e2e.spec.js -g "decade"`
Expected: FAIL — the decade click leaves `?year=` unset and the header label reading "All Dates".

Note: if the first run fails with a server/build error rather than an assertion, re-run once (worktree port races; see AGENTS.md learnings #10).

- [ ] **Step 3: Implement the activation**

In `frontend/src/components/TimelineSelector.svelte`, extend the `timelineLayout.js` import with `finerUnit` and `zoomToUnitRange`, then replace `activateColumn` entirely:

```js
  const activateColumn = (column) => {
    if (suppressClick) {
      suppressClick = false;
      return;
    }
    if (column.count === 0 || view === null || model.length === 0) return;

    // FR-001: every activation applies exactly the period the column shows, as
    // one filter write. A *decade* commits its grid-aligned ten years and a
    // *year* its grid-aligned calendar year — never the column's clipped
    // bounds, which are not a single period at the library's first and last
    // year (the 1962 column is March–December) and would disagree with what the
    // mobile dropdowns write for the same choice. A *month* column (`unit === 1`,
    // what a year drill-in or any zoom past the one-month-per-column floor
    // produces) commits its own single month.
    const period =
      unit === MONTHS_PER_DECADE
        ? { startIndex: column.gridStart, endIndex: column.gridStart + MONTHS_PER_DECADE - 1 }
        : unit === MONTHS_PER_YEAR
          ? { startIndex: column.gridStart, endIndex: column.gridStart + MONTHS_PER_YEAR - 1 }
          : { startIndex: column.startIndex, endIndex: column.endIndex };
    onchange(period, { commit: true });

    // FR-002/FR-003: the same activation zooms one level in — a decade shows its
    // year columns, a year its month columns, and a month the month itself at
    // one month per lane width — so the view is never left coarser than the
    // activated period. The drill is this gesture's own view change: the
    // selection-following effect must not re-frame it away.
    reframeSuppressed = true;
    view = zoomToUnitRange(period, finerUnit(unit) ?? unit, width, model);
  };
```

- [ ] **Step 4: Run the timeline specs to verify they pass**

```bash
npm run build
cargo build --bin turbo-pix
npx playwright test tests/e2e/specs/timeline.e2e.spec.js tests/e2e/specs/timeline-a11y.e2e.spec.js tests/e2e/specs/url-routing.e2e.spec.js
```

Expected: PASS. If a pre-existing timeline test now fails, fix the *test's* expectation only where the spec changes the behavior (a decade now filters, a month now fills the lane); do not weaken an assertion the spec keeps.

- [ ] **Step 5: Lint, format, commit**

```bash
npm run lint
npm run format
git add frontend/src/components/TimelineSelector.svelte tests/e2e/specs/timeline.e2e.spec.js
git commit -m "feat(timeline): select the activated period and drill one level in"
```

---

### Task 6: Decade / Year / Month granularity controls

**Files:**
- Modify: `frontend/src/components/TimelineSelector.svelte` (script + footer markup + scoped styles)
- Modify: `frontend/src/i18n/en.json`, `frontend/src/i18n/de.json`
- Modify: `tests/i18n-integrity.test.js` (scan the `LEVELS` keys)
- Test: `tests/e2e/specs/timeline.e2e.spec.js`

**Interfaces:**
- Consumes: `frameUnit` (Task 2), `canRenderUnit` (Task 1), the pressed-state source `unit = $derived(chooseUnit(view?.scale ?? 1))`.
- Produces: buttons `.timeline-level[data-level="120|12|1"]` inside `.timeline-levels` (a `role="group"` named by `ui.timeline_granularity`), each `aria-pressed={unit === level.unit}`; `activateLevel(levelUnit)` writes no filter and is a no-op when `unit === levelUnit`.

- [ ] **Step 1: Write the failing E2E tests**

Append to `tests/e2e/specs/timeline.e2e.spec.js`:

```js
  test('should switch granularity with the level controls without touching the filter', async ({
    page,
  }) => {
    // GIVEN: an active year filter, which the deep link frames at months
    await page.goto('/?year=2012');
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '1');
    const filteredState = TestHelpers.getUrlState(page);

    // WHEN: the Year control is activated
    await page.locator('.timeline-level[data-level="12"]').click();

    // THEN: the lane renders year columns, the pressed control is the rendered
    // level, and the filter — URL included — is untouched
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '12');
    await expect(page.locator('.timeline-level[data-level="12"]')).toHaveAttribute(
      'aria-pressed',
      'true'
    );
    await expect(page.locator('.timeline-level[data-level="1"]')).toHaveAttribute(
      'aria-pressed',
      'false'
    );
    expect(TestHelpers.getUrlState(page)).toEqual(filteredState);
    // AND: the filter's own year column is inside the window the control framed
    await expect(page.locator('.timeline-column[data-period-start="24144"]')).toBeVisible();

    // WHEN: the Month control is activated
    await page.locator('.timeline-level[data-level="1"]').click();

    // THEN: that filter's months fill the lane in one action (SC-003) and the
    // filter is byte-identical
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '1');
    await expect(page.locator('.timeline-column').first()).toHaveAttribute(
      'data-period-start',
      '24144'
    );
    expect(TestHelpers.getUrlState(page)).toEqual(filteredState);
    await expect(page.locator('.photo-card')).toHaveCount(1);
  });

  test('should frame a level window when no filter fits it', async ({ page }) => {
    // GIVEN: no filter at all, and the decade view at fit-all
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '120');

    // WHEN: the Month control is activated
    await page.locator('.timeline-level[data-level="1"]').click();

    // THEN: a month-column window appears, no filter is written and the control
    // is pressed
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '1');
    expect(await page.locator('.timeline-column').count()).toBeGreaterThan(30);
    await expect(page).not.toHaveURL(/year=/);
    await expect(page.locator('.timeline-level[data-level="1"]')).toHaveAttribute(
      'aria-pressed',
      'true'
    );

    // WHEN: a filter wider than the month window is active and the Month control
    // is activated again, from a state whose level is a decade
    await page.goto('/?year=1960&to_year=1969');
    await TestHelpers.waitForPhotosToLoad(page);
    await page.locator('.timeline-level[data-level="120"]').click();
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '120');
    await page.locator('.timeline-level[data-level="1"]').click();

    // THEN: the window is narrower than the filter, the columns stay months and
    // the filter is unchanged
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '1');
    expect(await page.locator('.timeline-column').count()).toBeGreaterThan(30);
    expect(TestHelpers.getUrlState(page)).toMatchObject({ year: 1960, toYear: 1969 });

    // AND: activating the level already in effect is a no-op — the view does not
    // move and the filter does not change (Scenario 2 acceptance 3)
    const before = await page.evaluate(() => ({
      start: document.querySelector('.timeline-column').dataset.periodStart,
      left: Math.round(document.querySelector('.timeline-column').getBoundingClientRect().left),
      search: location.search,
    }));
    await page.locator('.timeline-level[data-level="1"]').click();
    const after = await page.evaluate(() => ({
      start: document.querySelector('.timeline-column').dataset.periodStart,
      left: Math.round(document.querySelector('.timeline-column').getBoundingClientRect().left),
      search: location.search,
    }));
    expect(after).toEqual(before);
  });

  test('should activate a level control by keyboard with a visible focus ring', async ({ page }) => {
    const yearControl = page.locator('.timeline-level[data-level="12"]');
    await yearControl.focus();
    const shadow = await yearControl.evaluate((el) => getComputedStyle(el).boxShadow);
    expect(shadow).not.toBe('none');

    // WHEN: the control is activated by keyboard
    await page.keyboard.press('Enter');

    // THEN: it behaves exactly like the pointer path and announces its state
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '12');
    await expect(yearControl).toHaveAttribute('aria-pressed', 'true');
    // AND: the group names the level family for a screen reader
    await expect(page.locator('.timeline-levels')).toHaveAttribute('role', 'group');
    await expect(page.locator('.timeline-levels')).toHaveAttribute('aria-label', /.+/);
  });
```

Add the level controls to the existing `'should announce zoom, fit-all and clear with a visible focus ring'` loop — as their own assertion, because a text button carries its name in its content, not in `aria-label`:

```js
    // The level pills are text buttons: their accessible name is their content
    // (an aria-label would shadow it), and the ring comes from the same rule.
    for (const level of ['120', '12', '1']) {
      const control = page.locator(`.timeline-level[data-level="${level}"]`);
      await expect(control).toHaveText(/.+/);
      await control.focus();
      expect(await control.evaluate((el) => getComputedStyle(el).boxShadow)).not.toBe('none');
    }
```

- [ ] **Step 2: Run the new tests to verify they fail**

Run: `npx playwright test tests/e2e/specs/timeline.e2e.spec.js -g "level control|level window"`
Expected: FAIL — `.timeline-level[data-level="12"]` matches nothing (timeout).

- [ ] **Step 3: Implement the controls**

In `frontend/src/components/TimelineSelector.svelte`, extend the `timelineLayout.js` import with `canRenderUnit` and `frameUnit`, then add below the `unit` derived:

```js
  // FR-006: the three granularity controls, coarsest first. Their `key` fields
  // are i18n dot-paths the integrity guard scans (tests/i18n-integrity.test.js).
  const LEVELS = [
    { unit: MONTHS_PER_DECADE, key: 'ui.timeline_level_decade', fallback: 'Decade' },
    { unit: MONTHS_PER_YEAR, key: 'ui.timeline_level_year', fallback: 'Year' },
    { unit: 1, key: 'ui.timeline_level_month', fallback: 'Month' },
  ];
```

Add the handler next to `zoomIn`/`zoomOut`/`fitAll`:

```js
  // FR-006/FR-007: a granularity control changes only the view. It frames the
  // active filter — or, when that filter is wider than the level's window, a
  // window around the current view centre — and never writes a filter. The
  // rendered unit IS the pressed control, so activating the level already in
  // effect (or one the lane cannot render at this width) is a no-op.
  const activateLevel = (levelUnit) => {
    if (unit === levelUnit || view === null || width <= 0 || model.length === 0) return;
    const next = frameUnit(levelUnit, { selection, view, width, model });
    if (next.scale === view.scale && next.origin === view.origin) return;
    reframeSuppressed = true;
    view = next;
  };
```

In the footer, wrap the three pills in their own group before the zoom buttons:

```svelte
    <div class="timeline-controls">
      <div
        class="timeline-levels"
        role="group"
        aria-label={$t('ui.timeline_granularity', { default: 'Granularity' })}
      >
        {#each LEVELS as level (level.unit)}
          <button
            type="button"
            class="timeline-level"
            data-level={level.unit}
            aria-pressed={unit === level.unit}
            onclick={() => activateLevel(level.unit)}
          >
            {$t(level.key, { default: level.fallback })}
          </button>
        {/each}
      </div>
      <button
        type="button"
        class="timeline-control timeline-zoom-out"
        aria-label={$t('ui.zoom_out', { default: 'Zoom Out' })}
        onclick={zoomOut}
      >
        <Icon name="minus" width={16} height={16} />
      </button>
```

(keep the existing `zoom-in` and `fit-all` buttons where they are; the `timeline-level` comments below the controls stay untouched).

Add the scoped styles to `.timeline-controls`:

```css
  .timeline-levels {
    display: flex;
    gap: var(--space-1);
  }

  /* Pills, matching the control row's 32px target: the level name is the
     accessible name, so the pressed state is the only extra signal. */
  .timeline-level {
    height: var(--space-8);
    min-width: var(--space-8);
    padding: 0 var(--space-3);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-full);
    background: transparent;
    color: var(--text-secondary);
    font-size: var(--font-xs);
    cursor: pointer;
    transition:
      border-color var(--transition-fast),
      color var(--transition-fast),
      background-color var(--transition-fast);
  }

  .timeline-level:hover {
    border-color: var(--primary-color);
    color: var(--primary-color);
  }

  .timeline-level[aria-pressed='true'] {
    border-color: var(--primary-color);
    background: color-mix(in oklch, var(--primary-color) 12%, transparent);
    color: var(--primary-dark);
  }

  .timeline-level:focus-visible {
    outline: none;
    box-shadow:
      0 0 0 2px var(--surface-color),
      0 0 0 4px var(--primary-color);
  }
```

and add `.timeline-level` to the `prefers-reduced-motion` block's `transition: none` list.

- [ ] **Step 4: Add the dictionary entries and extend the i18n guard**

`frontend/src/i18n/en.json`, after `"timeline_decade_label": "{start}s",`:

```json
    "timeline_granularity": "Granularity",
    "timeline_level_decade": "Decade",
    "timeline_level_year": "Year",
    "timeline_level_month": "Month",
```

`frontend/src/i18n/de.json`, after `"timeline_decade_label": "{start}er",`:

```json
    "timeline_granularity": "Granularität",
    "timeline_level_decade": "Jahrzehnt",
    "timeline_level_year": "Jahr",
    "timeline_level_month": "Monat",
```

In `tests/i18n-integrity.test.js`, extend the map-key scan (the comment above it too):

```js
    // 3) Map-defined keys: Sidebar.svelte + SortControls.svelte + TimelineSelector.svelte
    //    `key:` fields, and App.svelte titleKeys values …
    if (
      file.endsWith('Sidebar.svelte') ||
      file.endsWith('SortControls.svelte') ||
      file.endsWith('TimelineSelector.svelte')
    ) {
```

- [ ] **Step 5: Run the tests, i18n guard, lint, build**

```bash
npm run test:i18n
npm run lint
npm run format
npm run build
cargo build --bin turbo-pix
npx playwright test tests/e2e/specs/timeline.e2e.spec.js tests/e2e/specs/timeline-a11y.e2e.spec.js
```

Expected: PASS, including the axe `target-size` scan (the pills are 32px tall with 8px gaps).

- [ ] **Step 6: Commit**

```bash
git add frontend/src/components/TimelineSelector.svelte frontend/src/i18n/en.json \
  frontend/src/i18n/de.json tests/i18n-integrity.test.js tests/e2e/specs/timeline.e2e.spec.js
git commit -m "feat(timeline): add decade, year and month granularity controls"
```

---
### Task 7: The host stops rewriting state the user did not change

**Files:**
- Modify: `frontend/src/components/TimelineSlider.svelte` (`handleChange`, `dropdownYears`, the mobile year options)
- Test: `tests/e2e/specs/timeline.e2e.spec.js`

**Interfaces:**
- Consumes: `filterEquals`, `filterFromSelection` (already imported), `filter` and `model` (already derived).
- Produces: `handleChange(next, { commit: true })` skips `pushState` when `filterFromSelection(next)` equals the active route filter; `dropdownYears` is an array of numbers (descending) that includes `filter.year` even when the library has no bucket in it.

- [ ] **Step 1: Write the failing E2E tests**

Append to `tests/e2e/specs/timeline.e2e.spec.js`:

```js
  test('should re-activate the active decade without a history entry or a coarser view', async ({
    page,
  }) => {
    // GIVEN: the 1960s filter, framed at the decade level so its column exists
    await page.goto('/?year=1960&to_year=1969');
    await TestHelpers.waitForPhotosToLoad(page);
    await page.locator('.timeline-level[data-level="120"]').click();
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '120');
    const historyLength = await page.evaluate(() => history.length);

    // WHEN: the active decade column is activated again
    await page.locator('.timeline-column[data-period-start="23520"]').click();

    // THEN: the filter is unchanged, no history entry was added (otherwise Back
    // would appear to do nothing), and the view only ever gets finer
    await expect(page).toHaveURL(/[?&]year=1960&to_year=1969(&|$)/);
    expect(await page.evaluate(() => history.length)).toBe(historyLength);
    await expect(page.locator('.timeline-column').first()).toHaveAttribute('data-unit', '12');

    // AND: Back leaves the state, because the re-activation never entered history
    await page.goBack();
    await expect(page).not.toHaveURL(/year=/);
  });

  test('should show a decade filter in the mobile dropdowns', async ({ page }) => {
    // GIVEN: a decade filter, whose start year (1960) the library has no bucket
    // in — the oldest bucket is March 1962
    await page.goto('/?year=1960&to_year=1969');
    await TestHelpers.waitForPhotosToLoad(page);

    // WHEN: the viewport drops below the desktop breakpoint
    await TestHelpers.setMobileViewport(page);

    // THEN: exactly one experience is on screen and the dropdowns report the
    // active filter instead of falling back to "All Years"
    await expect(page.locator('#timeline-year-select')).toBeVisible();
    await expect(page.locator('.timeline-selector')).toBeHidden();
    await expect(page.locator('#timeline-year-select')).toHaveValue('1960');
    await expect(page.locator('#timeline-month-select')).toHaveValue('');

    // AND: the grid is still the decade's
    await expect(page.locator('.photo-card')).toHaveCount(1);
  });
```

- [ ] **Step 2: Run the new tests to verify they fail**

Run: `npx playwright test tests/e2e/specs/timeline.e2e.spec.js -g "re-activate|mobile dropdowns"`
Expected: the history test FAILS (`history.length` grew by one) and the mobile test FAILS (`toHaveValue('1960')` — the select reads `''`).

- [ ] **Step 3: Implement both guards**

In `frontend/src/components/TimelineSlider.svelte`, replace the commit branch of `handleChange`:

```js
  const handleChange = (next, { commit = true } = {}) => {
    if (commit) {
      if (liveTimer !== null) {
        clearTimeout(liveTimer);
        liveTimer = null;
        pendingLive = null;
      }
      const nextFilter = filterFromSelection(next);
      // FR-010: re-activating the period that is already the filter — or ending
      // a gesture exactly where it started — is not a state change, and a
      // duplicate history entry would make Back appear to do nothing. `filter`
      // is the canonical route filter, so a non-canonical restored URL is still
      // left to the canonicalising effect below instead of being rewritten here.
      if (filterEquals(nextFilter, filter)) return;
      pushState(nextFilter);
      return;
    }
```

and add above the component's `filter`/`model` usage (next to `filter`):

```js
  // A decade filter names a year the library has no bucket in (1960 while the
  // oldest bucket is March 1962); without it the select would fall back to
  // "All Years" while the grid stays filtered.
  const dropdownYears = $derived(
    filter.year === null || model.years.includes(filter.year)
      ? model.years
      : [filter.year, ...model.years].sort((a, b) => b - a)
  );
```

and change the year options to iterate it:

```svelte
          {#each dropdownYears as year (year)}
            <option value={String(year)}>{year}</option>
          {/each}
```

- [ ] **Step 4: Run the timeline specs to verify they pass**

```bash
npm run build
cargo build --bin turbo-pix
npx playwright test tests/e2e/specs/timeline.e2e.spec.js tests/e2e/specs/url-routing.e2e.spec.js tests/e2e/specs/saved-searches.e2e.spec.js
```

Expected: PASS. If `'should adjust a range bound by dragging its handle'` or the Escape test fails, the guard is comparing the wrong thing — the commit's filter must still be pushed when it differs from the *route* filter (the live scrub writes the route through `replaceState`).

- [ ] **Step 5: Lint, format, commit**

```bash
npm run lint
npm run format
git add frontend/src/components/TimelineSlider.svelte tests/e2e/specs/timeline.e2e.spec.js
git commit -m "fix(timeline): skip identical filter commits and list a decade's year"
```

---

### Task 8: Learnings, spec/plan committed, whole-suite verification

**Files:**
- Modify: `AGENTS.md` (Learnings entries 2 and 10 — fold in, never append a standalone entry)
- Add: `.spec/timeline-selector-year-month-drilldown.md` (currently untracked), `.spec/timeline-selector-year-month-drilldown-plan.md`
- Test: the whole gate (no new test files; this task proves the branch)

**Interfaces:**
- Consumes: everything above.
- Produces: a green branch (unit + i18n + lint + build + clippy + the full E2E suite) and the committed spec/plan.

- [ ] **Step 1: Fold the session's learnings into AGENTS.md entry 2**

Insert this sentence set at the end of entry 2, right before the sentence that begins "Node-testable modules MUST NOT import `utils.js`":

```markdown
   The selector's granularity IS the rendered column unit (`chooseUnit(view.scale)`,
   `frontend/src/lib/timelineLayout.js`): the Decade/Year/Month pills are view-only and press whatever the lane
   renders (`aria-pressed={unit === level.unit}`, an activation while already at that level is a no-op), a click on a
   decade/year/month column commits exactly that period and drills one level in by clamping the period's span into the
   next unit's band (`zoomToUnitRange`, band edges from `unitScaleMin`/`unitScaleMax` with a 1e-3 margin so a clamped
   scale can never land on `chooseUnit`'s `>=` threshold; below the band the unit is kept and the period shows
   partially — never a coarser fallback), and the level pills frame the active filter when it fits the level's window
   and otherwise a window around the current view centre (`frameUnit`) without ever writing a filter. Both framings set
   `reframeSuppressed` before writing `view`, or the FR-010 selection-following effect reverts them. A grid-aligned
   decade filter (`?year=1960&to_year=1969`) is a *period*: `selectionFromFilter` keeps its own January–December-ten-year
   bounds (`isDecadeFilter`), so the header reads `1960s` instead of the clamped `March 1962 – December 1969` and the
   round trip stays the identity; a non-decade or shifted ten-year range keeps clamping as before. A commit whose
   canonical filter already equals the route filter is skipped, so re-activation adds no history entry. Decade labels go
   through `ui.timeline_decade_label` (`{start}s` / `{start}er`) — never hardcode the `s` — and the year dropdown gains
   an option for a filtered year the library has no bucket in, or it silently reads "All Years" over a filtered grid.
```

- [ ] **Step 2: Fold the fixture facts into AGENTS.md entry 10**

Insert into the fixture sentence of entry 10 (after the sentence ending "`updateTestPhotoDates()`, so decade/year granularity and gaps are reachable and the source image's dates never matter."):

```markdown
   The timeline specs also lean on the fixture's calendar: decade starts are `1960*12 = 23520`, `1980*12 = 23760`,
   `1990*12 = 23880` (the only empty decade) and `2012*12 = 24144`, and a decade activation writes
   `?year=1960&to_year=1969`, so a drill into the 1960s renders eleven year columns (1962…1972: the origin pins to the
   model's March 1962 start inside a 120-month span) — assert the first/last `data-period-start`, never a hard-coded
   lane width.
```

- [ ] **Step 3: Run the whole gate**

```bash
npm run format:check
npm run lint
npm run test:unit
npm run test:i18n
npm run build
cargo build --bin turbo-pix
cargo clippy --all-targets -- -D warnings
cargo test
npx playwright test
```

Expected: all green. E2E notes: the harness is serialized (`workers: 1`); let the previous run's teardown settle before re-running, and treat a first-run failure with a build/port error as infra — re-run once before suspecting a regression (AGENTS.md #10). With an empty `./data/models` the health check times out: pre-seed with `./target/debug/turbo-pix --download-models`.

- [ ] **Step 4: Commit the learnings and the spec/plan**

```bash
git add AGENTS.md .spec/timeline-selector-year-month-drilldown.md .spec/timeline-selector-year-month-drilldown-plan.md
git commit -m "docs(timeline): record the drill-down learnings and the spec"
```

---

## Self-Review

**Spec coverage** (every FR/SC has an owning task):

| Requirement | Task |
| --- | --- |
| FR-001 (decade/year/month activation commits that period) | 5 (E2E: decade URL `year=1960&to_year=1969`, year and month already covered by the updated 3-activation test) |
| FR-002, FR-003 (drill one level in, never coarser, period fills the lane) | 2 (band arithmetic, narrow/ultra-wide clamps), 5 (unit + first/last column assertions) |
| FR-004, FR-005 (empty period inert, gestures never activate) | 5 (`should refuse an empty decade…`, `should not activate the column beneath a drag…`; month cases pre-exist) |
| FR-006, FR-007 (controls exist, one level in effect, view-only framing) | 6 (all three E2E tests + the pill markup) |
| FR-008, SC-001 (year ≤ 2 activations, month ≤ 3) | 5 (the updated 3-activation test) |
| FR-009 (one month per lane width, no further zoom) | 5 (`should keep an activated month filling the lane…`) |
| FR-010 (re-activation is inert) | 7 (history + view assertions) |
| FR-011 (clear resets filter and view) | pre-existing `should clear the filter and fit the view from the reset control` |
| FR-012 (keyboard + announcements for columns and controls) | 5/6 (`activate a level control by keyboard…`, the pre-existing keyboard test) |
| FR-013 (URL round-trip, no re-framing revert) | 4 (round-trip unit test), 5 (reload/Back/Forward test + the 250ms no-revert assertion) |
| FR-014 (EN/DE parity) | 3 and 6 (both dictionaries), guarded by `npm run test:i18n` |
| FR-015 (desktop-only, mobile unchanged) | 7 (`should show a decade filter in the mobile dropdowns`), the pre-existing resize test |
| FR-016 (ranges, handles, pan, pinch, Escape, fit-all unchanged) | 2/5 (drill does not consume gestures), Task 7's full-spec E2E run |
| FR-017 (empty library, load failure) | pre-existing `should keep the selector off the page when the timeline fails to load` |
| SC-002, SC-006 | 5 (unit before/after, decade refusal) |
| SC-003 | 6 (byte-identical URL check) |
| SC-004, SC-005 | 5/6/7 |
| SC-007 (100 ms budget) | pre-existing layout perf test (200 layouts of a 100-year model under 100 ms) still covers the framing path |

**Placeholder scan:** none — every step carries its code, its command and its expected result.

**Type consistency:** `finerUnit`/`unitScaleMin`/`unitScaleMax`/`unitWindow`/`unitMinSpan`/`canRenderUnit` (Task 1) are spelled identically in Tasks 2, 5 and 6; `zoomToUnitRange(range, unit, width, model)` and `frameUnit(unit, { selection, view, width, model })` (Task 2) match their call sites in Tasks 5/6; `isDecadeFilter` (Task 4) is used only in `timelineRoute.js` and its test; `format.decadeLabel` (Task 3) is supplied by exactly the two call sites that build a `format` object; the level buttons' `data-level` values are the same `1/12/120` as `data-unit`.

**Review Focus:** each of the five lines has its test in the owning task (narrow lanes → Tasks 1/2 widths + axe; clipped edges → Tasks 4/5; empty periods → Tasks 4/5/6; non-canonical URLs → Task 4; writes that must not happen → Tasks 6/7).

## Execution Handoff

Tasks are strictly ordered: each one consumes the interfaces of the previous ones (band helpers → framings → label → route → activation → controls → host → docs), and Tasks 5–7 touch the same two components plus one E2E spec, so a fresh subagent per task must not run in parallel with its neighbors unless the integration owner merges the diffs explicitly.
<!-- PLAN-END -->


