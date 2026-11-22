# Feature Specification: People Merge + Bulk Actions

**Created**: 2026-11-22
**Status**: Draft — awaiting approval
**Input**: "merge faces / people, multi selection actions on people"

## Understood User Request

The People view lists automatically clustered persons. Clusters that belong to the
same human cannot be combined, and several persons cannot be acted on at once. This
feature adds person-level merge and multi-select with bulk actions on the People list,
so wrong splits can be repaired and junk clusters can be cleaned up in bulk.

## Motivation

Clustering is online single-link assignment against a fixed cosine threshold
(`CLUSTER_DISTANCE = 0.5`, `src/face_recognition/store.rs:20-23`). It cannot know that
two clusters are the same human, and it never overwrites a stored `person_id`
(`src/face_recognition/store.rs:1-6`). Today the only corrections are rename and hide
(`frontend/src/components/PeopleView.svelte:111-207`), so a split identity stays split
forever and the list fills with fragments. Merge is the missing repair primitive;
selection is what makes it usable at a few hundred clusters.

## Summary

The People list gains a selection surface that reuses the existing photo-surface
selection mechanics. With two or more persons selected, a merge action opens a
mandatory dialog that picks the surviving person (keeper), the resulting name and shows
the combined face count and hidden state. Merging moves every face of the source
persons to the keeper, deletes the source person rows and writes a single persisted undo
slot. Hide/unhide becomes a bulk action on the same selection. The last merge can be
undone once, from a toast or from a status chip in the People toolbar, with an honest
restore count if a rescan deleted and re-inserted affected faces.

Out of scope: per-face reassignment, split, delete-person, bulk rename, merge entry
points outside the People list, merge history beyond the last merge, and real
face-detection seeding in E2E.

## Decisions

- **Actions**: merge and hide/unhide only. No bulk rename (it only creates duplicate
  names) and no delete-person (faces would be orphaned, not deleted).
- **Merge flow**: mandatory dialog. Keeper defaults to the largest selected person
  (matching the list order), the name is prefilled from the keeper, the dialog shows
  faces, persons and hidden state.
- **Name prefill**: keeper name; if the keeper is unnamed and exactly one source is
  named, that source's name; otherwise empty (the card keeps showing `Person {id}`).
- **Undo**: exactly one persisted slot, replaced by the next merge, no TTL. Undo is
  keyed on `(photo_hash, face_index)` plus a per-source-person snapshot, never on face
  IDs, because face IDs are re-created on every re-index
  (`src/face_recognition/store.rs:137-210`). Restore is best-effort with an explicit
  count; it only touches faces still assigned to the keeper.
- **Stale undo**: the slot carries a monotone `undo_id`; requests send it and a
  mismatch answers 409 so a stale toast cannot undo a different merge.
- **Limit**: at most 20 source persons per merge (21 total). Above that the action is
  disabled with a hint, protecting against "select all visible" + merge.
- **During indexing**: merges are allowed while the faces phase runs. Both sides use
  short transactions and SQLite serialises writers; the clusterer never overwrites
  `person_id`.
- **Granularity**: person level only; no moving of individual faces.
- **Hidden**: hidden persons are selectable only while the hidden filter is on. The
  result keeps the keeper's hidden state; the dialog warns when visible sources would
  be merged into a hidden keeper. Toggling the hidden filter prunes the selection to
  the visible keys (`pruneSelection`, `frontend/src/lib/state.svelte.js:113-121`).
- **After an action**: selection ends, the list reloads, a toast reports the result
  (merge toast carries undo). Bulk-hide removes cards immediately, unlike the per-card
  hide that keeps a badged card until refresh.
- **Entry point**: People list only; no merge from the person-filtered grid or viewer.

## User Scenarios

### Scenario 1 — Repairing a split person (P1)

A user sees "Anna" with 40 faces and an unnamed "Person 47" with 6 faces of the same
childhood photos.

**Acceptance**
1. Selecting both persons and choosing merge opens a dialog listing both with their
   cover crops, a keeper radio defaulting to "Anna", the name "Anna" prefilled and
   "46 faces from 2 persons".
2. Confirming leaves one person "Anna" with 46 faces, removes "Person 47" from the list
   and shows a toast with an undo action.
3. The grid filter `?person=<Anna id>` returns the union of both clusters' photos.

### Scenario 2 — Undo after a wrong merge (P1)

A user merges two different people by mistake.

**Acceptance**
1. Clicking undo on the toast restores the source person with its snapshot name, hidden
   state and every face that is still assigned to the keeper.
2. After a page reload, the undo chip in the People toolbar still offers the same undo.
3. A second merge replaces the slot; the chip then names the newer merge, and the old
   toast's undo answers 409 and reports "no longer available".

### Scenario 3 — Bulk-hiding junk clusters (P2)

A user's library contains dozens of clusters formed from background people.

**Acceptance**
1. Selecting 15 persons and choosing hide removes all 15 cards from the unfiltered
   list in one step and reports the count.
2. With the hidden filter on, the same 15 cards appear badged; selecting them and
   choosing unhide returns them to the unfiltered list.

### Scenario 4 — Merge survives the nightly rescan (P1)

A merge is performed and the scheduler re-indexes the affected photos the same night.

**Acceptance**
1. After the scan, the merged person still owns all re-detected faces; no source person
   reappears.
2. Undo after such a scan restores whichever `(photo_hash, face_index)` pairs still
   exist and reports the honest restored/total count.

## Functional Requirements

- **FR-001**: The People list (`route.view === 'people'`, no `person` and no `query`)
  offers selection mode through the existing header "Select" button and through
  long-press on touch. `frontend/src/App.svelte:231` currently excludes exactly this
  surface from the button.
- **FR-002**: In selection mode a card click toggles the selection instead of routing
  to the person-filtered grid; Shift+click extends a display-order range;
  "select all visible" selects every card of the current filter; Escape exits; card
  actions (rename, hide) are hidden, mirroring `PhotoCard.svelte:154-220`.
- **FR-003**: Selection keys are `String(person.id)`; the People list publishes them as
  `selectionState.orderedKeys` in display order and never leaks them to photo surfaces.
- **FR-004**: The action bar on the People list offers merge, hide and unhide; every
  action is disabled with fewer than one selection, and merge is disabled with fewer
  than two selected persons.
- **FR-005**: Merge requires 2 to 21 selected persons. With more than 21 the merge
  action is disabled and a hint states the limit.
- **FR-006**: The merge dialog lists every selected person with cover crop, name,
  `Person {id}` fallback, face count and a keeper radio; it shows the summed face count,
  a name field prefilled per the decision rule and the keeper's hidden state.
- **FR-007**: Confirming merges in one transaction: snapshot the source persons and
  their `(photo_hash, face_index)` sets, reassign those faces to the keeper, delete the
  source person rows, apply the name and, if supplied, the hidden state, and replace
  the undo slot. The response carries the moved face count, removed person count and
  `undo_id`.
- **FR-008**: If the keeper is hidden and at least one source is visible, the dialog
  warns that the merged person will leave the default list; the merge still proceeds
  with the keeper's hidden state.
- **FR-009**: The People response exposes the current undo slot
  (`undo_id`, `target_id`, `target_name`, `source_count`) or `null`.
- **FR-010**: Undo restores faces to a re-created source person using its snapshot
  name and hidden state, touching only faces still assigned to the keeper; it reports
  `restored_faces` and `total_faces`. A source person is re-created only when at least
  one of its faces was restored; the slot is consumed either way.
- **FR-011**: Undo with a missing slot answers 404; with a mismatched `undo_id` answers
  409. The client surfaces both as "no longer available", never as a generic error.
- **FR-012**: Bulk hide/unhide updates every selected person in one request and answers
  the updated count; the list reloads and the selection ends.
- **FR-013**: Restored source persons with fewer than `MIN_VISIBLE_FACES` faces stay
  below the visibility threshold, exactly as before the merge.
- **FR-014**: After merge, undo, hide or unhide the People list reloads; renamed or
  hidden state is never guessed client-side beyond the existing optimistic toggle.
- **FR-015**: Every merge/dialog/hint string is added to `en.json` and `de.json`;
  `npm run test:i18n` stays green. The merge action uses the `git-merge` Feather icon,
  which must be registered in `frontend/src/components/Icon.svelte:44-85` (unregistered
  icons render empty).
- **FR-016**: The merge dialog is a native `<dialog>` following the rename dialog
  pattern (`PeopleView.svelte:406-480`): focus moves into the dialog, Escape cancels,
  focus returns to the merge action, the keeper group is a labelled radiogroup, the
  warning is `role="alert"`, and errors keep the dialog open with an inline message.
- **FR-017**: On mobile the dialog fits the viewport with an internally scrolling person
  list, and all controls keep 44 px hit targets; the selection bar keeps destructive
  actions last.

## API Contract

- `GET /api/people?include_hidden=<bool>` → `{ people: Person[], last_merge: MergeSlot | null }`
  where `Person = { id, name, hidden, face_count, cover_face_id }` (unchanged) and
  `MergeSlot = { undo_id, target_id, target_name, source_count }`.
- `POST /api/people/merge`
  body `{ target_id: i64, source_ids: [i64], name?: string | null, hidden?: bool }`
  → 200 `{ moved_faces, removed_people, undo_id }`;
  400 for a keeper outside the selection, fewer than 2 or more than 21 distinct ids, or
  a name longer than 100 characters; 404 for an unknown person id.
- `POST /api/people/merge/undo`
  body `{ undo_id: i64 }`
  → 200 `{ restored_faces, total_faces, restored_people }`; 404 without a slot; 409 on a
  stale id.
- `POST /api/people/batch/hide`
  body `{ ids: [i64], hidden: bool }` → 200 `{ updated }`; 400 for more than 1000 ids or
  an empty list; unknown ids are skipped and reported via `updated`.

Routes follow the existing `POST /api/{resource}/batch/{action}` convention
(`src/handlers_photo.rs:1530-1568`).

## Data Model

New additive migration, no changes to existing tables:

- `person_merge_undo(id INTEGER PRIMARY KEY AUTOINCREMENT, target_id INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')))` — exactly one row exists; a merge
  deletes the previous slot in the same transaction.
- `person_merge_undo_source(undo_id, person_id, name, hidden,
  PRIMARY KEY (undo_id, person_id))` with `ON DELETE CASCADE` on `undo_id`.
- `person_merge_undo_face(undo_id, person_id, photo_hash, face_index,
  PRIMARY KEY (undo_id, photo_hash, face_index))` with `ON DELETE CASCADE`.
  No foreign key to `photos` or `faces`: the rows are a snapshot by value, and faces are
  deleted and re-inserted on every re-index.

Merge transaction order: delete the previous slot → insert slot and snapshots (faces
`WHERE person_id IN sources`) → `UPDATE faces SET person_id = keeper WHERE person_id IN
sources` → `DELETE FROM people WHERE id IN sources` → apply name/hidden to the keeper →
commit.

Undo transaction order: load the slot and compare `undo_id` → per source person
`UPDATE faces SET person_id = source WHERE person_id = keeper AND EXISTS(snapshot row
with matching photo_hash and face_index)` → recreate each source person with its
snapshot id/name/hidden when at least one face moved → delete the slot → commit.

## Edge Cases

- One selected person: merge disabled; hide still available.
- More than 21 selected: merge disabled with the limit hint; hide unaffected.
- The keeper is outside `source_ids` or ids are duplicated: 400, nothing written.
- The keeper is renamed after the merge and the merge is undone: the source persons
  return under their snapshot names; the keeper keeps its current name.
- A merge while the faces phase inserts a new face: the transactions serialise; the new
  face is either part of the merge or assigned after it, never orphaned by a delete.
- Undo after some affected photos were deleted: those snapshot rows simply match no
  face; the response counts them in `total_faces` but not in `restored_faces`.
- Undo when every face was re-indexed into a different `face_index`: `restored_people`
  is 0, no empty person row is created, the slot is consumed and the toast says so.
- Undo of a merge whose keeper was itself merged later: impossible through the UI
  because the second merge replaces the slot; the old toast answers 409.
- Toggling the hidden filter with an active selection prunes hidden keys; the selection
  survives.
- A deletion/rename of a person in another tab: `GET /api/people` is refetched after
  every action; stale ids in a merge request answer 404 with an inline dialog error.

## Research Notes

- `src/face_recognition/store.rs:1-6, 239-309` — online single-link assignment; stored
  `person_id` is never overwritten; new faces join the nearest person within
  `CLUSTER_DISTANCE`, so a merge is stable across rescans.
- `src/face_recognition/store.rs:137-210` — every re-index deletes and re-inserts a
  photo's faces, so face IDs are not stable; `match_old_person` preserves the person
  association, not the ID.
- `migrations/20261118000001_create_face_recognition.sql:17-32` —
  `faces.person_id` is `ON DELETE SET NULL`; a bare person delete would orphan faces,
  which is why merge updates before deleting.
- `src/face_recognition/store.rs:311-330` — the People query joins faces, filters
  `face_count >= MIN_VISIBLE_FACES`, and derives `cover_face_id` by
  `quality DESC, id`; there is no photo count.
- `frontend/src/App.svelte:231, 302-304` — the header Select button excludes the People
  list; the SelectionBar mounts globally.
- `frontend/src/components/SelectionBar.svelte:36-59, 399-411` — per-surface action
  configuration and the Escape handling.
- `frontend/src/lib/state.svelte.js:58-121` — global selection state plus
  `pruneSelection`; `selectionRangeController.js:8-33` and `longpress.js:2-11` — the
  touch-only long-press entry shared by card surfaces.
- `tests/e2e/specs/people.e2e.spec.js:27-73` — the People E2E mocks `/api/people**`
  entirely; real backend seeding would need face detection with ≥3 faces per cluster.

## Assumptions

- The People list stays unpaginated; "select all visible" therefore equals every card
  of the current filter.
- Merge never changes photo rows, favorites or files; it only rewrites
  `faces.person_id` and `people` rows.
- `AUTOINCREMENT` on `people.id` keeps old ids out of reuse, so recreating a snapshot id
  is collision-free.
- The undo slot is a convenience, not a transaction log: one merge, one undo, replaced
  on the next merge.
- Hidden persons remain filtered by `MIN_VISIBLE_FACES`; the hidden filter changes the
  hidden flag only.
- E2E covers the UI through mocked routes; the transaction logic is covered by Rust
  tests against the in-memory database.

## Acceptance Criteria

- **SC-001**: From the People list a user can enter selection mode via the header
  button or a touch long-press, select several persons, and reach merge or hide within
  two interactions.
- **SC-002**: Merging 2..21 selected persons yields exactly one keeper with the summed
  faces; the source cards disappear after the list reloads.
- **SC-003**: The merge dialog defaults the keeper to the largest person and prefills
  the name per the decision rule; a name longer than 100 characters is rejected
  client-side and by the API.
- **SC-004**: Undoing the last merge restores the source persons and every face that is
  still assigned to the keeper, and reports an honest restored/total count.
- **SC-005**: The undo chip survives a reload and disappears after an undo or a newer
  merge; a stale toast's undo answers 409 and is surfaced as "no longer available".
- **SC-006**: Bulk hide/unhide applies in one request, reloads the list and ends the
  selection; hidden cards disappear from the unfiltered list.
- **SC-007**: Merging visible persons into a hidden keeper is impossible without the
  dialog warning; the result stays hidden.
- **SC-008**: Merging more than 21 persons is impossible through the UI and rejected by
  the API.
- **SC-009**: Toggling the hidden filter prunes hidden keys from the selection and keeps
  the mode active.
- **SC-010**: After a simulated re-index (faces deleted and re-inserted under new IDs)
  the merge persists and undo restores only the surviving `(photo_hash, face_index)`
  pairs.
- **SC-011**: `npm run lint`, `npm run format:check`, `npm run test:unit`,
  `npm run test:i18n`, `cargo fmt --check`, `cargo clippy --all-targets -- -D
  warnings`, `cargo test`, the extended People E2E spec and the existing batch-select
  E2E specs pass.
- **SC-012**: The new strings exist in both dictionaries and the `git-merge` icon is
  registered; no surface renders an empty icon.
