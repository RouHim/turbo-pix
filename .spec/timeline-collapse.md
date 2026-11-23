# Feature Specification: Foldable Timeline Card

**Created**: 2026-11-23
**Status**: Approved
**Input**: The desktop timeline card always occupies about 210px between the view title and the photo grid. Users browsing a large library want to fold it away and reclaim that vertical space, without losing the timeline on the next visit.

## Goal
The desktop timeline card folds down to its header row with one click and expands again, defaulting to open on a first visit. The fold is a pure presentation state: it never changes the timeline filter, the URL, or the grid contents. Folded, the card keeps the filter label and the clear control, so an active filter stays readable and removable. The selector keeps its zoom and pan while folded. Scope is the desktop timeline card only; the mobile dropdown row and the fixed date rail are untouched.

## User Scenarios

### Scenario 1 - Fold the card and reclaim the space (P1)
A desktop user with an open timeline card wants more grid rows.

**Acceptance**
1. Given the open card in a fresh session, when the page has loaded, then the selector is visible, the toggle announces "Collapse timeline" with `aria-expanded="true"`, and the card is 210px tall.
2. Given that state, when the user activates the toggle, then only the selector is hidden, the filter label and the clear control stay visible, the card is 74px tall, and the photo grid moves up by 136px.
3. Given the collapsed card, when the user activates the toggle again, then the selector returns in the same zoom/pan/selection state it had before.

### Scenario 2 - Persistence and keyboard focus (P2)
A user folds the card, reloads, and later collapses it from the keyboard.

**Acceptance**
1. Given a collapsed card, when the page is reloaded, then the card restores collapsed and the toggle announces "Expand timeline" with `aria-expanded="false"`.
2. Given a collapsed card, when the user expands it, then the stored preference records the open state.
3. Given keyboard focus inside the selector, when the fold is triggered without a pointer focus move, then focus lands on the toggle instead of the hidden selector or the document body.
4. Given focus on the toggle, when the user activates it with Enter or Space, then the fold changes exactly as with a pointer and focus stays on the toggle.

### Scenario 3 - Breakpoint isolation and accessibility (P3)
A user resizes across the mobile breakpoint, or runs an automated accessibility scan.

**Acceptance**
1. Given a collapsed desktop card, when the viewport narrows below 769px, then the toggle is hidden, the mobile year/month dropdowns remain visible and operable, and the stored preference is unchanged.
2. Given that state, when the viewport returns to desktop, then the card is still collapsed and the toggle is visible again.
3. Given the collapsed card, when the timeline container is scanned with axe (color-contrast, target-size, aria-valid-attr-value, aria-prohibited-attr), then there are no violations.

## Functional Requirements
- **FR-001**: The desktop timeline card starts expanded on a first visit; the collapsed state is stored under `turbopix_timeline_collapsed` and only the written marker `'true'` counts as collapsed.
- **FR-002**: The card exposes one native toggle button in its header, labelled by `ui.collapse_timeline` / `ui.expand_timeline`, with `aria-expanded` and `aria-controls="timeline-details"`.
- **FR-003**: Collapsing hides only the `#timeline-details` region (the selector); the filter label, the clear control and the toggle stay visible and operable.
- **FR-004**: The fold never writes the route, never changes the active date filter and never clears it.
- **FR-005**: Collapsing and expanding snap the card height; no height, margin or padding transition runs. The chevron rotates with `--transition-fast` and does not animate under `prefers-reduced-motion: reduce`.
- **FR-006**: The selector stays mounted while folded so its zoom, pan and selection survive the round trip.
- **FR-007**: A collapse triggered while focus is inside the selector moves focus to the toggle.
- **FR-008**: The folded card is 74px tall on desktop (32px header + 40px padding + 2px border); expanding restores the full 210px card.
- **FR-009**: The fold control exists on desktop widths only. The mobile dropdown row and the fixed date rail are unchanged, and a stored desktop preference never hides the mobile dropdowns.
- **FR-010**: The toggle is only rendered when the card itself renders (timeline data loaded, more than two populated years).
- **FR-011**: All labels exist in English and German with identical key structure, enforced by `npm run test:i18n`.
- **FR-012**: A denied or throwing `localStorage` leaves the in-memory fold state usable and never throws.

## Key Entities
- **Fold state**: a boolean UI preference, default expanded, persisted per browser under `turbopix_timeline_collapsed`; `'true'` means collapsed, anything else means expanded.
- **Timeline card**: `.timeline-container` in `TimelineSlider.svelte`; the header row (label, clear control, toggle) plus the collapsible `#timeline-details` region containing the desktop selector.
- **Mobile dropdowns / date rail**: separate experiences; neither reads nor writes the fold state.

## Edge Cases
- A library with two or fewer populated years renders no card, so there is no toggle; a stored collapsed preference applies whenever the card appears later.
- An initial timeline load failure keeps the existing error path; no toggle is shown without a card.
- Storage denied or full: the toggle still works for the session.
- Collapsing while a selector control has focus must not leave focus on a hidden element.
- Resizing to mobile and back must not drop the desktop preference.
- The clear control in the folded header still clears the filter and resets the selector view; the reset is deferred until the lane is visible and freshly measured, so it frames the width the lane actually has after expanding.
- No automatic folding: only the toggle changes the state.

## Assumptions
- Separating the fold from the filter means a user can keep a filter and hide its graph; the label keeps the active period visible.
- The header row is the only part left when folded, so the card remains discoverable without a floating tab or a top-bar control.
- The selector's ResizeObserver keeps the last non-zero lane width across a fold and tracks whether the lane is currently hidden, so zoom and pan survive the round trip while a folded reset knows to wait for a visible measurement.
- Mobile and the date rail are out of scope for this feature.
- Exact pixel heights (210px open, 74px folded, 136px reclaimed) are layout contracts and are asserted in E2E.

## Success Criteria
- **SC-001**: On a fresh desktop session the card is open with `aria-expanded="true"` and no stored preference; one activation collapses it to 74px, moves the grid up by 136px and stores `'true'`.
- **SC-002**: Reload restores the stored state with zero mismatches, and expanding stores `'false'`.
- **SC-003**: Across a fold/expand round trip the rendered column periods and the filter are byte-identical.
- **SC-004**: Collapsing with focus inside the selector leaves focus on the toggle in every covered case.
- **SC-005**: Crossing the mobile breakpoint never changes the stored preference and never hides the mobile dropdowns; the collapsed-state axe scan reports zero violations.
