# Learnings — indexing-progress-viz

## [2026-04-06] Session Start

### Key Conventions
- Use `window.i18nManager.t()` or `utils.t()` — NEVER `window.i18n`
- Use feather icons — NEVER emojis
- Rebuild binary after static file changes: `cargo build --bin turbo-pix`
- `.main-content` must remain scrollable with `overflow-y: auto` (InfiniteScroll dependency at `infiniteScroll.js:9`)
- Glassmorphism blur only works on `position: fixed` elements overlaying scrollable content

### Public API to Preserve
- `window.indexingStatus = new IndexingOrbitManager()` (or equivalent)
- `.init()` method that starts polling
- `window.dispatchEvent(new CustomEvent('indexingStatusChanged', { detail: ... }))` — consumed by `housekeeping.js:91,130`
- `window.photoGrid.loadPhotos()` called on indexing completion

### Polling Intervals (do NOT change)
- Active indexing: 1000ms
- Idle: 30000ms

### Phase IDs (in order from CANONICAL_PHASES)
1. `discovering` — kind: indeterminate, icon: camera
2. `metadata` — kind: determinate, icon: file-text
3. `semantic_vectors` — kind: determinate, icon: cpu
4. `geo_resolution` — kind: determinate, icon: map-pin
5. `collages` — kind: indeterminate, icon: grid
6. `housekeeping` — kind: indeterminate, icon: check-circle

### Data Attributes (testable)
- `data-phase-ring` — ring container
- `data-ring-mode="large|compact|hidden"` — sizing mode
- `data-phase-id="[phase-name]"` — per-phase SVG segment
- `data-phase-state="pending|active|done|error"` — phase state
- `data-orbit-dot` — indeterminate orbiting dot
- `data-bottom-sheet` — bottom sheet container

### E2E Skeleton Notes
- Keep new indexing-orbit tests in `test.fixme()` until the component exists.
- Reuse the exact `page.route('**/api/indexing/status', ...)` mocking pattern from `loading.e2e.spec.js`.
- Use `TestHelpers.setupConsoleMonitoring(page)` in `beforeEach` so browser errors surface during later implementation.

### Z-Index Budget
- Ring: 900
- Bottom sheet: 950
- Toasts/modals: 10000 (reserved, do not exceed)

### First-Run Detection
- Heuristic: `photos_indexed === 0 && is_indexing === true` → large mode
- localStorage key: `turbopix_has_indexed = true` set on first completion

### CSS Design Tokens (use these, never hardcode)
- `--primary-color: oklch(55% 0.08 250deg)` (blue)
- `--glass-bg`, `--glass-border`
- `backdrop-filter: blur(16px) saturate(1.5)`
- `--ease-spring: cubic-bezier(0.34, 1.56, 0.64, 1)`
- `--color-danger: oklch(55% 0.2 25deg)`

### SVG Arc Math
- Arc length for 60° segment of radius R = `R * π / 3`
- Use `stroke-dasharray`/`stroke-dashoffset` for partial arc fills

### Key File Locations
- Old JS: `static/js/indexingStatus.js` (303 lines) — preserve API shape
- New JS: `static/js/indexingOrbit.js` (create)
- Static registration: `src/handlers_static.rs` — swap `indexingStatus.js` → `indexingOrbit.js`
- Old banner HTML: `static/index.html:63-143`
- Old CSS: `static/css/components.css:1304-1577`
- Dark mode overrides: `static/css/main.css:703-707`
- Script load position: `static/index.html:624`
- App init: `static/js/app.js:48-49`
- Housekeeping consumers: `static/js/housekeeping.js:91,130`
- i18n keys: `static/i18n/en/index.js:192-207`, `static/i18n/de/index.js:190-205`
- E2E mock pattern: `tests/e2e/specs/loading.e2e.spec.js`
- Test helpers: `tests/e2e/setup/test-helpers.js`

## [2026-04-06] Task 2: SVG Ring Component
- `indexingOrbit.js` must normalize the backend response to include `status.phase` from `active_phase_id`, because existing `housekeeping.js` still reads `status.phase`.
- The foundation ring can preserve the old public API by keeping `window.indexingStatus.init()`, dispatching `indexingStatusChanged`, and calling `window.photoGrid.loadPhotos()` on completion.
- Base orbit styling can be added alongside old banner CSS; do not remove legacy banner styles in this task.

## [2026-04-06] Task 3: Bottom Sheet Scaffold
- Added HTML scaffold for bottom sheet at the end of the `body` in `index.html` (since `data-phase-ring` isn't physically in the static template as expected).
- Ensured any buttons without `type` are declared as `type="button"` to avoid linting issues.
- Fixed `alpha-value-notation` issue in `components.css`: changed `0.08` to `8%` in `oklch(from var(--primary-color) l c h / 0.08)`.
- Added `indexing_sheet_*` i18n keys to both `en/index.js` and `de/index.js`.

### Task 4 & 5 Findings
- Implemented visual arc filling for determinate indexing phases dynamically updating `stroke-dashoffset` using `progress`.
- Added an orbiting dot `<circle>` wrapped in a `<g>` rotating `-28deg` to `28deg` to visually represent indeterminate phases using CSS keyframes.
- Added smooth spring `stroke-dashoffset` transitions and respected `prefers-reduced-motion: reduce`.
- Confirmed error handling with `console.error` and post-indexing data reloads via `window.photoGrid.loadPhotos()`.
- Addressed stylelint quirks by properly separating rules and avoiding one-liners in CSS files.

## [2026-04-06] Task 6: Adaptive Ring Modes
- `IndexingOrbitManager.determineMode(status)` should prefer localStorage (`turbopix_has_indexed === 'true'`) over `photos_indexed` so later indexing runs never bounce back to the centered first-run state.
- The completion pulse is simplest as a two-step flow: set all phase segments to `done`, keep the ring visible for 2000ms, then switch `data-ring-mode` to `hidden` and reset segments.
- Hidden mode should use opacity/transform on the fixed compact-positioned ring instead of `display: none`; that preserves spring transitions between large, compact, and hidden states.
- Keep `.main-content` untouched; the orbit remains `position: fixed` so InfiniteScroll keeps using `.main-content` as its scroll container.

## [2026-04-06] Task 7: Bottom Sheet Interaction
- Ensure `pointer-events: auto` is set on `[data-phase-ring]` so the compact ring can capture click events.
- Creating elements dynamically via JS (like `.indexing-sheet-backdrop`) and appending to `document.body` keeps the DOM clean for UI overlays.
- A combination of `[aria-hidden='true']` and `.is-visible` on the backdrop provides a clean way to handle toggle states, avoiding inline styles.
- `oklch(0% 0 0 / 30%)` will cause a stylelint error without a hue unit (`0deg`); use `oklch(0% 0 0deg / 30%)`.
- Using `text-overflow: ellipsis` handles long string truncation cleanly without custom JS substring math.

## [2026-04-06] Task 9: Old Banner Cleanup
- Remove old stepper banner selectors from `components.css` and keep orbit styles scoped to `[data-phase-ring]` / `[data-bottom-sheet]`.
- Update E2E helpers to target `[data-phase-ring]` when disabling pointer events for indexing UI.
- Keep the `indexingStatusChanged` event name and `window.indexingStatus` API stable while deleting the legacy banner module.
