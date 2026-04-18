## Summary

| Metric | Value |
|--------|-------|
| Tasks Attempted | 30 |
| Tasks Succeeded | 17 |
| Tasks Failed | 13 |
| Tasks Skipped | 6190 |
| Duration | 60m 29s |
| Cost | $0.0000 |

## Category Breakdown

| Category | Attempted | Succeeded | Failed | Skipped |
|----------|-----------|-----------|--------|---------|
| Complexity | 30 | 17 | 13 | 0 |

## Commits

- `3a7b546` refactor(handlers_photo.rs): Function 'test_update_photo_metadata_endpoint' is 54 lines long (max 50)
- `31b7621` refactor(handlers_photo.rs): Function 'get_photo_exif' is 67 lines long (max 50)
- `9737831` refactor(handlers_photo.rs): Function 'list_photos' is 57 lines long (max 50)
- `b993d0d` refactor(handlers_indexing.rs): Function 'build_phases' is 51 lines long (max 50)
- `520ec00` refactor(handlers_collage.rs): Function 'build_collage_routes' is 52 lines long (max 50)
- `dc46c62` refactor(db_pool.rs): Function 'delete_orphaned_photos' is 76 lines long (max 50)
- `8d8a920` refactor(db_pool.rs): Function 'create_db_pool' is 54 lines long (max 50)
- `7a5b4af` revert: afca57a
- `8363c05` revert: c133541
- `593c7ae` revert: 3cfcb45
- `48b0105` revert: 52a15c6
- `72fd303` revert: bd5d89a
- `265fcf6` revert: 9970917
- `0a50adb` revert: fad7096
- `1c66969` revert: 34d65a7
- `603a0fb` revert: 1112890
- `9166bbc` revert: 5fd01d3
- `afca57a` refactor(db.rs): Function 'test_get_photos_needing_geo_resolution' is 51 lines long (max 50)
- `c133541` refactor(db.rs): Function 'test_transaction_atomicity' is 57 lines long (max 50)
- `3cfcb45` refactor(db.rs): Function 'update_with_old_hash' is 60 lines long (max 50)
- `52a15c6` refactor(collage_generator.rs): Function 'test_generate_collages_skips_existing_signature' is 53 lines long (max 50)
- `bd5d89a` refactor(collage_generator.rs): Function 'index_collage_file' is 53 lines long (max 50)
- `9970917` refactor(collage_generator.rs): Function 'chunk_photos' is 58 lines long (max 50)
- `fad7096` refactor(collage_generator.rs): Function 'stroke_rect' is 63 lines long (max 50)
- `34d65a7` refactor(collage_generator.rs): Function 'score_template' is 75 lines long (max 50)
- `1112890` refactor(collage_generator.rs): Function 'generate_template_cells' is 286 lines long (max 50)
- `5fd01d3` refactor(collage_generator.rs): Function 'build_collage_signature' is 51 lines long (max 50)

## Failures

Showing top 10 of 13 failures:

| # | Gate | File | Reason |
|---|------|------|--------|
| 1 | safety | src/bin/benchmark_video.rs | Diff too large: 152 changed lines exceeds limit of 100 |
| 2 | safety | src/collage_generator.rs | Diff too large: 103 changed lines exceeds limit of 100 |
| 3 | safety | src/collage_generator.rs | Diff too large: 129 changed lines exceeds limit of 100 |
| 4 | safety | src/collage_generator.rs | Diff too large: 103 changed lines exceeds limit of 100 |
| 5 | safety | src/db.rs | Diff too large: 104 changed lines exceeds limit of 100 |
| 6 | safety | src/db.rs | Diff too large: 101 changed lines exceeds limit of 100 |
| 7 | safety | src/db.rs | Diff too large: 177 changed lines exceeds limit of 100 |
| 8 | safety | src/db.rs | Diff too large: 181 changed lines exceeds limit of 100 |
| 9 | safety | src/db.rs | Diff too large: 112 changed lines exceeds limit of 100 |
| 10 | safety | src/db_pool.rs | Diff too large: 147 changed lines exceeds limit of 100 |

## Budget Usage

| Resource | Used | Limit |
|----------|------|-------|
| Cost | $0.0000 | unlimited |
| Time | 60m 29s | 60m 0s |
| Tasks | 17 | unlimited |

## Common Failure Patterns

1. **Diff too large: 103 changed lines exceeds limit of 100** — 2 occurrence(s)
2. **Diff too large: 104 changed lines exceeds limit of 100** — 1 occurrence(s)
3. **Diff too large: 101 changed lines exceeds limit of 100** — 1 occurrence(s)