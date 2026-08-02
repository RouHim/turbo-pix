# Tech Debt Audit — TurboPix

Run date: 2026-08-02 · Branch: `feat/svelte-frontend-migration` · Depth: standard
Audit mode: full (audit + fixes, everything except F9 by user decision)

## Executive summary

- Fixed **12 findings** this run (all dead code, three duplication clusters, one complexity pass), zero remaining open findings besides deliberately deferred ones.
- Deleted `src/db_schema.rs` (stale pre-migration schema, referenced `rusqlite` which isn't even a dependency) and the orphaned vitest files (`tests/i18n.test.js`, `tests/setup.js`) that tested a mocked i18n API that no longer exists and could not run.
- Consolidated the duplicated EXIF read/write pipeline (6 reader sites in 4 files + 2 identical writer blocks) into one `src/exif_helpers.rs` module; the two writer copies had already drifted (`"jpg" | "jpeg"` vs `"jpeg"`).
- Split 7 high-complexity functions (PhotoGrid.loadPhotos 30→~8, ViewerMetadataEdit.handleSubmit 22→~5, PhotoViewer onKeydown 19→~4, displayVideo 20→~10, onPan 17→~5, SwipeRecognizer.recognize 18→~6, IndexingOrbit/TimelineSlider transforms). One function remains at 16 (`api.getPhotos`) — verdict: inherent sequential param building, not worth it.
- Added a cross-language extension parity test: the frontend's `VIDEO_EXTENSIONS`/`RAW_EXTENSIONS` lists are now pinned against the backend detector sets (was silent-drift risk; both happen to match today).
- Verification: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (254 tests), `npm run lint`, `npm run format:check`, `npm run build`, full Playwright suite (150 tests).

## Architectural mental model

TurboPix is a self-hosted photo/video gallery: a single Rust binary (warp + sqlx/SQLite + sqlite-vec + candle ML) that scans photo directories, extracts EXIF/ffprobe metadata, generates CLIP embeddings and thumbnails, and serves an embedded SPA. The frontend is Svelte 5 (runes) + Vite, freshly migrated from vanilla JS (this branch), embedded into the binary via `build.rs` (which panics on stale dist — deliberate guard).

Key hot paths: the indexing pipeline (`scheduler.rs` → `photo_processor`/`metadata_extractor`/`video_processor`/`semantic_search`), photo listing/search (`db.rs`), collage generation (`collage_generator.rs`, the largest file at 2160 lines but well-structured), and the viewer (`PhotoViewer.svelte`, 1794 lines, the largest component — video transcoding + gestures + zoom, heavily pinned by E2E).

The repo has been through 7+ adversarial review rounds; most architectural debt was already paid. What remained was the residue: stale dead files from the migration, duplicated EXIF plumbing that had started to drift, and a few genuinely branchy functions.

## Findings table

| ID | Category | File:Line | Severity | Effort | Status | Description | Recommendation |
|---|---|---|---|---|---|---|---|
| F1 | Dead code | `src/lib.rs:15` (commented module); `src/db_schema.rs` whole file | Med | S | **FIXED** | Stale pre-migration schema; uses `rusqlite` (not a dep), superseded by sqlx migrations | Delete file |
| F2 | Dead code | `src/lib.rs:46-50` | Med | S | **FIXED** | 5 root re-exports (`Config`, `create_db_pool`, `SemanticSearch`, `extract_frames_batch`, `extract_video_metadata`) with zero consumers — benchmark and main use direct module paths | Remove re-exports |
| F3 | Test debt | `tests/i18n.test.js:1`, `tests/setup.js:2` | Med | S | **FIXED** | Orphaned vitest files: vitest not in package.json/lock; playwright testDir excludes them; they test `mockLL`/`global.utils` that don't exist post-migration | Delete files |
| F4 | Duplication | `image_editor.rs:317-357` vs `metadata_writer.rs:205-256` | Med | M | **FIXED** | Identical EXIF Writer→JPEG/PNG set_exif→write blocks; format strings already diverged (`"jpg"\|"jpeg"` vs `"jpeg"`) | Extract shared helper |
| F5 | Complexity | `PhotoGrid.svelte:62` (30), `ViewerMetadataEdit.svelte:116` (22), `PhotoViewer.svelte:586` (20), `:1058` (19), `:323` (17), `recognizers.js:37` (18) | Med | M | **FIXED** | 6 functions ≥17 cyclomatic complexity (playbook: 21+ critical) | Split per function (see Fixed section) |
| F6 | Dead code | `src/db.rs:935` | Low | S | **FIXED** | Empty `#[cfg(test)] impl Photo {}` block | Remove |
| F7 | Duplication | `PhotoViewer.svelte:67-81`, `ViewerMetadata.svelte:14-22` | Low | S | **FIXED** | `isVideoFile`/`isRawFile`/`isCollagePhoto` defined twice (plus a third definition consumed via `this.viewer.isVideoFile`) | Move to `utils.js` |
| F8 | Dead code | `blurhash.js:212` | Low | S | **FIXED** | `createCanvas` exported but only used internally by `toDataURL` | Drop `export` |
| F9 | Consistency rot | `app.css` (1761 lines) | Low | L | **REJECTED (user decision)** | Global `.viewer-*`/`.sidebar`/`.photo-card-*` rules coexist with scoped styles; learnings document hard-won battles over this split | Move into scoped styles — not worth the risk now; E2E pins behavior, payoff small |
| F10 | Duplication | `handlers_static.rs:27-89` | Low | S | **FIXED** | `build_route_for_file`/`build_route_for_binary_file` identical except str/bytes body | Merge into one byte-based builder |
| F11 | Duplication | `handlers_photo.rs:382`, `image_editor.rs:265`, `metadata_extractor.rs:103`, `metadata_writer.rs:88/284/919` | Med | M | **FIXED** (with F4) | `exif::Reader::new()` + `read_from_container` boilerplate at 6 sites | Shared reader helper |
| F12 | Deps | `Cargo.toml` `kamadak-exif` | — | — | **REJECTED** | cargo-machete false positive: crate's lib name is `exif`, used in 4 files | Add to machete ignore list if it annoys |
| F13 | Test debt | `housekeeping_manager.rs`, `file_scanner.rs`, `cache_manager.rs` | Low | M | **Not worth doing** | Small leaf modules (2.5-3.7KB) with zero tests; logic is trivial and exercised indirectly via E2E | Write unit tests only when the modules change |
| F14 | Documentation drift | `CHANGELOG.md` (1319 lines) | Low | S | **Not worth doing** | semantic-release generates empty sections for dependency bumps (normal for this toolchain) | Nothing to fix |
| F15 | Consistency rot | `constants.js:22` vs `mimetype_detector.rs:37-66` / `raw_processor.rs:361` | Low | S | **FIXED** | Frontend and backend extension lists match today but nothing pinned them; drift breaks video flagging or detection silently | Parity test added (`frontend_extension_lists_match_backend`) |
| F16 | Complexity | `api.js:84` `getPhotos` (16) | Low | S | **Not worth doing** | 11 sequential independent `if (param) set()` — complexity score misleading; a lookup table would hurt readability | Leave |
| F17 | Complexity | `TimelineSlider.svelte:253` route-restore effect | Low | S | **FIXED** (guarded) | Complexity 16; effect is documented as fragile (AGENTS.md learning: must read `route.year`/`route.month` before the drag guard) | Extracted `restoreFilterFromRoute`, kept the dependency-read contract intact |

## Fixed this run

### F1+F2+F3+F6+F8 — Dead code removal (5 files, one unit)
- Deleted `src/db_schema.rs`, `tests/i18n.test.js`, `tests/setup.js`.
- Removed 5 unused root re-exports from `src/lib.rs` and the empty `impl Photo {}` test block in `src/db.rs`.
- Dropped the `export` from `createCanvas` in `frontend/src/lib/blurhash.js`.
- Verification: `cargo check --all-targets`, `cargo clippy -- -D warnings` clean (unused imports would have errored).

### F4+F11 — Shared EXIF helpers (`src/exif_helpers.rs`, new, 76 lines)
- `read_exif<R: BufRead + Seek>` / `read_exif_from_path` / `build_exif_buffer` / `write_exif_to_image`.
- Updated 4 callers: `metadata_extractor.rs` (1 site), `handlers_photo.rs` (1 site, preserved distinct open-vs-read error responses), `image_editor.rs` (2 blocks), `metadata_writer.rs` (2 blocks).
- Error messages preserved byte-for-byte; `write_exif_to_image` accepts `jpg`|`jpeg`|`png` (both caller dialects).
- Tests: existing `metadata_writer` (19) + `image_editor` (6) + `metadata_extractor` (56) suites — all pass unchanged, covering JPEG/PNG round-trips, pixel preservation, multi-write cycles.

### F10 — Merged static route builders
- `build_route_for_file` + `build_route_for_binary_file` → one `build_route(path, content: &[u8])` + `build_asset_response` fn item.
- Added `static_routes_serve_index_assets_and_spa_fallback` (tokio test): index at `/` with no-cache, every embedded asset served byte-identical, SPA fallback for unknown GET paths, `/api/*` still 404.
- Requires warp `test` feature (added to Cargo.toml).

### F5 — Complexity splits (7 functions)
- **PhotoGrid.loadPhotos 30 → ~8**: extracted `applyResetState`, `loadSemanticPage` (returns `null` on stale, preserving the no-append early-return semantics), `loadRegularPage`, `appendPhotos`. All hard-won edge-case comments preserved (dedupe signature, semantic staleness guard, cooldown reset).
- **ViewerMetadataEdit.handleSubmit 22 → ~5**: extracted `buildUpdatesFromForm` (validation + payload; returns `{updates}` or `{error}`).
- **PhotoViewer.onKeydown 19 → ~4**: switch → `viewerKeyHandlers` dispatch table + `onKeydown` guard wrapper.
- **PhotoViewer.displayVideo 20 → ~10**: extracted `tryStartTranscode` (returns true when a transcode flow started / failed / photo went stale).
- **PhotoViewer.onPan 17 → ~5**: extracted `handleZoomedPan` (edge-swipe + pan consumption).
- **SwipeRecognizer.recognize 18 → ~6**: horizontal/vertical near-duplicate branches → single `classifySwipe` axis classifier.
- **IndexingOrbit sheetPhases arrow 16 → 2** (moved into `buildSheetPhase` + `phasePercent`; the transform stays a pure mapping) and **TimelineSlider route-restore effect 16 → 2** (`restoreFilterFromRoute` — the `$effect` still reads `route.year`/`route.month` before the `dragInProgress` guard, per the AGENTS.md learning).
- Verification: eslint complexity re-scan dropped from 10 warnings to 1 (`api.getPhotos`, verdict: not worth doing).

### F7 — File-type helpers consolidated
- `utils.js` now exports `isVideoFile`/`isRawFile`/`isCollagePhoto`; PhotoViewer and ViewerMetadata import them. `SwipeableViewer` still consumes `this.viewer.isVideoFile` via the exposed API (unchanged).

### F15 — Extension parity test
- `mimetype_detector.rs::frontend_extension_lists_match_backend` parses `frontend/src/lib/constants.js` (include_str) and asserts the video/RAW lists match the backend exactly, in both directions (detector + `raw_processor::is_raw_file`).

## Top 5 remaining

1. **F9 — Global vs scoped CSS split (`app.css`).** The only L-effort item. The viewer/sidebar/card layout lives partly in global CSS, partly in component scoped styles, and AGENTS.md documents repeated regressions when the two fight (scoped beats global of equal specificity). Consolidating into scoped styles is a large, E2E-pinned effort with no functional payoff — revisit only when touching those components for feature work.
2. **PhotoViewer.svelte (1794 lines).** The god component of the app. Splitting it further (e.g. a video-transcode composable) is the natural next structural step, but every async continuation inside carries a documented staleness guard (AGENTS.md) and the component is the most E2E-pinned file. Extract only alongside a feature touch, never as a standalone refactor.
3. **collage_generator.rs (2160 lines).** Largest Rust file; well-organized (template scoring/layout/rendering sections) and 23 tests. The `generate_collages` orchestration (1312-1441) is the only dense region. Same verdict as PhotoViewer: split when touched.
4. **Zero-test leaf modules** (`housekeeping_manager.rs`, `file_scanner.rs`, `cache_manager.rs`). Cheap to cover with unit tests when next modified.
5. **Machete ignore list for `kamadak-exif`.** One line in Cargo.toml (`[package.metadata.cargo-machete] ignored = ["kamadak-exif"]`) so future machete runs don't re-flag it.

## Quick wins (checklist)

- [x] Delete `src/db_schema.rs`, `tests/i18n.test.js`, `tests/setup.js`
- [x] Remove unused lib.rs re-exports and empty `impl Photo {}`
- [x] Unexport `blurhash.js` `createCanvas`
- [x] `utils.js` file-type helpers
- [x] Extension parity test
- [ ] Add machete ignore entry for `kamadak-exif` (didn't touch Cargo.toml metadata; 2-line change if desired)

## Things that look bad but are actually fine

- **`app.css` at 1761 lines with global `.viewer-*` rules while components are scoped.** Deliberate migration state; learnings document exactly which global rules are live vs shadowed. Flagged as F9 anyway (the split is drift-prone) but not fixed.
- **`kamadak-exif` "unused" per machete.** False positive — lib name is `exif`; used in 4 files.
- **CHANGELOG.md with dozens of empty release sections.** semantic-release autogenerates entries for dep bumps; conventional for this toolchain.
- **`db.rs` thin test helpers delegating to `db_pool`** (`create_test_db_pool` → `create_in_memory_pool` → `db_pool`). Two-line indirection with a purpose (test-module visibility).
- **`isVideoFile` exposed on the viewer API object.** SwipeableViewer calls `this.viewer.isVideoFile(...)` — an awkward coupling but pinned by E2E; removing it would require changing the viewer's public surface.
- **Warp route `.or()` chains in handlers.** Idiomatic warp; each handler module owns its routes.
- **`backend q` tokenization with `location:` absorbing words** — documented in AGENTS.md as a deliberate parser behavior.
- **En dash / em dash characters in comments and i18n defaults.** Not debt; consistent style.

## Open questions for the maintainer

- **F9 CSS consolidation**: is there a plan to eventually move `.viewer-*`/`.sidebar`/`.photo-card-*` global rules into scoped styles, or is the global layer the intended home for shared layout? The learnings say "keep scoped" but the global layer is still the primary home for viewer layout.
- **warp `test` feature** was enabled for the route smoke test — acceptable? (It compiles warp's test module into all builds; dead-code eliminated in release.)
- **GestureManager `handleTouchEnd`**: the old `if (!touch) return` skipped cleanup of the ended touch (potential stale-touch accumulation). The refactor now always cleans up. If the missing-touch case was load-bearing for some device, that behavior changed — E2E swipe/tap suites passed.

## Not audited

- `CHANGELOG.md` full contents (only format checked), `Cargo.lock` (dependency versions audited only via machete; `cargo audit` not installed — noted, not installed).
- `container-data/Containerfile` and docker-compose (deployment config; CI exercises the image build).
- `test-data/` binaries and `.spec/` planning docs.
- `knip`/`madge`/`depcheck`/`cargo-udeps`/`cargo-audit` not installed — dead-export scan done manually (found F8), unused-deps via machete only (found F12 false positive).
- E2E specs were counted (150 tests, 23 specs) but not line-audited.
