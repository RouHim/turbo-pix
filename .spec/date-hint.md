# Feature Specification: Current-Date Hint

**Created**: 2026-11-24
**Status**: Approved
**Input**: "Wie könnten wir beim Scrollen mobil und Desktop das aktuelle Datum anzeigen? Nur Hinweis, sobald der Header minimiert wird." Validated in the throwaway `prototype/date-hint` branch (variants A/B/C); variant A, the glass pill, was chosen.

## Goal

While the view header (title, Select, Sort) has scrolled under the fixed glass bar, a quiet glass pill names the date of the topmost visible photo. It is orientation, not chrome: non-interactive, hidden from assistive tech, and gone as soon as the view header returns.

## User Scenarios

### Scenario 1 - Orientation while scrolling (P1)

A user scrolls through All Photos on desktop or mobile.

**Acceptance**

1. Given the view header is visible at the top, then the hint is hidden. Given the header's bottom crosses the 64px bar, then the pill fades in 8px below the bar. The 8px hysteresis keeps it from flickering at the threshold; scrolling back until the header is visible again hides it with the 0.3s exit.
2. Given the hint is visible, then its text is the authored localized date (the same abbreviated-month shape as the grid caption) of the first card that starts below the header line and intersects the viewport. Scrolling to another day updates the text; multiple photos on the same day keep the same label.
3. Given the topmost photo has no usable `taken_at`, then the hint reads "No date" / "Ohne Datum".
4. Given the hint is visible, then its fixed layer sits 8px under the glass bar; its left edge follows the sidebar width, the collapsed sidebar and the mobile content edge, and its right edge honours the date rail's inset. It adds no layout and does not move with the grid's scroll.

### Scenario 2 - Scope (P1)

**Acceptance**

1. Given a grid view sorted by `date_desc` or `date_asc`, then the hint is available on All Photos, Favorites, Videos, album detail, and search results. Semantic search is excluded: its ranking is not chronological even when the route still carries a date sort.
2. Given a name/size sort, semantic search, a map, the albums index, collages, housekeeping, or an empty/loading grid, then the hint is hidden.
3. Given a date filter or a changed query, then the hint re-measures against the new listing and never shows the previous view's date.

### Scenario 3 - Collisions with existing chrome (P1)

**Acceptance**

1. Given the hint is visible and the date rail is present, then the rail's month caption is paused (hidden, not focusable) until the hint's 0.3s exit completes; it returns on rail hover or after the hint hides.
2. Given the rail bubble is visible (hover, drag, keyboard focus, pending jump, or failure), then the hint hides. It returns when the bubble closes.
3. Given the top earlier-loading dot-wave is showing, then the hint hides for that transient state.
4. Given the mobile drawer or the mobile search panel is open, then the hint hides; it returns when they close.
5. Given any of these states, then the hint's zero-height layer adds no layout and never covers the header, the rail, or the header controls.

### Scenario 4 - Material, motion and accessibility (P2)

**Acceptance**

1. Given the hint is visible, then it uses the glass material (`--glass-bg`, `blur(16px) saturate(1.5)`, glass hairline, `--shadow-medium`, full radius, 13px DM Sans with tabular numerals).
2. Given `prefers-reduced-transparency` or no `backdrop-filter`, then it falls back to the solid `--surface-color`.
3. Given `prefers-reduced-motion`, then the state change is instant; otherwise 0.15s in and 0.3s out on opacity and a 4px translate.
4. Given any state, then the hint is `aria-hidden="true"`, `pointer-events: none`, and has no focus target or live announcements.

## Functional Requirements

- **FR-001**: The hint is a `position: fixed` layer inside the app shell, 8px below `--header-height`; its left follows `--sidebar-width`, the collapsed sidebar and the mobile edge, and its right follows `--date-rail-inset` while the rail shows. It is never a sticky child of `.main-content` (the container's top padding would offset it).
- **FR-002**: Visibility uses the content header's bottom against the header line with an 8px hysteresis; the shared `firstVisibleCardIndex` helper bounds the card by the viewport.
- **FR-003**: The label is the top visible card's `taken_at` in the active locale's authored date shape (abbreviated month), with a localized no-date fallback.
- **FR-004**: The hint renders only for date sorts and only when `#photo-grid` has at least one card.
- **FR-005**: The hint shares `appState.dateHintVisible` with the rail (caption pause) and reads `dateRailBubbleVisible` and `dateRailLoading`; the mobile drawer and search states also pause it.
- **FR-006**: Route, date-filter, and listing changes re-measure; a route change hides the old label immediately.
- **FR-007**: The hint is non-interactive, `aria-hidden`, and never writes the route.
- **FR-008**: EN and DE strings stay structurally identical; the no-date key exists in both dictionaries.

## Edge Cases

- Undated photos sort at the end; the hint reads "Ohne Datum" there.
- The first card may start below the viewport while the timeline card is still visible: no card is visible, so the hint stays hidden until one is.
- A prepended page re-measures without hiding; a route change hides immediately.
- The rail exists only on desktop All Photos with enough history; elsewhere the caption-pause handshake is a no-op and the hint still works.
- A malformed timestamp is treated as no date.
- The viewer, selection bar, toasts and dialogs are above the hint and unaffected.

## Success Criteria

- **SC-001**: At every scroll position the hint either shows the correct top visible photo's date or hides; it never shows a date for an off-screen card.
- **SC-002**: No duplicate date labels: the rail caption and hint are never both visible.
- **SC-003**: The hint never changes the grid's layout or scroll geometry.
- **SC-004**: Unit tests cover the hysteresis, date sort gate, localized label, and visible-card bounds; the E2E spec covers desktop trigger, rail handshake, mobile, drawer/search pause, and non-date scope.
- **SC-005**: EN/DE parity and the frontend lint/format gates stay green.
