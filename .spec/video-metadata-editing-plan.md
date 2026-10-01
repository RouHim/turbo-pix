# In-Place Video Metadata Editing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `PATCH /api/photos/{hash}/metadata` edits MP4/MOV/M4V files in place (capture date + location) and mirrors the result into the `photos` row, refusing anything it cannot patch without touching media bytes.

**Architecture:** A new `src/mp4_metadata.rs` module owns all ISO-BMFF knowledge: it locates `moov` by walking top-level box headers with `seek` (never reading `mdat`), parses the box tree, and rewrites **only the `moov` region in a single write of unchanged length** — fixed-width `creation_time` fields in `mvhd`/`tkhd`/`mdhd` plus recognized carriers (mdta `keys`+`ilst` items, `©day`/`©xyz`, the ISO/3GPP `udta/loci` pair). Refusals are decided before any byte is written — except one the readback can only prove after the write (a container that holds no date carrier): that one is refused right after the write with the file rolled back — so a refused save leaves the file byte-identical. The handler branches on the row's `mime_type`, maps refusal variants to distinct 4xx `error_code`s, and mirrors the result into the row with `taken_at`, merged `location`, and the stat-derived fingerprint; the file's mtime is restored after the patch so every size+mtime-keyed cache (thumbnail, transcode, remux) stays valid.

**Tech Stack:** Rust (warp, sqlx/SQLite, chrono, tokio), hand-rolled ISO-BMFF byte parsing (no new crate), Svelte 5 runes frontend, Playwright E2E.

**Spec:** `.spec/video-metadata-editing.md`

## Global Constraints

- Writable containers are exactly **MP4, MOV, M4V** (extension whitelist) whose bytes are ISO-BMFF; MKV/WebM/AVI/3GP/other stay read-only.
- Editable fields are exactly capture date, latitude, longitude. Clearing stays client-refused as today (no new clear semantics in the API); no title/description/tags/rating; no rotation, thumbnail, transcode, or semantic-index changes.
- No re-encode, no remux, no full-file rewrite, no disk space proportional to the media payload; save duration must not grow with media size.
- All-or-nothing: any refusal or failure leaves the file byte-identical and the row unchanged.
- Values are stored as the same instant as the photo path (UTC epoch in fixed-width fields); text carriers keep the file's own representation style, byte length, precision, and offset style.
- Photos, rotation, deletion, transcoding, the DB-only batch date shift, and the existing photo EXIF path (JPEG/PNG) stay unchanged.
- New user-visible strings exist in **both** `frontend/src/i18n/en.json` and `de.json` with identical structure (`npm run test:i18n`).
- Gates: `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `cargo test`, `npm run test:unit`, `npm run test:i18n`, `npm run lint`, `npm run build`, then `cargo build --bin turbo-pix` (build order is mandatory), `npm run test:e2e`.
- Rust tests that shell out to `ffmpeg`/`ffprobe` gate on `RUN_VIDEO_TESTS=1` (existing convention, `src/video_processor.rs:1927`); byte-level tests never gate, because CI's `rust-tests` job installs ffmpeg but does not set `RUN_VIDEO_TESTS`.

## Measured facts the plan relies on

All verified on this checkout (ffmpeg 9.0.1, ffprobe on `test-data/test_video_with_date.mp4` and `test-data/test_video_hevc.mp4`):

- `ffprobe` reports `format.tags.creation_time` from **`moov/mvhd`** and `streams[*].tags.creation_time` from **`moov/trak/mdia/mdhd`** (`tkhd` is invisible to ffprobe but is a carrier for other readers). Patching only `mvhd` moves the format tag; only `mdhd` moves the stream tags.
- All `test-data/*.mp4` fixtures carry `moov/mvhd`, `trak/tkhd`, `trak/mdia/mdhd`, and `moov/udta/meta/ilst` with a `©too` (encoder) item; **no fixture carries a location carrier or an mdta `keys` box**.
- `ffmpeg -movflags +faststart+use_metadata_tags -metadata com.apple.quicktime.location.ISO6709=… -metadata com.apple.quicktime.creationdate=…` writes exactly those keys into `moov/udta/meta/keys`+`ilst` and is readable back by ffprobe — that is the new fixture.
- Rewriting an mdta item payload **in place at equal byte length** is read back by ffprobe, and a **shorter** value NUL-padded inside the same payload length is reported clean — nothing outside the edited payload bytes changes and no box is resized.
- `moov/udta/meta` here is a FullBox (version/flags precede the children) and `keys` entries are `[u32 size][4cc namespace][key string]` with the entry size covering the 8-byte prefix; ffmpeg writes no NUL terminator, Apple does — the reader must tolerate both.
- The scanner's fingerprint is `(file_path, file_size, file_modified)` with mtime **truncated to whole seconds** (`src/file_scanner.rs:147-152`, `src/db.rs:622-637`), so restoring the exact mtime keeps `find_unchanged_photo` matching and every `{hash}_{size}_{mtime}` cache keyed artifact valid.
- The scan upsert replaces `metadata` wholesale (`src/db.rs:933`), which today drops `location.city` and `metadata.video.capability_version` on every scan; the fix for FR-007 is a `json_patch` merge there.

## Review Focus

Inputs/conditions the spec implies but no single task's happy-path test covers — each is pinned by a test in the owning task:

1. `moov` at the **end** of the file (`test-data/test_video_moov_end.mp4`) — the save must be equally instant and must not move any media byte (T2).
2. A file whose bytes are not ISO-BMFF despite a video row (renamed MKV, truncated/garbage file, zero-length file) — refusal, no panic, byte-identical (T1, T2).
3. Two saves for the same hash at once — serialized, no mixed file, later request fully applied (T5).
4. A **read-only** file/permission-denied target — 403-class refusal, byte-identical, row unchanged (T2).
5. A save while the same video streams/converts — the stream is served from unchanged bytes and the run finishes (T8, plus T2's "mdat untouched" byte proof).

---

### Task 1: ISO-BMFF reader, carrier table, and the Apple-keys fixture

**Files:**
- Create: `src/mp4_metadata.rs`
- Modify: `src/lib.rs` (add `pub mod mp4_metadata;` after `metadata_writer`, alphabetical)
- Create: `test-data/test_video_quicktime_keys.mp4` (generated, committed)
- Test: unit tests inside `src/mp4_metadata.rs`

**Interfaces:**
- Consumes: nothing.
- Produces (later tasks rely on these exact names):
  - `pub const WRITABLE_EXTENSIONS: [&str; 3] = ["mp4", "mov", "m4v"];`
  - `pub fn has_writable_extension(path: &Path) -> bool;`
  - `pub struct Mp4Metadata { pub creation_time: Option<DateTime<Utc>>, pub creation_date_text: Option<String>, pub location_iso6709: Option<String> }`
  - `pub fn read_metadata(path: &Path) -> Result<Mp4Metadata, Mp4MetadataError>;`
  - `pub fn parse_iso6709(value: &str) -> Option<(f64, f64)>;`
  - `pub enum Mp4MetadataError { UnsupportedContainer(String), Fragmented, NoLocationCarrier, NoRoom(&'static str), Unrepresentable(&'static str), InvalidDate, InvalidCoordinates, MissingFile, ReadOnly(String), Io(std::io::Error) }` (derive `Debug`; implement `std::fmt::Display`)
  - `pub(crate) struct BoxSpan { pub offset: usize, pub size: usize, pub kind: [u8; 4], pub children: Vec<BoxSpan> }`
  - `pub(crate) fn parse_box_tree(buf: &[u8], start: usize, end: usize, path: &[[u8; 4]]) -> Result<Vec<BoxSpan>, Mp4MetadataError>` — container list: `moov, trak, mdia, minf, stbl, udta, edts, mvex, moof, traf, ilst`, plus `meta` (FullBox: children start 4 bytes in, with the QuickTime fallback: if the 4 bytes at the body do not look like a child header (`u32 size >= 8` followed by 4 ASCII bytes) treat the body as the first child header), plus every child of an `ilst` (item boxes, whose 4-byte type is either an mdta key index or a `©`-prefixed type).

- [ ] **Step 1: Generate and commit the fixture**

```bash
ffmpeg -y -v error -f lavfi -i testsrc2=size=320x240:rate=10:duration=2 \
  -f lavfi -i sine=frequency=440:duration=2 \
  -c:v libx264 -preset veryfast -pix_fmt yuv420p -c:a aac -shortest \
  -metadata com.apple.quicktime.creationdate=2024-05-01T10:00:00+0200 \
  -metadata com.apple.quicktime.location.ISO6709=+48.2082+016.3737/ \
  -movflags +faststart+use_metadata_tags test-data/test_video_quicktime_keys.mp4
ffprobe -v error -show_entries format_tags -of json test-data/test_video_quicktime_keys.mp4
```

Expected: `format_tags` contains `com.apple.quicktime.creationdate = 2024-05-01T10:00:00+0200`, `com.apple.quicktime.location.ISO6709 = +48.2082+016.3737/`, `encoder`, and **no** `creation_time` (mvhd stays 0). `+faststart` is mandatory: the scan rewrites a moov-at-end file with a copy-remux that drops these keys.

- [ ] **Step 2: Write the failing tests**

```rust
#[test]
fn reads_apple_key_carriers_from_the_fixture() {
    let m = read_metadata(Path::new("test-data/test_video_quicktime_keys.mp4")).unwrap();
    assert_eq!(m.location_iso6709.as_deref(), Some("+48.2082+016.3737/"));
    assert_eq!(m.creation_date_text.as_deref(), Some("2024-05-01T10:00:00+0200"));
    assert_eq!(m.creation_time, None); // mvhd creation_time is 0 in this fixture
}

#[test]
fn reads_mvhd_creation_time_from_the_date_fixture() {
    let m = read_metadata(Path::new("test-data/test_video_with_date.mp4")).unwrap();
    assert_eq!(m.creation_time.unwrap().to_rfc3339(), "2023-06-15T10:00:00+00:00");
    assert_eq!(m.creation_date_text, None);
    assert_eq!(m.location_iso6709, None);
}

#[test]
fn refuses_non_iso_bmff_bytes_and_unlisted_extensions() {
    assert!(matches!(read_metadata(Path::new("test-data/test_video_long.mkv")), Err(Mp4MetadataError::UnsupportedContainer(_))));
    assert!(matches!(read_metadata(Path::new("test-data/test_video_legacy.avi")), Err(Mp4MetadataError::UnsupportedContainer(_))));
    assert!(!has_writable_extension(Path::new("/x/a.3gp")));
    assert!(has_writable_extension(Path::new("/x/a.MOV")));
}

#[test]
fn parses_iso6709_forms_and_rejects_garbage() {
    assert_eq!(parse_iso6709("+48.2082+016.3737/"), Some((48.2082, 16.3737)));
    assert_eq!(parse_iso6709("-33.8688+151.2093/"), Some((-33.8688, 151.2093)));
    assert_eq!(parse_iso6709("+48.2082+016.3737+150.00/").unwrap().1, 16.3737);
    assert_eq!(parse_iso6709("nonsense"), None);
    assert_eq!(parse_iso6709("+91.0+016.0/"), None); // out of range
}

#[test]
fn parses_a_quicktime_meta_without_version_flags() {
    // QuickTime writes `udta/meta` without the ISO version/flags word: the body
    // starts directly with the `hdlr` child header. Build such a `meta` body,
    // parse it, and assert the children come out as hdlr, keys, ilst — the
    // fallback must not swallow the first 4 bytes as version/flags.
    let body = qt_meta_body_without_version_flags(); // test helper: hdlr + keys(1 entry) + ilst
    let tree = parse_box_tree(&body, 0, body.len(), &[b"meta"]).unwrap();
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].kind, *b"meta");
    assert_eq!(
        tree[0].children.iter().map(|c| c.kind).collect::<Vec<_>>(),
        vec![*b"hdlr", *b"keys", *b"ilst"]
    );
}

#[test]
fn refuses_an_inenar_and_an_empty_file() {
    // A file whose first box claims a size beyond EOF, and a zero-length file,
    // both return UnsupportedContainer — never a panic or a huge allocation.
}
```

`qt_meta_body_without_version_flags` (test helper): concatenates a `hdlr` box, a `keys` box with a single entry (`size`/namespace `mdta`/`com.apple.quicktime.location.ISO6709`), and an empty `ilst` box, each with a big-endian `u32` size prefix.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test mp4_metadata::tests --lib`
Expected: FAIL — `error[E0433]: failed to resolve: use of undeclared crate or module mp4_metadata`.

- [ ] **Step 4: Implement the reader**

Implementation notes (these are decisions, not suggestions):
- `locate_moov`: open the file once, walk top-level boxes by `seek`+`read_exact` of the 8-byte (16 for `size == 1`) header only; `size == 0` extends to EOF; return `(offset, size)`. A file without `ftyp` or without `moov`, or a box whose declared size overruns the file, is `UnsupportedContainer("<sniffed fourcc or 'not ISO-BMFF'>")`.
- Read the `moov` region (refuse with `NoRoom("moov")` when it exceeds 64 MiB, so a corrupt size cannot allocate the world), then `parse_box_tree`.
- `mvex` anywhere inside `moov` ⇒ `Fragmented`.
- `creation_time` = the `mvhd` creation field decoded from the QuickTime epoch (`1904-01-01T00:00:00Z`); a value of 0 stays `Some`ly decodable but the reader returns `None` for it (0 means "never set", matching ffprobe).
- Text carriers, resolved from the tree:
  - mdta: read `moov/udta/meta/keys`, build `index → key string` (key bytes = entry body after the 8-byte prefix, trailing NUL stripped), then for each `moov/udta/meta/ilst` child whose 4-byte type is a big-endian index, read its `data` child's payload (payload starts 8 bytes after the `data` box header: 4-byte type indicator + 4-byte locale).
  - date carriers: mdta key `com.apple.quicktime.creationdate` or `creation_time`, item type `©day`, or a direct `moov/udta/©day` child.
  - location carriers: mdta key `com.apple.quicktime.location.ISO6709` or `location`, item type `©xyz`, a direct `moov/udta/©xyz` child, or the ISO/3GPP `moov/udta/loci` box (longitude then latitude, each a 16.16 fixed-point `i32` — the carrier ffmpeg's own MP4 muxer writes for a location tag).
  - `creation_date_text`/`location_iso6709` are the verbatim first hit in the order listed.
- `parse_iso6709`: accept `[+-]D+(.D+)?[+-]D+(.D+)?[+-]D+(.D+)?/?` (altitude optional, trailing `/` optional), validate `-90..=90` / `-180..=180`, return `None` otherwise.
- Every failure path returns an `Err`; no `unwrap`/`expect` on file data.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test mp4_metadata::tests --lib`
Expected: PASS (all five tests).

- [ ] **Step 6: Commit**

```bash
git add src/mp4_metadata.rs src/lib.rs test-data/test_video_quicktime_keys.mp4
git commit -m "feat(video): read ISO-BMFF and QuickTime metadata carriers"
```

---

### Task 2: In-place `moov` patch — fixed-width timestamps, all-or-nothing, fingerprint

**Files:**
- Modify: `src/mp4_metadata.rs`
- Test: unit tests in `src/mp4_metadata.rs`, plus a `RUN_VIDEO_TESTS`-gated test in the same module

**Interfaces:**
- Consumes: Task 1's tree, reader, error enum, `WRITABLE_EXTENSIONS`.
- Produces:
  - `pub struct VideoMetadataEdit { pub taken_at: Option<DateTime<Utc>>, pub latitude: Option<f64>, pub longitude: Option<f64> }` (derive `Debug, Clone, Copy, Default`)
  - `pub struct VideoMetadataWrite { pub fingerprint: Fingerprint, pub undo: UndoToken }`
  - `pub struct Fingerprint { pub file_size: u64, pub file_modified: DateTime<Utc> }` (mtime truncated to whole seconds)
  - `pub struct UndoToken { moov_offset: u64, moov_bytes: Vec<u8>, modified: SystemTime }`
  - `pub fn write_metadata(path: &Path, edit: &VideoMetadataEdit) -> Result<VideoMetadataWrite, Mp4MetadataError>;`
  - `pub fn restore(token: &UndoToken) -> Result<(), Mp4MetadataError>;` — writes `moov_bytes` back at `moov_offset` and re-applies `modified`
  - `pub(crate) fn quicktime_seconds(dt: DateTime<Utc>) -> Option<u32>;` (epoch 1904-01-01T00:00:00Z; `None` outside the representable v0 range)
- `Mp4MetadataError` carries `std::io::Error` in one variant, so it has no `PartialEq`: every test asserts with `matches!(err, Mp4MetadataError::Variant)`, never `==`.
- Test helpers used by Tasks 2-4 (define once, in the module's `#[cfg(test)] mod tests`):
  - `fn temp_copy(name: &str, src: &str) -> (TempDir, PathBuf)` — a fresh `tempfile::TempDir` plus `std::fs::copy` of `src` into it, returning both so the directory outlives the call. `test-data/` fixtures are committed and must never be written to.
  - `fn count_creation_times(buf: &[u8], expected_seconds: u64) -> usize` — parse `buf` with the module's own tree walker and count `mvhd`/`tkhd`/`mdhd` creation fields equal to `expected_seconds`.

- [ ] **Step 1: Write the failing tests** (all ungated except the last)

```rust
#[test]
fn patching_the_date_writes_only_the_moov_region() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("clip.mp4");
    std::fs::copy("test-data/test_video_with_date.mp4", &path).unwrap();
    let before = std::fs::read(&path).unwrap();
    let before_mtime = std::fs::metadata(&path).unwrap().modified().unwrap();

    let edit = VideoMetadataEdit { taken_at: Some("2024-07-04T12:00:00Z".parse().unwrap()), ..Default::default() };
    let out = write_metadata(&path, &edit).unwrap();
    let after = std::fs::read(&path).unwrap();

    assert_eq!(after.len(), before.len(), "the file length must not change");
    // mdat payload byte-identical; the moov region is the only one that may differ,
    // and every byte that differs must sit inside a creation field:
    let (moov_off, moov_len) = locate_moov_in(&before).unwrap();
    assert_eq!(&after[..moov_off], &before[..moov_off]);
    assert_eq!(&after[moov_off + moov_len..], &before[moov_off + moov_len..]);
    assert_ne!(&after[moov_off..moov_off + moov_len], &before[moov_off..moov_off + moov_len]);
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before_mtime, "mtime restored");
    assert_eq!(out.fingerprint.file_size, before.len() as u64);
    let read_back = read_metadata(&path).unwrap();
    assert_eq!(read_back.creation_time.unwrap().to_rfc3339(), "2024-07-04T12:00:00+00:00");
    // Every creation field the file has carries the same instant now.
    // `test_video_with_date.mp4` has ONE trak → mvhd + tkhd + mdhd = 3 fields
    // (the two-trak case, `test_video_moov_end.mp4`, asserts 5 in the gated test below).
    let expected = u64::from(quicktime_seconds(edit.taken_at.unwrap()).unwrap());
    assert_eq!(count_creation_times(&after, expected), 3);
}

#[test]
fn a_refused_date_edit_leaves_the_file_byte_identical() {
    let (_dir, path) = temp_copy("clip.mp4", "test-data/test_video_with_date.mp4");
    let before = std::fs::read(&path).unwrap();
    let too_old = VideoMetadataEdit { taken_at: Some("1989-12-31T00:00:00Z".parse().unwrap()), ..Default::default() };
    assert!(matches!(write_metadata(&path, &too_old).unwrap_err(), Mp4MetadataError::InvalidDate));
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn restores_the_original_moov_after_a_successful_write() {
    // write a date, keep the UndoToken, call restore(), assert the bytes equal the original
    // and that the mtime is the original one again.
}

#[test]
fn read_only_targets_are_refused_without_writing() {
    // chmod 0o444 the copy (skip the test when running as root), expect
    // matches!(err, Mp4MetadataError::ReadOnly(_)) and identical bytes plus an unchanged mtime.
}

#[test]
fn truncated_or_empty_files_are_refused() {
    // empty file, and a file holding only the first 12 bytes of the fixture →
    // matches!(err, Mp4MetadataError::UnsupportedContainer(_)), no panic.
}
```

Gated test (`RUN_VIDEO_TESTS=1`):

```rust
#[test]
fn ffprobe_reports_the_patched_date_and_moov_at_end_files_work_too() {
    // for both test_video_with_date.mp4 and test_video_moov_end.mp4:
    //   write 2024-07-04T12:00:00Z, then `ffprobe -v error -show_entries
    //   format_tags=creation_time:stream_tags=creation_time -of json <copy>`
    //   must report 2024-07-04T12:00:00Z for format AND stream; `ffprobe -v error -i <copy>` exits 0.
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test mp4_metadata::tests --lib`
Expected: FAIL — `cannot find function write_metadata`.

- [ ] **Step 3: Implement `write_metadata`**

Algorithm (locked in):
1. `has_writable_extension(path)` else `UnsupportedContainer(ext)`; locate + read + parse `moov` exactly as Task 1 does; `mvex` ⇒ `Fragmented`.
2. Reject `edit.taken_at` outside `1990-01-01T00:00:00Z ..= 2040-02-06T06:28:15Z` with `InvalidDate` (the reader's own floor is year 1990, `src/metadata_extractor.rs:543`; `u32` QuickTime seconds overflow 2040-02-06).
3. Reject out-of-range or unpaired coordinates with `InvalidCoordinates` (mirrors `metadata_writer.rs:20-48`).
4. Build the patched `moov` **in memory** as a `Vec<u8>` of exactly the original length: for every `mvhd`/`tkhd`/`mdhd` in the tree, write the new instant at body offset 4 as `u32` (version 0) or `u64` (version 1) QuickTime seconds; copy every other byte range verbatim. Text carriers arrive in Tasks 3 and 4 — this task must already route them through a `carriers` list that is currently only the fixed-width ones.
5. The rebuilt buffer must be exactly the original `moov` length (this task has no text carriers, so it always is — `debug_assert_eq!` it). Task 4 defines what happens when a render changes the content length; this task must not write anything when that invariant cannot hold.
6. Open the file once with `OpenOptions::new().write(true).open(path)` (map `ErrorKind::NotFound` → `MissingFile`, `PermissionDenied`/`ReadOnlyFilesystem` → `ReadOnly(e.to_string())`), `write_all` the whole patched `moov` at `moov_offset`, then `File::set_modified(original_modified)` and log a warning when that fails.
7. Return `Fingerprint { file_size: metadata.len(), file_modified: truncated_to_seconds(metadata.modified()) }` plus the `UndoToken` (original `moov` bytes + original mtime).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test mp4_metadata::tests --lib` then `RUN_VIDEO_TESTS=1 cargo test mp4_metadata::tests --lib`
Expected: PASS both times (the gated test prints the skip message when the variable is unset).

- [ ] **Step 5: Commit**

```bash
git add src/mp4_metadata.rs
git commit -m "feat(video): patch moov timestamps in place with an undo token"
```

---

### Task 3: Text date carriers with representation-preserving rendering

**Files:**
- Modify: `src/mp4_metadata.rs`
- Test: unit tests in `src/mp4_metadata.rs` (fixture-driven, ungated)

**Interfaces:**
- Consumes: Task 1's tree/carrier resolution, Task 2's rebuilding writer.
- Produces:
  - `pub(crate) fn render_date_in_shape(existing: &str, new: DateTime<Utc>) -> Option<Vec<u8>>;`
  - `pub(crate) fn write_payload_slot(slot: &mut [u8], rendered: &[u8]);` — copies `rendered` into the front of `slot` and NUL-fills the remainder; debug-asserts `rendered.len() <= slot.len()` (callers must use `NoRoom`/`Unrepresentable` before calling it)
- Test helper (define here, reused by Task 4): `enum CarrierSpec { MdtaKey(&'static str), ItemType([u8; 4]) }` and `fn synthetic_mp4_with_carrier(carrier: CarrierSpec, payload: &[u8], free_payload: usize) -> (TempDir, PathBuf)` — writes a minimal valid ISO-BMFF file (`ftyp` + `moov` holding `mvhd`, one `trak`/`tkhd`/`mdia`/`mdhd`, `udta/meta/hdlr` plus `keys`+`ilst` for `MdtaKey` or a bare 4-character item for `ItemType`, and a `free` box of `free_payload` bytes as the last child of `moov`). It is the only way to reach carriers the committed fixtures do not have (unparsable values, room for a longer render).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn renders_the_new_instant_in_the_files_own_date_shape() {
    let new = "2024-07-04T12:00:00Z".parse().unwrap();
    assert_eq!(render_date_in_shape("2024-05-01T10:00:00+0200", new).unwrap(), b"2024-07-04T14:00:00+0200");
    assert_eq!(render_date_in_shape("2024-05-01T10:00:00Z", new).unwrap(), b"2024-07-04T12:00:00Z");
    assert_eq!(render_date_in_shape("2024-05-01T10:00:00.123+02:00", new).unwrap(), b"2024-07-04T14:00:00.000+02:00");
    assert_eq!(render_date_in_shape("2024-05-01 10:00:00", new).unwrap(), b"2024-07-04 12:00:00");
    assert_eq!(render_date_in_shape("2024-05-01", new).unwrap(), b"2024-07-04");
    assert_eq!(render_date_in_shape("May 1st, 2024", new), None);
}

#[test]
fn a_shorter_rendering_is_nul_padded_inside_the_existing_payload() {
    let mut slot = *b"2024-05-01T10:00:00+0200";
    write_payload_slot(&mut slot, b"2025-01-02T05:04:05+02");
    assert_eq!(&slot[..], b"2025-01-02T05:04:05+02\0\0");
}

#[test]
fn a_date_save_updates_every_carrier_including_the_text_ones() {
    let (_dir, path) = temp_copy("keys.mp4", "test-data/test_video_quicktime_keys.mp4");
    let before = std::fs::read(&path).unwrap();
    let edit = VideoMetadataEdit { taken_at: Some("2025-01-02T03:04:05Z".parse().unwrap()), ..Default::default() };
    write_metadata(&path, &edit).unwrap();

    assert_eq!(read_metadata(&path).unwrap().creation_time.unwrap().to_rfc3339(), "2025-01-02T03:04:05+00:00");
    // The +0200-style carrier keeps its offset style and expresses the same instant:
    assert_eq!(read_metadata(&path).unwrap().creation_date_text.as_deref(), Some("2025-01-02T05:04:05+0200"));
    let after = std::fs::read(&path).unwrap();
    assert_eq!(after.len(), before.len());
    assert!(after.windows(24).any(|w| w == b"2025-01-02T05:04:05+0200"));
    assert!(!after.windows(24).any(|w| w == b"2024-05-01T10:00:00+0200"));
    // No box was resized: the data box that held the old string still declares the same size.
    assert_eq!(data_boxes(&after), data_boxes(&before), "payload slots are rewritten, never resized");
}

#[test]
fn an_unparsable_carrier_refuses_the_whole_save() {
    let (_dir, path) = synthetic_mp4_with_carrier(CarrierSpec::ItemType(*b"\xa9day"), b"May 1st, 2024", 0);
    let before = std::fs::read(&path).unwrap();
    let edit = VideoMetadataEdit { taken_at: Some("2025-01-02T03:04:05Z".parse().unwrap()), ..Default::default() };
    assert!(matches!(write_metadata(&path, &edit).unwrap_err(), Mp4MetadataError::Unrepresentable(t) if t == "\u{a9}day"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test mp4_metadata::tests --lib`
Expected: FAIL — `cannot find function render_date_in_shape`.

- [ ] **Step 3: Implement**

- Shape parser: `YYYY-MM-DD` (date only), `YYYY-MM-DD[T ]hh:mm:ss` with optional `.fraction` and optional `Z`/`±hhmm`/`±hh:mm`. Render the new instant with the **same separator, the same fraction digit count, and the same offset style**; a `Z` carrier gets `Z`, an offset carrier expresses the instant in that same offset, an offset-less carrier writes the UTC wall clock. A date-only carrier renders the date.
- Wire text carriers into the writer: mdta items (by key), `©day` ilst items, and direct `moov/udta/©day` children. Each edited payload keeps its `data` box size: the rendered value is written with `write_payload_slot` at the payload offset. A render that does not fit its slot returns `NoRoom(<carrier>)` (Task 4 adds the room search), and a value that cannot be rendered at all returns `Unrepresentable(<carrier>)`; both are decided before the first byte is written.
- A save with no `taken_at` performs no date work (and therefore cannot fail on a carrier's shape).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test mp4_metadata::tests --lib`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/mp4_metadata.rs
git commit -m "feat(video): preserve QuickTime date text representations on save"
```

---

### Task 4: Location writes (`ISO6709`) and the room model for longer renders

**Files:**
- Modify: `src/mp4_metadata.rs`
- Test: unit tests in `src/mp4_metadata.rs` (fixture- and synthetic-driven, ungated)

**Interfaces:**
- Consumes: Tasks 1-3, including `synthetic_mp4_with_carrier`.
- Produces: `pub(crate) fn render_iso6709_in_shape(existing: &str, latitude: f64, longitude: f64) -> Option<Vec<u8>>;`
- Room model (the writer's rule for any render that does not fit its slot, stated once): rebuild `moov` as `children' || free(pad)` with `pad = original_moov_len - len(children')`, where every pre-existing `free`/`skip` box inside `moov` is dropped from `children'` and its bytes — header included — count as padding, so a dropped box can never shorten the rebuilt region. `pad == 0` writes no box; `pad >= 8` writes exactly one `free` box of `pad` bytes; `pad` in `1..=7` (or negative) cannot be represented and is `NoRoom(<carrier>)`. Growth is therefore allowed exactly while the file's own free room in `moov` covers it — never by shifting media bytes.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn renders_coordinates_in_the_files_own_iso6709_shape() {
    assert_eq!(render_iso6709_in_shape("+48.2082+016.3737/", 52.52, 13.405).unwrap(), b"+52.5200+013.4050/");
    assert_eq!(render_iso6709_in_shape("+48.2082+016.3737+150.00/", 52.52, 13.405).unwrap(), b"+52.5200+013.4050+150.00/");
    assert_eq!(render_iso6709_in_shape("-33.8688+151.2093", -33.9, 151.3).unwrap(), b"-33.9000+151.3000");
    // A narrower file width is widened, never kept: the value decides, the writer pays for the extra bytes.
    assert_eq!(render_iso6709_in_shape("+8.2082+016.3737/", 52.52, 13.405).unwrap(), b"+52.5200+013.4050/");
    assert_eq!(render_iso6709_in_shape("garbage", 1.0, 2.0), None);
}

#[test]
fn a_location_save_replaces_the_entry_without_duplicating_it() {
    let (_dir, path) = temp_copy("keys.mp4", "test-data/test_video_quicktime_keys.mp4");
    let before_len = std::fs::metadata(&path).unwrap().len();
    let edit = VideoMetadataEdit { latitude: Some(52.52), longitude: Some(13.405), ..Default::default() };
    write_metadata(&path, &edit).unwrap();

    assert_eq!(read_metadata(&path).unwrap().location_iso6709.as_deref(), Some("+52.5200+013.4050/"));
    let buf = std::fs::read(&path).unwrap();
    // The key entry is 8 + 36 bytes ("com.apple.quicktime.location.ISO6709"); it must appear once.
    assert_eq!(buf.windows(36).filter(|w| *w == b"com.apple.quicktime.location.ISO6709").count(), 1);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), before_len);
}

#[test]
fn a_file_without_a_location_carrier_refuses_the_save() {
    let (_dir, path) = temp_copy("plain.mp4", "test-data/test_video_with_date.mp4");
    let before = std::fs::read(&path).unwrap();
    let edit = VideoMetadataEdit { latitude: Some(52.52), longitude: Some(13.405), ..Default::default() };
    assert!(matches!(write_metadata(&path, &edit).unwrap_err(), Mp4MetadataError::NoLocationCarrier));
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn a_longer_render_uses_the_files_free_room_or_is_refused() {
    let edit = VideoMetadataEdit { latitude: Some(52.52), longitude: Some(13.405), ..Default::default() };
    let spec = || CarrierSpec::MdtaKey("com.apple.quicktime.location.ISO6709");
    // The stored value "+8.2082+016.3737/" is 17 bytes; the render "+52.5200+013.4050/" is 18 → growth 1.
    // (a) 16 bytes of free payload inside moov: the save succeeds, the file length is unchanged and a
    //     23-byte free box (15 bytes of payload) is left where the previous boxes were.
    let (_dir, path) = synthetic_mp4_with_carrier(spec(), b"+8.2082+016.3737/", 16);
    write_metadata(&path, &edit).unwrap();
    assert_eq!(read_metadata(&path).unwrap().location_iso6709.as_deref(), Some("+52.5200+013.4050/"));
    assert_eq!(trailing_free_box_size(&std::fs::read(&path).unwrap()), 23);

    // (b) no room at all: refusal, byte-identical file.
    let (_dir2, path2) = synthetic_mp4_with_carrier(spec(), b"+8.2082+016.3737/", 0);
    let before = std::fs::read(&path2).unwrap();
    assert!(matches!(write_metadata(&path2, &edit).unwrap_err(), Mp4MetadataError::NoRoom(_)));
    assert_eq!(std::fs::read(&path2).unwrap(), before);

    // (c) room that would leave 7 bytes of padding: also refused, never a malformed box.
    let (_dir3, path3) = synthetic_mp4_with_carrier(spec(), b"+8.2082+016.3737/", 8);
    assert!(matches!(write_metadata(&path3, &edit).unwrap_err(), Mp4MetadataError::NoRoom(_)));
}

#[test]
fn the_ffmpeg_style_location_key_is_writable_too() {
    let (_dir, path) = synthetic_mp4_with_carrier(CarrierSpec::MdtaKey("location"), b"+48.2082+016.3737/", 0);
    let edit = VideoMetadataEdit { latitude: Some(-33.9), longitude: Some(151.3), ..Default::default() };
    write_metadata(&path, &edit).unwrap();
    assert_eq!(read_metadata(&path).unwrap().location_iso6709.as_deref(), Some("-33.9000+151.3000/"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test mp4_metadata::tests --lib`
Expected: FAIL — `cannot find function render_iso6709_in_shape`.

- [ ] **Step 3: Implement**

- `render_iso6709_in_shape` preserves, from the existing value: each field's decimal count, the altitude token **verbatim** (it is not editable), and the trailing `/`; each field is rendered with `max(existing integer-digit count, digits the new magnitude needs)` and an explicit sign. A growth in bytes is not decided here — the writer prices it against `pad` and answers `NoRoom(<carrier>)` when the file's own room cannot absorb it. `None` is returned only for a value whose shape cannot be parsed at all (which the writer reports as `Unrepresentable(<carrier>)`).
- Test helper for this task: `fn trailing_free_box_size(buf: &[u8]) -> usize` — parse `buf`, find the last `free`/`skip` child of `moov`, return its size (`0` when absent).
- Wire the location carriers (mdta keys `com.apple.quicktime.location.ISO6709` and `location`, ilst `©xyz`, direct `moov/udta/©xyz`) into the same payload-rewriting path as Task 3, all-or-nothing across every carrier in the file, and implement the room model exactly as stated in the Interfaces block (`children'` may grow only into `pad`).
- When `latitude` is requested and no location carrier exists → `NoLocationCarrier`; when a carrier exists but its shape cannot be rendered → `Unrepresentable(<carrier>)`; when the render is longer and `pad` cannot absorb it → `NoRoom(<carrier>)`. All decided before the first byte is written.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test mp4_metadata::tests --lib`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/mp4_metadata.rs
git commit -m "feat(video): write ISO6709 location into the file's own carrier"
```

---

### Task 5: Handler — video branch, refusal codes, row mirror, concurrency

**Files:**
- Modify: `src/handlers_photo.rs` (branch inside `update_photo_metadata`, `src/handlers_photo.rs:518`), `src/warp_helpers.rs` (`ErrorResponse` + new rejection + `handle_rejection` arm)
- Test: `#[cfg(test)]` tests in `src/handlers_photo.rs`

**Interfaces:**
- Consumes: `mp4_metadata::{read_metadata, write_metadata, restore, VideoMetadataEdit, Mp4MetadataError, Fingerprint}`.
- Produces:
  - `src/warp_helpers.rs`: `pub struct VideoMetadataError { pub status: warp::http::StatusCode, pub code: &'static str, pub message: String }` implementing `reject::Reject`, plus `ErrorResponse.error_code: Option<&'static str>` serialized only when present.
  - `src/handlers_photo.rs`: `fn video_metadata_rejection(err: Mp4MetadataError) -> Rejection;` and `async fn apply_video_metadata_edit(photo: Photo, edit: VideoMetadataEdit, db_pool: &DbPool) -> Result<warp::reply::Json, Rejection>;`
  - status/code table (exact): `UnsupportedContainer → 415 unsupported_container`, `Fragmented | NoRoom → 422 no_writable_slot`, `NoLocationCarrier → 422 no_location_carrier`, `Unrepresentable → 422 unrepresentable_value`, `InvalidDate → 400 invalid_date`, `InvalidCoordinates → 400 invalid_coordinates`, `MissingFile → 404 file_missing`, `ReadOnly → 403 file_read_only`, `Io → 500` (generic).

- [ ] **Step 1: Write the failing tests** (warp::test against a temp DB, following the existing handler-test pattern in `src/handlers_photo.rs`)

```rust
#[tokio::test]
async fn patch_metadata_edits_a_video_file_and_the_row() {
    // GIVEN a photo row whose file_path is a temp copy of test-data/test_video_with_date.mp4
    //       and mime_type "video/mp4"
    // WHEN PATCH /api/photos/{hash}/metadata {"taken_at":"2024-07-04T12:00:00Z","latitude":52.52,"longitude":13.405}
    // THEN 200, the body's taken_at is 2024-07-04T12:00:00Z, metadata.location.latitude == 52.52,
    //      file_size unchanged, and the file on disk carries the new date (read_metadata)
    //      — for a file WITHOUT a location carrier this request must be 422 instead,
    //      so this test uses test-data/test_video_quicktime_keys.mp4.
}

#[tokio::test]
async fn patch_metadata_refuses_a_video_without_a_location_carrier() {
    // WHEN PATCH {"latitude":52.52,"longitude":13.405} on test_video_with_date.mp4
    // THEN 422, JSON {"error_code":"no_location_carrier"}, the file bytes are identical,
    //      and the row is unchanged (taken_at, metadata, file_modified all as before).
}

#[tokio::test]
async fn patch_metadata_refuses_a_matroska_video_with_a_machine_readable_code() {
    // row with mime_type "video/x-matroska" and file_path test-data/test_video_long.mkv
    // THEN 415 {"error_code":"unsupported_container"} — never 500.
}

#[tokio::test]
async fn an_empty_video_request_touches_nothing() {
    // PATCH {} on a video row: 200 with the unchanged row, identical file bytes,
    // identical file_modified and metadata (FR-013).
}

#[tokio::test]
async fn a_failed_row_write_rolls_the_file_back() {
    // Point the handler at a pool whose write fails (close the pool or drop the row's table),
    // expect 500 and byte-identical file content AND mtime.
}

#[tokio::test]
async fn concurrent_saves_do_not_interleave() {
    // tokio::join! of a date-only and a location-only save for the same hash:
    // both return 200; afterwards read_metadata reports the date AND the coordinates;
    // the file is not corrupt (parse + read_metadata succeed).
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test handlers_photo::tests --lib`
Expected: FAIL — the video cases currently answer 500 (`Unsupported file extension '.mp4' for EXIF writing`).

- [ ] **Step 3: Implement**

- In `update_photo_metadata`, after `find_by_hash` and after the shared `taken_at` parse (`src/handlers_photo.rs:536-547`, unchanged): `if photo.mime_type.as_deref().is_some_and(|m| m.starts_with("video/")) { return apply_video_metadata_edit(photo, VideoMetadataEdit { .. }, &db_pool).await; }` — the photo path below stays byte-for-byte as it is today.
- `apply_video_metadata_edit`:
  1. Empty edit (all three `None`) ⇒ respond `warp::reply::json(&photo)` **without** touching file or DB (FR-013) and log at debug.
  2. `let _guard = VIDEO_EDIT_LOCK.lock().await;` with `static VIDEO_EDIT_LOCK: LazyLock<tokio::sync::Mutex<()>>` near the handler — serializes saves so two requests cannot interleave reads and writes of the same or another file.
  3. `let write = write_metadata(Path::new(&photo.file_path), &edit)` — on `Err`, map through `video_metadata_rejection` (log the message; the response body carries the code).
  4. Mirror into the loaded row exactly like the photo path does for location (`src/handlers_photo.rs:573-607`): `taken_at`, merged `location` object, then `file_size`/`date_modified` from `write.fingerprint` when the row already describes that file (a row whose file was replaced behind its back keeps its stale fingerprint, so the next scan re-extracts the file instead of skipping it forever), `updated_at = Utc::now()`. A requested date the readback finds in no carrier at all is refused as `unrepresentable_value` with the file rolled back first, never mirrored.
  5. `photo.update(&db_pool).await` — on `Err`, call `mp4_metadata::restore(&write.undo)` (log a warning if that also fails) and return `DatabaseError` (500).
  6. Respond `warp::reply::json(&photo)`.
- `video_metadata_rejection` maps the variant table above into `VideoMetadataError`; `handle_rejection` gains an arm **before** the `ValidationError` arm that replies with that status and the shared JSON body (now including `error_code`). Do not change the photo path's `ValidationError`/`DatabaseError` behaviour.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test handlers_photo::tests --lib`
Expected: PASS.

- [ ] **Step 5: Verify the photo path is untouched**

Run: `cargo test --lib` and `cargo clippy --all-targets -- -D warnings`
Expected: all existing tests pass (the photo metadata tests among them), no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/handlers_photo.rs src/warp_helpers.rs
git commit -m "feat(video): edit video metadata over the photo PATCH route"
```

---

### Task 6: Scan reconciliation — video location extraction and a merge-preserving upsert

**Files:**
- Modify: `src/metadata_extractor.rs` (`extract_video_metadata`, `src/metadata_extractor.rs:333`), `src/db.rs` (`create_or_update_with_transaction`, `metadata = excluded.metadata` at `src/db.rs:933`)
- Test: unit tests in `src/metadata_extractor.rs`, tests in `src/db.rs`

**Interfaces:**
- Consumes: `mp4_metadata::{read_metadata, parse_iso6709}`.
- Produces: nothing new outside the two modules; `PhotoMetadata.latitude/longitude` become populated for videos whose file carries a location.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] // src/metadata_extractor.rs
fn extracts_location_from_the_files_iso6709_carrier() {
    let m = MetadataExtractor::extract_with_metadata(
        Path::new("test-data/test_video_quicktime_keys.mp4"), None);
    assert_eq!(m.latitude, Some(48.2082));
    assert_eq!(m.longitude, Some(16.3737));
}

#[test]
fn leaves_location_empty_for_a_video_without_a_carrier() {
    let m = MetadataExtractor::extract_with_metadata(Path::new("test-data/test_video.mp4"), None);
    assert_eq!(m.latitude, None);
    assert_eq!(m.longitude, None);
}
```

```rust
#[tokio::test] // src/db.rs
async fn scan_upsert_preserves_city_and_the_capability_record() {
    // GIVEN a video row with metadata {"location":{"latitude":48.2,"longitude":16.4,"city":"Vienna"},
    //       "video":{"codec":"h264","capability_version":1}}
    // WHEN create_or_update_with_transaction writes the row a fresh scan would produce
    //       (location nulls, video facts without capability_version)
    // THEN metadata.location.city is still "Vienna" and metadata.video.capability_version is still 1
    //      (record_is_complete(&photo) stays true, so no re-probe).
}

#[tokio::test]
async fn scan_upsert_still_clears_coordinates_the_file_no_longer_has() {
    // the same upsert with {"location":{"latitude":null,"longitude":null}} must remove
    // $.location.latitude / $.location.longitude (the file is authoritative), while unrelated
    // keys survive.
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib db::tests::scan_upsert metadata_extractor::tests -- --nocapture`
Expected: FAIL — latitude is `None` for the fixture and the upsert drops `city`/`capability_version`.

- [ ] **Step 3: Implement**

- `extract_video_metadata`: after the ffprobe block, add `Self::apply_container_location(path, metadata)`; it calls `mp4_metadata::read_metadata(path)`, and on `Ok(m)` with `Some(iso6709)` sets `latitude`/`longitude` via `parse_iso6709`. Any `Err` is ignored (MKV/AVI/webm simply have no container location; do not log per-file noise). No new subprocess.
- `src/db.rs`: replace `metadata = excluded.metadata` with
  ```sql
  metadata = json_patch(
      CASE WHEN json_type(photos.metadata) = 'object' THEN photos.metadata ELSE '{}' END,
      excluded.metadata),
  ```
  (RFC 7396: keys absent from the fresh extraction survive — that is `location.city` and `video.capability_version`; explicit nulls in the fresh extraction delete the key — that is how a file that lost its GPS clears stale coordinates, exactly as today's wholesale replace did).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib`
Expected: PASS, including the existing search/filter tests that read `$.location.city`.

- [ ] **Step 5: Commit**

```bash
git add src/metadata_extractor.rs src/db.rs
git commit -m "feat(video): extract location from the container and keep scan metadata merges lossless"
```

---

### Task 7: Frontend — enable the editor for writable videos, localize refusals

**Files:**
- Modify: `frontend/src/lib/utils.js`, `frontend/src/lib/api.js` (`request`, `frontend/src/lib/api.js:45-53`), `frontend/src/components/ViewerMetadata.svelte`, `frontend/src/components/ViewerMetadataEdit.svelte`
- Create: `frontend/src/lib/metadataErrors.js`
- Modify: `frontend/src/i18n/en.json`, `frontend/src/i18n/de.json`
- Create: `tests/metadata-errors.test.js`

**Interfaces:**
- Consumes: the backend's `error_code` values from Task 5.
- Produces:
  - `frontend/src/lib/utils.js`: `export function isMetadataEditable(p)` — `true` for the existing photo formats **or** a video whose filename ends in `.mp4`/`.mov`/`.m4v`; `isFormatSupported` keeps its current meaning.
  - `frontend/src/lib/metadataErrors.js`: `export const METADATA_ERROR_KEYS = { invalid_date: 'ui.metadata.edit_error_invalid_date', invalid_coordinates: 'ui.metadata.edit_error_invalid_coordinates', unsupported_container: 'ui.metadata.edit_error_unsupported_container', no_location_carrier: 'ui.metadata.edit_error_no_location_carrier', no_writable_slot: 'ui.metadata.edit_error_no_writable_slot', unrepresentable_value: 'ui.metadata.edit_error_unrepresentable_value', file_read_only: 'ui.metadata.edit_error_file_read_only', file_missing: 'ui.metadata.edit_error_file_missing' };`
  - `frontend/src/lib/api.js`: the thrown `Error` for a non-2xx response carries `errorCode` (from the response JSON `error_code`, when present) and keeps the current `HTTP <status>: <message>` text.

- [ ] **Step 1: Write the failing test**

```js
// tests/metadata-errors.test.js  (node --test, picked up by `npm run test:unit`)
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { METADATA_ERROR_KEYS } from '../frontend/src/lib/metadataErrors.js';

const expectedCodes = ['invalid_date','invalid_coordinates','unsupported_container',
  'no_location_carrier','no_writable_slot','unrepresentable_value','file_read_only','file_missing'];

test('every backend refusal code has a localized message in both bundles', () => {
  const en = JSON.parse(readFileSync(new URL('../frontend/src/i18n/en.json', import.meta.url)));
  const de = JSON.parse(readFileSync(new URL('../frontend/src/i18n/de.json', import.meta.url)));
  assert.deepEqual(Object.keys(METADATA_ERROR_KEYS).sort(), [...expectedCodes].sort());
  for (const key of Object.values(METADATA_ERROR_KEYS)) {
    assert.equal(typeof getPath(en, key), 'string', `${key} missing in en.json`);
    assert.equal(typeof getPath(de, key), 'string', `${key} missing in de.json`);
  }
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm run test:unit`
Expected: FAIL — cannot find module `frontend/src/lib/metadataErrors.js`.

- [ ] **Step 3: Implement**

- `utils.js`: add `isMetadataEditable` (video extension whitelist `['.mp4', '.mov', '.m4v']` next to `VIDEO_EXTENSIONS` in `frontend/src/lib/constants.js` — add `METADATA_VIDEO_EXTENSIONS` there and consume it).
- `ViewerMetadata.svelte`: `showEditBtn = photo && !isCollagePhoto(photo) && isMetadataEditable(photo)`; extend `getFormatName`'s map with `'video/x-matroska': 'Matroska'`, `'video/webm': 'WebM'`, `'video/x-msvideo': 'AVI'` so the disabled tooltip names the container.
- `ViewerMetadataEdit.svelte`: `openModal` guard → `isMetadataEditable(photo)`; modal title picks `ui.metadata.edit_modal_title_video` for videos; in `handleSubmit`'s catch: when `error.errorCode` maps through `METADATA_ERROR_KEYS`, show `get(t)(key)` instead of the raw text; otherwise keep today's message extraction. Values stay in the form on failure (already the case — do not clear).
- `api.js` `request`: on `!response.ok`, attempt `JSON.parse(errorText)`; when the body has a string `error`, throw `Error('HTTP <status>: <error>')` with `error.errorCode = body.error_code`; otherwise keep the current behaviour byte-for-byte.
- i18n: add the eight `edit_error_*` keys plus `edit_modal_title_video` to **both** bundles, e.g. `"edit_error_no_location_carrier": "This video has no location field that can be updated."`, `"edit_error_no_writable_slot": "The video's metadata has no room for this value."`, `"edit_error_unrepresentable_value": "The video's existing metadata format cannot express this value."`, `"edit_error_unsupported_container": "Editing metadata is only supported for MP4, MOV and M4V videos."`, `"edit_error_file_read_only": "The video file is read-only."`, `"edit_error_file_missing": "The video file is missing."`, `"edit_error_invalid_date": "The date is not supported (1990 or later)."`, `"edit_error_invalid_coordinates": "The coordinates are invalid."` — German equivalents in `de.json` under the same paths.

- [ ] **Step 4: Run the tests and the frontend gates**

```bash
npm run test:unit && npm run test:i18n && npm run lint && npm run format
npm run build && cargo build --bin turbo-pix
```

Expected: unit test passes, i18n parity clean, no lint/format findings, both builds succeed.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/utils.js frontend/src/lib/constants.js frontend/src/lib/api.js \
  frontend/src/lib/metadataErrors.js frontend/src/components/ViewerMetadata.svelte \
  frontend/src/components/ViewerMetadataEdit.svelte frontend/src/i18n/en.json \
  frontend/src/i18n/de.json tests/metadata-errors.test.js
git commit -m "feat(ui): offer video metadata editing with localized refusal messages"
```

---

### Task 8: E2E — video metadata editing against the real server

**Files:**
- Create: `tests/e2e/specs/video-metadata.e2e.spec.js`
- Modify: `tests/e2e/setup/global-setup.js` (seed `test_video_quicktime_keys.mp4` with the existing pinned-date pattern)
- Modify: `tests/e2e/specs/map.e2e.spec.js` (its "videos carry no GPS" premise changes once the located fixture is indexed)
- Modify: `tests/e2e/specs/map-filters.e2e.spec.js` (same two-continent reason, one layer up: its located-video case is scoped to the video it seeds itself, not to the quicktime fixture)

Why the two map specs: the located fixture is real library data from Step 1, so it shows up in every
unfiltered map listing as a point in Vienna. Each affected case was narrowed to the media type it is
actually about, which keeps its own premise and only removes the foreign point:
- `map.e2e.spec.js`: the empty-state case asks for `?q=type%3Avideo test_video.mp4` (its own "no
  markers at all" premise, scoped to the videos it seeds), and the cluster-count / location-marker
  cases ask for `?q=type%3Aimage` — with the located video in the listing the fit spans two
  continents, and `focusLocation` then clicks a cluster whose zoom animation has detached the element.
- `map-filters.e2e.spec.js`: a video-scoped query narrowed to the seeded file —
  `?q=type%3Avideo%20test_video.mp4`, the same query the map shell spec uses for its own
  empty-state case — for the same reason: the quicktime fixture brings its own container
  coordinates in Vienna, so an unfiltered map holds a second location, the fit spans two
  continents, and the case's `focusLocation` call would have to click the Berlin+Wien cluster,
  a zoom whose animation detaches the element under the click. Its premise is "a video WITH
  coordinates is plotted and opens in the viewer", not the library's whole geometry.

**Interfaces:**
- Consumes: everything above; `TestHelpers` as-is.
- Produces: the acceptance evidence for SC-001..SC-003 at the UI level.

- [ ] **Step 1: Seed the fixture**

In `seedTestMedia()` (`tests/e2e/setup/global-setup.js`), next to the other videos, copy `test-data/test_video_quicktime_keys.mp4` into the photos dir and pin its mtime with `CLUSTER_DAYS_AGO + 6` (same pattern as the matrix fixtures) so it cannot displace the first video card. Do not remux it: `+faststart` keeps the indexing pass from rewriting it.

- [ ] **Step 2: Write the failing spec**

```js
// tests/e2e/specs/video-metadata.e2e.spec.js
// Fixture notes (test-data/): test_video.mp4 carries mvhd/tkhd/mdhd only (no location);
// test_video_quicktime_keys.mp4 carries com.apple.quicktime.creationdate and
// com.apple.quicktime.location.ISO6709; test_video_long.mkv is Matroska.
async function findVideoByFilename(page, filename) { /* GET /api/photos?q=type:video&limit=200 */ }
async function probe(path, entries) { /* execFileSync('ffprobe', ['-v','error','-show_entries', entries,'-of','json', path]) */ }
const fixturePath = (filename) => path.join('test-e2e-data', 'photos', filename);

test('GIVEN an MP4 video WHEN the capture date is saved THEN the file, the row and the grid agree', async ({ page }) => {
  // GIVEN test_video.mp4 (h264, no location carrier) indexed
  // WHEN the viewer is opened, #metadata-edit-btn is enabled, the modal is opened,
  //      #edit-taken-at is set to 2015-06-01T12:00 and Save is clicked
  // THEN the success toast appears, GET /api/photos?q=... reports taken_at 2015-06-01T12:00Z,
  //      ffprobe on the file reports format_tags.creation_time AND stream_tags.creation_time
  //      2015-06-01T12:00:00Z, and the file size is unchanged
  // AND reloading the videos view shows the card (sorted under the new date).
});

test('GIVEN a video carrying a QuickTime location WHEN coordinates are saved THEN the carrier is replaced', async ({ page }) => {
  // WHEN 52.52 / 13.405 are saved on test_video_quicktime_keys.mp4
  // THEN the modal closes with the success toast, the API reports location {52.52, 13.405},
  //      ffprobe reports format_tags["com.apple.quicktime.location.ISO6709"] == "+52.5200+013.4050/",
  //      the creationdate carrier still parses, and the file size is unchanged.
});

test('GIVEN a video without a location carrier WHEN coordinates are saved THEN the refusal is specific and nothing changes', async ({ page }) => {
  // GIVEN the byte snapshot + mtime of test_video.mp4
  // WHEN coordinates are entered in the modal and Save is clicked
  // THEN #metadata-edit-error shows the localized "no location field" text, the entered values
  //      are still in the form, and the file bytes + mtime are identical.
});

test('GIVEN a Matroska video WHEN the viewer is opened THEN the editor offers no save', async ({ page }) => {
  // #metadata-edit-btn is disabled and its title names Matroska; no modal opens on click.
});
```

The four scenarios above are the UI-level evidence; the review pass added the
cases the acceptance criteria ask for and these four do not reach:

- **Scenario 3 AC4 — the returned row.** `captureMetadataPatch` re-issues the
  request with `route.fetch()` and fulfills the route with that very response,
  so the response body the page received is assertable. The date save pins
  `taken_at`, `file_size` and `file_modified` to the patched file's own identity
  (the row described that file, so the handler restates the fingerprint and the
  next scan sees no change — the other half of SC-004), the location save pins
  `metadata.location`. The replaced-file branch, where the row disagrees with
  the file and keeps its stale fingerprint on purpose, stays in the handler's
  unit test: an e2e version would have to overwrite a shared fixture in place
  and restore it.
- **Scenario 1 AC3 / SC-002 — nothing else is lost.** `payloadDigest` skips the
  `moov` region, which is the region the save rewrites, so the date case reads
  the full `format_tags`/`stream_tags` map minus `creation_time` before and
  after the save and compares them: `encoder`, `comment` and the brand entries
  must all still be there.
- **Scenario 4 AC1 — the conversion overlap.** A sixth test takes
  `test_video_10bit.mp4` (the matrix's converting fixture: h264 High 10, so a
  Chromium client gets a conversion), claims the whole-file conversion with
  `?transcode=true`, waits for `InProgress`, saves the capture date while that
  job runs and waits for `Completed`. The artifact is still served cached, the
  row and the file carry the new instant, the payload outside `moov` is
  unchanged — and the finished artifact decodes in the viewer, because a
  conversion truncated by the write still renders a `<video>` and would pass a
  presence-only assertion.
- **Scenario 2 AC1 — the surfaces.** The location case also reads `#meta-gps` in
  the still-open viewer and plots the video on a `q=`-scoped map (stubbed
  tiles), so "the viewer shows the new coordinates" is observed instead of
  inferred from the row.

- [ ] **Step 3: Run the spec to verify it fails**

Run: `npm run test:e2e -- tests/e2e/specs/video-metadata.e2e.spec.js`
Expected: FAIL at the first test — `#metadata-edit-btn` is disabled today (`isFormatSupported` accepts JPEG/PNG only).

- [ ] **Step 4: Adjust the map spec's premise and make the spec pass**

`map.e2e.spec.js`'s "filters with matching photos but no coordinates show the empty state" currently uses `q=type:video`; the located fixture now plots, so target a video that still has no coordinates (free-text `test_video.mp4`) and keep asserting the empty state.

Run: `npm run test:e2e -- tests/e2e/specs/video-metadata.e2e.spec.js tests/e2e/specs/map.e2e.spec.js tests/e2e/specs/map-filters.e2e.spec.js`
Expected: PASS.

- [ ] **Step 5: Run the whole suite**

Run: `npm run test:e2e`
Expected: PASS (treat a first-run global-setup error as the documented infra race and re-run once).

- [ ] **Step 6: Commit**

```bash
git add tests/e2e/specs/video-metadata.e2e.spec.js tests/e2e/specs/map.e2e.spec.js tests/e2e/setup/global-setup.js
git commit -m "test(e2e): cover in-place video metadata editing end to end"
```

---

### Task 9: Full gate, docs touch-up, and learnings

**Files:**
- Modify: `AGENTS.md` (Learnings section), `README.md` (only if it documents supported metadata editing)

- [ ] **Step 1: Run every gate in order**

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
npm run test:unit && npm run test:i18n && npm run lint
npm run build && cargo build --bin turbo-pix
```

Expected: all clean. On a host with ffmpeg, additionally: `RUN_VIDEO_TESTS=1 cargo test`.

- [ ] **Step 2: Smoke the real server**

```bash
nohup cargo run & 
curl --retry 5 --retry-delay 2 http://localhost:18473/health
# copy test-data/test_video_quicktime_keys.mp4 into the data dir, wait for indexing,
# open the viewer, save a date and a location, then confirm with
# ffprobe -v error -show_entries format_tags -of json <the indexed file>
pkill -f 'target/debug/turbo-pix'
```

Expected: both saves succeed instantly; ffprobe shows the new `creation_time`, the new ISO6709 value, and the untouched `encoder` tag; the file's size and mtime are unchanged.

- [ ] **Step 3: Merge the session's learnings into `AGENTS.md`**

Fold into existing entries (never append an 11th): the ISO-BMFF carrier map (which box ffprobe reads for which tag), the "restore the mtime or every size+mtime cache invalidates" rule, the `json_patch` scan-merge fix for `location.city`/`capability_version`, and the `+faststart` requirement for any fixture that must keep QuickTime keys.

- [ ] **Step 4: Commit**

```bash
git add AGENTS.md README.md
git commit -m "docs: record in-place video metadata editing learnings"
```

---

## Self-review

**Spec coverage.** FR-001 → T5 (route branch by `video/*`) + T7 (button enabled for `.mp4/.mov/.m4v`); FR-002 → T5 (rollback on DB failure, refusal before write); FR-003 → T2 (single same-length `moov` write, mdat byte proof, moov-at-end test); FR-004 → T2 (mvhd/tkhd/mdhd) + T3 (every text date carrier); FR-005 → T4; FR-006 → T2 (`UndoToken`, refusal-before-write tests in every task); FR-007 → T6 (container location extraction + merge-preserving upsert) and T2/T3/T4 (read-back via `read_metadata`); FR-008 → T2 (mtime restore + stat-derived fingerprint) + T6 (`capability_version` survives); FR-009 → T5 (edit lock, concurrent test) + T2/T7 (mdat untouched, playback during save); FR-010 → T5 (eight distinct codes and statuses) + T7 (localized strings); FR-011 → T5 (photo path untouched) + T6 (photo upsert semantics unchanged except preservation); FR-012 → T7; FR-013 → T5 (empty request no-op) + T7 (clearing keeps today's client behaviour); FR-014 → T3/T4 (shape-preserving rendering, NUL padding, refusal instead of truncation). Scenarios 1-4 and SC-001..SC-005 each map onto the tasks above; SC-005's "existing suites pass" is T9.

**Type consistency.** `Mp4MetadataError` variants, `VideoMetadataEdit`, `VideoMetadataWrite`, `Fingerprint`, `UndoToken`, `render_date_in_shape`, `render_iso6709_in_shape`, `isMetadataEditable`, `METADATA_ERROR_KEYS` and the eight `error_code` strings are used with identical names and shapes in every task that references them.

**Open decisions made here (not left to the implementer).** mtime is restored after a save; a shorter text value is NUL-padded inside the existing payload slot; `moov` is always rebuilt to its original byte length as `children' || free(pad)` so a render that grows is paid for only out of the file's own free room inside `moov` (a leftover of 1-7 bytes is `NoRoom`, never a malformed box or a media shift); the fingerprint stored in the row is the post-write stat truncated to whole seconds, adopted only when the row already described the patched file (a row whose file was replaced behind its back keeps its stale fingerprint so the next scan re-extracts it); a requested date the readback finds in no carrier at all is refused as `unrepresentable_value` (the file is rolled back first); the empty request is a 200 no-op; refusals for fragmented layouts and for exhausted room share the `no_writable_slot` code; `test_video_quicktime_keys.mp4` is generated with `+faststart` so the indexing pass cannot remux its QuickTime keys away.
