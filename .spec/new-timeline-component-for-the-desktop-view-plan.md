# Desktop Timeline Redesign (Year Rail + Month Strip) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the desktop timeline slider with a two-level year-rail plus month-strip navigator so any year+month is reachable in at most two coarse clicks with zero label overlap at 60+ sparse years.

**Architecture:** Keep `TimelineSlider.svelte` as the data/route container (fetch, error, empty, mobile dropdowns untouched); delete the desktop slider block and render a scrollable year rail plus a 12-month strip derived synchronously from the existing `/api/photos/timeline` density payload via a new pure helper in `lib/timeline.js`. Clicks push immediately to `route.year`/`route.month`; a route-restore effect reads route first for Back/Forward.

**Tech Stack:** Svelte 5 runes (`$state`, `$derived`, `$effect`), `svelte-i18n` `$t`, `route`/`pushState` from `lib/router.svelte.js`, Playwright E2E, `node --test` unit + i18n-integrity guard.

**Spec:** `.spec/new-timeline-component-for-the-desktop-view.md`

## Global Constraints

- Desktop scope only: the `768px` desktop-only versus mobile-only split stays exact; mobile year/month selects are preserved unchanged and exactly one experience is visible at any width.
- Timeline data shape unchanged: reuse `GET /api/photos/timeline` `{ density: [{ year, month, count }] }`; no backend change.
- Single source of truth is `route.year` plus optional `route.month` (`frontend/src/lib/router.svelte.js`); a month is never active without its year (`normalizeState` drops orphan months).
- `en.json` and `de.json` MUST stay structurally identical — every new key lands in BOTH; `$t` `values` go INSIDE the options object; template `` `${…}` `` placeholders must be one of the integrity-guard `enums` (so new code uses literal keys only).
- `$state` fields MUST use `let` (never `const`); arrow functions; `const` over `let` elsewhere; template literals over concatenation.
- Responsive overrides MUST live in the component's scoped `<style>`, never in `app.css`; keep `build.cssMinify: false` in `vite.config.js`.
- Route-restore `$effect` MUST read `route.year`/`route.month` BEFORE any early-return guard, or the effect unsubscribes permanently.
- Icons: feather only via registered `<Icon name>` (`x` already registered); size via `:global(svg)`, never `:global(.feather)`.
- Zero linting issues (`npm run lint`, `npm run format:check`), `npm run test:i18n` green, `npm run build` before `cargo build --bin turbo-pix`.
- Breaking changes allowed: delete the desktop slider code/CSS/tests outright, no shims or backwards-compat.

---

## File Structure

- Modify: `frontend/src/i18n/en.json` — add 3 literal keys under `ui` (`timeline_years_label`, `timeline_months_label`, `timeline_no_photos_month`).
- Modify: `frontend/src/i18n/de.json` — identical 3 keys with German text (parity gate).
- Create: `frontend/src/lib/timeline.js` — pure aggregation, no Svelte imports:
  - `buildYearAggregates(density) => [{ year, total, months: [{ month, count }] }]` newest-first, 12 month slots per year (zero-filled), year totals summed.
  - `getYearAggregate(aggregates, year) => aggregate | null`.
  - Responsibility: single place where density becomes rail/strip view-models; unit-tested with `node --test`.
- Create: `tests/timeline-aggregates.test.js` — `node:test` coverage for the two helpers (empty, single-year, sparse, zero-months, newest-first).
- Modify: `frontend/src/components/TimelineSlider.svelte` — delete desktop slider/ribbon/ticks/tooltip/debounce code; add year rail + month strip desktop block, immediate-push click handlers, route-restore effect, keep fetch/error/empty/loading skeleton and the entire mobile dropdown block untouched.
- Modify: `tests/e2e/specs/timeline.e2e.spec.js` — replace slider interaction tests with rail/strip tests (selectors, URL asserts, overlap check).
- Modify: `tests/e2e/specs/timeline-a11y.e2e.spec.js` — retarget axe include + selectors from `.timeline-input` to the rail container.
- Delete (within TimelineSlider edits): `.timeline-track*`, `.timeline-ribbon`, `.timeline-bar`, `.timeline-input` + thumbs, `.timeline-ticks`, `.timeline-year-tick`, `.timeline-tooltip*` CSS and `handleSliderInput`/`handleTrackHover`/`handleTrackLeave`/`yearTicks`/`maxSlider`/`sliderValue`/`selectedIndex`/`dragInProgress`/`debounceTimer` JS.

Year-rail non-overlap mechanism (locked): horizontal scroll container (`overflow-x: auto`, `flex-wrap: nowrap`, each year button `flex: 0 0 auto`) — buttons never overlay by construction at any count; every photo-year stays discoverable via scroll; numeric jumps (e.g. `1998 → 2005`) make gaps apparent without rendering fake selectable years. No grouping/abbreviation logic.

Filter semantics (locked):
- Click unselected year → `{ year, month: null }` + `pushState({ year, month: null })`.
- Click selected year again → `null` + `pushState({ year: null, month: null })`.
- Click unselected non-empty month in selected year → `{ year, month }` + push.
- Click selected month again → `{ year, month: null }` (keeps year) + push.
- Clear control → `null` + `pushState({ year: null, month: null })`.
- Empty (zero-count) months render `disabled` and are never pushed.
- Month strip renders only when a year is selected; clearing the year unmounts the strip (FR-008 by construction).

---

### Task 1: i18n keys for rail/strip labels

**Files:**
- Modify: `frontend/src/i18n/en.json`
- Modify: `frontend/src/i18n/de.json`
- Test: `tests/i18n-integrity.test.js` (existing guard, no new file)

**Interfaces:**
- Consumes: existing `ui.clear_timeline_filter`, `ui.photos_count`, `ui.months.*` (unchanged).
- Produces: `ui.timeline_years_label`, `ui.timeline_months_label`, `ui.timeline_no_photos_month` for Tasks 3–4.

- [ ] **Step 1: Write the failing check run**

Run: `npm run test:i18n`
Expected: PASS now (baseline, proves guard works before adding keys).

- [ ] **Step 2: Add the three keys to en.json**

In `frontend/src/i18n/en.json` inside `"ui"`, after `"clear_timeline_filter"` insert exactly:

```json
"timeline_years_label": "Years",
"timeline_months_label": "Months",
"timeline_no_photos_month": "No photos",
```

- [ ] **Step 3: Add the identical structure to de.json**

In `frontend/src/i18n/de.json` inside `"ui"`, after `"clear_timeline_filter"` insert exactly:

```json
"timeline_years_label": "Jahre",
"timeline_months_label": "Monate",
"timeline_no_photos_month": "Keine Fotos",
```

- [ ] **Step 4: Run the guard to verify it passes**

Run: `npm run test:i18n`
Expected: PASS with `i18n integrity: … keys checked, 0 unresolved, en/de parity OK` (count grows by 3).

- [ ] **Step 5: Commit**

```bash
git add frontend/src/i18n/en.json frontend/src/i18n/de.json
git commit -m "feat(timeline): add year-rail and month-strip i18n keys"
```

---

### Task 2: Pure density aggregation helper + unit tests

**Files:**
- Create: `frontend/src/lib/timeline.js`
- Test: `tests/timeline-aggregates.test.js`

**Interfaces:**
- Consumes: density rows `{ year: number, month: 1-12, count: number }` from `GET /api/photos/timeline`.
- Produces:
  - `buildYearAggregates(density) => Array<{ year: number, total: number, months: Array<{ month: number, count: number }> }>` — newest year first; each aggregate holds exactly 12 month slots (zero-filled); `total` is the sum of its 12 slots.
  - `getYearAggregate(aggregates, year) => { year, total, months } | null`.

- [ ] **Step 1: Write the failing test**

Create `tests/timeline-aggregates.test.js` with exactly:

```js
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildYearAggregates, getYearAggregate } from '../frontend/src/lib/timeline.js';

test('empty density builds no aggregates', () => {
  assert.deepEqual(buildYearAggregates([]), []);
});

test('aggregates one year with zero-filled months and total', () => {
  const aggregates = buildYearAggregates([
    { year: 1998, month: 3, count: 4 },
    { year: 1998, month: 7, count: 2 },
  ]);
  assert.equal(aggregates.length, 1);
  assert.equal(aggregates[0].year, 1998);
  assert.equal(aggregates[0].total, 6);
  assert.equal(aggregates[0].months.length, 12);
  assert.equal(aggregates[0].months[2].count, 4);
  assert.equal(aggregates[0].months[6].count, 2);
  assert.equal(aggregates[0].months[0].count, 0);
});

test('sparse years sort newest-first', () => {
  const aggregates = buildYearAggregates([
    { year: 1970, month: 1, count: 1 },
    { year: 2024, month: 12, count: 3 },
    { year: 1998, month: 3, count: 2 },
  ]);
  assert.deepEqual(
    aggregates.map((a) => a.year),
    [2024, 1998, 1970]
  );
});

test('getYearAggregate returns null for unknown year', () => {
  const aggregates = buildYearAggregates([{ year: 2005, month: 5, count: 1 }]);
  assert.equal(getYearAggregate(aggregates, 1999), null);
  assert.equal(getYearAggregate(aggregates, 2005).total, 1);
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test tests/timeline-aggregates.test.js`
Expected: FAIL with `Cannot find module '../frontend/src/lib/timeline.js'`.

- [ ] **Step 3: Write minimal implementation**

Create `frontend/src/lib/timeline.js` with exactly:

```js
// Pure view-model for the desktop year-rail + month-strip navigator.
// No Svelte imports: testable with node --test.

/**
 * @param {Array<{ year: number, month: number, count: number }>} density
 * @returns {Array<{ year: number, total: number, months: Array<{ month: number, count: number }> }>}
 */
export const buildYearAggregates = (density) => {
  const byYear = new Map();
  for (const { year, month, count } of density ?? []) {
    if (!byYear.has(year)) {
      byYear.set(
        year,
        Array.from({ length: 12 }, (_, i) => ({ month: i + 1, count: 0 }))
      );
    }
    const slot = byYear.get(year)[month - 1];
    if (slot) slot.count += count;
  }
  return [...byYear.entries()]
    .sort(([a], [b]) => b - a)
    .map(([year, months]) => ({
      year,
      total: months.reduce((sum, m) => sum + m.count, 0),
      months,
    }));
};

/**
 * @param {Array<{ year: number, total: number, months: Array<{ month: number, count: number }> }>} aggregates
 * @param {number | null} year
 */
export const getYearAggregate = (aggregates, year) =>
  aggregates.find((a) => a.year === year) ?? null;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test tests/timeline-aggregates.test.js`
Expected: PASS, 4 passing.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/timeline.js tests/timeline-aggregates.test.js
git commit -m "feat(timeline): add year-aggregate helper with unit tests"
```

---

### Task 3: Desktop year rail (select/clear, counts, route restore)

**Files:**
- Modify: `frontend/src/components/TimelineSlider.svelte:1-223` (script: imports, state, derived, handlers, restore effect)
- Modify: `frontend/src/components/TimelineSlider.svelte:225-350` (template: replace `.desktop-only` slider block; mobile block untouched)
- Test: manual via `npm run build` + E2E in Task 6; fast check `npm run test:i18n`

**Interfaces:**
- Consumes: `buildYearAggregates`/`getYearAggregate` from `../lib/timeline.js`; `route`/`pushState` from `../lib/router.svelte.js`; `$t` keys from Task 1 plus existing `ui.clear_timeline_filter`, `ui.photos_count`.
- Produces: `selectedYear`/`currentFilter` state and `selectYear`/`resetFilter` handlers consumed by Task 4's month strip.

- [ ] **Step 1: Replace the script state with rail state**

Delete `debounceTimer`, `selectedIndex`, `sliderValue`, `ribbonEl`, `hoveredIndex`, `tooltipX`, `tooltipY`, `dragInProgress`, `positions`, `years`, `maxSlider`, `maxCount`, `yearTicks`, `handleSliderInput`, `handleTrackHover`, `handleTrackLeave`. Replace with exactly:

```js
import { onMount } from 'svelte';
import { locale } from 'svelte-i18n';
import { t } from '../lib/i18n.js';
import { api } from '../lib/api.js';
import { addToast } from '../lib/state.svelte.js';
import { route, pushState } from '../lib/router.svelte.js';
import { APP_CONSTANTS } from '../lib/constants.js';
import { buildYearAggregates, getYearAggregate } from '../lib/timeline.js';
import Icon from './Icon.svelte';

const activeLocale = $derived($locale || 'en');

let data = $state(null);
let currentFilter = $state(null);
let selectedYear = $state(null);
let yearSelectEl = $state(null);
let monthSelectEl = $state(null);
let initError = $state(false);

const aggregates = $derived(buildYearAggregates(data?.density ?? []));

const selectedAggregate = $derived(
  selectedYear === null ? null : getYearAggregate(aggregates, selectedYear)
);

const labelText = $derived.by(() => {
  if (!currentFilter) {
    return $t('ui.all_dates', { locale: activeLocale, default: 'All Dates' });
  }
  if (!currentFilter.month) {
    return String(currentFilter.year);
  }
  return monthYearLabel(currentFilter.year, currentFilter.month);
});

const monthYearLabel = (year, month) => {
  const monthKey = APP_CONSTANTS.MONTH_KEYS[month - 1];
  const monthName = $t(`ui.months.${monthKey}`, {
    locale: activeLocale,
    default: `${monthKey.charAt(0).toUpperCase()}${monthKey.slice(1)}`,
  });
  return `${monthName} ${year}`;
};
```

Notes: `monthYearLabel` uses the pre-existing `` `ui.months.${monthKey}` `` template whose `monthKey` placeholder is in the integrity-guard `enums` map — no new placeholder. `const` arrow form per repo style; `$state` stays `let`.

- [ ] **Step 2: Replace fetch + filter handlers with immediate-push versions**

Replace `fetchTimelineData`/`applyFilter`/`resetFilter`/`handleDropdownChange`/`restoreFilterFromRoute`/restore effect with exactly:

```js
$effect(() => {
  fetchTimelineData();
});

const fetchTimelineData = async () => {
  try {
    data = await api.request('/api/photos/timeline');
  } catch (error) {
    console.error('Failed to initialize timeline:', error);
    addToast(
      $t('notifications.error', { default: 'Error' }),
      $t('errors.timeline_load_failed', { default: 'Failed to load timeline data' }),
      'error',
      4000
    );
    initError = true;
  }
};

const pushFilter = () => {
  const year = currentFilter?.year ?? null;
  const month = currentFilter?.month ?? null;
  pushState({ year, month: year ? month : null });
};

const selectYear = (year) => {
  if (selectedYear === year && (currentFilter?.month ?? null) === null) {
    currentFilter = null;
    selectedYear = null;
  } else if (selectedYear === year) {
    currentFilter = { year, month: null };
  } else {
    currentFilter = { year, month: null };
    selectedYear = year;
  }
  if (selectedYear !== null && currentFilter !== null && currentFilter.year !== selectedYear) {
    selectedYear = currentFilter.year;
  }
  if (currentFilter === null) selectedYear = null;
  pushFilter();
};

const resetFilter = () => {
  currentFilter = null;
  selectedYear = null;
  if (yearSelectEl) yearSelectEl.value = '';
  if (monthSelectEl) monthSelectEl.value = '';
  pushFilter();
};

const handleDropdownChange = () => {
  const year = yearSelectEl?.value;
  let month = monthSelectEl?.value;
  if (!year) {
    month = null;
    if (monthSelectEl) monthSelectEl.value = '';
  }
  if (!year && !month) {
    currentFilter = null;
    selectedYear = null;
  } else {
    const parsedYear = year ? parseInt(year, 10) : null;
    currentFilter = {
      year: parsedYear,
      month: month ? parseInt(month, 10) : null,
    };
    selectedYear = parsedYear;
  }
  pushFilter();
};

const restoreFilterFromRoute = (year, month) => {
  if (!year && !month) {
    if (currentFilter) {
      currentFilter = null;
      selectedYear = null;
      if (yearSelectEl) yearSelectEl.value = '';
      if (monthSelectEl) monthSelectEl.value = '';
    }
  } else if (year) {
    currentFilter = { year, month: month || null };
    selectedYear = year;
    if (yearSelectEl) yearSelectEl.value = String(year);
    if (monthSelectEl) monthSelectEl.value = month ? String(month) : '';
  }
};

// Restore filter from route state (URL restore / popstate).
// Reads route BEFORE any guard: an early return that reads nothing empties
// the effect's dependency set and permanently unsubscribes it.
$effect(() => {
  const year = route.year;
  const month = route.month;
  restoreFilterFromRoute(year, month);
});
```

Keep `fetchTimelineData` error path byte-for-byte (toast keys + `initError = true`) to satisfy FR-012. No debounce: coarse clicks push synchronously.

- [ ] **Step 3: Replace the desktop template block, keep mobile untouched**

Replace everything between `<!-- Desktop: Slider -->` and the closing of `.timeline-slider.desktop-only` with exactly:

```svelte
<!-- Desktop: Year rail + month strip -->
<div class="timeline-rail desktop-only">
  <button
    type="button"
    class="timeline-reset"
    title={$t('ui.clear_timeline_filter', { default: 'Clear timeline filter' })}
    aria-label={$t('ui.clear_timeline_filter', { default: 'Clear timeline filter' })}
    onclick={resetFilter}
  >
    <Icon name="x" width={14} height={14} />
  </button>
  <div
    class="timeline-year-rail"
    role="group"
    aria-label={$t('ui.timeline_years_label', { default: 'Years' })}
  >
    {#each aggregates as agg (agg.year)}
      <button
        type="button"
        class="timeline-year"
        class:active={selectedYear === agg.year}
        aria-pressed={selectedYear === agg.year}
        aria-label={`${agg.year}, ${$t('ui.photos_count', { values: { count: agg.total }, default: '{count} photos' })}`}
        onclick={() => selectYear(agg.year)}
      >
        <span class="timeline-year-label">{agg.year}</span>
        <span class="timeline-year-count">{agg.total}</span>
      </button>
    {/each}
  </div>
  <div class="timeline-label" class:filtered={currentFilter !== null}>{labelText}</div>
</div>
```

Do NOT touch `{#if !data}` skeleton, `{:else if positions.length === 0}` (rewrite as `aggregates.length === 0` rendering nothing), or the `.timeline-dropdowns.mobile-only` block. The skeleton keeps `{labelText}` so loading still announces.

- [ ] **Step 4: Quick static check**

Run: `npm run test:i18n && npm run lint:js`
Expected: both PASS (new literal keys resolve; no Svelte/ESLint warnings).

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/TimelineSlider.svelte
git commit -m "feat(timeline): replace desktop slider with year rail"
```

---

### Task 4: Month strip (12 months, empty-dimmed, toggle semantics)

**Files:**
- Modify: `frontend/src/components/TimelineSlider.svelte` (script: add `selectMonth`; template: month strip under the rail)
- Test: `tests/timeline-aggregates.test.js` (already covers zero-month slots from Task 2)

**Interfaces:**
- Consumes: `selectedAggregate` + `selectedYear`/`currentFilter` from Task 3; `APP_CONSTANTS.MONTH_KEYS` for names.
- Produces: complete desktop navigator consumed by Task 5 styles and Task 6 E2E.

- [ ] **Step 1: Add the month toggle handler next to selectYear**

Insert after `selectYear` exactly:

```js
const selectMonth = (month, count) => {
  if (count === 0 || selectedYear === null) return;
  if (currentFilter?.month === month) {
    currentFilter = { year: selectedYear, month: null };
  } else {
    currentFilter = { year: selectedYear, month };
  }
  pushFilter();
};
```

Zero-count guard enforces SC-005 (empty periods never become filters); `selectedYear === null` guard enforces FR-008.

- [ ] **Step 2: Render the month strip below the rail inside the same desktop container**

Insert after the closing `</div>` of `.timeline-year-rail` and before `.timeline-label`, exactly:

```svelte
{#if selectedAggregate}
  {@const agg = selectedAggregate}
  <div
    class="timeline-month-strip"
    role="group"
    aria-label={$t('ui.timeline_months_label', { default: 'Months' })}
  >
    {#each agg.months as slot (slot.month)}
      {@const monthKey = APP_CONSTANTS.MONTH_KEYS[slot.month - 1]}
      {@const monthName = $t(`ui.months.${monthKey}`, {
        locale: activeLocale,
        default: `${monthKey.charAt(0).toUpperCase()}${monthKey.slice(1)}`,
      })}
      <button
        type="button"
        class="timeline-month"
        class:active={currentFilter?.month === slot.month}
        class:empty={slot.count === 0}
        disabled={slot.count === 0}
        aria-pressed={currentFilter?.month === slot.month}
        aria-label={slot.count === 0
          ? `${monthName} ${agg.year}, ${$t('ui.timeline_no_photos_month', { default: 'No photos' })}`
          : `${monthName} ${agg.year}, ${$t('ui.photos_count', { values: { count: slot.count }, default: '{count} photos' })}`}
        onclick={() => selectMonth(slot.month, slot.count)}
      >
        <span class="timeline-month-label">{monthName}</span>
        <span class="timeline-month-count">{slot.count}</span>
      </button>
    {/each}
  </div>
{/if}
```

Rapid successive year clicks re-derive `selectedAggregate` synchronously from `selectedYear`, so the strip always reflects the latest year (edge case). Clearing the year unmounts the strip with the month (FR-008). All 12 months render; empties are `disabled` + dimmed + announced (FR-004, SC-005).

- [ ] **Step 3: Verify unit + i18n still green**

Run: `node --test tests/timeline-aggregates.test.js && npm run test:i18n`
Expected: 4 passing; i18n 0 unresolved (template uses only the `monthKey` enum).

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/TimelineSlider.svelte
git commit -m "feat(timeline): add month strip with empty-month guard"
```

---

### Task 5: Scoped styles (non-overlap rail, focus, reduced motion, breakpoint)

**Files:**
- Modify: `frontend/src/components/TimelineSlider.svelte:352-718` (`<style>`: delete slider CSS, add rail/strip CSS)

**Interfaces:**
- Consumes: `.desktop-only`/`.mobile-only` breakpoint contract (`@media (width <= 768px)`), `Icon` sizing via `:global(svg)`.
- Produces: zero-overlap desktop navigator styling verified by Task 6 overlap assertion.

- [ ] **Step 1: Delete dead slider CSS**

Delete these rule sets in full: `.timeline-slider`, `.timeline-track-stack`, `.timeline-track`, `.timeline-groove`, `.timeline-ribbon`, `.timeline-bar` (+ `.selected`/`.hovered`/keyframes `timeline-bar-grow`), `.timeline-input` (+ all `::-webkit-slider-thumb`/`::-moz-range-thumb` rules), `.timeline-ticks`, `.timeline-year-tick` (+ `.first`/`.last`), `.timeline-tooltip` (+ `-date`/`-count`/keyframes `timeline-tooltip-in`). Keep `.timeline-container`, `.timeline-label` (+ `.filtered`), `.timeline-reset` (+ hover/active/focus-visible), `.timeline-skeleton`, `.timeline-dropdowns` + selects, `.desktop-only`/`.mobile-only` + the `(width <= 768px)` swap, the `(width <= 480px)` mobile tweak, and the `prefers-reduced-motion` block (retargeted below).

- [ ] **Step 2: Add rail/strip styles in the component scope**

Insert after `.timeline-reset:focus-visible` exactly:

```css
.timeline-rail {
  display: flex;
  align-items: flex-start;
  gap: var(--space-4);
}

.timeline-year-rail {
  flex: 1;
  display: flex;
  gap: var(--space-2);
  min-width: 0;
  overflow-x: auto;
  flex-wrap: nowrap;
  padding-block: var(--space-1);
}

.timeline-year {
  flex: 0 0 auto;
  display: flex;
  align-items: baseline;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--divider-color);
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--text-primary);
  cursor: pointer;
  white-space: nowrap;
}

.timeline-year:hover {
  border-color: var(--primary-color);
}

.timeline-year.active {
  background: color-mix(in oklch, var(--primary-color) 12%, transparent);
  border-color: var(--primary-color);
  color: var(--primary-dark);
}

.timeline-year:focus-visible,
.timeline-month:focus-visible,
.timeline-reset:focus-visible {
  outline: none;
  box-shadow:
    0 0 0 2px var(--surface-color),
    0 0 0 4px var(--primary-color);
}

.timeline-year-label {
  font-size: var(--font-sm);
  font-weight: var(--font-medium);
}

.timeline-year-count,
.timeline-month-count {
  font-size: var(--font-xs);
  color: var(--text-secondary);
}

.timeline-month-strip {
  display: grid;
  grid-template-columns: repeat(6, minmax(0, 1fr));
  gap: var(--space-2);
  margin-top: var(--space-3);
}

.timeline-month {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--divider-color);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-primary);
  cursor: pointer;
  min-width: 0;
}

.timeline-month:hover:not(:disabled) {
  border-color: var(--primary-color);
}

.timeline-month.active {
  background: color-mix(in oklch, var(--primary-color) 12%, transparent);
  border-color: var(--primary-color);
}

.timeline-month.empty {
  opacity: 0.55;
  cursor: not-allowed;
}

.timeline-rail :global(svg) {
  width: 14px;
  height: 14px;
}
```

Non-overlap proof: rail children are `flex: 0 0 auto` + `nowrap` + `overflow-x: auto` — buttons size to content and scroll instead of colliding, so SC-001 holds for 60 or 600 years. No global rules added (scoped-beats-global trap).

- [ ] **Step 3: Retarget the reduced-motion block**

Replace the selector list `.timeline-bar, .timeline-label, .timeline-label.filtered, .timeline-tooltip, .timeline-input::-webkit-slider-thumb, .timeline-reset` with `.timeline-year, .timeline-month, .timeline-label, .timeline-label.filtered, .timeline-reset` (keep the separate `-moz` comment/rule deleted with the slider; no pseudo-element lists that Chromium drops). Keep `animation: none; transition: none;`.

- [ ] **Step 4: Verify style gates**

Run: `npm run format:check && npm run lint`
Expected: PASS, zero warnings (prettier + eslint + stylelint).

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/TimelineSlider.svelte
git commit -m "feat(timeline): style year rail and month strip"
```

---

### Task 6: E2E + a11y retarget, full verification, dead-code sweep

**Files:**
- Modify: `tests/e2e/specs/timeline.e2e.spec.js`
- Modify: `tests/e2e/specs/timeline-a11y.e2e.spec.js`
- Test: `tests/e2e/setup/test-helpers.js` (read-only; reuse `waitForUrlParam`, `waitForPhotosToLoad`, `goto`)

**Interfaces:**
- Consumes: desktop selectors `.timeline-year-rail .timeline-year`, `.timeline-month-strip .timeline-month`, `.timeline-reset`, `.timeline-label` from Tasks 3–5.

- [ ] **Step 1: Replace slider interaction tests with rail/strip tests**

In `tests/e2e/specs/timeline.e2e.spec.js` delete the tests `should filter photos by date range`, `should announce readable value via aria-valuetext`, `should scrub with keyboard`, `should show year tick labels`, `should keep tooltip inside viewport`, `should suppress animations with reduced motion`, `should animate bars on load with motion allowed`, `should fetch timeline data exactly once per mount` where they touch `.timeline-input`/`.timeline-bar`/`.timeline-tooltip`. Replace with exactly:

```js
test('should filter to year then month in two clicks', async ({ page }) => {
  // GIVEN a cleared filter with timeline data
  const density = await page.evaluate(() =>
    fetch('/api/photos/timeline')
      .then((r) => r.json())
      .then((d) => d.density || [])
  );
  test.skip(density.length === 0, 'Timeline needs at least one month bucket');
  const target = density[0];
  const yearButton = page.locator('.timeline-year-rail .timeline-year', {
    hasText: String(target.year),
  });
  await expect(yearButton.first()).toBeVisible();

  // WHEN activating a year
  await yearButton.first().click();
  await TestHelpers.waitForUrlParam(page, 'year', String(target.year));

  // THEN the month strip for that year appears with twelve months
  await expect(page.locator('.timeline-month-strip .timeline-month')).toHaveCount(12);

  // WHEN activating a non-empty month
  const bucket = density.find((d) => d.year === target.year && d.count > 0);
  const monthButton = page.locator('.timeline-month-strip .timeline-month').nth(bucket.month - 1);
  await expect(monthButton).toBeEnabled();
  await monthButton.click();

  // THEN grid + URL reflect year+month and the selection is visibly active
  await TestHelpers.waitForUrlParam(page, 'month', String(bucket.month));
  await TestHelpers.waitForPhotosToLoad(page);
  await expect(monthButton).toHaveClass(/active/);
});

test('should render 50+ sparse years with zero label overlap', async ({ page }) => {
  const boxes = await page
    .locator('.timeline-year-rail .timeline-year')
    .evaluateAll((els) => els.map((el) => el.getBoundingClientRect().toJSON()));
  test.skip(boxes.length === 0, 'Timeline needs at least one year');
  for (let i = 0; i < boxes.length; i++) {
    for (let j = i + 1; j < boxes.length; j++) {
      const a = boxes[i];
      const b = boxes[j];
      const overlaps = a.x < b.x + b.width && b.x < a.x + a.width;
      expect(overlaps).toBe(false);
    }
  }
});

test('should disable empty months and clear via reset', async ({ page }) => {
  const density = await page.evaluate(() =>
    fetch('/api/photos/timeline')
      .then((r) => r.json())
      .then((d) => d.density || [])
  );
  test.skip(density.length === 0, 'Timeline needs at least one month bucket');
  const targetYear = density[0].year;
  await page
    .locator('.timeline-year-rail .timeline-year', { hasText: String(targetYear) })
    .first()
    .click();
  const filled = new Set(
    density.filter((d) => d.year === targetYear).map((d) => d.month)
  );
  test.skip(filled.size === 12, 'Year needs at least one empty month');
  const emptyIndex = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12].find((m) => !filled.has(m));
  await expect(
    page.locator('.timeline-month-strip .timeline-month').nth(emptyIndex - 1)
  ).toBeDisabled();

  await page.locator('.timeline-rail .timeline-reset').click();
  await expect(page).not.toHaveURL(/[?&]year=/);
});
```

Keep `should display timeline controls` and `should show date range when timeline is available` (selectors `.timeline-slider, .timeline-container` still match via `.timeline-rail` inside `.timeline-container` — update first selector to `.timeline-rail, .timeline-container`).

- [ ] **Step 2: Retarget the a11y spec**

In `tests/e2e/specs/timeline-a11y.e2e.spec.js` replace `await expect(page.locator('.timeline-input')).toHaveCount(1);` with `await expect(page.locator('.timeline-year-rail .timeline-year').first()).toBeVisible();`. Keep the mobile test (`#timeline-year-select`) unchanged. Keep `AXE_RULES` and `.include('.timeline-container')` as-is.

- [ ] **Step 3: Sweep dead references**

Run: `grep -rn "timeline-input\|timeline-bar\|timeline-ribbon\|yearTicks\|sliderValue\|handleSliderInput" frontend/src tests/e2e | head -20`
Expected: zero hits (slider fully excised; mobile selects remain).

- [ ] **Step 4: Run the full verification chain**

Run: `npm run test:i18n && node --test tests/timeline-aggregates.test.js && npm run lint && npm run format:check && npm run build`
Expected: all PASS; `dist/` regenerated.

Then run: `cargo build --bin turbo-pix`
Expected: success (build.rs finds `dist/`).

Then run: `npx playwright test tests/e2e/specs/timeline.e2e.spec.js tests/e2e/specs/timeline-a11y.e2e.spec.js tests/e2e/specs/url-routing.e2e.spec.js`
Expected: all PASS sequentially (workers: 1 per Playwright config); Back/Forward restore covered by `url-routing` plus the rail `waitForUrlParam` asserts.

- [ ] **Step 5: Commit**

```bash
git add tests/e2e/specs/timeline.e2e.spec.js tests/e2e/specs/timeline-a11y.e2e.spec.js
git commit -m "test(timeline): cover year rail and month strip flows"
```

---

## Self-Review

**1. Spec coverage:** FR-001 (rail/strip buttons, Tasks 3–4) ✓; FR-002 (scroll rail, Task 5 + overlap test Task 6) ✓; FR-003 (only photo-years render, gaps via jumps; empties disabled Task 4) ✓; FR-004 (12 slots zero-filled Task 2, strip Task 4) ✓; FR-005 (year/month counts rendered + announced Tasks 3–4) ✓; FR-006 (toggle matrix Task 3–4 + E2E Task 6) ✓; FR-007 (pushFilter + route-first restore effect Tasks 3–4, URL asserts Task 6) ✓; FR-008 (strip gated on selectedYear, null-cascade Tasks 3–4) ✓; FR-009 (native buttons, focus-visible, aria-label/pressed, axe Tasks 3–6) ✓; FR-010 (3 keys both locales + guard Tasks 1–2) ✓; FR-011 (desktop block only, mobile untouched, breakpoint CSS Task 5) ✓; FR-012 (fetch error path + empty-render-nothing preserved Task 3) ✓. Edge cases: empty (render nothing), single-year (no scroll needed), century-scale (scroll), one-month year (11 disabled), load failure (toast + initError), year-clear cascades month, rapid years (synchronous derived), boundary 768px (exclusive classes) — all assigned. SC-001..005 each have an E2E or guard assertion.

**2. Placeholder scan:** no TBD/TODO/similar-to/appropriate-handling language; every step names exact files, exact code, exact commands with expected output.

**3. Type consistency:** `buildYearAggregates`/`getYearAggregate` signatures identical in Tasks 2–4; `currentFilter { year, month }`, `selectedYear number|null`, `selectYear(year)`, `selectMonth(month, count)`, `pushFilter()`, `resetFilter()` named identically across tasks; i18n keys spelled identically in JSON, Svelte, and tests.
