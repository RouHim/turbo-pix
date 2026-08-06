# Feature Specification: Batch Select + Actions

**Created**: 2026-08-06
**Status**: Approved
**Input**: Selection mode (multi-select → delete, favorite, date-shift, export) as the UX primitive completing the one-by-one review workflows in housekeeping, collages, and semantic results.

## Understood User Request

Users want to review and cull photos in bulk: select multiple photos at once across all library surfaces and apply shared actions (delete, favorite/unfavorite, date-shift, export, plus surface-specific keep and accept/reject) in a single step instead of one photo at a time.

## Motivation

Housekeeping, collages, and semantic search results are all reviewed one photo at a time today. That per-item loop is the bottleneck of the culling workflow those features were built to support — dismissing dozens of housekeeping candidates, choosing among pending collages, or pruning search results means dozens of clicks with no way to act on a set. A selection mode with batch actions is the shared UX primitive that completes those workflows, and every future bulk feature (albums, tagging, sharing) builds on the same mechanism.

## Summary

A per-surface selection mode is added to every photo surface — All, Favorites, Videos, semantic search results, housekeeping candidates, and pending collages. Users enter selection mode, tap or keyboard-range-select multiple photos, and apply batch actions from an action bar: permanent delete (with confirmation), explicit add/remove favorite, date-shift by ±N days on the taken date, and export of the original files as a single downloadable archive. Housekeeping additionally offers batch "keep" for candidates; the collages surface offers batch accept/reject of pending collage cards only, because collage cards are duplicate groups rather than single photos.

Out of scope: undo or trash for deletions (deletion stays permanent, matching today's single-photo delete), selection that spans multiple surfaces, sharing, select-all across the entire library (only "select all visible" is included), and photo-level actions on the collages surface.

## User Scenarios

### Scenario 1 - Culling housekeeping candidates (P1)

A user has 60 housekeeping candidates from a recent import. They enter selection mode, tap 42 of them, and delete them in one action; the remaining 18 get dismissed with batch keep.

**Acceptance**
1. Given 42 candidates selected, when the user confirms batch delete, then all 42 photos are permanently deleted and no longer appear in the candidate list or any other view.
2. Given 10 candidates selected, when the user triggers batch keep, then all 10 candidates are dismissed in one action and the photos remain in the library.

### Scenario 2 - Fixing a camera clock offset (P1)

A user's travel camera was set one day ahead. In the All view they select the 200 trip photos and shift their taken dates by -1 day.

**Acceptance**
1. Given 200 photos with a taken date selected, when the user applies a -1 day shift, then every selected photo's taken date moves back exactly one day and the grid reflects the new order.
2. Given a selection that includes photos without a taken date, when the user applies a date shift, then those photos are skipped and the result message states how many were skipped.

### Scenario 3 - Mass-favoriting semantic results (P2)

A semantic search for "beach sunsets" returns 15 relevant photos; the user selects all visible results and adds them to favorites in one action.

**Acceptance**
1. Given 15 semantic search results selected, when the user triggers "Add to favorites", then all 15 photos are favorited and appear in the Favorites view.

### Scenario 4 - Exporting a curated set (P1)

A user selects 50 photos (JPG, RAW, and one video) from the All view and exports them as a single archive to share with a friend.

**Acceptance**
1. Given 50 photos of mixed formats selected, when the user triggers export, then one downloadable archive containing all 50 original files is produced and opens on a standard desktop OS.
2. Given two selected photos share the same file name, when the user exports, then both files are present in the archive with distinct names.

### Scenario 5 - Batch-accepting collages (P2)

A user has 8 pending collages and wants to keep the best photo of each group; they select all 8 cards and accept them in one action.

**Acceptance**
1. Given 8 pending collage cards selected, when the user triggers batch accept, then all 8 collages are accepted in one action and disappear from the pending list.
2. Given selected collage cards, when the user triggers batch reject, then a confirmation is shown before any card is rejected.

## Functional Requirements

- **FR-001**: Every photo surface (All, Favorites, Videos, semantic search results, housekeeping candidates, pending collages) offers an explicit way to enter and exit selection mode; exiting clears the selection.
- **FR-002**: While in selection mode, activating a card toggles its selection; selected cards show a clear visual state; the selection persists across pagination within the same surface.
- **FR-003**: Selection can be made with pointer, with touch (long-pressing a card enters selection mode and selects that card), and with keyboard (Shift+click extends a contiguous range; Esc exits selection mode).
- **FR-004**: While in selection mode, an action bar shows the selected count, offers "select all visible", and presents the actions available for the current surface; every action is disabled while nothing is selected.
- **FR-005**: Batch delete permanently deletes every selected photo after a single confirmation stating the count; each deleted photo disappears from every view, consistent with the existing single-photo delete behavior.
- **FR-006**: Batch favorite offers explicit "Add to favorites" and "Remove from favorites" actions; the result is reflected in the Favorites view and the photo viewer.
- **FR-007**: Batch date-shift lets the user shift the taken date of every selected photo by a specified number of days, forward or backward; photos without a taken date are skipped and the skip count is reported.
- **FR-008**: Batch export produces a single downloadable archive containing the original files of every selected photo, all supported formats included; files sharing a name are disambiguated; the user receives progress feedback and a result summary.
- **FR-009**: In housekeeping, batch "keep" dismisses every selected candidate in one action while keeping the photos in the library.
- **FR-010**: In collages, selection supports batch accept and batch reject (with confirmation) of pending collage cards only; photo-level actions are not offered on this surface.
- **FR-011**: Every batch action reports an outcome summary (applied / skipped / failed counts); when some items fail, the successful items remain applied and the failures are identified.
- **FR-012**: Batch actions on large selections (hundreds of items) keep the interface responsive and show progress while running.
- **FR-013**: Selection never leaks across surfaces: navigating to a different surface clears it.
- **FR-014**: "Select all visible" selects every currently loaded photo in the surface; selecting beyond the loaded set (entire library) is out of scope.

## Key Entities

- **Selection**: the per-surface set of selected photos or collage cards; cleared on mode exit or surface change, survives pagination.
- **Batch Action**: delete / add-to-favorites / remove-from-favorites / date-shift / export / keep / accept / reject, initiated once for the whole selection, each producing a result summary.
- **Photo**: the existing library entity targeted by all photo-level batch actions.
- **Pending Collage**: the existing duplicate-group entity targeted by accept/reject batch actions.

## Edge Cases

- Empty selection: every action is disabled.
- Selected items disappear mid-session (background rescan removes a housekeeping candidate, another surface deletes a photo): the selection prunes itself and the count updates.
- The selection includes the photo currently open in the viewer: grid and viewer synchronization follows the existing single-delete contract.
- Date-shift selection includes photos without a taken date: they are skipped and counted, never silently dropped and never given an invented date.
- Export selection contains duplicate file names: every file is present once with a distinct name.
- Export is interrupted or partially fails: the user is told what succeeded and what failed.
- A batch accept/reject in collages where some cards fail: successes stay applied, failures are reported, and the list remains consistent.
- Batch favorite on photos already in the target state: idempotent, no error, reflected in the outcome summary.
- The user exits selection mode while a batch action is still running: the action completes and its result is still reported.
- Mixed favorite states within one selection: explicit add/remove actions remove the ambiguity — no "toggle" behavior in batch mode.

## Research Notes

- `frontend/src/components/PhotoViewer.svelte` and `HousekeepingView.svelte` — single-photo delete is permanent with a confirmation dialog; batch delete must match these semantics (no trash, no undo).
- `frontend/src/components/CollagesView.svelte` — collage cards represent duplicate groups with accept/reject actions; photo-level actions do not map onto them, which drives the accept/reject-only scope for that surface.
- `frontend/src/components/ViewerMetadataEdit.svelte` — per-photo taken-date editing exists today; batch date-shift extends that capability to selections.
- No external research was required; the interaction conventions here (selection toggle, long-press entry, range selection, action bar with counts) are standard gallery-application behavior.

## Assumptions

- Deletion remains permanent with confirmation — no trash, no undo; mirrors the existing single-photo delete (user-confirmed scope).
- Favorite actions are explicit "Add" / "Remove", never a toggle, for predictable batch behavior.
- Date-shift applies only to the taken date (`taken_at`), not to file creation or modification dates.
- Export is a single downloadable archive of the original files, all formats (photos, RAW, video) included; name collisions are disambiguated.
- All new visible UI text is added to both language dictionaries (en and de), per repo convention.
- Each batch action is initiated once for the whole selection; progress and outcome are reported to the user.
- Performance follows standard web-app expectations: the interface stays responsive and shows progress for large selections.
- Selection mode covers all six surfaces (user decision); batch "keep" is included for housekeeping candidates (user decision); collage selection is limited to accept/reject (user decision).
- "Select all visible" is in scope; select-all across the entire library is not.

## Acceptance Criteria

- **SC-001**: On any of the six surfaces, a user can enter selection mode and select multiple photos or cards within two interactions, using pointer, touch, or keyboard.
- **SC-002**: Batch delete of N selected photos permanently removes all N from the library and every view after a single confirmation; none of the deleted photos remain accessible.
- **SC-003**: Batch date-shift of 100 selected photos applies the chosen offset to every photo with a taken date and reports the count of photos skipped for missing dates.
- **SC-004**: Batch export of N selected photos of mixed formats yields one archive containing all N original files that opens on a standard desktop OS.
- **SC-005**: Batch add/remove favorite updates every selected photo and is reflected in the Favorites view and the photo viewer.
- **SC-006**: A batch keep in housekeeping dismisses every selected candidate in one action without deleting the photos.
- **SC-007**: Batch accept/reject in collages resolves every selected pending collage in one action, with rejection requiring confirmation.
- **SC-008**: A batch action on 200 selected photos keeps the interface responsive and shows progress until completion.
- **SC-009**: When part of a batch action fails, the successful items remain applied and the user sees an outcome summary with the failed count.
- **SC-010**: Exiting selection mode or navigating to another surface clears the selection, and no action can be triggered on an empty selection.
