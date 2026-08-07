# Feature Specification: Saved Searches (Smart Albums)

**Created**: 2026-08-06
**Status**: Approved
**Input**: Smart albums / saved searches — save the current URL-addressable view (/?q=... + filters) as a clickable sidebar entry; one table, click restores the URL.

## Understood User Request
Let me save the current search/filter view as a named, clickable entry in the sidebar that restores that view any time, so recurring looks ("Beach 2023", "Favorites from the Fuji", "raw files to edit") are one click away.

## Motivation
Views are already URL-addressable (`/?q=...` plus filters), so a saved search is nearly free persistence on top of existing behavior — one table, a sidebar section, click to restore. It is the missing organizational layer: every recurring query becomes a named, always-current collection instead of a typed-out URL. Future features (ratings, tags, faces) compose into the same mechanism, keeping the app's daily value high at small cost.

## Summary
A "Saved searches" section in the sidebar plus a save control in the search area. Saving captures the current view state (query, view filter, sort, year/month) as a named entry; clicking an entry navigates to that state and re-runs the query live, so results always reflect the current library contents. Entries can be renamed and deleted. Exact duplicate saves are blocked with a notice.

Out of scope: rule-based collections independent of the current search UI (free-form "smart album" rules), folders or manual ordering, sharing/export, and ratings, tags, or faces — those compose into this mechanism later, they are not built here.

## User Scenarios

### Scenario 1 - Save and restore a recurring search (P1)
Rouven searches `beach`, filters to 2023, and saves the view as "Beach 2023". It appears in the sidebar immediately.

**Acceptance**
1. Given a non-default view state (query and/or filters), When the user saves it from the search area, Then a named entry appears in the sidebar immediately and the view state is captured.
2. Given a saved search in the sidebar, When the user clicks it, Then the exact query, filters, sort, and year/month of the saved state are restored and current results are shown.
3. Given a saved search clicked after new matching photos were added to the library, When results render, Then the new photos are included (the query re-runs live against the current library).

### Scenario 2 - Rename and delete (P2)
The auto-generated name of a saved search is wrong; the user renames it. Later, the search is no longer needed and is deleted.

**Acceptance**
1. Given a saved search, When the user renames it, Then the new name is shown in the sidebar immediately.
2. Given a rename attempt to an empty name, When the user confirms, Then the rename is rejected and the previous name is kept.
3. Given a saved search, When the user deletes it, Then the entry disappears from the sidebar and the current view is unaffected.

### Scenario 3 - Duplicate save is blocked (P2)
The user saves the same view twice by accident.

**Acceptance**
1. Given a view state identical to an existing saved search, When the user saves it, Then no new entry is created and a friendly notice explains the view is already saved.

## Functional Requirements
- **FR-001**: From any searchable view state that is not the fully-default state, the user can save the current view with the save control in the search area.
- **FR-002**: A saved search captures the query, view filter, sort order, and year/month of the current view state at the moment of saving.
- **FR-003**: Clicking a saved search navigates to its captured view state; results are computed from the current library contents at click time, not from a stored result list.
- **FR-004**: The sidebar lists saved searches newest-first under a "Saved searches" section, each showing its name and an active-state indication when the current view matches it.
- **FR-005**: The user can rename any saved search; empty names are rejected.
- **FR-006**: The user can delete any saved search; deletion is immediate and does not change the currently displayed view.
- **FR-007**: Saving a view state identical to an existing saved search creates no new entry and surfaces a notice to the user.
- **FR-008**: Saved searches persist across browser sessions and application restarts.
- **FR-009**: All new user-visible text is provided in both English and German, consistent with the existing translation system.

## Key Entities
- **SavedSearch**: a named, ordered view state (query, view filter, sort, year/month) plus its creation time. Identity is defined by the captured view state (exact duplicates are one entry); multiple entries may share a name.

## Edge Cases
- Saving a fully-default view (no query, no filters) is not offered.
- Two saved searches may have the same name — names are labels, the view state is the identity.
- A saved search whose query matches nothing shows the normal empty-results state; the entry itself remains valid and clickable.
- Long names are truncated in the sidebar with the full name available on hover.
- Deleting a saved search is permanent; recreating it is a normal save.
- The active-state indication must track the current view across navigation, including via browser back/forward.

## Research Notes
- No external research was required: the behavior is standard photo-app UX, and the enabling premise (URL-addressable views, sidebar, en/de i18n parity) was confirmed present in the current codebase.

## Assumptions
- Saved searches are persisted server-side (the brief's "one table"), shared across browsers and sessions, surviving app restarts.
- The save control lives in the search area and is available whenever the current view state is not fully default.
- Deleting a saved search requires no confirmation (entries are trivial to recreate).
- No manual ordering, folders, or grouping in v1; order is newest-first.
- Auto-named from the query at save time (e.g. "Beach 2023"), always renameable afterwards.
- Exact duplicates are defined as identical captured view state.
- New UI strings follow the project's existing English/German translation parity convention.
- Out of scope: rule-based collections independent of the search UI, folders/ordering, sharing, ratings, tags, and faces.

## Acceptance Criteria
- **SC-001**: From any searchable non-default view state, a user can save it with at most two interactions, and the entry appears in the sidebar immediately.
- **SC-002**: Clicking a saved search restores the exact query, filters, sort, and year/month it was saved with, and shows results including photos added to the library after the save.
- **SC-003**: Saving a view state identical to an existing saved search adds no new entry and shows a notice.
- **SC-004**: Every saved search can be renamed and deleted from the sidebar without leaving the currently displayed view.
- **SC-005**: Saved searches persist across browser sessions and application restarts.
- **SC-006**: All new user-visible strings exist in both English and German, with no parity drift.
