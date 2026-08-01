# Feature Specification: Reduce Idle Memory Footprint

**Created**: 2026-06-08
**Status**: Approved
**Input**: Production with >90k media items results in ~1.2 GiB RAM usage even with no users visiting — reduce idle footprint

## Goal

TurboPix consumes ~1.2 GiB of RAM at idle (zero active HTTP requests, no indexing in progress) with a 90k+ media library. The dominant contributors are SQLite's oversized `mmap_size` (512 MB), oversized `cache_size` (128 MB), and an over-provisioned connection pool. The CLIP model (~400 MB) for semantic search remains resident to guarantee query responsiveness. This specification reduces idle RAM through conservative, research-backed hardcoded defaults for SQLite and connection pool sizing, with no new configuration mechanisms.

## User Scenarios

### Scenario 1 — Server idle with large library (P1)

A self-hosted TurboPix instance with 90,000+ photos and videos sits idle overnight. No user is browsing the web UI, no background indexing is active, and no collage generation is running. The server process should consume significantly less RAM than the current ~1.2 GiB.

**Acceptance**
1. Given a TurboPix server with a 90k-item library and no active HTTP requests, When the process has been idle for 5 minutes, Then resident memory is measurably lower than the pre-change baseline (targeting a reduction of at least 100 MB from SQLite tuning alone).
2. Given the server is idle, When a user sends the first photo-grid query, Then response latency is not measurably worse than pre-change (within 10% of baseline).

### Scenario 2 — Normal browsing after idle period (P2)

A user returns to the gallery after the server has been idle. They browse the photo grid, open a photo in the viewer, and perform a text search. All operations complete with normal responsiveness.

**Acceptance**
1. Given the server has been idle for several hours, When a user browses pages of the photo grid, Then each page loads within the same latency envelope as pre-change.
2. Given the server has been idle, When a user opens the viewer for a photo, Then the full-resolution image and metadata load without perceptible delay compared to pre-change.

## Functional Requirements

- **FR-001**: SQLite `mmap_size` SHALL be set to 268435456 (256 MB), replacing the current value of 536870912 (512 MB).
- **FR-002**: SQLite `cache_size` SHALL be set to -32000 (32 MB), replacing the current value of -128000 (128 MB).
- **FR-003**: The connection pool maximum size SHALL be reduced from `(num_cores * 2) + 10` to `(num_cores * 2) + 2`. The minimum idle connections SHALL remain at 2.
- **FR-004**: All other SQLite PRAGMA settings (journal_mode=WAL, synchronous=Normal, temp_store=MEMORY, busy_timeout=30s, wal_autocheckpoint=10000, analysis_limit=1000) SHALL be preserved unchanged.
- **FR-005**: The CLIP ViT-B/32 model for semantic search SHALL remain loaded at all times (no lazy loading, no unloading at idle).
- **FR-006**: The geo-coordinate cache (NominatimClient, 100k-entry cap with clear-on-full eviction) SHALL remain unchanged.

## Edge Cases

- **Database larger than mmap_size**: If the database file grows beyond 256 MB, SQLite will still function correctly — the first 256 MB is memory-mapped and the remainder uses `read()`/`write()` syscalls. Performance on queries touching pages beyond the mmap window may degrade slightly; this is acceptable for the target library size.
- **Extremely small libraries**: For libraries with <1,000 items, the 32 MB cache and 256 MB mmap are over-provisioned but harmless — SQLite only maps up to the file size and the OS will not allocate physical pages for unused virtual address space.
- **Concurrent indexing + browsing**: During active indexing (photo discovery, embedding generation), the reduced connection pool max of `(cores*2)+2` may cause brief connection contention. The 30-second acquire timeout provides backpressure; indexing tasks will queue rather than fail.
- **Very high-core machines**: On a 32-core server, the pool max would be 66 connections (down from 74). The reduction is proportionally smaller, which is acceptable — the pool size was never the dominant memory consumer.

## Research Notes

- SQLite official documentation recommends `mmap_size` "usually 256MB or larger" for performance (https://sqlite.org/mmap.html). For a database of ~200-300 MB (typical for 90k photos with 512-dim embeddings), 256 MB covers the entire file.
- SQLite's default `cache_size` is -2000 (~2 MB). Read-heavy server workloads commonly use 16-64 MB. The Phiresky SQLite performance tuning guide and community consensus point to 32 MB as a conservative sweet spot for single-user servers (https://phiresky.github.io/blog/2020/sqlite-performance-tuning/).
- The connection pool formula `(num_cores*2)+10` was designed when concurrent photo indexing could saturate many connections. The `+10` API_REQUEST_BUFFER is excessive for a single-user gallery — reducing it to `+2` still reserves one connection per core for indexing plus two for API requests.

## Assumptions

- The database file is approximately 200-300 MB for a 90,000-item library, so a 256 MB `mmap_size` still memory-maps the entire file.
- No other in-memory data structures or caches beyond those addressed contribute meaningfully to idle RAM.
- Reducing SQLite memory parameters will not cause measurable query latency regression for typical browsing patterns (photo grid pagination, viewer open, text search).
- Hardcoded defaults are acceptable; no environment variable or configuration file plumbing is needed for these values.
- The existing `min_connections=2` is appropriate for idle and should not be reduced further (dropping to 1 or 0 would cause first-request latency spikes).

## Success Criteria

- **SC-001**: Idle process resident memory (RSS) after 5 minutes of inactivity is measurably lower than the pre-change baseline, with a target reduction of at least 100 MB attributable to SQLite tuning changes.
- **SC-002**: Photo grid page load latency (first page, cold after idle) is within 10% of the pre-change baseline when measured against a 90k-item library.
- **SC-003**: No SQLite connection pool exhaustion errors occur during normal browsing (grid pagination, viewer open/close, text search) on the reduced pool size.
- **SC-004**: All existing unit and integration tests pass without modification after the SQLite PRAGMA and pool size changes.
