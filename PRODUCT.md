# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

One operator per instance: the author runs TurboPix on his own hardware against his own photo/video collection. He migrated from cloud photo services to self-hosting and wants his existing directory structure untouched. The project is published for other self-hosters (GitHub releases, Docker Hub, README): author-first, but behavior and UI must stay understandable for strangers — without building onboarding machinery. Desktop/laptop and phone/tablet are equally important viewing surfaces.

## Product Purpose

TurboPix shows an existing photo and video collection — fast, simple, and nothing else. It scans directories at startup and nightly, indexes metadata and CLIP embeddings locally, and serves browsing, timeline, map, semantic search, favorites, albums, collages and video playback from one self-hosted binary. Success means the collection is fast and pleasant to look at, upkeep is near zero, and nothing beyond the core viewing experience gets built.

## Positioning

A viewer, not a management suite. Where Immich, PhotoPrism, Lychee or Piwigo ask users to import or reorganize their library, TurboPix reads existing directories in place, needs no login, keeps everything local, and competes on speed (Rust, on-the-fly thumbnails, low resource use). Deliberate simplicity is the differentiator — not feature parity.

## Operating Context

Self-hosted on the operator's own machine and reached over the LAN; no cloud services. Runs as a Docker container or a native binary; `ffmpeg` is the only external prerequisite. The directory layout is never reorganized or renamed in the background — files change only through explicit user actions (EXIF metadata edits, rotation, housekeeping cleanup). Indexing is automatic: a startup scan plus a nightly incremental rescan. UI locales are EN and DE; dark mode is supported. Video playback is decided server-side per request and may transcode through a bounded worker pool.

## Capabilities and Constraints

- Browsing: photo/video grid, timeline (density over time), map (OSM tiles, clustered), favorites, manual albums, collages, saved searches, batch selection, housekeeping for duplicate candidates, viewer with metadata display and explicit in-place metadata editing.
- Semantic search over CLIP embeddings; nightly scans keep the index fresh.
- Images: JPEG/PNG/GIF/WebP/BMP/TIFF plus common RAW formats. Videos: MP4/MOV/AVI/MKV/WebM/M4V.
- Confirmed constraint: LAN-only, no authentication — a conscious architectural boundary; TurboPix must never be exposed publicly.
- Confirmed constraint: simplicity — no feature-parity ambitions; reject anything outside the core.
- Confirmed constraint: speed and low resource usage are product promises.
- Confirmed constraint: one binary / one container, trivial installation.
- Phase: pre-production; breaking changes are allowed, database and caches may be recreated — no migration or legacy support required.

## Brand Commitments

- Name and tagline: "TurboPix — A straightforward photo and video viewer—fast, simple, and nothing else."
- Icons: feather only; no emojis in the UI.
- EN + DE localization with structural parity is a release gate (`tests/i18n-integrity.test.js`).

## Evidence on Hand

- README: features, config table (env vars), run instructions.
- Visual assets: `.github/readme/banner.svg`, `frontend/public/favicon.svg`.
- Behavioral documentation: E2E specs under `tests/e2e/specs/`, planning docs under `.spec/`.
- Fixtures: `test-data/` (images/videos incl. EXIF and codec variants).
- No testimonials, benchmarks, customer counts or pricing claims exist — never fabricate any.

## Product Principles

1. Local and trusted: LAN-only without accounts; the security boundary is part of the product, not a shortcut.
2. Simplicity is the product: build the core well and nothing else.
3. Speed is a feature: performance and low resource usage are the pitch and must not regress.
4. Self-hosting stays trivial: one binary or container, minimal prerequisites.
5. Author-first, community-usable: optimize for the operator's own library while keeping behavior explainable to other self-hosters.

## Accessibility & Inclusion

Accessibility is treated as a release gate: semantic markup, keyboard operability, focus management, `prefers-reduced-motion`, and axe-core checks in E2E. Both EN and DE locales are maintained with structural parity.
