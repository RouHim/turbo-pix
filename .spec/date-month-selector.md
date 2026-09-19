# Feature Specification: Desktop Date/Month Selector Redesign (Zoomable Timeline Overview)

**Created**: 2026-09-19
**Status**: Approved
**Input**: Redesign the desktop date/month selector — it is broken when a library spans many years/months; mobile is fine.

## Goal
Desktop users with decades-spanning photo libraries cannot reliably browse to a period: the selector degrades as the number of years and months grows (earlier designs collided label text; the current year rail hides years behind horizontal scrolling and signals "more content" only weakly, and an active year can stay outside the visible rail after a URL restore). This spec replaces the desktop selector with a single zoomable overview of the library's whole date span — subtle background density, adaptive collision-free labels, and both single-period and month-granular range selection. Scope is desktop widths only: the mobile year/month dropdowns, the timeline density data source, and all non-timeline features stay unchanged; the year-pill rail is discarded.

## User Scenarios

### Scenario 1 - Jump to a month without scrolling (P1)
A desktop user with a library spanning 30+ years wants photos from March 1998, starting from a cleared timeline filter.

**Acceptance**
1. Given a cleared filter and a library spanning 30+ years at a desktop width, when the selector renders, then the entire date span is visible in the selector with no horizontal scrolling, and the label shows the unfiltered state.
2. Given the full-span view, when the user zooms toward 1998 and activates March, then the grid filters to exactly March 1998 and the selector shows the month as the active selection with a label naming the period.
3. Given an active month filter, when the user activates the clear control, then the grid returns to unfiltered and the active selection disappears in one action.

### Scenario 2 - Select and adjust a month-granular range (P1)
A user wants every photo from March 2012 through August 2015, then widens the start by one month.

**Acceptance**
1. Given a cleared filter, when the user drags from March 2012 to August 2015, then the grid filters to that inclusive range and the label names both bounds.
2. Given the active range, when the user drags the start handle one month earlier, then the grid and the label show the widened range.
3. Given the active range, when the user drags the selection body to a different position, then the range keeps its span and the grid and label follow.
4. Given a drag whose end is earlier than its start, when the drag completes, then the selection is normalized to ascending order.
5. Given a drag confined to a single month, when the drag completes, then the selection is exactly that month and never smaller.

### Scenario 3 - Restore, resize, and reset (P2)
A user reloads, uses Back/Forward, resizes the window, and clears the filter.

**Acceptance**
1. Given an active single-period or range selection, when the page is reloaded or Back/Forward is used, then the grid filters identically and the selection is visible inside the selector viewport, not hidden outside it.
2. Given an active selection, when the browser window is resized between supported desktop widths, then all visible labels remain fully legible and the selection stays visible.
3. Given a restored selection whose periods no longer exist in the library, when the page loads, then the filter is narrowed to the overlap with available data; if there is no overlap, the filter clears to unfiltered.
4. Given any active state (zoomed, selected, or both), when the user activates the clear control, then the filter clears and the view returns to the full span in one action.

### Scenario 4 - Keyboard-only operation (P2)
A keyboard-only user performs the same selection tasks without a pointer.

**Acceptance**
1. Given keyboard focus only, when the user moves through the selector, then each reachable period announces its name and photo count (or "no photos") and shows visible focus.
2. Given keyboard focus on the selector, when the user activates a period, then the filter applies exactly as with a pointer.
3. Given an active range, when the user adjusts each bound by keyboard, then bounds move one month per activation and the grid follows.
4. Given keyboard focus only, when the user zooms, fits the view to all data, and clears the filter, then each of these is reachable and has an announced name.

### Scenario 5 - Density glance before choosing (P3)
A user wants to see where photos cluster and where years are empty before picking a period.

**Acceptance**
1. Given a library with markedly dense and empty stretches, when the selector renders at any zoom level, then a density profile is visible without obscuring labels, the selection, or controls.
2. Given a period with zero photos, when the user attempts to activate it as a single-period filter, then no filter is applied and the period's emptiness is indicated before activation.

## Functional Requirements
- **FR-001**: The desktop selector displays the library's entire date span (oldest through newest period containing photos) in a single view; no horizontal scrolling or paging is required to see the whole span.
- **FR-002**: The selector renders a subtle background density profile derived from per-month photo counts, communicating relative photo volume across the span without obscuring labels, selection graphics, or controls.
- **FR-003**: The user can zoom in and out and pan horizontally by direct manipulation (wheel, trackpad, touch, dragging) as well as through explicit zoom controls; the widest view shows the full span, the closest view spans exactly one month, and panning never leaves the data span.
- **FR-004**: A single "fit all" control returns the view to the full span without altering the active selection.
- **FR-005**: Labels adapt their granularity (decade, year, month) to the visible span and available width; at every zoom level, pan position, and supported desktop width, all visible labels are fully legible — no pair overlaps, none is clipped by the selector viewport, and each keeps a minimum spacing from its neighbors.
- **FR-006**: Pointing at or moving keyboard focus onto a period highlights it and reveals its name and photo count without changing the filter.
- **FR-007**: A single activation of a period containing photos applies a single-period filter — a year, or a year plus a month — and the smallest single period is one month; a period with zero photos cannot become a single-period selection.
- **FR-008**: Dragging across the timeline applies a contiguous, month-granular, inclusive range filter from the start period to the end period; a reversed drag normalizes to ascending order; the shortest selectable range is one month.
- **FR-009**: An active range presents draggable edge handles for each bound and supports translating the whole selection; every adjustment updates the label and the grid as it happens.
- **FR-010**: The selector always displays the active selection: whenever a selection is set or restored, the view adjusts so the entire selection is visible, and the selection label names it — the unfiltered state, a single period, or both bounds of a range.
- **FR-011**: The selection is the single source of truth for the grid: the grid shows exactly the selected period or inclusive range; a month is never present without its year.
- **FR-012**: Every selection change round-trips through the URL; reload and Back/Forward restore the same selection and grid filter, subject to the visibility rule (FR-010) and the clamping rule (Scenario 3).
- **FR-013**: One clear control resets the filter to unfiltered and the view to the full span in a single action from any state; if a drag or zoom gesture is in progress, it is cancelled.
- **FR-014**: The selector is fully operable by keyboard: period navigation, single-period and range selection, adjustment of both range bounds, zooming, fit-all, and clearing; focus is always visible and every target announces its period name and photo count.
- **FR-015**: All selector text — labels, period names, counts, tooltips, control names, and empty-state messaging — is available in English and German with identical key structure.
- **FR-016**: The selector renders only at desktop widths (the existing breakpoint); below it the current mobile year/month dropdowns remain unchanged, and exactly one of the two experiences is active at the boundary.
- **FR-017**: An empty library renders no selector; a timeline data load failure surfaces the existing error path with no stale selection and no partially rendered selector.
- **FR-018**: Zooming, panning, dragging, and hover feedback remain responsive with libraries spanning 100+ years and 1200+ populated months: each pointer or key interaction's visual update follows the input without perceptible lag.

## Key Entities
- **Period**: the selection unit — a calendar year, or a year with a month; the smallest period is one month.
- **TimelineSelection**: the active filter — either empty (unfiltered) or a contiguous, inclusive start-period through end-period range; a single-period selection is a range whose bounds are the same period.
- **DensityBucket**: the photo count of one month, used for the background density profile; year values aggregate their months.

## Edge Cases
- Empty library: no selector renders and no filter can be set.
- Single-year library: the selector shows one year and month selection still works.
- Multi-decade gaps between populated periods: gaps render as empty stretches with no density, and labels still never overlap.
- Century-scale span (100+ years) with sparse data.
- Every photo in one month: other periods are unselectable and the density profile shows a single peak.
- Range confined to one month: the selection remains that month, never smaller.
- A range bound landing on a month with zero photos: the bound stays valid (ranges are contiguous), while a single-period selection of such a month is not applied.
- Reversed drags and reversed restored bounds: normalized to ascending order.
- Restored selection whose data no longer exists: narrowed to the available overlap, or cleared when there is none.
- Clear activated while a drag or zoom gesture is in progress: the gesture is cancelled and the unfiltered full-span state remains.
- Input at the zoom limits: further zoom input has no effect and never corrupts the view or the selection.
- Window resized while zoomed and selected: labels re-adapt and the selection stays visible.
- Timeline data fails to load: the existing error path runs; a retry restores the selector.

## Research Notes
- https://d3js.org/d3-axis — tick values derive from a suggested count or a time interval, so label density follows the available pixel budget; grounds FR-005's adaptive granularity and non-overlap guarantee.
- https://d3js.org/d3-brush — defines drag-select, edge-handle adjustment, selection translation, and programmatic selection with a minimum-width constraint; grounds FR-008 and FR-009.
- https://d3js.org/d3-zoom — establishes wheel/drag pan-zoom bounded by scale and translate extents, and the combination of zoom with brushing (focus + context); grounds FR-003 and the single-view overview model.
- Repo verification (2026-09-19, rebuilt binary against a 37-year, 323-photo dataset): the shipped year rail renders 37 year targets with zero label overlap, but needs horizontal scrolling (2939px of content in a 1374px rail) and can leave the active year outside the visible rail after a URL restore — the failure modes FR-001 and FR-010 exist to remove.
- Web search engines were unavailable during research (bot walls/CAPTCHA) and https://uxpatterns.dev/patterns/forms/date-range returned HTTP 403; only the three sources above were read, and the remaining grounding is repo evidence.

## Assumptions
- Month is the finest filter granularity; day-level filtering is out of scope.
- Density data continues to come from the existing month-bucket timeline endpoint; the density profile's resolution is one month.
- The range-capable selection model supersedes today's year/month filter parameters; URL parameter naming and encoding are implementation choices, and breaking URL changes are acceptable.
- The desktop year-pill rail is replaced; its rail-scrolling interaction model is not preserved.
- Photo counts remain available for periods and for the active selection.
- "Desktop" means the existing desktop-only breakpoint; mobile dropdowns, sorting, search, and other views are untouched except for consuming the new selection.
- Single-period selection of empty periods stays disallowed, matching the current month behavior.

## Success Criteria
- **SC-001**: With a library spanning at least 60 years and 240+ populated months, at every supported desktop width, zoom level, and pan position: zero pairs of visible labels overlap and no label is clipped (verified by measuring label bounding boxes).
- **SC-002**: From the full-span view, a year is selectable in one activation; any month is selectable in at most three discrete interactions (e.g. zoom toward the year, then activate the month).
- **SC-003**: A month-granular range between any two bounds is set with a single drag, and either bound can be changed by dragging its handle; the grid contents and the label match the inclusive range exactly.
- **SC-004**: Reload, Back, and Forward restore an active selection and its grid filter with zero failures across the covered scenarios.
- **SC-005**: A keyboard-only user completes year selection, month selection, range selection, both-bound adjustment, zoom, fit-all, and clear, with each target announcing period name and count.
- **SC-006**: A selection is visible inside the selector viewport immediately after being set or restored — never hidden off-screen.
- **SC-007**: Clear returns to unfiltered plus full-span view in one action from any state (zoomed, selected, mid-gesture).
- **SC-008**: With 100+ years and 1200+ populated months, each pointer/key interaction's visual update follows the input within 100 ms on the reference desktop, with no dropped interaction.
