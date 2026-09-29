# Bulk Date-Shift Removal and File-Only Photo Dates/Coordinates — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete the bulk date-shift capability end to end and make the photo file the only place a capture date or coordinate lives — the database keeps no column, index or metadata key for either value, and every date/location-dependent view is served from an in-memory index that is rebuilt from the files at every scan.

**Architecture:** A new process-wide `MediaFactsIndex` (module `src/media_facts.rs`) maps photo **file path → `MediaFacts { taken_at, latitude, longitude }`**. Every scan reads the facts of every file it sees (changed files already do; unchanged files gain a facts-only read) and publishes them to the index; a successful metadata save re-reads the file and updates the index; deletes remove the entry. Listings/timeline/map/collage read the index for date ordering, month filtering, density and coordinates, and every photo leaving the API is enriched from the index (which injects `taken_at` and `metadata.location.latitude/longitude` into the response object only — the DB write path strips those keys and has no `taken_at` column). The schema change ships as a new migration that drops `photos.taken_at` and its index and removes the coordinate keys from stored `metadata`.

**Tech Stack:** Rust (warp, sqlx 0.9 / SQLite, chrono, kamadak-exif), Svelte 5 runes, Vite, Playwright E2E, node `--test` unit/i18n tests.

**Spec:** `.spec/remove-bulk-date-shift-and-file-only-dates.md`

## Global Constraints

- Breaking changes are allowed; this project is pre-production and not in production. Stored dates and coordinates are **dropped, never migrated**; the database and caches may be recreated.
- FR-001: at rest the library database stores no capture date and no coordinates for a photo — no column, no index over such a value, no key inside a photo's stored `metadata`, and no new cache/sidecar file.
- API response shapes stay unchanged: photo JSON keeps `taken_at` (nullable RFC3339 string) and `metadata.location.latitude/longitude`; `/api/photos/timeline` (`{min_date, max_date, density}`) and `/api/photos/map` (`{photos}`) keep their payloads, so displaying components need no rework beyond sourcing.
- Date resolution order stays exactly as today: embedded metadata (EXIF for images, ffprobe `creation_time`/`date` tags for videos) → date encoded in the filename → the file's own creation/modification time.
- In-app date/coordinate editing stays JPEG/PNG-only (`metadata_writer::update_metadata` and `frontend/src/lib/utils.js::isFormatSupported`). Every other type must state the limitation and write nothing — both already exist; do not widen them.
- i18n: every key added or removed lands in **both** `frontend/src/i18n/en.json` and `de.json`; `npm run test:i18n` must stay green and the bundles structurally identical.
- Never edit an applied migration (SQLx verifies SHA-384 checksums). The schema change is the new file `migrations/20260928000001_drop_taken_at_and_location_coordinates.sql`.
- Build order when frontend files change: `npm run build` first, then `cargo build --bin turbo-pix` (build.rs embeds `dist/` and panics without it).
- Per-task gates: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`; `npm run format:check && npm run lint && npm run build` when frontend files changed; `npm run test:unit && npm run test:i18n`. Zero warnings policy.
- E2E runs in Tasks 1, 7 and 8 only (`npm run test:e2e`). With a cold model cache pre-seed it: `./target/debug/turbo-pix --download-models`. Let a previous run's teardown settle before re-running; a failed first run on a dirty harness is infra — re-run once before suspecting the app.
- Because every date in the E2E library is file-seeded after Task 1, a regression there fails global setup (`Indexing did not reach is_complete`, `Failed to update photo dates`), not a spec assertion.

## Review Focus

These are the input classes and failure modes this change creates that the spec implies and that are easiest to get wrong. Each line names the owning task's test.

1. **A photo with no facts** (file unreadable, deleted mid-scan, or the index not yet populated after a restart) must never be shown an invented or stale date and must never match a month-range filter — Task 3 (`enrich_leaves_unknown_photos_undated`) and Task 4 (`test_sort_photos_orders_unknown_dates_like_sql`, `test_search_photos_range_ignores_unknown_dates`).
2. **Videos can never acquire map coordinates** — the extractor reads no GPS from video files and the DB-only path is gone; the removed E2E case is replaced by the existing `?q=type:video` empty-state assertion — Task 1 (deleted `map-filters` video-location test) and Task 4 (`test_map_photos_returns_no_coordinates_for_a_video`).
3. **Coordinates at file precision**: a saved coordinate must be reported as the file's DMS round trip, never as the requested decimal, and never one coordinate without the other — Task 3 (`read_media_facts_round_trips_written_coordinates`) and Task 7 (map plots the saved photo at the file's precision).
4. **An unsupported or failed save writes nothing anywhere**: a video/RAW/WebP save attempt and a save whose file vanished must leave the index and the DB exactly as they were — Task 5 (`test_update_metadata_rejects_a_video_without_touching_anything`, `test_update_metadata_missing_file_keeps_the_index_unchanged`).
5. **A save must not persist date/coordinates**: a photo that is enriched and then written back (favorite toggle, the edit's own `updated_at` bookkeeping write) must still store no coordinate keys, and no date column exists to write — Task 6 (`test_db_writes_never_store_coordinates` covers both the plain and the enriched-then-updated write; Task 5 keeps `enrich` strictly after the DB write at every response site).

---

## File Structure

**Create**

- `src/media_facts.rs` — `MediaFacts`, `MediaFactsIndex` (in-memory, path-keyed), `read_media_facts*`, `enrich`, test helpers.
- `migrations/20260928000001_drop_taken_at_and_location_coordinates.sql` — drop `idx_photos_taken_at`, drop `photos.taken_at`, strip `$.location.latitude` / `$.location.longitude`.
- `tests/e2e/specs/metadata-edit.e2e.spec.js` — the new end-to-end proof that a save lands in the file, not the database.

**Modify (backend)**

- `src/lib.rs` — register `pub mod media_facts;`.
- `src/db.rs` — `Photo` field docs, `FromRow`, the four write statements, `build_order_clause`, `build_search_where`, `search_photos`, `list_with_pagination`, `list_all_filtered`, `get_timeline_data`, `get_photos_needing_geo_resolution`, `sort_photos`/month filter helpers, delete `update_from_extracted` and the coordinate accessors, `stored_metadata`.
- `src/photo_processor.rs` — unchanged-file facts read, facts publication during a scan, orphan-facts removal.
- `src/scheduler.rs` — hold the index, pass it into both scan paths and the geo phase.
- `src/main.rs` — construct the index, thread it into services and routes.
- `src/handlers_photo.rs` — facts in listings/map/timeline/routes, delete hooks, enrichment, file-only metadata PATCH, remove bulk date-shift.
- `src/handlers_video.rs`, `src/handlers_housekeeping.rs`, `src/handlers_albums.rs`, `src/albums.rs`, `src/handlers_collage.rs`, `src/collage_generator.rs`, `src/image_editor.rs`, `src/warp_helpers.rs` — facts plumbing/enrichment.
- Tests inside those modules plus `src/db_pool.rs`'s deletion helper if needed.

**Modify (frontend)**

- `frontend/src/components/SelectionBar.svelte` — remove the date-shift action, dialog, CSS.
- `frontend/src/lib/api.js` — remove `batchDateShift`.
- `frontend/src/lib/state.svelte.js` — remove the `'dateShift'` busy value from the comment/union.
- `frontend/src/i18n/en.json` + `de.json` — remove 5 keys.
- `frontend/src/components/PhotoViewer.svelte` — reload derived views when a saved date changed.
- `frontend/src/components/TimelineSlider.svelte` — refetch density on `photosReloadRequested`.

**Modify (E2E)**

- `tests/e2e/setup/global-setup.js`, `tests/e2e/setup/test-helpers.js`, `tests/e2e/specs/map-filters.e2e.spec.js`, `tests/e2e/specs/viewer.e2e.spec.js`, `tests/e2e/specs/batch-select.e2e.spec.js`.

**Leave alone**

- `.spec/batch-select-actions.md` and the other `.spec/*` files are historical design records; the new spec supersedes them. Do not rewrite history files, `CHANGELOG.md` (semantic-release owns it) or `test-data/` fixtures (they are inputs; only copied fixtures are rewritten).

---

### Task 1: E2E fixtures pin their dates inside the files

The E2E library's dates currently come from `UPDATE photos SET taken_at …` against the DB. That column disappears later; the fixtures must already carry their dates in the files before the server starts reading them (this task runs against the **unchanged** server, which still reads the DB and is written by PATCH — so the suite stays green).

**Files**

- Modify: `tests/e2e/setup/global-setup.js` (seed + `updateTestPhotoDates` + `verifyTestPhotoDates`)
- Modify: `tests/e2e/setup/test-helpers.js:88-125`
- Modify: `tests/e2e/specs/map-filters.e2e.spec.js` (delete the video-coordinate test, keep the `?q=type:video` empty-state premise)
- Modify: `tests/e2e/specs/viewer.e2e.spec.js:201-230`

**Interfaces**

- Consumes: nothing new.
- Produces: the fixture contract every later E2E run depends on —
  - `cluster_*.jpg` / `archive_*.jpg` / `legacy_*.jpg` carry their pinned EXIF dates,
  - every seeded video carries its pinned date in the container (`creation_time` for MP4/MKV, `date` for the AVI),
  - no helper writes `taken_at` or coordinates through `sqlite3` any more.

- [ ] **Step 1: Add the remux helper and pin every seeded video's date**

In `global-setup.js` replace `copyFile` with a remux for the eight videos that `updateTestPhotoDates` pins today (`test_video.mp4` +1, `test_video_long.mkv` +3, `test_video_ac3.mp4` +4, `test_video_moov_end.mp4` +5, `test_video_10bit.mp4` +6, `test_video_multitrack.mp4` +7, `test_video_noaudio.mp4` +8, `test_video_legacy.avi` +9 — all `CLUSTER_DAYS_AGO + N`):

```js
// A video's taken_at must come out of the container; the DB pin it used to
// come from is gone. ffmpeg writes the tag ffprobe reads (creation_time for
// mov/mp4/matroska, the generic `date` tag for AVI which has no
// creation_time). No -f: the muxer is inferred from the destination extension
// (`mkv` needs the `matroska` muxer, which only inference gets right).
function pinVideoDate(source, destination, date) {
  const iso = date.toISOString();
  const tag = path.extname(destination) === '.avi' ? 'date' : 'creation_time';
  execFileSync(ffmpegPath, ['-v', 'error', '-y', '-i', source, '-c', 'copy',
    '-metadata', `${tag}=${iso}`, destination]);
}
```

Keep the existing `utimes` calls (the mtime still keys the conversion cache); the date now also lives in the container. `seedMultitrackFixture` must set the same tag; `reseedNonProgressiveFixture` must seed through the same helper instead of `copyFile` (a plain `-c copy` writes mp4 with moov at the end, which is the premise that fixture exists for).

- [ ] **Step 2: Verify the pinned dates really land in the files**

Run: `cd /tmp && ffmpeg -v error -y -i <repo>/test-data/test_video_legacy.avi -c copy -metadata date=2018-01-01T12:00:00Z out.avi && ffprobe -v error -show_entries format_tags=date -of default=noprint_wrappers=1:nokey=1 out.avi` (and the matching `creation_time` probe for an MP4).
Expected: `2018-01-01T12:00:00Z` — the extractor's `parse_video_creation_time` (`src/metadata_extractor.rs:438-486,511`) reads exactly these tags.

- [ ] **Step 3: Replace the SQL date seeding with PATCH seeding**

Rewrite `updateTestPhotoDates(baseURL)` to keep its name and signature but seed through the product path:

```js
async function updateTestPhotoDates(baseURL) {
  const response = await fetch(`${baseURL}/api/photos?limit=200`);
  if (!response.ok) throw new Error(`Failed to list photos: ${response.statusText}`);
  const { photos } = await response.json();
  // PATCH writes the file, so this is the product's own write path; the old
  // sqlite3 block (and the video rows in it) is gone. Videos need no PATCH —
  // their dates were pinned in seedTestMedia.
  const legacy = LEGACY_PHOTOS.map(([filename, takenAt]) => ({ match: (p) => p.filename === filename, takenAt }));
  const groups = [
    { match: (p) => p.filename?.startsWith('cluster_'), takenAt: recentDate.toISOString() },
    { match: (p) => p.filename?.startsWith('archive_'), takenAt: archiveDate.toISOString() },
    ...legacy,
  ];
  for (const { match, takenAt } of groups) {
    for (const photo of photos.filter(match)) {
      const patched = await fetch(`${baseURL}/api/photos/${photo.hash_sha256}/metadata`, {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ taken_at: takenAt }),
      });
      if (!patched.ok) throw new Error(`Failed to pin ${photo.filename}: ${patched.status}`);
    }
  }
}
```

Delete the whole `sqlite3 … UPDATE photos SET taken_at` block (including the `videoTakenAt` list and the `legacySql` builder). Keep the function's position in `globalSetup` (after `waitForIndexingComplete`, before `verifyTestPhotoDates`).

- [ ] **Step 4: Extend `verifyTestPhotoDates` to prove the video pin**

Add one assertion to the existing verification (it already fetches `/api/photos?limit=200`): the first pinned video's API date must start with its pinned `YYYY-MM-DD` (compute it from `CLUSTER_DAYS_AGO + 1`). Throw with the filename and both values when it does not — this is what catches a remux that silently failed to write the tag.

- [ ] **Step 5: Delete the DB coordinate helpers and the video-location test**

In `test-helpers.js` delete `setPhotoLocationInDb` and `clearPhotoLocationInDb` (imports `execSync` / `TEST_DB_PATH` too if they become unused). Keep `setPhotoCoordinates` (it goes through the PATCH endpoint, which is the file path).

In `map-filters.e2e.spec.js` delete `test('videos with coordinates are plotted and open in the viewer')` entirely, with a comment where the test stood: videos carry no file-level coordinates, so a video can never be plotted — the shell spec's `?q=type:video` empty-state assertion already covers the negative.

- [ ] **Step 6: Drop `taken_at` from the viewer spec's re-insert**

In `viewer.e2e.spec.js:201-230` remove `taken_at` from the `INSERT OR REPLACE INTO photos (…)` column list and the `deletedPhoto.taken_at ?? null` value. The column is nullable, so this works before and after the drop.

- [ ] **Step 7: Run the suite on the unchanged server**

Run: `rm -rf test-e2e-data && npm run test:e2e`
Expected: PASS (all specs) — proving the fixtures carry their dates and no spec depends on the removed helpers.

- [ ] **Step 8: Commit**

```bash
git add tests/e2e
git commit -m "test(e2e): pin fixture dates inside the media files"
```

---

### Task 2: Remove the bulk date-shift capability end to end

**Files**

- Modify: `src/handlers_photo.rs:444-463,913-960,1219-1228,1368-1370,2351-2412`
- Modify: `frontend/src/components/SelectionBar.svelte`
- Modify: `frontend/src/lib/api.js:434-438`
- Modify: `frontend/src/lib/state.svelte.js:42`
- Modify: `frontend/src/i18n/en.json`, `frontend/src/i18n/de.json`
- Modify: `tests/e2e/specs/batch-select.e2e.spec.js:156-207`

**Interfaces**

- Consumes: nothing.
- Produces: `BatchResult` without the `skipped` field; no `/api/photos/batch/date-shift` route.

- [ ] **Step 1: Delete the backend handler, request type, route and tests**

Remove `BatchDateShiftRequest`, `batch_date_shift`, the `api_photo_batch_date_shift` filter and its `.or(…)` entry, and the tests `test_batch_date_shift_moves_and_skips` / `test_batch_date_shift_zero_days_rejected` (plus `BATCH_H1..H3` if nothing else uses them). Remove the `skipped` field from `BatchResult` and the `skipped: Vec::new()` initialisers in `batch_delete` / `batch_favorite` — `skipped` existed only for this action.

- [ ] **Step 2: Delete the frontend action, dialog, helper and busy value**

In `SelectionBar.svelte` remove the two `dateShift` entries in `actionConfig`, the `actionDataName` mapping, `dateShiftOpen`, `daysInput`, `daysValid`, the `runAction` case, `applyDateShift`, the `.date-shift-row` markup and its CSS, and the `res.skipped` toast branch inside the success path. In `api.js` delete `batchDateShift`. In `state.svelte.js` delete `'dateShift'` from the busy comment/union. `ui.working` and `errors.batchActionFailed` stay (shared).

- [ ] **Step 3: Delete the five i18n keys from both bundles**

`ui.shift_dates`, `ui.days`, `ui.apply` (en.json:191,195,196 / de.json same lines) and `notifications.batchDateShifted`, `notifications.batchSkippedNoDate` (en.json:240,241 / de.json same lines). Keep `ui.export`, `ui.exporting`, `ui.working`, `ui.cancel_selection`, `notifications.batchUnfavorited`, `notifications.batchExported`, `errors.batchActionFailed`.

- [ ] **Step 4: Delete the E2E date-shift test**

Remove `test('batch date-shift applies and reports')` from `batch-select.e2e.spec.js` and the seed-restore comment at the top of the file. Do not replace it: the capability no longer exists.

- [ ] **Step 5: Run the gates and smoke the removal**

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
npm run test:i18n && npm run test:unit && npm run build && cargo build --bin turbo-pix
```

Then start the binary and confirm the route is gone and the remaining bulk actions still work:

```bash
nohup cargo run & sleep 3
curl -s -o /dev/null -w '%{http_code}\n' -X POST localhost:18473/api/photos/batch/date-shift \
  -H 'Content-Type: application/json' -d '{"hashes":["x"],"days":1}'   # expect 404
curl -s -X POST localhost:18473/api/photos/batch/favorite \
  -H 'Content-Type: application/json' -d '{"hashes":[],"is_favorite":true}'  # expect 400 (validation), route alive
pkill -f 'target/debug/turbo-pix'
```

Also grep the frontend for leftovers: `grep -rn "dateShift\|shift_dates\|batchDateShifted\|batchSkippedNoDate\|batch-days-input" frontend/src tests/e2e` → no hits.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat!: remove bulk date-shift"
```

---

### Task 3: `src/media_facts.rs` — the file-derived facts index

**Files**

- Create: `src/media_facts.rs`
- Modify: `src/lib.rs` (add `pub mod media_facts;` in alphabetical order, after `image_editor`)

**Interfaces**

- Consumes: `crate::metadata_extractor::MetadataExtractor::extract_with_metadata(&Path, Option<&std::fs::Metadata>) -> PhotoMetadata`; `crate::db::Photo`.
- Produces (later tasks rely on these exact names):

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MediaFacts {
    pub taken_at: Option<DateTime<Utc>>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

/// Process-wide in-memory index of file-derived capture facts, keyed by the
/// photo's file path (a path survives the in-app content-hash re-key on
/// rotation and is what every DB row already carries).
pub struct MediaFactsIndex { /* RwLock<HashMap<String, MediaFacts>> */ }

impl MediaFactsIndex {
    pub fn new() -> Self;
    pub fn get(&self, file_path: &str) -> Option<MediaFacts>;
    pub fn set(&self, file_path: &str, facts: MediaFacts);
    pub fn remove(&self, file_path: &str);
    pub fn remove_many(&self, file_paths: &[String]);
    /// Reads the file and stores what it yields; returns that value.
    pub fn reload(&self, file_path: &str) -> MediaFacts;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    /// Attach the file's facts to `photo` for a response: `taken_at` and the
    /// coordinate pair inside `metadata.location` (merging, never dropping an
    /// existing `city`). A photo with no index entry stays undated and keeps
    /// its metadata untouched. Response-only: the DB has no taken_at column
    /// and strips coordinate keys before storing.
    pub fn enrich(&self, photo: &mut Photo);
}

/// Facts of `path`, with the indexing fallback order (embedded → filename →
/// file timestamp).
pub fn read_media_facts(path: &Path) -> MediaFacts;
pub fn read_media_facts_with_metadata(path: &Path, file_metadata: Option<&std::fs::Metadata>) -> MediaFacts;

#[cfg(test)]
pub(crate) fn test_facts(entries: &[(&str, &str)]) -> MediaFactsIndex;            // (path, RFC3339 date)
#[cfg(test)]
pub(crate) fn test_facts_with_coords(entries: &[(&str, &str, f64, f64)]) -> MediaFactsIndex;
```

Lock poisoning: recover with `unwrap_or_else(|poisoned| poisoned.into_inner())` on both read and write (repo convention: never `.unwrap()` a lock).

- [ ] **Step 1: Write the failing tests for the reader**

In a `#[cfg(test)] mod tests` at the bottom of the new module (helpers: `TempDir`, `chrono::TimeZone::with_ymd_and_hms`, `crate::metadata_writer::update_metadata`, `test-data/IMG_9377.jpg`):

```rust
#[test]
fn read_media_facts_round_trips_written_date_and_coordinates() {
    // copy test-data/IMG_9377.jpg to a temp dir, write 2024-03-15T14:30:00Z / 40.7128 / -74.0060
    // assert taken_at == exactly the written instant (second precision, UTC — no offset drift)
    // assert (lat, lng) within 1e-4 of the written values (DMS is the file's precision)
}

#[test]
fn read_media_facts_does_not_drift_across_rewrites() {
    // write → read → write the read-back value to the same file → read again
    // assert the two read-backs are equal
}

#[test]
fn read_media_facts_falls_back_to_the_filename_date() {
    // std::fs::write a temp file named `20240215_185056.jpg` with bytes that are not an image
    // assert taken_at == 2024-02-15T18:50:56Z  (existing fallback order, FR-011)
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test media_facts -- --nocapture`
Expected: FAIL to compile (`media_facts` module/function missing).

- [ ] **Step 3: Implement the reader**

`read_media_facts_with_metadata` calls `MetadataExtractor::extract_with_metadata(path, file_metadata)` and copies `taken_at` / `latitude` / `longitude`; `read_media_facts` delegates with `std::fs::metadata(path).ok().as_ref()`. Do **not** reimplement the fallback chain — reusing the extractor is what keeps indexing and the file-only read in agreement.

- [ ] **Step 4: Write the failing tests for the index and enrichment**

```rust
#[test]
fn enrich_sets_the_date_and_merges_coordinates() {
    // facts index with test_facts_with_coords([("/tmp/a.jpg", "2012-03-15T10:00:00Z", 52.52, 13.405)])
    // photo.metadata = {"location":{"city":"Berlin"}}
    // after enrich: taken_at set; location.city == "Berlin"; location.latitude/longitude appended
}

#[test]
fn enrich_leaves_unknown_photos_undated() {
    // empty index, photo.metadata = {"location":{"city":"Berlin"}}
    // after enrich: taken_at.is_none() and metadata is EXACTLY unchanged (no lat/lng keys)
}

#[test]
fn index_set_remove_and_len() {
    // set two paths → len 2; remove one → len 1 and get() returns None; remove_many removes the rest
}
```

- [ ] **Step 5: Implement the index and `enrich`; run the tests**

`enrich` reads `self.get(&photo.file_path)`, writes `photo.taken_at`, and (only when both coordinates are present) ensures `metadata` is an object, ensures `metadata["location"]` is an object, and inserts both keys. Never remove keys it did not add.

Run: `cargo test media_facts` — Expected: PASS.

- [ ] **Step 6: Gate and commit**

```bash
cargo fmt && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
git add src/media_facts.rs src/lib.rs
git commit -m "feat: add file-derived media facts index"
```

---

### Task 4: Dates come from the files

After this task the DB's `taken_at` column is never written or read (it is dropped in Task 6), every scan republishes facts, and listing/filtering/timeline/collage ordering reads the index. The E2E library seeded in Task 1 keeps the suite green.

**Files**

- Modify: `src/db.rs` (queries, sort/filter helpers, `FromRow`, write SQL, `From<ProcessedPhoto>`)
- Modify: `src/photo_processor.rs` (unchanged-file facts read, publication, orphan removal)
- Modify: `src/scheduler.rs` (field + both scan paths + geo call to be wired in Task 5)
- Modify: `src/main.rs` (construct and thread the index)
- Modify: `src/handlers_photo.rs`, `src/warp_helpers.rs` (`with_facts` filter), `src/albums.rs`, `src/handlers_albums.rs`, `src/collage_generator.rs`, `src/handlers_collage.rs`, `src/image_editor.rs`
- Tests in the same modules

**Interfaces**

- Consumes: `MediaFacts`, `MediaFactsIndex`, `read_media_facts_with_metadata`, `test_facts` (Task 3).
- Produces:

```rust
// db.rs
impl From<&crate::photo_processor::ProcessedPhoto> for MediaFacts;   // in photo_processor.rs
pub(crate) fn sort_photos(photos: &mut [Photo], sort: Option<&str>, order: Option<&str>);
pub(crate) fn photo_in_month_range(photo: &Photo, bounds: &(String, String)) -> bool;
impl Photo {
    pub async fn search_photos(pool: &DbPool, facts: &MediaFactsIndex, query: &SearchQuery,
                               limit: i64, offset: i64, sort: Option<&str>, order: Option<&str>)
        -> Result<(Vec<Photo>, i64), Box<dyn std::error::Error>>;
    pub async fn list_with_pagination(pool: &DbPool, facts: &MediaFactsIndex, limit: i64, offset: i64,
                                      sort: Option<&str>, order: Option<&str>)
        -> Result<(Vec<Photo>, i64), Box<dyn std::error::Error>>;
    pub async fn list_all_filtered(pool: &DbPool, facts: &MediaFactsIndex, query: &SearchQuery,
                                   sort: Option<&str>, order: Option<&str>, album: Option<i64>)
        -> Result<Vec<Photo>, Box<dyn std::error::Error>>;
    pub async fn get_timeline_data(pool: &DbPool, facts: &MediaFactsIndex)
        -> Result<TimelineData, Box<dyn std::error::Error>>;
}
// albums.rs
pub async fn photos_for_album(pool: &DbPool, facts: &MediaFactsIndex, album_id: i64, limit: i64,
                              offset: i64, sort: Option<&str>, order: Option<&str>)
    -> Result<(Vec<Photo>, i64), Box<dyn std::error::Error>>;
// collage_generator.rs
pub async fn generate_collages(pool: &DbPool, facts: &MediaFactsIndex, data_path: &Path, locale: &str)
    -> Result<usize, Box<dyn std::error::Error>>;
// image_editor.rs
pub async fn delete_photo(photo: &Photo, db_pool: &DbPool, cache_manager: &CacheManager,
                          facts: &MediaFactsIndex) -> Result<(), ImageEditError>;
// warp_helpers.rs
pub fn with_facts(facts: Arc<MediaFactsIndex>) -> impl Filter<Extract = (Arc<MediaFactsIndex>,), Error = Infallible> + Clone;
// photo_processor.rs
impl PhotoProcessor {
    pub async fn full_rescan_and_cleanup(&self, db_pool: &DbPool, cache_manager: &CacheManager,
                                         status: &IndexingStatus, facts: &MediaFactsIndex)
        -> Result<Vec<ProcessedPhoto>, Box<dyn std::error::Error>>;
}
// scheduler.rs
impl PhotoScheduler {
    pub fn new(photo_paths: Vec<PathBuf>, db_pool: DbPool, cache_manager: CacheManager,
               semantic_search: Arc<dyn SemanticSearch>, data_path: PathBuf, locale: String,
               nominatim_url: String, media_facts: Arc<MediaFactsIndex>) -> Self;
}
// route builders gain a trailing media_facts: Arc<MediaFactsIndex>
```

- [ ] **Step 1: Write the failing unit tests for the comparator and month filter**

In `src/db.rs`'s test module:

```rust
#[test]
fn test_sort_photos_orders_unknown_dates_like_sql() {
    // three photos, facts dates 2012-03-15 and 2015-08-31, one with no entry
    // asc  -> [unknown, 2012, 2015]   (SQLite: NULL first ASC)
    // desc -> [2015, 2012, unknown]   (SQLite: NULL last DESC)
}

#[test]
fn test_sort_photos_tiebreaks_by_hash_like_sql() {
    // equal dates, hashes "aa…" < "bb…": asc -> aa,bb ; desc -> bb,aa
}

#[test]
fn test_sort_photos_supports_filename_and_size() { /* asc/desc on filename and file_size */ }

#[test]
fn test_photo_in_month_range_uses_lexicographic_year_month() {
    // month_range_bounds(2012,3,Some(2015),None) == ("2012-03","2015-12")
    // 2012-03-15 matches, 2015-12-31 matches, 2012-02-29 does not, unknown does not
}
```

- [ ] **Step 2: Run them; implement `sort_photos` / `photo_in_month_range`; run again**

Run: `cargo test db::tests::test_sort_photos` — Expected: FAIL (missing), then PASS.
`month_range_bounds` stays exactly as it is; only the SQL clause is replaced by the Rust predicate.

- [ ] **Step 3: Port the DB date tests to the facts index (red first)**

Rewrite these existing tests so no `Photo` carries a date in the DB: `test_get_timeline_data` (4 facts entries incl. the 2010-05/2011-12/2024-01 buckets), `test_get_timeline_data_empty`, `test_search_photos_filters_inclusive_month_range` (6 entries, same totals 1/3/4/2/0/6/6/0), `test_search_photos_range_ignores_null_taken_at` (the undated photo has **no index entry** — rename to `test_search_photos_range_ignores_unknown_dates`), `test_list_all_filtered_applies_search_tokens_and_year`, `test_list_all_filtered_returns_every_match_without_pagination` (120 photos, no coords needed — they assert the unpaginated set; keep them coordinate-free).

Delete the helpers that exist only to seed DB dates: `create_test_photo_with_date` and `dated_photo`; make `create_test_photo(filename: String, hash: String)` build the struct directly with `taken_at: None`, and switch their non-date callers (rotate/album/geo tests) to it. Add one test-module helper:

```rust
/// Builds the row and seeds its file-derived date, keeping the DB date-free.
async fn create_photo_with_facts(pool: &DbPool, facts: &MediaFactsIndex, hash: &str, filename: &str, taken_at: &str) -> Photo
```

Run: `cargo test db::tests` — Expected: FAIL until Steps 4-5 land.

- [ ] **Step 4: Rewrite the four query functions and both write-path SQL lists**

- `build_search_where`: delete the `strftime('%Y-%m', taken_at)` clause (the token grammar and city LIKE stay).
- `build_order_clause`: drop the `taken_at` arm and make the default `created_at`; document that date ordering is `sort_photos`'s job. `list_with_pagination` / `search_photos` / `list_all_filtered` / `photos_for_album` take the SQL fast path only when `month_range_bounds(…)` is `None` **and** `sort` is `Some("filename"|"name"|"file_size"|"size"|"created_at")`; otherwise they fetch every matching row with the filter WHERE, `facts.enrich` each row, `retain` by `photo_in_month_range`, `sort_photos`, then slice `skip(offset).take(limit)` and return the post-filter count as `total`.
- `get_timeline_data(pool, facts)`: `SELECT hash_sha256, file_path FROM photos`, look each path up in the index, and build `min_date`/`max_date` (`to_rfc3339()`) and `density` (`density[].year/month` from `taken_at`, `count` summed; ordered by year then month).
- `FromRow`: stop reading `taken_at` (leave `None`); INSERT / UPDATE / `update_with_old_hash` / UPSERT: remove the `taken_at` column and bind.
- `From<ProcessedPhoto> for Photo`: `taken_at: None` (the location block stays until Task 6).

Run: `cargo test db::tests` — Expected: PASS.

- [ ] **Step 5: Tests and wiring for the scan**

In `photo_processor.rs`:
- Unchanged-file branch: replace the DB reads `existing_photo.taken_at/latitude()/longitude()` with `let facts = read_media_facts_with_metadata(path, Some(&photo_file.metadata));` and use `facts.taken_at/latitude/longitude`.
- After each completed metadata task (`record_metadata_task_outcome` returning a photo) call `facts.set(&photo.file_path, MediaFacts::from(&photo))`; add `impl From<&ProcessedPhoto> for MediaFacts`.
- After `delete_orphaned_photos` returns `deleted_paths`, call `facts.remove_many(&paths)` where `paths` are the deleted rows' `file_path`s.

Tests (BDD style, using a temp photo dir and `create_in_memory_pool`):

```rust
#[tokio::test]
async fn test_rescan_publishes_facts_for_unchanged_files() {
    // GIVEN a photo row whose file is unchanged (size+mtime match) and an EMPTY facts index
    // WHEN full_rescan_and_cleanup runs
    // THEN facts.get(path).taken_at == the date the file carries (EXIF/filename fallback)
}
#[tokio::test]
async fn test_rescan_picks_up_an_external_date_change() {
    // GIVEN facts for a file's old date (the row's size/mtime also match the OLD file)
    // WHEN the file's EXIF date is rewritten outside the app (metadata_writer) and a rescan runs
    // THEN the index carries the NEW date (FR-013/SC-006)
}
#[tokio::test]
async fn test_rescan_removes_facts_of_orphaned_photos() { /* file gone → index entry gone */ }
```

- [ ] **Step 6: Keep the date a video file carries when the scan faststarts it**

`maybe_fix_moov_for_video` rewrites every non-progressive video **in place**, and its remux command (`ffmpeg -c copy -movflags +faststart`, `src/video_processor.rs:786-800`) drops the container's `creation_time` tag: ffmpeg does not carry that tag without `-map_metadata 0` (verified in this worktree: a tagged MP4 → plain remux → tag gone; → `+faststart` remux → tag gone; → `-map_metadata 0 -movflags +faststart` → tag kept). After this task the file is the only place the date lives, so the app's own rewrite must not destroy it — otherwise the next scan re-reads a tagless file and silently re-dates the video from its birth time.

Add `-map_metadata 0` to `fix_moov_atom`'s argument list and a test next to the existing `fix_moov_atom` tests (`src/video_processor.rs:~2117`, reuse their fixture/lock pattern):

```rust
#[tokio::test]
async fn test_fix_moov_atom_preserves_creation_time() {
    // seed a moov-at-end MP4 whose format carries a pinned creation_time
    // WHEN fix_moov_atom runs
    // THEN has_moov_at_start is true AND the file's format creation_time is unchanged
}
```

Run: `cargo test video_processor::tests::test_fix_moov_atom_preserves_creation_time` — Expected: FAIL (tag dropped) before the flag, PASS after.

- [ ] **Step 7: Wire the index through scheduler, main and routes**

- `PhotoScheduler` gains `media_facts: Arc<MediaFactsIndex>` and passes `&self.media_facts` into `full_rescan_and_cleanup` in both `start()` and `run_startup_rescan()`.
- `main.rs`: `let media_facts = Arc::new(MediaFactsIndex::new());` before `initialize_services`, pass it in, then pass clones to `build_photo_routes`, `build_albums_routes`, `build_collage_routes` (and, in Task 5, `build_housekeeping_routes`).
- Add `warp_helpers::with_facts`; in `build_photo_routes` add `.and(with_facts(media_facts.clone()))` to the routes whose handlers need it **in this task** (`/api/photos`, `/api/photos/map`, `/api/photos/timeline`, `PATCH /api/photos/{hash}/metadata`, single delete, batch delete) and thread it through `fetch_photos`, `list_photos`, `list_map_photos`, `get_timeline`, `delete_photo`, `batch_delete`, `update_photo_metadata`. Do **not** add the filter/parameter to `get_photo`, `toggle_favorite`, `rotate_photo`, `get_video_file` or the housekeeping listing yet — Task 5 does that together with the enrichment call in each body (adding a parameter no body uses trips clippy's `unused_variables` under `-D warnings`).
- `image_editor::delete_photo(photo, pool, cache_manager, facts)` ends with `facts.remove(&photo.file_path)`.
- `albums::photos_for_album` + `handlers_albums::list_album_photos`: fetch all member rows, enrich, sort, page (no SQL date ordering).
- `collage_generator::find_photo_clusters(pool, facts)`: fetch all photos, enrich, keep `taken_at >= now - 30 days`, group by `taken_at.date_naive()`, keep groups of ≥ 10, sort each group by `taken_at`; `generate_collages(pool, facts, data_path, locale)` threads it, `handlers_collage::generate_collages_manual` passes it.
- `handlers_photo::update_photo_metadata` (minimal version now): after a successful `metadata_writer::update_metadata`, call `facts.reload(&photo.file_path)` so the index immediately reflects the file even before Task 5 reworks the response.

- [ ] **Step 8: Port the affected route/collage tests and run the full Rust suite**

`src/handlers_photo.rs`: `create_dated_photo_row` becomes `create_photo_row_at(…, taken_at: &str) -> (String, PathBuf)` that seeds the row **and** a passed-in index; `test_list_photos_month_range_filter` and `test_map_photos_applies_filters_and_sort` build routes with `with_facts(Arc::new(index))` and seed the dates they assert on; `test_map_photos_returns_all_matches_beyond_page_limit` asserts the same coordinates as before via the index (`setPhotoCoordinates`-equivalent is Task 5, so seed coords in the index).

`src/collage_generator.rs`: `insert_photo(pool, path, hash_seed)` plus an explicit facts entry, and `mock_photo` drops its date; the two collage tests keep their `generated == 1` / `== 2` expectations.

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

- [ ] **Step 9: Measure the browsing budget (throwaway), then re-run the E2E ordering specs**

SC-003 wants a 5,000-photo library to answer a date sort, a month-range filter and the timeline within 1 s. Add a temporary `#[ignore]`d test to `src/db.rs`'s test module: insert 5,000 rows into an in-memory pool, seed 5,000 index entries, print the wall time of `search_photos(…, sort=Some("date"), order=Some("desc"))`, of `search_photos` with a `year`/`month` filter, and of `get_timeline_data`. Run it with `cargo test db::tests::test_scale_search_budget -- --ignored --nocapture`, record the three numbers in the commit message, then **delete the test** — it is a measurement, not a deliverable.

Run: `rm -rf test-e2e-data && npm run test:e2e -- specs/timeline.e2e.spec.js specs/url-routing.e2e.spec.js specs/batch-select.e2e.spec.js`
Expected: PASS — dates now come from the files through the index.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "feat!: serve photo dates from the files"
```

---

### Task 5: Every response and the geo phase read the files

**Files**

- Modify: `src/handlers_photo.rs` (PATCH handler, get/toggle/rotate), `src/handlers_video.rs`, `src/handlers_housekeeping.rs`, `src/db.rs` (geo query), `src/scheduler.rs` (geo phase), `src/main.rs`
- Tests in those modules

**Interfaces**

- Consumes: everything from Tasks 3-4.
- Produces:

```rust
pub async fn get_photos_needing_geo_resolution(pool: &DbPool, facts: &MediaFactsIndex)
    -> Result<Vec<(String, f64, f64)>, Box<dyn std::error::Error>>;   // still (file_path, lat, lng)
pub async fn update_photo_metadata(photo_hash: String, metadata_req: MetadataUpdateRequest,
                                   db_pool: DbPool, facts: Arc<MediaFactsIndex>) -> Result<impl Reply, Rejection>;
```

- [ ] **Step 1: Write the failing handler tests**

In `src/handlers_photo.rs`'s test module, replace `test_update_photo_metadata_endpoint`'s DB read-back assertions with file assertions:

```rust
#[tokio::test]
async fn test_update_photo_metadata_writes_the_file_and_reports_it() {
    // GIVEN a JPEG row + empty index (handler called directly with facts)
    // WHEN PATCH {taken_at: "2024-03-15T14:30:00.123Z", latitude: 40.7128, longitude: -74.0060}
    // THEN metadata_writer read-back (read_media_facts) == 2024-03-15T14:30:00Z (seconds only)
    // AND the response's taken_at == 2024-03-15T14:30:00Z — the file's value, NOT the
    //     sub-second precision the request asked for (FR-006)
    // AND response metadata.location coordinates are within 1e-4 of the file's read-back
    // AND the row's hash_sha256, is_favorite and album_members rows are unchanged
}

#[tokio::test]
async fn test_update_metadata_rejects_a_video_without_touching_anything() {
    // video row (mime video/mp4) + index entry; handler returns an error;
    // index entry and file mtime unchanged
}

#[tokio::test]
async fn test_update_metadata_missing_file_keeps_the_index_unchanged() {
    // file deleted after indexing; PATCH errors; facts.get(path) still the old entry
}

#[tokio::test]
async fn test_two_saves_leave_the_index_at_the_last_file_value() {
    // PATCH date A, then PATCH date B; after both, facts.get(path).taken_at == B
    // and a third read of the API returns B (the file is the only source)
}
```

- [ ] **Step 2: Implement the file-only PATCH handler; run the tests**

Order inside the handler: parse `taken_at` (ValidationError on parse failure) → `metadata_writer::update_metadata(path, taken_at, latitude, longitude)` (range/pair errors stay `ValidationError`, everything else `DatabaseError` with the writer's message) → `facts.reload(&photo.file_path)` → `photo.updated_at = Utc::now(); photo.update(&db_pool).await` (bookkeeping only) → `facts.enrich(&mut photo)` → reply with the enriched photo. Delete the DB mutations of `taken_at` and of `metadata.location.latitude/longitude` and the now-dead `metadata.is_object()` guard.

Run: `cargo test handlers_photo::tests::test_update_metadata` — Expected: PASS.

- [ ] **Step 3: Enrich the remaining single-photo responses**

Add `facts.enrich(&mut photo)` immediately before the reply (never before a DB write) in: `get_photo`, `toggle_favorite`, `rotate_photo`; in `handlers_video::get_video_file`'s `?metadata=true` JSON (enrich the fetched photo before building `taken_at`); in `handlers_housekeeping::list_housekeeping_candidates` (thread `with_facts`, enrich each `HousekeepingCandidate.photo`). Each of those routes also needs the `with_facts` filter and its handler the extra `facts` parameter — Task 4 deliberately left them out. Add a test per surface:

```rust
#[tokio::test]
async fn test_get_photo_returns_the_files_date_and_coordinates() { /* route built with with_facts */ }
#[tokio::test]
async fn test_toggle_favorite_response_keeps_file_facts() { /* favorite flips, date/coords preserved */ }
#[tokio::test]
async fn test_video_metadata_payload_carries_the_files_date() { /* ?metadata=true */ }
#[tokio::test]
async fn test_housekeeping_candidates_carry_file_facts() { /* candidate photo has taken_at from the index */ }
```

- [ ] **Step 4: Derive geo resolution from the index**

`get_photos_needing_geo_resolution(pool, facts)` queries `SELECT file_path FROM photos WHERE geo_location_resolved IS NULL OR geo_location_resolved = 0`, then keeps only paths whose `facts.get(path)` has **both** coordinates. `run_geo_resolution_phase` passes `&self.media_facts`. Keep `mark_photo_geo_resolved` / `update_photo_city` untouched (labels stay cached, FR-010).

Port `test_get_photos_needing_geo_resolution`, `test_mark_photo_geo_resolved`, `test_update_photo_city` to seed coordinates in the index (`test_facts_with_coords`) instead of in `metadata`.

- [ ] **Step 5: Gate and commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

Sanity smoke (real server, real file): start `cargo run`, PATCH a JPEG's date through `curl`, then `curl /api/photos/{hash}` and confirm the returned date equals the saved value; stop the server.

```bash
git add -A
git commit -m "feat!: serve dates/coordinates from files in every response"
```

---

### Task 6: The database stores no dates and no coordinates

**Files**

- Create: `migrations/20260928000001_drop_taken_at_and_location_coordinates.sql`
- Modify: `src/db.rs` (`stored_metadata`, delete `update_from_extracted` and the coordinate accessors), `src/photo_processor.rs` (`From<ProcessedPhoto>` location block)
- Tests: `src/db.rs`

**Interfaces**

- Consumes: Task 5's file-only edit path.
- Produces: `fn stored_metadata(metadata: &serde_json::Value) -> String` (db.rs, private) — the single serializer used by all four write statements.

- [ ] **Step 1: Write the failing storage tests**

```rust
#[tokio::test]
async fn test_photos_schema_has_no_taken_at_column() {
    // PRAGMA table_info(photos) names contain no "taken_at"; sqlite_master has no idx_photos_taken_at
}
#[tokio::test]
async fn test_db_writes_never_store_coordinates() {
    // Photo with metadata {"location":{"latitude":48.1,"longitude":11.5,"city":"Munich"}}
    // create() -> SELECT metadata -> no latitude/longitude keys, city preserved
    // then enrich a copy from an index with coords, update() -> same assertion
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test db::tests::test_photos_schema_has_no_taken_at_column db::tests::test_db_writes_never_store_coordinates`
Expected: FAIL — column still present, coordinates still stored.

- [ ] **Step 3: Add the migration**

```sql
-- Capture dates and coordinates live in the photo files only (spec
-- .spec/remove-bulk-date-shift-and-file-only-dates.md, FR-001). The stored
-- copies were never authoritative: they are dropped, not migrated.
DROP INDEX IF EXISTS idx_photos_taken_at;
ALTER TABLE photos DROP COLUMN taken_at;
UPDATE photos SET metadata = json_remove(metadata, '$.location.latitude', '$.location.longitude');
```

- [ ] **Step 4: Route every metadata write through `stored_metadata`**

`stored_metadata` clones the value, removes `$.location.latitude` / `$.location.longitude` when `location` is an object, and serializes. Use it for `.bind(self.metadata.to_string())` in `create_with_transaction`, `update_with_transaction`, `update_with_old_hash`, `create_or_update_with_transaction`. Delete `Photo::latitude()` / `Photo::longitude()` and `Photo::update_from_extracted` (dead), and remove the `location` block (latitude/longitude) from `From<ProcessedPhoto> for Photo`.

- [ ] **Step 5: Add the migration data-cleanup test**

In `src/db.rs`'s tests, build a temp **file** DB: first call a new `#[cfg(test)] pub(crate) fn register_vector_extension()` that `src/db_pool.rs` factors out of `create_in_memory_pool` (migration 2 creates a `vec0` virtual table, so the sqlite-vec extension must be registered before the old migrations run — registration is process-global, so call it explicitly rather than hoping another test ran first). Then execute the ten pre-existing migration files in order with `sqlx::raw_sql(include_str!("../migrations/<file>"))` (they are plain DDL), insert a photo row carrying `taken_at` and `metadata` with coordinates, then execute `include_str!("../migrations/20260928000001_drop_taken_at_and_location_coordinates.sql")` and assert: the column is gone and `SELECT metadata` has neither coordinate key. (This is the only test that pins the migration's row cleanup; keep it.)

- [ ] **Step 6: Run the full gates**

```bash
cargo fmt && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

Also delete the DB once by hand to prove a fresh start: `rm -rf data/database && cargo run` briefly (or run the E2E library fresh) — a database created by the migrated schema must index and serve dates.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat!: drop stored photo dates and coordinates"
```

---

### Task 7: Client refresh after an edit + the metadata-edit E2E

**Files**

- Modify: `frontend/src/components/PhotoViewer.svelte:1849-1857`
- Modify: `frontend/src/components/TimelineSlider.svelte:117-121`
- Create: `tests/e2e/specs/metadata-edit.e2e.spec.js`

**Interfaces**

- Consumes: `photoUpdated` / `photosReloadRequested` window events (existing), `PATCH /api/photos/{hash}/metadata` (existing route).
- Produces: after a saved **date** change the grid and the timeline density re-derive from the server; the E2E spec pins SC-001/SC-005.

- [ ] **Step 1: Refresh the derived views when the date changed**

`PhotoViewer.onMetadataSaved(updatedPhoto)`: keep the in-place replacement and the `photoUpdated` dispatch, and additionally, when `updatedPhoto.taken_at !== currentPhoto?.taken_at` before the assignment, dispatch `new CustomEvent('photosReloadRequested')` so the grid re-fetches the server's order and an active month filter drops a photo that moved out of it. Coordinates need no reload — `MapView.handlePhotoUpdated` already recomputes locations.

- [ ] **Step 2: Let the timeline density refetch on that event**

In `TimelineSlider.svelte`'s mount effect (`$effect(() => { fetchTimelineData(); })`), register a `photosReloadRequested` listener that calls `fetchTimelineData()` and return the cleanup that removes it. Keep the effect's dependency set empty (the listener is not a reactive read).

- [ ] **Step 3: Write the new E2E spec (red first)**

`tests/e2e/specs/metadata-edit.e2e.spec.js`:

```js
test('saving a date lands in the file, never in the database', async ({ page }) => {
  // GIVEN the `sample_with_exif.jpg` fixture (no other spec depends on its date)
  // read its current API taken_at/lat/lng; open the viewer via ?photo=<hash>
  // WHEN the metadata edit modal saves a new date (2024-05-06 14:30)
  // THEN the viewer shows the value read back from the file
  // AND `sqlite3 test-e2e-data/database/turbo-pix.db "PRAGMA table_info(photos);"` has no taken_at column
  // AND the row's metadata JSON contains no latitude/longitude key
  // WHEN the page is reloaded and the API is read again
  // THEN the date is unchanged (the file, not the request, is the source)
  // finally: restore the original date through the same modal
});

test('a video cannot be edited and the interface says so', async ({ page }) => {
  // GIVEN a video fixture open in the viewer
  // THEN #metadata-edit-btn is disabled and its accessible name contains 'not supported'
});
```

The coordinate half of the first test edits the same fixture: after saving coordinates the API must return the file's DMS round trip (within 1e-4 of the requested decimal) and the map must plot the photo there. Restore the *date* in a `finally`; the coordinates stay for the rest of the run on purpose — clearing a coordinate is unsupported by design, and the map specs compute their geo-located count dynamically (`map.e2e.spec.js`'s "plots every geo-located photo" reads the expected number from the API and anchors on the densest location), so one extra located photo cannot break them. Say so in a comment next to the seeding call.

- [ ] **Step 4: Run it**

```bash
npm run build && cargo build --bin turbo-pix
npm run test:e2e -- specs/metadata-edit.e2e.spec.js specs/metadata.e2e.spec.js specs/map.e2e.spec.js
```

Expected: PASS.

- [ ] **Step 5: Run the frontend gates and commit**

```bash
npm run format:check && npm run lint && npm run test:unit && npm run test:i18n
npm run build && cargo build --bin turbo-pix
git add -A
git commit -m "feat: refresh derived views after a metadata edit"
```

---

### Task 8: Learnings and the full gate

**Files**

- Modify: `AGENTS.md` (Learnings section only)
- No production code changes: any failure here sends you back to the owning task

**Interfaces**

- Consumes: everything.
- Produces: the branch's final verified state.

- [ ] **Step 1: Fold the session's learnings into the capped Learnings section**

Keep the section at 10 entries; fold, never append. Required content:
- Entry 2 (state & routing) and entry 10 (E2E & test harness): `taken_at` no longer exists as a stored value; the E2E fixture contract is file-seeded (EXIF through PATCH for photos, `-metadata creation_time=` / `date=` for videos), `setPhotoLocationInDb`/`clearPhotoLocationInDb` are gone, a video can never carry map coordinates, and `viewer.e2e.spec.js`'s re-insert no longer names the column.
- A folded entry (the DB/query entry is the natural host) recording the new invariant: the database stores no capture date/coordinates — `MediaFactsIndex` is rebuilt by every scan (unchanged files included), listings sort/filter date in Rust with SQLite's NULL ordering, every response is enriched from the index, `db::stored_metadata` strips coordinates on every write, and an applied migration must be replaced by a new one.
- Re-verify each existing entry for validity (in particular the timeline fixture-calendar entry: decade maths and the empty 1990s decade are unchanged by this work).

- [ ] **Step 2: Run the complete gate set**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
npm run format:check
npm run lint
npm run test:unit
npm run test:i18n
npm run build
cargo build --bin turbo-pix
./target/debug/turbo-pix --download-models      # only if ./data/models is empty
rm -rf test-e2e-data
npm run test:e2e
```

Expected: every command green, E2E included.

- [ ] **Step 3: Verify the spec's success criteria by inspection**

- SC-001/SC-004: Task 7's spec + Task 5/6 tests show the app and a file-level reader (the app's own extractor) agree with zero DB storage.
- SC-002: Task 2's grep + curl smoke.
- SC-003: Task 4 Step 9's throwaway 5,000-photo measurement (record the numbers; the budget is 1 s per operation).
- SC-005: `metadata_writer` refuses video/RAW/WebP; the disabled control states it (Task 7).
- SC-006: Task 4's rescan tests (external change → new facts after a reindex).
- SC-007/SC-008: the E2E suite runs with no stored dates/coordinates and both bundles intact.

- [ ] **Step 4: Commit**

```bash
git add AGENTS.md
git commit -m "docs: record file-only photo date/coordinate learnings"
```
