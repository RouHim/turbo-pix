# Feature Specification: Timeline Selector Period Selection, Click Drill-Down and Month-Level Zoom

**Created**: 2026-09-25
**Status**: Approved
**Input**: In the new desktop timeline selector no concrete year can be selected (only time ranges), a click on a year zooms out instead of selecting and drilling into it, and zooming further down to month level should become possible.

## Goal
Desktop users of the timeline selector cannot pick a concrete period: activating a year-shaped target does not reliably filter to that year and does not reliably drill into it, and there is no direct way to reach month granularity. This spec makes one activation on any rendered period — decade, year, or month — select exactly that period and zoom one level into it, and adds explicit Decade / Year / Month view controls so month level is reachable in a single action. Scope is the desktop selector only; the existing range drag, keyboard operation, URL round-trip and the mobile dropdowns stay unchanged.

## User Scenarios

### Scenario 1 - Click a decade, a year, then a month (P1)
A desktop user with a library spanning 1962–2019 opens the app with no filter and works from the full-span view down to one month.

**Acceptance**
1. Given a cleared filter, a library spanning 50+ years and a supported desktop width, when the user activates a decade column, then the grid filters to exactly that grid-aligned decade (1960-01 … 1969-12), the selection label names the decade, and the lane renders that decade's year columns.
2. Given that state, when the user activates a year column, then the grid filters to exactly that calendar year (January through December) and the lane renders that year's twelve month columns.
3. Given that state, when the user activates a month column, then the grid filters to exactly that month.
4. Given each of those activations, when it completes, then the view shows the activated period at least one granularity level finer than the activated period — never at a coarser one.

### Scenario 2 - Reach month level in one action (P1)
A user who already filtered to a year wants month columns immediately, and a user with a wide filter wants a shortcut into month granularity.

**Acceptance**
1. Given an active filter that fits the month-level window (for example a single year), when the user activates the Month control, then the lane renders that filter's months and the filter value is unchanged.
2. Given an active filter wider than the month-level window (for example a decade) or no filter at all, when the user activates the Month control, then the lane renders a month-column window around the current view center and the filter value is unchanged.
3. Given the view already renders the activated level's columns, when the user activates that same granularity control, then the view and the filter are unchanged.
4. Given any granularity control activation, when it completes, then the grid contents are identical to before the activation.

### Scenario 3 - Keyboard operation, restore and repeat activation (P2)
A keyboard-only user performs the same selections, and any user reloads or navigates Back/Forward.

**Acceptance**
1. Given keyboard focus only, when the user moves focus onto a decade, year or month column and activates it, then the filter and the view change exactly as with a pointer, focus stays visible, and the column announces its period name and photo count.
2. Given keyboard focus only, when the user activates each granularity control, then the view changes as with a pointer and the active level is announced.
3. Given an active filter, when the user reloads the page or navigates Back/Forward, then the grid filter is identical and the view shows the selected period's columns.
4. Given the period that is currently the filter, when the user activates it again, then the filter value is unchanged and the view does not become coarser.

### Scenario 4 - Empty periods and gesture safety (P3)
A user clicks a period without photos, or a press turns into a drag.

**Acceptance**
1. Given a period with zero photos, when the user attempts to activate it, then neither the filter nor the view changes, and the period's emptiness is announced before activation.
2. Given a press inside the lane that becomes a drag, brush, or handle gesture, when the gesture ends, then the underlying column is not additionally activated and only the drag gesture's own result stands.

## Functional Requirements
- **FR-001**: Activating a rendered period column applies exactly that period as the filter: a month column selects that month, a year column selects that whole calendar year (January through December), and a decade column selects that grid-aligned ten-year period (for example 1960-01 … 1969-12), independent of where the library's data begins or ends.
- **FR-002**: The same activation zooms the view one granularity level into the activated period — a decade shows its year columns, a year shows its month columns — so the activated period's own columns fill the lane; at month granularity one month per lane width is the finest level.
- **FR-003**: No period activation ever coarsens the view: after the activation the view is at least one level finer than the activated period and shows the activated period.
- **FR-004**: A period with zero photos is not activatable: activation applies no filter and no view change; the period stays focusable and announces its name and that it has no photos.
- **FR-005**: A pointer press that becomes a drag, brush, handle, or pinch gesture never also activates the column beneath it.
- **FR-006**: The selector offers Decade, Year and Month granularity controls that change only the view; exactly one level is in effect at a time and the controls show which one it is.
- **FR-007**: Each granularity control frames the active filter when that filter fits the level's maximum on-screen window, and otherwise frames a window around the current view center; it never changes the filter value.
- **FR-008**: From the full-span view of a library spanning 50+ years, any populated calendar year is reachable in at most two activations and any populated month in at most three, with the existing range drag still available for wider selections.
- **FR-009**: The deepest zoom level renders one month per lane width; further zoom input has no effect and never corrupts the view or the selection.
- **FR-010**: Re-activating the currently filtered period leaves the filter unchanged and never coarsens the view; clearing the filter stays on the existing clear control.
- **FR-011**: The clear control resets the filter to unfiltered and the view to the full span in one action from any state.
- **FR-012**: Every period column and every granularity control is operable by keyboard with visible focus and announces its period name, photo count and level, and period activations behave identically for pointer and keyboard.
- **FR-013**: Every filter change round-trips through the URL; reload, Back and Forward restore the same filter and re-frame the view on the selected period, and no automatic re-framing may revert the view an activation or a granularity control just produced.
- **FR-014**: All new labels and announcements exist in English and German with identical key structure.
- **FR-015**: The selector renders at desktop widths only; below the existing breakpoint the mobile year and month dropdowns stay unchanged, with exactly one experience active.
- **FR-016**: Range selection by dragging, edge handles, selection translation, pinch, pan, wheel zoom, Escape abort and fit-all remain available and behave as before; the new activation rule never consumes a drag gesture and no drag gesture ever changes the filter on activation.
- **FR-017**: An empty library renders no period targets and no filter can be set; a timeline data load failure surfaces the existing error path with no stale selection.

## Key Entities
- **Period**: the activation unit of a column and of a filter — a calendar decade, a calendar year, or a month; a decade and a year are represented as an inclusive range of months.
- **Granularity level**: the column unit the lane currently renders — month, year or decade — derived from the view scale and the library's span; the finest level is one month per lane width.
- **TimelineSelection**: the active filter, either empty or an inclusive month range; identical to the existing route filter, so a decade, a year and a month are all expressible in the current filter shape.

## Edge Cases
- Grid-aligned decade and year filters may contain months with no photos at the edges of the library (a library starting in March 1962 with the 1960s selected); this stays legal because a range is allowed to contain empty months.
- A coarser granularity than the full-span view cannot be reached, because the full-span view is the outermost zoom; activating such a control leaves the view and filter unchanged.
- A level whose columns number fewer than two (a library inside one decade) still renders and remains operable.
- A zero-photo year inside a populated decade: the decade is activatable, that year is not.
- Re-activating the active period must not be mistaken for clearing it (no toggle).
- A press that starts on the active selection's body or a handle must not activate the column beneath it.
- An in-flight live scrub or drag followed immediately by an activation must not commit a mixed filter.
- A restored filter whose period no longer exists in the library is narrowed or cleared by the existing rule, and the granularity controls must not resurrect it.
- An empty library renders no targets; a timeline data load failure leaves no partially rendered selector.
- Window resize while a granularity level is active re-derives the columns without changing the filter.

## Research Notes
- https://zoomcharts.com/en/microsoft-power-bi-custom-visuals/documentation/drill-down-timeline-pro/interactivity/ — left-clicking a data point drills down to the next date/time unit while a separate toolbar button changes the display unit; grounds the split between click-drill and the Decade/Year/Month granularity controls.
- https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/semantic-zoom — tapping a group header switches between detail and group views while the content scope stays the same; grounds allowing one activation to filter and zoom without changing what the grid shows for other periods.
- Repo evidence (commit 8522185, 2026-09-20, first released in 2.42.0): the drill-in plus grid-aligned year commit and the `reframeSuppressed` guard were added there; before it, the selection-following effect reverted a column's drill-in (the reported zoom-out) and a clipped multi-month range was committed for a year (the reported "only ranges").
- `frontend/src/lib/timelineLayout.js` — `MIN_COLUMN_PX = 28` with `chooseUnit` sets the granularity thresholds (month columns while the span is at most `width / 28` months, year columns while at most `3 * width / 7` months, decades beyond that) and `maxScale = width` sets one month per lane width as the deepest zoom; these constants make the reachability guarantees of FR-008 measurable.
- No external pattern was found for a photo-library decade/year/month drill on a fixed 28px pointer-target floor; the reachability arithmetic above is grounded in the repository constants.

## Assumptions
- A period activation selects that period and zooms into it; a decade selection is the whole grid-aligned decade; a second activation of the active period neither toggles nor zooms out; the feature offers both the click-drill path and explicit granularity controls.
- The granularity controls live in the selector's existing control row next to zoom in, zoom out and fit-all, and are always enabled; a level that cannot be rendered at the current span is a no-op that changes neither the view nor the filter.
- A granularity control frames the active filter when it fits that level's maximum on-screen window, otherwise a window around the current view center, and it never changes the filter value.
- Activating a period replaces the filter with that period (progressive narrowing); it never extends or merges with the previous filter.
- A decade filter and a year filter are expressed through the existing route filter shape; no new filter semantics are introduced.
- Counts continue to come from the existing per-month timeline density payload.
- Month is the finest granularity; day and week granularities are out of scope.
- Mobile dropdowns, sorting, search, the map view and every other filter consumer keep working against the same filter.
- Breaking URL or filter-encoding changes are acceptable, as the application has no production users.

## Success Criteria
- **SC-001**: From a cleared filter at the full-span view of a library spanning at least 50 years, a user reaches the filter for any populated calendar year in at most two pointer activations and any populated month in at most three, with no drag operation.
- **SC-002**: Across the covered scenarios, every period activation leaves the view at least one granularity level finer than the activated period, and no activation produces a coarser view than the one before it (verified by comparing the rendered column granularity before and after each activation).
- **SC-003**: For an active filter that fits the month window, the Month control renders that filter's months in one activation, and the filter value is byte-identical before and after; for a wider or absent filter the same holds with a month-column window around the view center.
- **SC-004**: A keyboard-only user completes decade, year and month activations, all three granularity controls, clear, and range adjustment, each producing the same filter and view result as the pointer path and each announcing period name and photo count.
- **SC-005**: Reload, Back and Forward restore the active filter together with a view showing the selected period's columns, with zero failures across the covered scenarios.
- **SC-006**: Attempting to activate a period with zero photos leaves the filter and the view unchanged in every covered case.
- **SC-007**: With 100+ years and 1200+ populated months, each activation's or granularity control's visual update follows the input within 100 ms on the reference desktop, with no dropped interaction.
