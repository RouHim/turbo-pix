# Feature Specification: In-Place Video Metadata Editing (MP4/MOV/M4V)

**Created**: 2026-09-28
**Status**: Approved
**Input**: Support editing video metadata, especially MP4 files.

## Goal

Videos in the TurboPix library get the same metadata editing the photo library already has: the viewer's editor offers capture date and location for a single video, and the values are written into the video file's own container metadata and mirrored into the library database. The write is an in-place patch of the existing metadata region — the media payload is never rewritten, re-muxed, or re-encoded, so a save is effectively instant regardless of file size. When a value cannot be written in place, the file stays byte-identical and the user gets a specific, localized refusal.

Out of scope: editing descriptive metadata (title, description, tags, rating), bulk/selection editing of video files, the existing database-only batch date shift, MKV/WebM/AVI/3GP containers, rotation, transcoding, thumbnails, and any change to the photo EXIF path.

## User Scenarios

### Scenario 1 - Edit a video's capture date (P1)

A video shot on a phone shows a wrong capture date (wrong camera clock, timezone slip, or a filename-derived fallback), so it sits in the wrong place in the timeline. The user opens the video, opens the metadata editor, corrects the date, and saves — the save is immediate, the timeline and grid agree, and the file itself now carries the corrected date.

**Acceptance**
1. Given an MP4/MOV/M4V video in the library, When the user opens it in the viewer, Then the metadata editor is available for it and offers the same three fields the photo editor offers (capture date, latitude, longitude).
2. Given the editor is open on such a video, When the user saves a new capture date, Then the save finishes without a long-running progress state and in a time that does not grow with the media payload size.
3. Given the save succeeded, When an independent reader (e.g. `ffprobe`) inspects the file, Then it reports the new capture date, and every metadata entry the file already had (device/make/model keys, location, encoder) is still present.
4. Given the save succeeded, When the user reloads grid and timeline, Then the video is listed and sorted under the new date.
5. Given the save succeeded, When the library is rescanned (startup or nightly) or the server restarts, Then the value is unchanged.
6. Given the save succeeded, When the next scan runs, Then the video is not re-encoded, its thumbnail is not regenerated, and its search vector is not recomputed.

### Scenario 2 - Edit a video's location (P1)

A video already carries the shooting location written by the recording device. The user corrects the coordinates (or adds the pair where the file's own metadata region has room for it), and the video moves on the map.

**Acceptance**
1. Given a video whose container already holds a location entry, When the user saves a coordinate pair, Then that entry is replaced (never duplicated), the viewer shows the new coordinates, and the video appears at the new position on the map.
2. Given a video whose container holds no location entry, When the user attempts to save coordinates, Then the file stays byte-identical, the database is unchanged, and the editor shows a specific localized message naming the reason (and keeps the entered values in the form).
3. Given a coordinate pair outside -90..90 / -180..180, or a latitude without longitude (or vice versa), When the user saves, Then the request is rejected as invalid input (4xx) and nothing is written.
4. Given a video with saved coordinates, When the library is rescanned, Then the app still reports the same coordinates and the resolved place name is not dropped by the rescan.

### Scenario 3 - Refusals never damage the file (P2)

Some files cannot take the change: an unsupported container, a media file on a read-only mount, a file that offers no writable location slot, or a file that vanished between selection and save. The user must get a clear reason and must never end up with a damaged or partially updated video.

**Acceptance**
1. Given an MKV, WebM, AVI, or other non-ISO-BMFF video, When the viewer's editor is opened for it, Then it offers no save (the same way unsupported photo formats already behave).
2. Given an MP4/MOV/M4V whose file or filesystem is not writable, or which provides no slot for the requested value, When the user saves, Then the request fails with a specific, machine-readable, localized error — never the generic server-error response — and both the file and the database are unchanged.
3. Given any failed save, When the file is compared with its pre-save state, Then it is byte-identical.
4. Given a save that returns success, When the response body is inspected, Then the returned row already carries the new values and the new file fingerprint (size, modification time).

### Scenario 4 - Saving while the video is in use (P2)

The user watches a video (or its conversion is running) and edits the metadata in a second window.

**Acceptance**
1. Given a video is being played, streamed, or converted, When its metadata is saved, Then playback/conversion continues without an error and the resulting media stream is unchanged.
2. Given two save requests for the same video arrive close together, When both are processed, Then the file never ends up in a mixed state (capture date from one request, location from the other) — the later save wins as a whole.

## Functional Requirements

- **FR-001**: The viewer's metadata editor MUST accept videos of type MP4, MOV, and M4V, exposing exactly the photo editor's editable fields (capture date, latitude, longitude).
- **FR-002**: A successful save MUST write the values into the video file's own container metadata AND mirror them into the library database as one outcome; if either part cannot be applied, neither the file nor the database may change.
- **FR-003**: The file write MUST be an in-place patch of the existing metadata region: no re-encode, no container remux, no full-file rewrite, no additional free disk space proportional to the file size, and a duration that does not grow with the media payload size.
- **FR-004**: Every capture-date carrier the file already contains MUST be updated to the new value within the same save, so that no reader — TurboPix or third-party — can observe the previous value afterwards.
- **FR-005**: A location write MUST target the location carrier the file already provides and replace its value in place; if the file provides no location carrier, or the carrier cannot take the new value without moving media data, the save MUST be refused without modifying the file or the database.
- **FR-006**: Saving MUST be all-or-nothing: any failure at any point MUST leave the file byte-identical and the database row unchanged.
- **FR-007**: The written values MUST be readable back by TurboPix's own indexing, so that a library scan reproduces the same capture date and coordinates; video metadata extraction MUST therefore resolve the location carrier (today no location is extracted for videos at all), and must keep the database's resolved place name across a rescan. *(Scope note: this governs the carriers a file holds when the editor writes them. One carrier class can be lost before the editor ever sees the file — the scan's own faststart fix, which is out of this feature's scope; see [Known Limitations](#known-limitations).)*
- **FR-008**: After a successful save, the stored file fingerprint (size, modification time) and mirrored metadata MUST match the file on disk, so the next scan treats the video as unchanged: no re-extraction, no re-embedding, no thumbnail regeneration, no transcode/cache invalidation.
- **FR-009**: A save MUST NOT corrupt or interrupt an in-flight playback, stream, or conversion of the same video, and MUST NOT leave a concurrent save's values half-applied.
- **FR-010**: Every refusal class (unparsable date, out-of-range or unpaired coordinates, unsupported container, no writable slot for the value, read-only file/module/filesystem, file missing) MUST be reported as a distinct, machine-readable, localized error; client input errors MUST be 4xx and MUST NOT surface as the generic server-error response.
- **FR-011**: Photo behavior MUST stay unchanged: photo EXIF writing, the database-only batch date shift for photos and videos, rotation, deletion, transcoding, and the set of containers accepted for writing beyond MP4/MOV/M4V.
- **FR-012**: All new user-visible strings MUST exist in both language bundles with identical key structure.
- **FR-013**: Clearing a previously set value MUST keep today's photo-editor semantics (not offered), and an empty request MUST NOT silently rewrite the file.
- **FR-014**: Where a carrier stores the value as text, the save MUST preserve the existing representation's format and byte length (precision, offset style, separators) instead of re-serializing it in the app's own style; a new value that cannot be represented at the existing length MUST be refused (never truncated).

## Key Entities

- **Library row (`photos`)**: path-hash primary key, `file_path`, `taken_at`, the `metadata` JSON holding `$.location.latitude` / `$.location.longitude` / `$.location.city` and the `$.video.*` capability record, plus the change-detection fingerprint `file_size` / `file_modified`.
- **Video container metadata (ISO-BMFF)**: the fixed-width creation timestamp every file carries, plus optional text date carriers and an optional location carrier — the only carriers that exist per file are the ones that may be edited.
- **Metadata edit request**: one request per single video carrying capture date, latitude, longitude.
- **Refusal**: the outcome of a save that changes nothing, carrying a specific machine-readable reason.

## Edge Cases

- MOV/M4V variants, 64-bit timestamp variants, and values that are unparsable or before the supported minimum are refused as invalid input.
- Fragmented or otherwise unpatched-layout ISO-BMFF files offer no in-place slot: refusal, file untouched.
- A shorter replacement value is allowed; a longer one is only allowed when the file's metadata region has unused room — otherwise refusal, never a silent shrink or a media-data shift.
- A video may hold several date carriers with different encodings; after a save they agree.
- Two concurrent saves of the same video; saves while the same video streams or converts.
- Read-only file/module/filesystem, missing file, permission denied.
- Multi-gigabyte videos: save cost stays independent of media size.
- Videos whose date was previously moved by the database-only batch action disagree with their file; the file is authoritative, so the next scan restores the file's value.
- A video whose capture date is only derivable from filename/creation-time fallbacks: after a save the value lives in the file and no longer depends on those fallbacks.

## Research Notes

- Measured locally (ffmpeg 9.0.1, 2026-09-28) on `test-data/test_video_long.mp4`: a copy-remux preserves the media payload bit-identically (equal SHA256 over the copied streams), but without an explicit faststart flag the output places the container index behind the media data, which regresses progressive playback — a rewrite-based save therefore contradicts the "instant" constraint.
- Measured: a plain copy-remux of a file carrying QuickTime keys silently dropped `com.apple.quicktime.creationdate`, `.make`, `.model`, and `.location.ISO6709`; only an explicit tag-preserving muxer flag kept them — i.e. the rewrite route loses third-party metadata that an in-place patch leaves untouched.
- Measured: the muxer's `date` tag lands in the `udta`/`meta` item list, while the `creation_time` tag drives the fixed-width movie-header timestamp that every ISO-BMFF file has.
- Measured (ffmpeg 9.0.2, 2026-09-30) with the scanner's exact vector (`video_processor::fix_moov_atom`: `ffmpeg -y -i <in> -c copy -movflags +faststart <out>`) on a moov-at-end source stamped `2015-06-01T12:00:00Z` — all three fixed-width headers at `3516004800` (the 1904 epoch, which ffprobe reports in `format.tags.creation_time` AND `stream.tags.creation_time`) — the output carries `mvhd`/`tkhd`/`mdhd` `creation_time=0` in all three boxes and no `creation_time` in either tag set. The copy-remux erases the carrier this feature writes, independently of the QuickTime mdta keys dropped alongside it; only a source that is already progressive, and is therefore never remuxed, keeps its headers.
- Existing app read priority for video dates (`src/metadata_extractor.rs`): format-level creation time → QuickTime `creationdate` key → stream-level creation time → format-level date → filename/file-creation fallback. A save must satisfy the highest-priority carrier actually present in the file.
- Local tooling: ffmpeg and ffprobe are installed; exiftool, MP4Box, AtomicParsley, and mediainfo are absent, so the feature cannot rely on an external tag editor.
- In-repo precedent for touching a source video: the scan-time moov fix already rewrites a video with a copy-remux plus an atomic rename, and the photo metadata path already writes file + database with a sibling temporary file and rename.
- Sources: https://developer.apple.com/documentation/quicktime-file-format/quicktime_metadata_keys — defines the QuickTime metadata keys (creation date, ISO-6709 location, device keys) that phone-recorded files use; https://exiftool.org/TagNames/QuickTime.html — names and containers of QuickTime tags, including the item-list date/name tags and the location-related entries; https://docs.rs/mp4ameta — a Rust reader/writer for iTunes-style MP4 metadata, limited to the item list and therefore not covering the QuickTime key carriers.

## Assumptions

- Only the single-video editor in the viewer gains this capability; the existing database-only batch date shift stays as it is (no file writes, no new bulk path).
- "Like photos" means file first, database mirrored — there is no silent database-only fallback.
- A location is saveable only when the file already provides a writable location carrier; the capture date is always saveable because every ISO-BMFF file carries a fixed-width movie-header timestamp.
- A request carrying both date and location is all-or-nothing.
- Values are written as UTC, matching the photo path; text carriers keep the file's existing representation and byte length.
- Clearing a previously set value stays unsupported, matching the photo editor.
- MKV, WebM, AVI, and 3GP remain read-only; rotation, thumbnails, transcoding, and the semantic index are untouched.
- The photo EXIF path, including its existing behavior for unsupported photo formats, is not modified by this feature.

## Success Criteria

- **SC-001**: For every video that already carries the value being edited, a save writes only within the metadata region: the media payload bytes are byte-identical before and after (100% of saves in the regression suite), and the save duration does not scale with the media payload size (a multi-gigabyte video saves in the same order of magnitude as a small one).
- **SC-002**: After a successful save, an independent reader and TurboPix's own re-scan both report the new capture date and, where applicable, the new coordinates; no other metadata entry present before the save is missing afterwards. *(Scope note: "present before the save" means present in the file the editor wrote to. A video can lose its QuickTime key carriers earlier, in the scan's own faststart fix — see [Known Limitations](#known-limitations); no save contributes to that loss.)*
- **SC-003**: Every refusal case (unsupported container, no writable slot, read-only target, invalid input, missing file) produces a byte-identical file, an unchanged database row, a distinct localized message, and never the generic server-error response.
- **SC-004**: A save never triggers a re-encode, a thumbnail regeneration, a semantic re-embedding, or transcode-cache invalidation for the video.
- **SC-005**: Both language bundles stay in parity, and the existing photo metadata, video playback, and library-scan suites pass unchanged.

## Known Limitations

- **The scan's faststart fix removes the carriers a save would have edited — it happens before the editor, not because of it.** A video whose `moov` atom sits behind the media data is rewritten during indexing (`photo_processor::maybe_fix_moov_for_video` → `video_processor::fix_moov_atom`, `-c copy -movflags +faststart`). That copy-remux does not carry the QuickTime mdta key carriers across — `com.apple.quicktime.creationdate`, `com.apple.quicktime.location.ISO6709` and the device keys are gone from the file afterwards — and it does not merely move the fixed-width headers either: it ZEROES the `mvhd` / `tkhd` / `mdhd` creation timestamps this feature also writes. Measured on this repo's own ffmpeg with the scanner's exact argument vector, a file whose three headers all carry a real instant (`2015-06-01T12:00:00Z`, reported by ffprobe in both the format and the stream tags) comes out of `-c copy -movflags +faststart` with all three boxes at `0` and no `creation_time` left in either tag set — so a fixed video carries no date carrier at all. Metadata extraction has already run when the fix fires, so that scan's row first lands with the values read from the pre-fix bytes; the next scan re-extracts the rewritten file, finds no carrier, and states the absent position as an explicit null — which is what clears the row's now-unsourced coordinates and name — while `MetadataExtractor::extract_taken_at_from_ffprobe_json` finds no `creation_time`, no QuickTime key and no `date` tag, so the row's capture date falls through to the filename / file-time fallbacks and is silently replaced. Measured, see Research Notes; the same trap is why the test fixture has to be seeded already progressive.
- **Consequence for this feature:** a capture-date save on such a video still lands — the boxes exist and the save fills them, and FR-008's unchanged size and mtime keep the scan skipping the file before `process_file_metadata_only` ever runs — but it writes into an already-emptied container rather than repairing one, so the first scan that does re-extract (any fingerprint change: another in-place edit, a restore, a move to a new path) reads no carrier and replaces the saved date with the filename / file-time fallback. A location save is refused outright for the missing carrier instead of writing into a container that no longer has one. Nothing a save does triggers the fix, and nothing a save does preserves the carriers the fix emptied. Repairing it means making the faststart pass tag- and timestamp-preserving in `video_processor::fix_moov_atom` (`src/video_processor.rs`) — a change to the scanner, outside this PR's diff, deferred. Out of scope here, recorded so the refusal and the date loss are read as the pre-existing container state they are.
