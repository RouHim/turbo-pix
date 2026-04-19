## Summary

| Metric | Value |
|--------|-------|
| Tasks Attempted | 56 |
| Tasks Succeeded | 49 |
| Tasks Failed | 7 |
| Tasks Skipped | 6240 |
| Duration | 120m 11s |
| Cost | $0.0000 |

## Category Breakdown

| Category | Attempted | Succeeded | Failed | Skipped |
|----------|-----------|-----------|--------|---------|
| Complexity | 56 | 49 | 7 | 0 |

## Commits

- `53252d3` refactor(scheduler.rs): Function 'run_startup_rescan' is 129 lines long (max 50)
- `413aed8` refactor(scheduler.rs): Function 'start' is 143 lines long (max 50)
- `a25aafc` refactor(scheduler.rs): Function 'batch_write_photos' is 74 lines long (max 50)
- `56d4956` refactor(raw_processor.rs): Function 'decode_raw_to_dynamic_image' is 112 lines long (max 50)
- `20c12fc` refactor(photo_processor.rs): Function 'batch_compute_semantic_vectors' is 152 lines long (max 50)
- `0e16e19` refactor(photo_processor.rs): Function 'process_file_metadata_only' is 68 lines long (max 50)
- `8cf6ad1` refactor(photo_processor.rs): Function 'full_rescan_and_cleanup' is 144 lines long (max 50)
- `b1c99bf` refactor(mimetype_detector.rs): Function 'from_extension' is 57 lines long (max 50)
- `62136e1` refactor(metadata_writer.rs): Function 'test_read_write_cycle_with_complete_exif' is 52 lines long (max 50)
- `578cca6` revert: f55a490
- `9fba321` revert: 4ff379c
- `7a89376` revert: 9862e64
- `a1e6545` revert: 74491ec
- `66a0053` revert: 6992ea2
- `eecef22` revert: 3de249d
- `d2c953e` revert: d34891a
- `94c232b` revert: eac6c50
- `803e9a5` revert: 6e796a1
- `ff89a5c` revert: aafeaba
- `f55a490` refactor(metadata_writer.rs): Function 'test_pixel_perfect_preservation' is 61 lines long (max 50)
- `4ff379c` refactor(metadata_writer.rs): Function 'update_metadata' is 257 lines long (max 50)
- `9862e64` refactor(metadata_extractor.rs): Function 'test_parse_video_creation_time' is 52 lines long (max 50)
- `74491ec` refactor(metadata_extractor.rs): Function 'extract_video_metadata' is 84 lines long (max 50)
- `6992ea2` refactor(metadata_extractor.rs): Function 'extract_camera_info' is 77 lines long (max 50)
- `3de249d` refactor(metadata_extractor.rs): Function 'extract_basic_info' is 61 lines long (max 50)
- `d34891a` refactor(main.rs): Function 'main' is 81 lines long (max 50)
- `eac6c50` refactor(image_editor.rs): Function 'test_delete_photo_read_only_directory_returns_permission_denied' is 53 lines long (max 50)
- `6e796a1` refactor(image_editor.rs): Function 'delete_photo' is 61 lines long (max 50)
- `aafeaba` refactor(image_editor.rs): Function 'reset_exif_orientation' is 93 lines long (max 50)
- `1f7baa6` revert: d8370c3
- `be86340` revert: 518eca4
- `14a1de1` revert: 44d0acd
- `eea0243` revert: ceb5a41
- `ba20821` revert: e04c030
- `76d4ce7` revert: 2eedd6b
- `e5d87f2` revert: 834e589
- `2d04d5e` revert: 70055ff
- `649ecc6` revert: b43a237
- `5f7deee` revert: c2429b3
- `d8370c3` refactor(image_editor.rs): Function 'rotate_image' is 155 lines long (max 50)
- `518eca4` refactor(housekeeping_manager.rs): Function 'run_housekeeping_scan' is 97 lines long (max 50)
- `44d0acd` refactor(handlers_video.rs): Function 'get_video_file' is 267 lines long (max 50)
- `ceb5a41` refactor(handlers_thumbnail.rs): Function 'get_photo_thumbnail' is 51 lines long (max 50)
- `e04c030` refactor(handlers_static.rs): Function 'build_static_routes' is 64 lines long (max 50)
- `2eedd6b` refactor(handlers_search.rs): Function 'semantic_search' is 79 lines long (max 50)
- `834e589` refactor(handlers_photo.rs): Function 'build_photo_routes' is 126 lines long (max 50)
- `70055ff` refactor(handlers_photo.rs): Function 'update_photo_metadata' is 91 lines long (max 50)
- `b43a237` refactor(handlers_photo.rs): Function 'get_photo_file' is 92 lines long (max 50)
- `c2429b3` refactor(db_pool.rs): Function 'test_delete_orphaned_photos_cleans_feature_vectors' is 80 lines long (max 50)
- `e299908` revert: ea991da
- `c311de5` revert: 69a94a8
- `a957a64` revert: a7c43b3
- `555e2a3` revert: 7cdbee8
- `e55f528` revert: 7993e98
- `63392c1` revert: 3aa6c09
- `727fae7` revert: c6a5c34
- `c55e7ba` revert: bac67cb
- `e381c0c` revert: 3d53208
- `ea991da` refactor(db_pool.rs): Function 'delete_not_in' is 58 lines long (max 50)
- `69a94a8` refactor(db.rs): Function 'test_get_photos_needing_geo_resolution' is 51 lines long (max 50)
- `a7c43b3` refactor(db.rs): Function 'test_transaction_atomicity' is 57 lines long (max 50)
- `2bcfbc2` refactor(db.rs): Function 'test_get_timeline_data' is 81 lines long (max 50)
- `7cdbee8` refactor(db.rs): Function 'from' is 112 lines long (max 50)
- `7993e98` refactor(db.rs): Function 'search_photos' is 106 lines long (max 50)
- `3aa6c09` refactor(db.rs): Function 'create_or_update_with_transaction' is 70 lines long (max 50)
- `c6a5c34` refactor(db.rs): Function 'update_with_old_hash' is 60 lines long (max 50)
- `bac67cb` refactor(db.rs): Function 'from_row' is 69 lines long (max 50)
- `3d53208` refactor(collage_generator.rs): Function 'test_generate_collages_skips_existing_signature' is 53 lines long (max 50)
- `134f4e7` revert: 500adc3
- `6720584` revert: 50c7eca
- `7c138a7` revert: eb3b276
- `deaaa38` revert: 8186142
- `2e88ffc` revert: f3283f3
- `0e4a51f` revert: a2a3092
- `499aefc` revert: 9d3f5bd
- `85415e3` revert: 9882eef
- `a0e31be` revert: d68b65e
- `b23b609` revert: 9ed3830
- `500adc3` refactor(collage_generator.rs): Function 'test_list_pending_cleaned_removes_missing_and_duplicates' is 77 lines long (max 50)
- `50c7eca` refactor(collage_generator.rs): Function 'index_collage_file' is 53 lines long (max 50)
- `eb3b276` refactor(collage_generator.rs): Function 'generate_collages' is 129 lines long (max 50)
- `8186142` refactor(collage_generator.rs): Function 'chunk_photos' is 58 lines long (max 50)
- `f3283f3` refactor(collage_generator.rs): Function 'create_collage_image' is 216 lines long (max 50)
- `a2a3092` refactor(collage_generator.rs): Function 'stroke_rect' is 63 lines long (max 50)
- `9d3f5bd` refactor(collage_generator.rs): Function 'score_template' is 75 lines long (max 50)
- `9882eef` refactor(collage_generator.rs): Function 'generate_template_cells' is 286 lines long (max 50)
- `d68b65e` refactor(collage_generator.rs): Function 'build_collage_signature' is 51 lines long (max 50)
- `9ed3830` refactor(benchmark_video.rs): Function 'main' is 120 lines long (max 50)

## Failures

Showing top 7 of 7 failures:

| # | Gate | File | Reason |
|---|------|------|--------|
| 1 | safety | src/semantic_search.rs | Potential secret detected: 'TOKEN: u32' |
| 2 | safety | src/semantic_search.rs | Potential secret detected: 'TOKEN: u32' |
| 3 | safety | src/semantic_search.rs | Potential secret detected: 'TOKEN: u32' |
| 4 | safety | src/semantic_search.rs | Potential secret detected: 'TOKEN: u32' |
| 5 | safety | src/semantic_search.rs | Potential secret detected: 'TOKEN: u32' |
| 6 | safety | src/semantic_search.rs | Potential secret detected: 'TOKEN: u32' |
| 7 | safety | src/semantic_search.rs | Potential secret detected: 'TOKEN: u32' |

## Budget Usage

| Resource | Used | Limit |
|----------|------|-------|
| Cost | $0.0000 | unlimited |
| Time | 120m 11s | 120m 0s |
| Tasks | 49 | unlimited |

## Common Failure Patterns

1. **Potential secret detected: 'TOKEN: u32'** — 7 occurrence(s)