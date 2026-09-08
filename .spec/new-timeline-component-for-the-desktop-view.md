# Feature Specification: Desktop Timeline Redesign (Year Rail + Month Strip)

**Created**: 2026-09-08
**Status**: Approved
**Input**: Redesign the desktop timeline from scratch for fast year+month navigation, fixing label overlap with many years and an over-precise slider.

## Goal
Desktop users with decades-spanning photo libraries cannot reliably pick a year or month today: year labels overlap at scale and the per-month slider thumb covers many months per pixel. This spec replaces the desktop timeline with a two-level year-rail plus month-strip navigator that makes any year and month reachable by coarse mouse clicks. Scope is desktop only; mobile year/month dropdowns, the timeline density data source, and route.year/month filter semantics stay unchanged.

## User Scenarios
### Scenario 1 - Jump to a year and month (P1)
A desktop user with a large library wants to see photos from a specific month, e.g. March 1998, without dragging a slider precisely.

**Acceptance**
1. Given a cleared timeline filter, when the user activates a year and then a month, then the photo grid filters to exactly that year and month and the selection is visible as the active state.
2. Given an active year+month filter, when the user activates the clear control, then the grid returns to unfiltered and the active state clears.

### Scenario 2 - Browse 50+ sparse years without overlap (P1)
A desktop user with 50+ distinct years including large gaps wants to scan available years and pick one without overlapping or clipped labels.

**Acceptance**
1. Given a library spanning 50+ sparse years, when the timeline renders at desktop width, then every visible year label is fully legible with no pairwise overlap.
2. Given years with no photos adjacent to years with photos, when the user scans the year rail, then years without photos are visually de-emphasized and never selectable as a filter.

### Scenario 3 - Keyboard navigation and URL restore (P2)
A keyboard-only user wants the same year+month filtering, and any user expects Back/Forward to restore the timeline selection.

**Acceptance**
1. Given keyboard focus only, when the user tabs through the timeline, then every year, month, and the clear control is reachable and activatable with visible focus and an announced label naming the period.
2. Given an active year or year+month filter written to the URL, when the user navigates Back/Forward, then the timeline active state and the photo grid match the restored URL filter.

## Functional Requirements
- **FR-001**: The desktop timeline presents years as discrete selectable targets in a year rail; each target is activatable with a single coarse pointer action, never requiring sub-target drag precision.
- **FR-002**: The year rail renders 50+ sparse years with zero overlapping visible labels, by grouping, collapsing, scrolling, or abbreviating, while keeping every year with photos discoverable and selectable.
- **FR-003**: Years without photos are visually de-emphasized and are not selectable as filters; gaps in coverage are apparent without suggesting selectable content.
- **FR-004**: Activating a year reveals a month strip for that year showing all twelve months with per-month photo counts; months without photos are dimmed and indicate emptiness before activation.
- **FR-005**: Each year target shows its total photo count; each month target shows its photo count.
- **FR-006**: Activating a year alone filters to that year; activating a month within the selected year filters to that year+month; activating the selected month or year again, or the clear control, clears toward unfiltered.
- **FR-007**: Every filter change updates the photo grid and synchronizes the single source of truth filter state (year with optional month) including the URL, and restores correctly on Back/Forward navigation.
- **FR-008**: A month can never be active without its year; clearing the year clears the month.
- **FR-009**: The full flow is keyboard operable with visible focus indicators and accessible names announcing the period and count for each target and the purpose of the clear control.
- **FR-010**: All visible labels, announcements, and empty states are available in English and German with identical key structure.
- **FR-011**: The redesign applies at desktop widths only; the existing mobile year/month selection experience is preserved unchanged.
- **FR-012**: Timeline data load failure surfaces the existing error path instead of a broken or empty navigator, and an empty library renders no filter targets.

## Key Entities
- **YearAggregate**: A year with photos, its total photo count, and which of its twelve months contain photos.
- **MonthBucket**: A year+month combination with its photo count, including zero-count months used for dimmed display.
- **TimelineFilter**: The active selection, either empty, a year alone, or a year plus a month; a month is never present without its year.

## Edge Cases
- Empty library: no year or month targets render and no filter can be set.
- Single-year library: the navigator is usable without rail scrolling or grouping.
- Century-scale spans: label non-overlap holds as the distinct-year count grows into the hundreds.
- All photos in one month: the year shows its total, eleven months render dimmed, and selecting an empty month is not possible.
- Timeline data fails to load: the error path triggers and no stale selection persists.
- Month selected then year cleared: the month clears with it and the grid returns to unfiltered.
- Rapid successive year activations: the month strip always reflects the most recently selected year.
- Narrow desktop window at the desktop/mobile boundary: exactly one of the desktop or mobile experiences is active, never both or neither.

## Research Notes
- frontend/src/components/TimelineSlider.svelte — per-month-bucket range input maps hundreds of buckets onto one track so the thumb covers many months per pixel, and the year-tick rule still overlaps past a handful of years.
- frontend/src/components/TimelineSlider.svelte mobile dropdowns — year+month selects already validate the two-level select model reused by this spec.
- frontend/src/lib/router.svelte.js — route.year plus optional route.month is the single source of truth the new navigator must read and write.
- https://uxpatterns.dev/patterns/forms/date-range — generic date-range input guidance only, with no reusable pattern for a 50-year photo timeline, so scale handling is grounded in local failure modes.
- https://www.eleken.co/blog-posts/calendar-ui — calendar survey confirms flexible view-switching over single-precision controls for large spans, supporting the rail-plus-strip split.

## Assumptions
- Year and month totals reuse the existing timeline density payload shape (per year+month counts) with year totals aggregated from it.
- Desktop scope equals the existing desktop-only versus mobile-only breakpoint split.
- Single-period selection only; ranges and day-level selection are out of scope.
- Counts render as numerals next to labels; proportional histogram bars are out of scope.
- Clear-filter and Back/Forward restore semantics match current behavior.

## Success Criteria
- **SC-001**: With 60 sparse distinct years at desktop width, zero pairs of visible year labels overlap.
- **SC-002**: A mouse user reaches any year+month filter from a cleared state in at most two coarse activations with no drag operation.
- **SC-003**: A keyboard-only user reaches and activates any year, any month in the selected year, and the clear control with announced period names.
- **SC-004**: Every filter change is reflected in the grid and the URL, and Back/Forward restores both the grid and the navigator active state.
- **SC-005**: Users identify months without photos without activating them, and cannot set a filter to an empty period.
