# Feature Specification: Svelte Frontend Migration

**Created**: 2026-07-30
**Status**: Approved
**Input**: Replace plain vanilla JavaScript frontend with Svelte while keeping the exact functionality

## Goal

Replace the entire TurboPix frontend — currently ~15,400 lines of vanilla JavaScript across 28 script files served without a bundler — with a Svelte 5 + Vite build pipeline. Every existing user-facing feature, behavior, and visual appearance must be preserved identically. The Rust backend, REST API, database schema, and all server-side logic remain untouched. The Vite build output replaces the current `static/` directory as the source of files embedded into the Rust binary via `include_static!`.

## User Scenarios

### Scenario 1 - Browse and view photos (P1)
A user opens TurboPix and sees the photo grid with infinite scroll. They scroll down to load more photos, click a photo to open the viewer, swipe left/right to navigate, pinch-zoom to inspect details, use keyboard arrows for navigation, and close the viewer by clicking outside or pressing Escape. Every interaction matches the current behavior exactly.

**Acceptance**
1. Given the app is loaded, when the user scrolls to the bottom of the photo grid, then more photos load automatically with BlurHash placeholders displayed during loading.
2. Given a photo is clicked, when the viewer opens, then the View Transitions API animates the entry and the photo displays with functional swipe/zoom/keyboard navigation.
3. Given the viewer is open, when the user swipes vertically or presses Escape, then the viewer closes and returns to the previous scroll position in the grid.

### Scenario 2 - Search and filter (P1)
A user types a search query, triggers semantic search, and sees results filtered by relevance. They apply timeline year/month filters and sort by different criteria. URL query parameters update to reflect the active filters, and sharing the URL reproduces the same filtered view.

**Acceptance**
1. Given a search query is entered and submitted, when results return, then only matching photos appear in the grid and the URL contains `?q=<query>`.
2. Given a year and month are selected on the timeline heatmap, when applied, then only photos from that period appear and the URL contains `?year=<yyyy>&month=<mm>`.
3. Given a filtered view with URL parameters, when the URL is opened in a new tab, then the same filters and results are restored.

### Scenario 3 - Favorites, videos, collages, and housekeeping (P1)
A user switches between the five views (All Photos, Favorites, Videos, Collages, Housekeeping) via the sidebar navigation. Each view loads its respective data and updates the URL path. Favoriting a photo from the viewer or grid immediately updates the favorites view.

**Acceptance**
1. Given the user clicks "Favorites" in the sidebar, when the view switches, then only favorited photos appear and the URL path is `/favorites`.
2. Given a photo is favorited from the viewer, when returning to the grid, then the photo's favorite state is reflected in its card.
3. Given the user is on the Collages view, when a collage is accepted or rejected, then the candidate is removed and the next candidate loads.

### Scenario 4 - Indexing progress (P1)
During initial or rescan indexing, the user sees an orbit ring with animated phase segments and a bottom sheet showing per-phase progress. The UI updates in real time and auto-opens on first visit when indexing is active.

**Acceptance**
1. Given indexing is in progress, when the app loads, then the orbit ring displays animated segments for each indexing phase and the bottom sheet shows progress bars and current item name.
2. Given indexing completes, when the last phase finishes, then a completion animation plays and the bottom sheet dismisses.
3. Given indexing is idle, when the user opens the app, then the orbit ring shows a static "idle" state.

### Scenario 5 - Responsive layout and theme (P2)
A user accesses the app on a mobile device. The sidebar becomes an off-canvas drawer, the search input moves to a dedicated overlay, and the viewer metadata sidebar becomes a draggable bottom sheet. Switching between dark and light theme updates all colors, and the preference persists across sessions.

**Acceptance**
1. Given a viewport narrower than 768px, when the app renders, then the sidebar is hidden and accessible via a hamburger toggle.
2. Given the theme toggle is clicked, when the theme changes, then all UI elements reflect the new color scheme and the preference is saved to localStorage.

### Scenario 6 - Build and deploy (P2)
A developer runs the build command chain and the compiled frontend is embedded into the Rust binary. The resulting binary serves the complete application.

**Acceptance**
1. Given a clean checkout, when `npm run build && cargo build --bin turbo-pix` is executed, then the binary starts and serves the full Svelte-based frontend on `http://localhost:18473`.
2. Given the binary is running, when a browser requests `/`, then the Svelte-rendered index page is served with all assets resolving correctly.

## Functional Requirements

- **FR-001**: The frontend MUST be built using Svelte 5 (runes mode) with Vite as the bundler, without SvelteKit. No server-side rendering.
- **FR-002**: The Vite build MUST produce static JS and CSS bundles in a `dist/` directory. The Rust `include_static!` macro MUST be updated to embed files from the Vite output directory instead of `static/`.
- **FR-003**: All five views (All Photos, Favorites, Videos, Collages, Housekeeping) MUST render with identical content and behavior to the current implementation.
- **FR-004**: The photo viewer modal MUST support: swipe-based horizontal navigation, pinch-to-zoom, double-tap-to-zoom, pan-when-zoomed with edge boundaries, vertical swipe-to-dismiss, keyboard navigation (arrows, Escape), haptic feedback, and View Transitions API entry/exit animations.
- **FR-005**: Infinite scroll MUST trigger photo loading when the user scrolls near the bottom of `.main-content`, loading photos in batches of 50 with BlurHash placeholders during image fetch.
- **FR-006**: Semantic search MUST send queries to `/api/search/semantic`, map returned hashes to photos via `/api/photos`, and display results in the grid with the search query reflected in the URL as `?q=<query>`.
- **FR-007**: The timeline heatmap MUST display year/month photo counts as an interactive slider, filtering the grid by `?year=<yyyy>&month=<mm>` URL parameters.
- **FR-008**: Client-side routing MUST support the same URL structure as the current implementation: path-based views (`/`, `/favorites`, `/videos`, `/collages`, `/housekeeping`) and query parameters (`?q=`, `?sort=`, `?year=`, `?month=`, `?photo=`). Browser history (back/forward) and direct URL entry MUST resolve correctly.
- **FR-009**: The router MUST implement the anti-loop pattern: components called from `popstate` handlers MUST accept an `updateUrl=false` parameter to skip re-pushing to history.
- **FR-010**: The indexing progress UI MUST poll `/api/indexing/status` every 1 second during active indexing and every 30 seconds when idle. It MUST render the SVG orbit ring with animated phase segments, per-phase progress bars (determinate and indeterminate), error counts, current item display, and completion animation.
- **FR-011**: The indexing empty-state guard MUST be preserved: when `indexingStatus.isIndexing && !currentQuery`, an empty photo grid MUST display "Indexing in progress" messaging rather than "No Photos Found".
- **FR-012**: Dark and light theme MUST be togglable via a UI control, persisted to localStorage, and applied as a class on the `<html>` element. All colors MUST use the existing OKLCH-based design token system.
- **FR-013**: Responsive layout MUST support three breakpoints (768px, 1024px, 1200px): sidebar becomes off-canvas on mobile, viewer metadata sidebar becomes a draggable bottom sheet, search input gets a dedicated mobile overlay.
- **FR-014**: Touch gesture support MUST include pinch-to-zoom, swipe navigation, pan, and double-tap recognition, matching the current `GestureManager`/`GestureRecognizer` behavior.
- **FR-015**: The i18n system MUST support English (en) and German (de) locales with all current translation keys preserved. Locale resolution order MUST be localStorage → backend `/api/config` `default_locale` → browser language → `en` (operator config beats browser; browser kept as fallback).
- **FR-016**: All CSS MUST be migrated to Svelte component-scoped `<style>` blocks. Global design tokens (spacing, typography, radii, z-index, shadows, transitions, OKLCH colors) MUST be extracted into a shared CSS custom properties file loaded globally.
- **FR-017**: Glassmorphism effects (`backdrop-filter`) on header and sidebar MUST use fixed positioning overlaying scrollable content. The scroll container `.main-content` MUST remain a scrollable element with `overflow-y: auto` to preserve infinite scroll behavior.
- **FR-018**: Photo cards MUST use `[data-photo-id]` selectors and render with the same DOM structure as the current implementation, supporting blurhash placeholders, favorite toggling, and video duration badges.
- **FR-019**: All E2E Playwright tests MUST continue to pass after migration. Test selectors (`data-*` attributes, ARIA roles, CSS classes used in tests) MUST be preserved or updated with equivalent selectors in the Svelte output.
- **FR-020**: The favicon, site manifest, Feather Icons library, and all static assets served outside the Svelte build MUST remain accessible at their current URL paths.
- **FR-021**: The metadata edit modal MUST support editing photo metadata (title, description, taken_at date) with the same form validation and API calls.
- **FR-022**: Favoriting a photo from the viewer or grid MUST immediately update the photo's state via `/api/photos/:hash/favorite` and dispatch the `favoriteToggled` equivalent event so all views reflect the change.
- **FR-023**: Collages view MUST load collage candidates, display them with accept/reject controls, and remove candidates from the list upon action.
- **FR-024**: Housekeeping view MUST display near-duplicate and low-quality photo candidates with remove/skip actions, updating the list after each action.

## Key Entities

- **Photo**: The core domain object with hash, file path, mime type, taken_at timestamp, width, height, favorite flag, blurhash, and metadata fields. Displayed in the grid as cards and in the viewer as full-resolution media.
- **IndexingStatus**: Polled state object with `is_indexing` flag, `current_phase` identifier, phase-level progress (files found, processed, errors), and current item path.
- **ViewState**: Client-side state tracking the active view (all/favorites/videos/collages/housekeeping), search query, sort order, timeline year/month, and currently opened photo hash.

## Edge Cases

- The `npm run build` step must complete before `cargo build` embeds the output. A missing or stale `dist/` directory must produce a clear build error.
- The router must handle the "month without year" case: `?month=3` without `?year=` must not be written to the URL; `restore` path already ignores it.
- When the viewer is open and a new photo is preloaded, the gesture state must reset to prevent stale swipe offsets.
- Theme switching must not cause a flash of unstyled content (FOUC). The theme class must be applied before the first paint.
- Video files must check for transcoding status via the API and display appropriate UI (transcoding indicator vs. playable video).
- Touch and mouse events must not conflict; the gesture manager must handle both input modes on hybrid devices.
- The infinite scroll observer must be disconnected and reconnected when switching views to prevent stale scroll triggers.

## Assumptions

- Svelte 5 runes (`$state`, `$derived`, `$effect`) are used; Svelte 4 patterns (`$:`, `export let`, stores) are not carried forward.
- The build chain is `npm run build` (Vite) followed by `cargo build`. Developer workflows requiring a single command can use a Makefile or script — this is not part of the migration scope.
- The existing i18n translation dictionaries (en/index.js, de/index.js) are ported as data (JSON or similar) consumed by the new i18n library, not reused as code.
- The i18n library is Paraglide JS or an equivalent Svelte 5-compatible solution. The spec does not mandate a specific library, only that all current translation keys resolve and locale switching works.
- The Rust `include_static!` macro is updated to embed files from `dist/` using a glob or explicit file list. Binary files (favicon, etc.) continue to use `include_static_binary!`.
- The `static/` directory is removed or repurposed after migration; the Svelte source code lives in a new `src/` (frontend) or `frontend/` directory.
- E2E test selectors are updated to match the Svelte DOM output where necessary. Test logic (scenarios, assertions, flows) remains unchanged.
- The Feather Icons library continues to be loaded as a static script or imported as an npm package — either approach is acceptable as long as icon rendering works identically.

## Success Criteria

- **SC-001**: Running `npm run build && cargo build --bin turbo-pix` produces a binary that starts and serves the full application on port 18473 without errors.
- **SC-002**: All five views (All, Favorites, Videos, Collages, Housekeeping) render with correct content and respond to navigation, filtering, and sorting identically to the current vanilla JS implementation.
- **SC-003**: The photo viewer supports swipe navigation, pinch-zoom, keyboard shortcuts, and vertical dismiss on both desktop and mobile viewports.
- **SC-004**: Typing a search query, submitting it, and clicking a result opens the viewer for the correct photo, and the URL reflects the search query.
- **SC-005**: Selecting a year and month on the timeline filters the grid to only photos from that period, and the URL parameters match.
- **SC-006**: All existing Playwright E2E tests pass against the Svelte-based frontend without test logic changes (selector updates allowed).
- **SC-007**: Switching between English and German updates all UI text, including navigation labels, button text, viewer metadata fields, and indexing phase names.
- **SC-008**: The dark/light theme toggle changes the entire UI color scheme, persists across page reloads, and applies without a visible flash.
- **SC-009**: On a 375px-wide mobile viewport, the sidebar is off-canvas, the search input uses the mobile overlay, and the viewer metadata is accessible as a bottom sheet.
- **SC-010**: During active indexing, the orbit ring animates phase segments and the bottom sheet displays real-time progress; when indexing completes, the completion animation plays.
