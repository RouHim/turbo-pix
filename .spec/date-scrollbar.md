# Feature Specification: Date Scrollbar (Google-Photos/Immich-style)

**Created**: 2026-11-17
**Status**: Approved
**Input**: A date scrollbar at the right edge of the photo grid that jumps to any month and loads in both directions while staying bounded; validated in a throwaway prototype.

## Goal

Desktop users with decades-spanning libraries need to reach any month quickly and continue browsing from there. The timeline lane filters; the scrollbar navigates. This spec adds a quiet, density-proportional rail (the "Leise Schiene" prototype variant) over the all-photos grid, scoped to the active date filter, with an anchored, bounded window in both directions and a top loading indicator that matches the bottom one.

## User Scenarios

### Scenario 1 - Jump to a month and continue browsing both ways (P1)

A desktop user opens All Photos, drags the rail to March 1998, and browses.

**Acceptance**

1. Given the unfiltered all-photos view, when the user activates a month on the rail, then the grid shows the listing from that month's first photo and that month's first row sits directly under the fixed header whenever the content below it can still fill the viewport; at the terminal edge of the listing the scroll clamps and the target is visible in the viewport instead.
2. Given that jump, when the user scrolls up, then newer photos load above and the card the user is looking at does not move.
3. Given that jump, when the user scrolls down, then older photos append and the window stays bounded.
4. Given the jump, when it completes, then the rail thumb names the target month and its photo count.

### Scenario 2 - The rail follows the active date filter (P1)

A user filters to 2017 with the timeline lane.

**Acceptance**

1. Given `year=2017`, when the rail renders, then its track spans only 2017's populated months and the only year label is 2017.
2. Given `2012–2015`, then the track spans that range and labels 2012–2015.
3. Given a filtered track, when the user jumps to a month, then the route filter is unchanged (no month is written).
4. Given the filter is cleared, then the rail returns to the whole-library scale.
5. Given a single-month filter or fewer than two populated months, then the rail is hidden and the native scrollbar is available.
6. Given a filtered period with at most one page of photos, then the rail is hidden.

### Scenario 3 - Density, sorting and other surfaces (P2)

**Acceptance**

1. Given an unsupported sort (name/size), a search result, an album, favorites, videos or semantic search, then the rail is hidden and the native scrollbar remains.
2. Given date-ascending sort, then the track runs oldest-at-top and the keyboard steps follow that direction.
3. Given fewer than three distinct years without a filter, then the rail is hidden (the timeline stays the filter).
4. Given the library density loads, then the rail appears at desktop widths only; the mobile dropdowns are unchanged.

### Scenario 4 - Bounded window and loading feedback (P1)

**Acceptance**

1. Given more than 300 loaded cards, when the grid grows, then the window trims a whole page from the side away from the viewport and the visible card stays put; the cap is `WINDOW_MAX_ITEMS = 300`.
2. Given an upward load, then the top dot-wave indicator appears (same three-dot language as the bottom loader), stays at least 350 ms, and announces itself to assistive tech.
3. Given the window has reached the oldest loaded page, then upward scrolling loads that page again without duplicates or gaps.
4. Given the viewport is scrolled away from the page top, then the upward trigger comes from a top sentinel; a wheel at the absolute top still triggers a load.

### Scenario 5 - Keyboard, reload and accessibility (P2)

**Acceptance**

1. Given keyboard focus on the rail, then Arrow Up/Down steps one month, Page Up/Down one year, Home/End the first/last populated month, and `aria-valuetext` names the month and count.
2. Given a date filter in the URL, then a reload restores the filter and the scoped rail.
3. Given any rail state, no URL parameter is written by the rail itself; Back/Forward only restores the filter.
4. Given `prefers-reduced-motion`, the loader dots do not animate.

## Functional Requirements

- **FR-001**: The rail is navigation only: it never writes the route filter; the timeline lane keeps filtering.
- **FR-002**: The rail renders at the right edge at desktop widths only, replacing the native scrollbar while visible; the native scrollbar returns whenever the rail is hidden.
- **FR-003**: The track is count-proportional over the populated months of the current scope, with at least 1 px per month and collision-filtered year labels.
- **FR-004**: The thumb names the month of the top visible photo; the bubble shows that month and its photo count; the jump places the target's first photo directly under the header, except at the terminal edge of the listing where the scroll range clamps and the target stays visible.
- **FR-005**: Without a date filter the track spans the whole library; with one it spans exactly the filtered months, sliced client-side from the existing density payload (no backend change).
- **FR-006**: A jump resolves the month to the listing offset from the density, fetches that page, and keeps the listing from the anchor onward. Upward loads fetch the page containing the preceding item and take only the prefix before the window start.
- **FR-007**: The loaded window never exceeds `WINDOW_MAX_ITEMS`; trims happen on the side away from the viewport, whole pages from the bottom, and preserve the visible card by identity.
- **FR-008**: Upward and downward loading are both automatic; the top indicator uses the app's dot-wave language.
- **FR-009**: The native scrollbar is hidden only while the rail is shown; `overflow-anchor` is opted out deliberately and card pinning is done explicitly.
- **FR-010**: Cards keep `content-visibility: auto`; the desktop card is square (`aspect-ratio: 1`) so its height is correct under size containment. The mobile card keeps its below-tile caption (aspect ratio unset there).
- **FR-011**: The rail is keyboard operable with slider semantics, a visible focus ring, and localized labels in English and German.
- **FR-012**: The rail is hidden for unsupported surfaces, non-date sorts and periods too small to scroll; no stale filter or selection is written.

## Key Entities

- **Month index**: `year * 12 + (month - 1)`; the unit of density, anchors and trims.
- **Density model**: the dense month model built from `GET /api/photos/timeline`, sliced to the active date filter.
- **Anchor**: the tuple (month, absolute listing offset) used to load a page that starts at a month.
- **Window**: a contiguous slice of the listing with `startOffset`, bounded by `WINDOW_MAX_ITEMS`.
- **Rail visibility**: desktop ∧ all photos ∧ no query/album/semantic search ∧ date sort ∧ enough content.

## Edge Cases

- Undated photos sort last in descending and first in ascending order; the anchor offset adds them only for ascending.
- A mid-page anchor starts the window with the page's prefix, so upward growth begins without a hole.
- A period whose months are all empty never activates; activation is a no-op on zero-photo months.
- A failed density fetch hides the rail; the grid and the native scrollbar are untouched.
- A metadata save re-reads the density so a moved photo changes the track.
- A date filter change resets the window and the thumb to the scoped scale; the grid reloads its first page as before.
- Opening the viewer and returning does not reset the window (the viewer is an overlay).
- The window does not trim on unsupported surfaces.

## Research Notes

- Immich's scrubber is height-proportional (`scrubMonth.height / timelineFullHeight`) with label/dot distance thresholds, `viewportTopMonth` + within-month percent, a virtual timeline (`totalViewerHeight`) and a sliding window (`updateSlidingWindow`), and reserves the scrubber width as a layout margin. Sources: `web/src/lib/components/timeline/Scrubber.svelte`, `Timeline.svelte`, `managers/timeline-manager/timeline-manager.svelte.ts`.
- Native scroll anchoring is suppressed by transform changes in the anchor chain; the card entrance animation animates `translateY`. Preferring explicit card pinning and `overflow-anchor: none` is the deliberate contract; the alternative is an opacity-only entrance animation. Source: CSS Scroll Anchoring spec overview and the prepend/anchor analysis article.
- `contain-intrinsic-size: auto <length>` uses the last rendered size and falls back to the length for never-rendered boxes; the card's square shape makes the height deterministic under `aspect-ratio: 1`. Source: MDN `contain-intrinsic-size`.
- Prototype evidence (throwaway branch): 1,254 photos / 361 months; jumps place the month at the header; upward/downward cycles stay at the 300 cap with zero console errors; filter scoping (`2017`, `2012–2015`) and the small-filter hide were verified.

## Assumptions

- Desktop means `width > 768px`, matching the app breakpoint.
- The route filter is canonical before use (`normalizeDateFilter`); the density slice is the filtered listing's density because the filter is a pure `taken_at` predicate.
- The rail leaves the URL untouched; scroll position is session state.
- The viewer navigates within the loaded window.
- The window cap is a constant (`300`); larger caps trade DOM for fewer trims.

## Success Criteria

- **SC-001**: Any populated month is reachable with one drag/activation; the target's first row sits under the header when the content below can fill the viewport, and stays visible at the terminal edge.
- **SC-002**: With 1,000+ photos, continuous up/down scrolling never exceeds 300 cards and never leaves gaps or duplicates.
- **SC-003**: A filtered track shows exactly the filtered months and a jump within it does not change the filter.
- **SC-004**: The rail is absent (and the native scrollbar present) on every unsupported surface and for periods with at most one populated month or one page of photos.
- **SC-005**: Keyboard-only users can step, jump and clear with announced month and count; EN/DE labels are complete.
- **SC-006**: The top loader is visible and announced during upward loads; reduced-motion users see static dots.
