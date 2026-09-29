# Feature Specification: Bulk Date-Shift Removal and File-Only Photo Dates/Coordinates

**Created**: 2026-09-28
**Status**: Approved
**Input**: Remove the bulk date-shift feature and stop storing date values in the library database — a photo's date (and, per clarification, its coordinates) may only ever be adjusted in the file, never in the database.

## Goal
TurboPix currently keeps a photo's capture date and coordinates in its own database as if that copy were authoritative: the bulk date-shift action rewrote only the database and never the file, so the library and the file disagreed, and any later rescan or in-place edit could silently take the shift back. This spec removes that capability entirely and makes the photo file the single place where a capture date or a coordinate lives. The database stops persisting either value and every date- or location-dependent view is served from the files themselves. Scope boundary: date and coordinate values only — the library's own audit timestamps, resolved location labels, and generated artifacts are out of scope.

## User Scenarios

### Scenario 1 - Editing one photo's date lands in the file only (P1)
A user corrects a single photo's taken date from the viewer.

**Acceptance**
1. Given a photo whose file type can carry metadata, when the user saves a new taken date, then the date is written into the photo file and is what the app shows for that photo afterwards (card, viewer, timeline position), while the database holds no date value for it.
2. Given that saved photo, when the library is reindexed and the file is opened in another application, then the app and the other application report the same date, and the value shown by the app is unchanged by the reindex.
3. Given the saved photo, when the user reopens it in the same running session, then the shown date is the file's date — no server restart and no manual rescan is required.

### Scenario 2 - Editing coordinates lands in the file only (P1)
A user corrects a photo's location.

**Acceptance**
1. Given a photo whose file type can carry metadata, when the user saves coordinates, then they are written into the file, the photo's place on the map (and any location grouping) follows immediately, and the database holds no coordinate value for it.
2. Given that saved photo, when the file is read by another application, then the same coordinates appear, at the precision the file can represent.
3. Given photos whose coordinates exist only in their files, when the map view renders, then each photo is placed from its file's coordinates.

### Scenario 3 - The bulk date-shift action is gone (P1)
A user works with a multi-selection of photos.

**Acceptance**
1. Given a selection on any surface that offers bulk actions, when the action bar renders, then no date-editing action is offered, and every remaining action (delete, favorite, export, and the surface-specific keep/accept/reject) behaves exactly as before.
2. Given the removed capability, when a client sends the request the feature used to send, then it is rejected as an unknown request and no photo's date or coordinates change anywhere.
3. Given the removal, when the interface is viewed in English or German, then no text about shifting dates remains and both language bundles stay complete and structurally identical.

### Scenario 4 - Files that cannot carry the value say so (P2)
A user tries to re-date a video, a RAW file, or an unwritable photo.

**Acceptance**
1. Given a photo whose file type cannot carry a date or coordinates, when the user looks for date/location editing, then the app states that this file type cannot be edited in-app, and nothing is written to the file or the database.
2. Given an unwritable file, a read-only file, or a file that disappeared since it was indexed, when the user saves, then the save fails with an explicit error, nothing is stored anywhere, and the previously shown value remains.

### Scenario 5 - Date and location browsing stays correct from the files (P1)
A user browses a library whose dates and coordinates exist only in its files.

**Acceptance**
1. Given a library with no stored dates, when the grid is sorted by date (ascending or descending), filtered to a year/month range, or the timeline renders its span and per-month density, then the results match the files' dates exactly.
2. Given a file whose date was changed outside the app, when the library is reindexed, then the app shows the file's date and the affected ordering, filter buckets and timeline density follow it.
3. Given a photo whose file yields no readable date, when its date is displayed or used for ordering, then the app applies the existing fallback order (embedded date, then a date encoded in the filename, then the file's own timestamp) and never invents a different date.
4. Given photos that carry a database-only date from the removed feature, when the change ships, then that value is gone and the file's own date is what the app shows.

### Scenario 6 - Undated and imprecise cases stay honest (P3)
A user works with files that carry partial or absent metadata.

**Acceptance**
1. Given a saved date at a precision the file cannot represent (sub-second, or a time zone), when the photo is shown after the save, then the app reports the value the file actually carries and does not claim the extra precision.
2. Given a saved value, when it is read back from the file, then it is not shifted by a time-zone offset and does not drift on repeated read/write cycles.
3. Given a photo with neither a file date nor a filename date, when it appears in a date-ordered listing, then it keeps the same position and the same "unknown" presentation as today.

## Functional Requirements
- **FR-001**: At rest, the library database stores no capture date and no coordinates for a photo — no table column, no index over such a value, and no key inside a photo's stored metadata.
- **FR-002**: A photo's date and coordinates are the ones its file carries; when a derived copy exists anywhere, the file wins on conflict, and no derived copy may be edited as if it were the authority.
- **FR-003**: Date-dependent browsing remains available and correct while dates come from files: date sort (both directions), year/month range filtering, the timeline's oldest/newest span and its per-month density, and any date-based grouping such as same-day clusters.
- **FR-004**: Location-dependent behaviour derives from the files' coordinates: map markers, their clustering, location grouping, and location-based filtering.
- **FR-005**: Saving an edited taken date writes it into the file and nowhere else; the value shown afterwards equals the value the file carries, without a restart or a manual rescan.
- **FR-006**: Saving edited coordinates writes them into the file and nowhere else, with the same immediacy rule, and the app never reports a precision the file cannot represent.
- **FR-007**: Any failure to write the file (unsupported file type, unwritable or read-only file, file no longer present, write refused) fails the save with an explicit error, stores nothing anywhere, and leaves the previously shown value in place.
- **FR-008**: In-app date/coordinate editing is offered only for file types whose files can carry the value; for every other file type the app states that it cannot be edited in-app, and no value is stored.
- **FR-009**: The bulk date-shift capability is removed end to end: no action in any selection UI, no client request helper, no server route, no request/response shape for it, no user-facing text about it, and no test asserting it — while the remaining bulk actions and their shared machinery keep working unchanged.
- **FR-010**: Bookkeeping timestamps of the library itself (index, creation, update and file-modification bookkeeping) and dates belonging to generated artifacts are not photo capture dates and remain unaffected; resolved location labels may stay cached as long as they remain recomputable from the file's coordinates.
- **FR-011**: A photo whose date cannot be read from the file keeps the existing resolution order (embedded date, then a date encoded in the filename, then the file's own timestamp), so undated photos keep a stable, non-invented position.
- **FR-012**: Date- and location-dependent browsing stays usable at library scale: once the library's files have been read (indexing/startup), ordinary browsing does not re-read the whole library per request, and response times stay within the budget of SC-003.
- **FR-013**: Changes made to library files outside the app become visible after a reindex/rescan; the app never presents a stored value as more recent than the file.
- **FR-014**: Every user-visible string touched by this change — the removal and the "cannot be edited in-app" statement included — exists in English and German with an identical key structure.

## Key Entities
- **Photo file**: the authoritative record for a photo's capture date and coordinates; the only thing a date or location save may modify.
- **Library database**: the catalogue of photos (identity, paths, favorites, album membership, thumbnails, resolved labels, bookkeeping timestamps); holds no capture date and no coordinates.
- **Derived date/location view**: whatever serves sorting, filtering, timeline density and map placement; reproducible from the files and never authoritative over them.
- **Bulk action set**: the actions a multi-selection offers; after this change it contains no date-writing action.

## Edge Cases
- File types that cannot carry the value in-app (video, RAW, WebP, HEIC), plus read-only, unwritable or vanished files: explicit failure or an explicit statement, nothing stored.
- Embedded dates carry no time zone and only second precision: no false precision, and no offset drift when a value is read back from the file.
- Coordinates are stored by the file as degrees/minutes/seconds: the round trip is exact only to that precision, and the app must not claim more.
- A photo with no embedded date relies on the filename or file-timestamp fallback; a photo with neither stays undated and keeps today's presentation.
- Date/coordinate values that exist only in the database (from the removed feature) are gone after the change: nothing is migrated, repaired or announced.
- Photo identity must survive a metadata edit: favorites and album membership stay attached through the file write.
- A file replaced at the same path since it was indexed: the file's current date wins as soon as the library notices the change.
- Cold start / first browse before the library's metadata has been read: browsing must not display dates that contradict the files.
- Removing a stored column and its index is a schema change; already-applied migrations are checksum-verified and must not be edited — the schema change is carried by a new migration.
- A save racing a reindex, and two saves on the same photo: no interleaving may leave a value that contradicts the file.
- Bulk selection surfaces that previously offered the removed action must still be fully operable without it in both languages.

## Research Notes
- https://www.iptc.org/std/photometadata/documentation/userguide/ — "Exif currently does not hold time zone information in its time stamp … Most software will apply the local time zone of the receiving computer system", and a shown Date Created "can be derived from the Exif DateTimeOriginal"; the file's capture-date tag is therefore the interoperable target of a date edit, and the app must not promise time-zone-exact values.
- https://exiftool.org/TagNames/EXIF.html — `DateTimeOriginal` (0x9003) and `CreateDate` (0x9004) are the capture-date tags, while `OffsetTime`/`OffsetTimeOriginal`/`OffsetTimeDigitized` (0x9010–0x9012) exist but are not written today; the precision the app may claim is bounded by the tags it writes.
- https://superuser.com/questions/983804/mp4-video-editing-creation-date-and-other-metadata — an MP4's creation date can be rewritten with a lossless `-c copy -metadata creation_time=…` remux, so excluding video files from in-app date editing is a deliberate scope choice, not a technical impossibility.
- Web search back ends were mostly unavailable during this session (CAPTCHA/bot walls); the three sources above were read directly, and all remaining statements rest on repository evidence.

## Assumptions
- API responses keep their existing shape: a photo's date and coordinates are still exposed to clients, only their source changes, so displaying components need no rework beyond sourcing.
- Resolved location labels and their "already resolved" bookkeeping may stay cached; the coordinates that produced them are the value that must live only in the file.
- No replacement is introduced for bulk date editing: after removal, dates and coordinates can be changed one photo at a time, and only for file types that can carry them.
- The "cannot be edited in-app" limitation is expressed through the existing edit affordance that already gates on file type, not through a new workflow.
- Clearing a previously set date or coordinate remains unsupported, as today; the app never writes an empty value, and the file's fallback order applies.
- External edits to files are picked up on the next reindex or rescan; there is no filesystem watcher, and adding one is not part of this change.
- Test and fixture seeding move to files: test data that pinned a date by writing the database writes dates into the fixture files instead, because no stored date remains.
- The database and caches may be recreated (pre-production project), so previously stored dates/coordinates are not migrated, preserved or reported.
- "Date values" in the brief means capture/presentation dates; the library's own audit timestamps and generated-artifact dates are explicitly not part of the removal.

## Success Criteria
- **SC-001**: After a date edit, the database contains no stored date for that photo, a file-level metadata reader and the app report the same date, and a reindex leaves the app's value unchanged.
- **SC-002**: The bulk date-shift capability is unreachable from both UI and API in every supported language, and the remaining bulk actions pass their existing test suites unchanged.
- **SC-003**: On a library of at least 5,000 photos, date sorting, a month-range filter and the timeline load each respond within 1 second on the reference desktop, with results matching the files' dates.
- **SC-004**: Across the whole reference library, every photo's displayed date and coordinates equal what a file-level metadata reader reports for that file — zero photos disagree.
- **SC-005**: For a video/RAW/WebP photo, no edit path writes a date or coordinate anywhere, and the interface states the limitation.
- **SC-006**: After a reindex, a date changed outside the app is displayed and reorders the affected listings; no stored value overrides the file.
- **SC-007**: Date and location browsing (sort, filters, timeline, map placement) is fully operable with no stored dates or coordinates in the database.
- **SC-008**: English and German language bundles remain complete and structurally identical after the removal of the date-shift text.
