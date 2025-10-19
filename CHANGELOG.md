## [1.6.1](https://github.com/RouHim/turbo-pix/compare/1.6.0...1.6.1) (2025-10-19)


### Bug Fixes

* implement HTTP range request support for video streaming ([6e7e90f](https://github.com/RouHim/turbo-pix/commit/6e7e90f5a9a456f26c11ca08a63a968a9f18f4ef))

# [1.6.0](https://github.com/RouHim/turbo-pix/compare/1.5.12...1.6.0) (2025-10-19)


### Features

* add animated SVG banner to README ([8cc24a1](https://github.com/RouHim/turbo-pix/commit/8cc24a1e0f9dc13b87e665f83c8c354b4e532e5b))

## [1.5.12](https://github.com/RouHim/turbo-pix/compare/1.5.11...1.5.12) (2025-10-19)

## [1.5.11](https://github.com/RouHim/turbo-pix/compare/1.5.10...1.5.11) (2025-10-17)


### Performance Improvements

* skip reprocessing unchanged photos during rescan ([085b465](https://github.com/RouHim/turbo-pix/commit/085b465c7c1832904aff6e5f030ce81c1a1d83cd))

## [1.5.10](https://github.com/RouHim/turbo-pix/compare/1.5.9...1.5.10) (2025-10-17)


### Bug Fixes

* use docker buildx imagetools to avoid rate limit ([c319dee](https://github.com/RouHim/turbo-pix/commit/c319dee9a3e2945a3a58ed6b56267c478579e914))

## [1.5.9](https://github.com/RouHim/turbo-pix/compare/1.5.8...1.5.9) (2025-10-17)


### Bug Fixes

* add safe.directory config for git operations in Docker ([389b5bb](https://github.com/RouHim/turbo-pix/commit/389b5bb534ead6a7c11c0b4d2d999f1db33b03be))

## [1.5.8](https://github.com/RouHim/turbo-pix/compare/1.5.7...1.5.8) (2025-10-17)


### Bug Fixes

* add CFLAGS for rust-musl-cross builds ([bf4a786](https://github.com/RouHim/turbo-pix/commit/bf4a78601c433c15707f8215d1367cbc6358c3db))

## [1.5.7](https://github.com/RouHim/turbo-pix/compare/1.5.6...1.5.7) (2025-10-17)


### Bug Fixes

* prevent symlink to self in CI workflow ([8e62e17](https://github.com/RouHim/turbo-pix/commit/8e62e177959ecffc91bb4e110672bed9f7cbf679))

## [1.5.6](https://github.com/RouHim/turbo-pix/compare/1.5.5...1.5.6) (2025-10-16)


### Reverts

* Revert "fix: use g++ with musl specs for C++ compilation" ([eb7ceb7](https://github.com/RouHim/turbo-pix/commit/eb7ceb74b0b4b09624dd574ff2407e60f8e7ef73))

## [1.5.5](https://github.com/RouHim/turbo-pix/compare/1.5.4...1.5.5) (2025-10-16)


### Bug Fixes

* use rust-musl-cross Docker images for cross-compilation ([f01eac6](https://github.com/RouHim/turbo-pix/commit/f01eac6a925eaffb0be213dcbcc1b94cc4bc1027))

## [1.5.4](https://github.com/RouHim/turbo-pix/compare/1.5.3...1.5.4) (2025-10-16)


### Bug Fixes

* use g++ with musl specs for C++ compilation ([e84a3bc](https://github.com/RouHim/turbo-pix/commit/e84a3bcc05b24a78fd28cbfa07dcd64880adadb6))
* use musl-gcc wrapper for both architectures ([97b8df1](https://github.com/RouHim/turbo-pix/commit/97b8df1ce5d07818ec8a711a78bb8b253c5a9691))

## [1.5.3](https://github.com/RouHim/turbo-pix/compare/1.5.2...1.5.3) (2025-10-16)


### Bug Fixes

* add __COSMOPOLITAN__ define to prevent type redefinition in sqlite-vec ([677445f](https://github.com/RouHim/turbo-pix/commit/677445f3bcdf577160adaa7db966a3a45a0e7cfa))

## [1.5.2](https://github.com/RouHim/turbo-pix/compare/1.5.1...1.5.2) (2025-10-16)


### Bug Fixes

* add C compiler wrapper for aarch64 musl builds ([48d87c8](https://github.com/RouHim/turbo-pix/commit/48d87c8fded997fa5c2cdfddef2f34dd623fffc7))

## [1.5.1](https://github.com/RouHim/turbo-pix/compare/1.5.0...1.5.1) (2025-10-16)


### Bug Fixes

* install g++-aarch64-linux-gnu and configure CXX env for ARM64 musl builds ([3b8c2ad](https://github.com/RouHim/turbo-pix/commit/3b8c2ad1320ab345be1acbee7159efc23eb774c5))

# [1.5.0](https://github.com/RouHim/turbo-pix/compare/1.4.0...1.5.0) (2025-10-16)


### Bug Fixes

* install C++ musl toolchain for esaxx-rs native dependency ([115154a](https://github.com/RouHim/turbo-pix/commit/115154a655d2a580caa796c0cc988f2eb958cee4))


### Features

* add radar pulse animation to search button ([2f600a4](https://github.com/RouHim/turbo-pix/commit/2f600a497be8741a6745f897e553218e6b3965a4))

# [1.4.0](https://github.com/RouHim/turbo-pix/compare/1.3.3...1.4.0) (2025-10-16)


### Features

* add debug logging for semantic search query timing ([0c33422](https://github.com/RouHim/turbo-pix/commit/0c3342228162a851e2a2f803e5ca63dafe2fd59c))

## [1.3.3](https://github.com/RouHim/turbo-pix/compare/1.3.2...1.3.3) (2025-10-15)


### Bug Fixes

* update paths to use production directory for binary builds ([a5e75b5](https://github.com/RouHim/turbo-pix/commit/a5e75b5a50ac1efc0725555222d4d44839b5bfbf))

## [1.3.2](https://github.com/RouHim/turbo-pix/compare/1.3.1...1.3.2) (2025-10-15)

## [1.3.1](https://github.com/RouHim/turbo-pix/compare/1.3.0...1.3.1) (2025-10-15)


### Bug Fixes

* use native architecture runners for ARM64 binary builds ([65ed209](https://github.com/RouHim/turbo-pix/commit/65ed20998837404487efe3ad693848bd3ab2bac2))

# [1.3.0](https://github.com/RouHim/turbo-pix/compare/1.2.0...1.3.0) (2025-10-15)


### Bug Fixes

* use native ARM64 runners for container builds ([e5f0d30](https://github.com/RouHim/turbo-pix/commit/e5f0d30435c767ffe1cc89bf3bcfab1c5e27ff90))
* use platform-agnostic C types for ARM64 compatibility ([2ee82f4](https://github.com/RouHim/turbo-pix/commit/2ee82f4898040db6098ac05def210cc0b9e439a3))


### Features

* add ESC key to reset search field ([dc33435](https://github.com/RouHim/turbo-pix/commit/dc3343538d437dd054eb94e08bce00ee48810185))

# [1.2.0](https://github.com/RouHim/turbo-pix/compare/1.1.0...1.2.0) (2025-10-15)


### Bug Fixes

* resolve semantic-release configuration issues ([cbd9dbc](https://github.com/RouHim/turbo-pix/commit/cbd9dbc9ac1edfdfc63a2e83039188e4f463bdb7))


### Features

* add semantic-release configuration for automated versioning and changelog generation ([76df787](https://github.com/RouHim/turbo-pix/commit/76df787fad7e7ad205c00208aa0488bee861f887))

# 1.0.0 (2025-10-15)


### Bug Fixes

* add missing blurhash field and ThumbnailFormat arguments in tests ([30daba7](https://github.com/RouHim/turbo-pix/commit/30daba75f1479a7a0e671141882e18a835b730b7))
* add missing blurhash field to Photo update query ([9753819](https://github.com/RouHim/turbo-pix/commit/9753819d540fe221229b26a33f4d384633b25d10))
* Add missing is_favorite field to Photo struct initializations in tests ([54b56be](https://github.com/RouHim/turbo-pix/commit/54b56beb803faade6415cfdde6dbbb0c53fe99e5))
* add missing translation keys and parameter interpolation support ([643938e](https://github.com/RouHim/turbo-pix/commit/643938e9e1ced59211991280d38ab3facc09ef69))
* add RAW file extensions to file scanner ([c9007be](https://github.com/RouHim/turbo-pix/commit/c9007bef1101c371b99b9bf4782c4f33b210e88e))
* apply cargo fmt to fix formatting issues ([b8e1dc2](https://github.com/RouHim/turbo-pix/commit/b8e1dc29ffad5f16d941b5685bff8f2b9099761c))
* apply EXIF orientation to thumbnails ([8588797](https://github.com/RouHim/turbo-pix/commit/8588797d8a1f8194feef5ef088fa0f488aabdfd7))
* center banner by repositioning icon and text ([d67b64d](https://github.com/RouHim/turbo-pix/commit/d67b64d7275ee3344f9e8bba4075e75a6658f3a5))
* clean EXIF strings to remove malformed data ([1636ac8](https://github.com/RouHim/turbo-pix/commit/1636ac8f4742a5e1e3f9feca738c8ea94f4c520e))
* close mobile menu after selecting navigation item ([e4c0e4c](https://github.com/RouHim/turbo-pix/commit/e4c0e4c7a8f37f8eac21bb543de736f9e22eb729))
* correct container build context in CI pipeline ([edfaae1](https://github.com/RouHim/turbo-pix/commit/edfaae13872d224ad1fcb4c913c2484c4b0c9413))
* correct container health check port mapping and permissions ([49e5c19](https://github.com/RouHim/turbo-pix/commit/49e5c19b2d13dae879b4ac0fd9ee515aa54c0d03))
* enable CLIP embeddings for RAW image files ([99d3afe](https://github.com/RouHim/turbo-pix/commit/99d3afecf9e80adbd6026179b2019556125258c6))
* enable scrolling for entire viewer sidebar ([07c5b26](https://github.com/RouHim/turbo-pix/commit/07c5b26bcc5d060e47448ec7849b3414ee84c273))
* ensure theme toggle button shows default moon icon ([d732424](https://github.com/RouHim/turbo-pix/commit/d7324240c54639b5796e5388e614d588089e567d))
* favorites menu now correctly filters favorited photos ([3360b97](https://github.com/RouHim/turbo-pix/commit/3360b97efc86234a8d3ea8a98b4347fc3521abb5))
* improve dark mode visibility for theme toggle and sort dropdown ([767b6f9](https://github.com/RouHim/turbo-pix/commit/767b6f9f0c55f0a6fed2868599c8252fe8faec8b)), closes [#1e293](https://github.com/RouHim/turbo-pix/issues/1e293) [#f1f5f9](https://github.com/RouHim/turbo-pix/issues/f1f5f9)
* improve infinite scroll and loading indicator visibility ([3ed166d](https://github.com/RouHim/turbo-pix/commit/3ed166d276d07cfb25d179995d6676b5ebbc9c0e))
* improve port binding error handling and disable search suggestions ([73d5396](https://github.com/RouHim/turbo-pix/commit/73d53969c65e69424fb766ad0d2c84e340d6091e))
* improve theme toggle event binding with proper context ([6042013](https://github.com/RouHim/turbo-pix/commit/60420134df58807b283a83a1ab597f94a4a3c5e9))
* mobile search bar visibility and positioning ([b166315](https://github.com/RouHim/turbo-pix/commit/b166315b89f0f7fe09636e44faab8bc82e4f5a9e))
* pause video when navigating to next photo ([a5eb395](https://github.com/RouHim/turbo-pix/commit/a5eb3951cd7ab6266cfe574897474f7ff775d70c))
* Position mobile info-toggle button to bottom-right to avoid overlay with zoom controls ([50a9c45](https://github.com/RouHim/turbo-pix/commit/50a9c45ddc56dd850bdd9d532c317766b482353c))
* pre-download AI models during container build ([ef7db36](https://github.com/RouHim/turbo-pix/commit/ef7db365d405c1843491c1c547c1a3caf4411f53))
* prevent thumbnail cropping by changing CSS object-fit to contain ([f39214e](https://github.com/RouHim/turbo-pix/commit/f39214e1896578f5c2a365f74368c9ed7fb5f01e))
* Remove hardcoded video path fallback and improve video switching ([593ba6b](https://github.com/RouHim/turbo-pix/commit/593ba6bfd4bad8e8c182f6ec1ce95d65f4b70d5d))
* resolve semantic-release configuration issues ([cbd9dbc](https://github.com/RouHim/turbo-pix/commit/cbd9dbc9ac1edfdfc63a2e83039188e4f463bdb7))
* theme toggle button now correctly updates icon ([6f4cb58](https://github.com/RouHim/turbo-pix/commit/6f4cb5888324e262231bd35009590bbb3d00106b))
* theme toggle now works correctly by removing duplicate event listeners ([559b0a8](https://github.com/RouHim/turbo-pix/commit/559b0a858377c513d8328a0ed7d3ad810df0d322))
* update 'all_photos' label to 'all media' for improved clarity in multiple languages ([96c347e](https://github.com/RouHim/turbo-pix/commit/96c347e763421165d89ee0678276eb4f97d12c72))
* update test helpers to use new Photo metadata structure ([e9d372c](https://github.com/RouHim/turbo-pix/commit/e9d372c848ac4a8e09b3a9293b4c8bea676cf0b1))


### Features

* add --download-models flag for test setup ([77c5d1d](https://github.com/RouHim/turbo-pix/commit/77c5d1d89d26398558345dae8fe6184730944128))
* add clickable logo with favicon in header ([a04c4a6](https://github.com/RouHim/turbo-pix/commit/a04c4a61e20ac231497644758c33116f3eab1a3a))
* Add comprehensive i18n system and enhance photo management ([b0e3a89](https://github.com/RouHim/turbo-pix/commit/b0e3a89e40936c5f9185ac72beec2f92abcbfa8c))
* add comprehensive i18n translations for UI strings ([90d8bda](https://github.com/RouHim/turbo-pix/commit/90d8bda38019eb4f5aca8ce52295e6c583eebb97))
* add comprehensive on-demand metadata viewer ([c1ea9ee](https://github.com/RouHim/turbo-pix/commit/c1ea9eec980f855c999d0408097266ae20f8fa55))
* Add comprehensive video support ([7ad0a31](https://github.com/RouHim/turbo-pix/commit/7ad0a3100b254ae87739cb1f584f11a662fb16c5))
* add disk cache limit with LRU eviction ([775ca69](https://github.com/RouHim/turbo-pix/commit/775ca69e72fb962fe2fee55f57b00323f2b1ecf9))
* add fast loading indicator to photo viewer ([6fd18d6](https://github.com/RouHim/turbo-pix/commit/6fd18d6a510fd771e4b0525c4a973ca4e1cc0fd7))
* add favicon and web manifest for improved site branding ([131c5cf](https://github.com/RouHim/turbo-pix/commit/131c5cffb62ce102e50bbb5b08afe681a9abb62d))
* Add file creation date fallback for photos without EXIF data ([3aa2354](https://github.com/RouHim/turbo-pix/commit/3aa23547004c9b89350be926674ac485e5df5626))
* add fullscreen support and improve UI consistency ([772c465](https://github.com/RouHim/turbo-pix/commit/772c465b1fa8149ac4763d7d597bea7d9f24aedc))
* add interactive timeline slider for date-based photo navigation ([5e40970](https://github.com/RouHim/turbo-pix/commit/5e409701cd5502f38fb6782c5dfc426ad369dbda))
* add multi-architecture support for container builds and update README for project clarity ([dd57858](https://github.com/RouHim/turbo-pix/commit/dd57858fdde9b8c5f39c17af09cb475f1450295f))
* add RAW image format support - Phase 1: Foundation ([899ac25](https://github.com/RouHim/turbo-pix/commit/899ac251cedb9ef2645030b3ff56bbfeac25af1c)), closes [#5](https://github.com/RouHim/turbo-pix/issues/5)
* add RAW image format support - Phase 2-4: Integration & Testing ([bed810e](https://github.com/RouHim/turbo-pix/commit/bed810ea91fc11d5ec30bb58c177eb22f0a6e263))
* add semantic search with ML embeddings for natural language photo queries ([fad0bed](https://github.com/RouHim/turbo-pix/commit/fad0bed5fba51d481d17f6372a1ee06917dd1c66))
* add semantic-release automation and binary builds ([0f37fab](https://github.com/RouHim/turbo-pix/commit/0f37fab95c81aa1039f32cf18af6509706010014))
* add static ffmpeg/ffprobe binaries to container image ([083f641](https://github.com/RouHim/turbo-pix/commit/083f641b7c9fd77a024d84ed0849e467bffa7fd8))
* add TURBO_PIX_DATA_PATH parameter for centralized data storage ([50ff790](https://github.com/RouHim/turbo-pix/commit/50ff790b7dc73ff961ea5223155555891ac2e887))
* Add video play icon overlays to distinguish videos from photos ([4884de7](https://github.com/RouHim/turbo-pix/commit/4884de7a32a0eca6a1752540b345d7cce69e18d3))
* Complete module flattening project ([73abc0a](https://github.com/RouHim/turbo-pix/commit/73abc0ae68bc2ac947b314afa2b80fef00bfb550))
* enhance CI/CD pipeline with modern tooling and Docker Hub integration ([b519d23](https://github.com/RouHim/turbo-pix/commit/b519d23042fa2c21a7c352b8268b522bfd0437c1))
* enhance static file routing with dynamic content type handling ([c4f09b8](https://github.com/RouHim/turbo-pix/commit/c4f09b86f3382818196ccabe36fa45d0abfc29ad))
* Implement dark theme with CSS variables and localStorage persistence ([1aa3774](https://github.com/RouHim/turbo-pix/commit/1aa37745050f9bfde70b540293447a199cde7f44))
* implement modern mobile gesture system for photo viewer ([bc2c6ff](https://github.com/RouHim/turbo-pix/commit/bc2c6ff6e059d0a98cc3cb096a35b5e88d18f225))
* Implement pure hash-based thumbnail system ([42a29f4](https://github.com/RouHim/turbo-pix/commit/42a29f4b0da271e06df46d55dfb2569a46a80854))
* implement RAW image processing and orientation correction ([ac14a0f](https://github.com/RouHim/turbo-pix/commit/ac14a0fd004d5efbd0dc27750b91436bd963a250))
* improve metadata button visibility and layout on mobile ([b4f3315](https://github.com/RouHim/turbo-pix/commit/b4f3315749f6d063b4347bfb91030bec67574248))
* improve mobile UX with Instagram-style grid and enhanced navigation ([ae72680](https://github.com/RouHim/turbo-pix/commit/ae72680d823f1493282b3c266f06c33cc87b37b8))
* improve mobile UX with Instagram-style photo grid ([82f5cb4](https://github.com/RouHim/turbo-pix/commit/82f5cb40f825612474f18c99fb46c1e644a4d895))
* improve photo viewer sidebar UX and mobile layout ([2099f6f](https://github.com/RouHim/turbo-pix/commit/2099f6faef831c972a92ac273ad851f1b1d1e6a8))
* migrate from emoji icons to Feather Icons with semantic consistency ([37e8cd3](https://github.com/RouHim/turbo-pix/commit/37e8cd358985a6d8125dfa92f7e00a6fab37a0ea))
* migrate from integer IDs to SHA256 hash-based primary keys ([ff5ce2d](https://github.com/RouHim/turbo-pix/commit/ff5ce2d2ca3b1f196a2077d17fde9f7170af6bbe))
* move favorite button to viewer controls in detail view ([d13908f](https://github.com/RouHim/turbo-pix/commit/d13908fa7ab8d8d7edb3cc4678316543d9b15646))
* reduce bottom spacing of viewer controls on mobile ([112fbea](https://github.com/RouHim/turbo-pix/commit/112fbeae9a41c8428d496d2aa213824afe4b3f8b))
* refactor Photo entity with JSON metadata storage ([1ac7003](https://github.com/RouHim/turbo-pix/commit/1ac7003540347d0064d68891733bf2634e4cc1e5))
* store AI models in configurable data path and optimize musl performance ([891b71e](https://github.com/RouHim/turbo-pix/commit/891b71efa0f80531a963cdf84831bfc8d8309506))
* support separate caching for WebP and JPEG thumbnails ([d805831](https://github.com/RouHim/turbo-pix/commit/d80583181264afa858c6797822cb96af4e3dd85a))


### Performance Improvements

* Replace content-based hashing with path-based hashing for 26,000x speed improvement ([a6ea4d0](https://github.com/RouHim/turbo-pix/commit/a6ea4d03f2d57340b0209013f672c38c842c650b))


### Reverts

* restore object-fit cover and aspect-ratio preserving thumbnails ([1bfd1ea](https://github.com/RouHim/turbo-pix/commit/1bfd1ea2f73af4154a68eecdd9c962416c4ed2f7))


### BREAKING CHANGES

* Reduced Photo struct from 53 fields to 19 fields (64% reduction)
by moving all EXIF/camera/location/video metadata into a single JSON column with
nested structure. No backward compatibility layer - clean break for better architecture.

Backend changes:
- Photo struct: 19 core fields + metadata JSON column
- Database schema: Separated computational fields (width, height, orientation,
  duration, taken_at) from informational metadata
- JSON structure: Nested groups (camera, settings, location, video)
- Type-safe accessor methods for Rust code
- Search queries updated to use json_extract() for nested metadata

Frontend changes:
- Updated viewerMetadata.js to access photo.metadata.camera.make, etc.
- Updated photoCard.js to read nested metadata structure
- Direct breaking change with no compatibility wrapper

Benefits:
- Clear semantic boundary: computational vs informational data
- Reduced code complexity and database columns
- Type preservation in JSON (booleans, numbers, strings)
- Easier to extend with new metadata fields
- Better performance with WITHOUT ROWID optimization

Verified:
- E2E tests passing (photo grid, viewer, navigation, favorites, filters, search)
- Metadata display correct for all nested fields
- Search working with JSON queries
- Zero compiler warnings

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
* Photo identification changed from integer IDs to SHA256 hashes

Database Schema:
- Replace id INTEGER PRIMARY KEY with hash_sha256 TEXT PRIMARY KEY
- Add CHECK constraint for 64-character hex validation
- Implement WITHOUT ROWID optimization for ~15% storage reduction
- Remove redundant hash_sha256 field (now primary key)

Backend Changes:
- Update Photo struct to use hash_sha256 as primary field
- Modify all database methods (find_by_hash, create, update, delete)
- Update API handlers to accept String hash parameters
- Change route definitions from i64 to String parameters
- Fix column indexing in from_row() method for new schema

Frontend Changes:
- Update JavaScript to use photo.hash_sha256 instead of photo.id
- Modify URL helpers to use hash-based endpoints
- Update photo grid and viewer components for hash identification
- Change API client methods to work with hash parameters

Benefits:
- Content-addressable URLs enable better caching
- Natural deduplication prevents duplicate content storage
- Hash-based primary keys provide data integrity
- Unified addressing scheme across all endpoints

Tests: All 45 tests pass including hash validation constraints

# [1.1.0](https://github.com/RouHim/turbo-pix/compare/1.0.0...1.1.0) (2025-10-15)


### Features

* add multi-architecture support for container builds and update README for project clarity ([dd57858](https://github.com/RouHim/turbo-pix/commit/dd57858fdde9b8c5f39c17af09cb475f1450295f))

# 1.0.0 (2025-10-15)


### Bug Fixes

* add missing blurhash field and ThumbnailFormat arguments in tests ([30daba7](https://github.com/RouHim/turbo-pix/commit/30daba75f1479a7a0e671141882e18a835b730b7))
* add missing blurhash field to Photo update query ([9753819](https://github.com/RouHim/turbo-pix/commit/9753819d540fe221229b26a33f4d384633b25d10))
* Add missing is_favorite field to Photo struct initializations in tests ([54b56be](https://github.com/RouHim/turbo-pix/commit/54b56beb803faade6415cfdde6dbbb0c53fe99e5))
* add missing translation keys and parameter interpolation support ([643938e](https://github.com/RouHim/turbo-pix/commit/643938e9e1ced59211991280d38ab3facc09ef69))
* add RAW file extensions to file scanner ([c9007be](https://github.com/RouHim/turbo-pix/commit/c9007bef1101c371b99b9bf4782c4f33b210e88e))
* apply cargo fmt to fix formatting issues ([b8e1dc2](https://github.com/RouHim/turbo-pix/commit/b8e1dc29ffad5f16d941b5685bff8f2b9099761c))
* apply EXIF orientation to thumbnails ([8588797](https://github.com/RouHim/turbo-pix/commit/8588797d8a1f8194feef5ef088fa0f488aabdfd7))
* center banner by repositioning icon and text ([d67b64d](https://github.com/RouHim/turbo-pix/commit/d67b64d7275ee3344f9e8bba4075e75a6658f3a5))
* clean EXIF strings to remove malformed data ([1636ac8](https://github.com/RouHim/turbo-pix/commit/1636ac8f4742a5e1e3f9feca738c8ea94f4c520e))
* close mobile menu after selecting navigation item ([e4c0e4c](https://github.com/RouHim/turbo-pix/commit/e4c0e4c7a8f37f8eac21bb543de736f9e22eb729))
* correct container build context in CI pipeline ([edfaae1](https://github.com/RouHim/turbo-pix/commit/edfaae13872d224ad1fcb4c913c2484c4b0c9413))
* correct container health check port mapping and permissions ([49e5c19](https://github.com/RouHim/turbo-pix/commit/49e5c19b2d13dae879b4ac0fd9ee515aa54c0d03))
* enable CLIP embeddings for RAW image files ([99d3afe](https://github.com/RouHim/turbo-pix/commit/99d3afecf9e80adbd6026179b2019556125258c6))
* enable scrolling for entire viewer sidebar ([07c5b26](https://github.com/RouHim/turbo-pix/commit/07c5b26bcc5d060e47448ec7849b3414ee84c273))
* ensure theme toggle button shows default moon icon ([d732424](https://github.com/RouHim/turbo-pix/commit/d7324240c54639b5796e5388e614d588089e567d))
* favorites menu now correctly filters favorited photos ([3360b97](https://github.com/RouHim/turbo-pix/commit/3360b97efc86234a8d3ea8a98b4347fc3521abb5))
* improve dark mode visibility for theme toggle and sort dropdown ([767b6f9](https://github.com/RouHim/turbo-pix/commit/767b6f9f0c55f0a6fed2868599c8252fe8faec8b)), closes [#1e293](https://github.com/RouHim/turbo-pix/issues/1e293) [#f1f5f9](https://github.com/RouHim/turbo-pix/issues/f1f5f9)
* improve infinite scroll and loading indicator visibility ([3ed166d](https://github.com/RouHim/turbo-pix/commit/3ed166d276d07cfb25d179995d6676b5ebbc9c0e))
* improve port binding error handling and disable search suggestions ([73d5396](https://github.com/RouHim/turbo-pix/commit/73d53969c65e69424fb766ad0d2c84e340d6091e))
* improve theme toggle event binding with proper context ([6042013](https://github.com/RouHim/turbo-pix/commit/60420134df58807b283a83a1ab597f94a4a3c5e9))
* mobile search bar visibility and positioning ([b166315](https://github.com/RouHim/turbo-pix/commit/b166315b89f0f7fe09636e44faab8bc82e4f5a9e))
* pause video when navigating to next photo ([a5eb395](https://github.com/RouHim/turbo-pix/commit/a5eb3951cd7ab6266cfe574897474f7ff775d70c))
* Position mobile info-toggle button to bottom-right to avoid overlay with zoom controls ([50a9c45](https://github.com/RouHim/turbo-pix/commit/50a9c45ddc56dd850bdd9d532c317766b482353c))
* pre-download AI models during container build ([ef7db36](https://github.com/RouHim/turbo-pix/commit/ef7db365d405c1843491c1c547c1a3caf4411f53))
* prevent thumbnail cropping by changing CSS object-fit to contain ([f39214e](https://github.com/RouHim/turbo-pix/commit/f39214e1896578f5c2a365f74368c9ed7fb5f01e))
* Remove hardcoded video path fallback and improve video switching ([593ba6b](https://github.com/RouHim/turbo-pix/commit/593ba6bfd4bad8e8c182f6ec1ce95d65f4b70d5d))
* theme toggle button now correctly updates icon ([6f4cb58](https://github.com/RouHim/turbo-pix/commit/6f4cb5888324e262231bd35009590bbb3d00106b))
* theme toggle now works correctly by removing duplicate event listeners ([559b0a8](https://github.com/RouHim/turbo-pix/commit/559b0a858377c513d8328a0ed7d3ad810df0d322))
* update 'all_photos' label to 'all media' for improved clarity in multiple languages ([96c347e](https://github.com/RouHim/turbo-pix/commit/96c347e763421165d89ee0678276eb4f97d12c72))
* update test helpers to use new Photo metadata structure ([e9d372c](https://github.com/RouHim/turbo-pix/commit/e9d372c848ac4a8e09b3a9293b4c8bea676cf0b1))


### Features

* add --download-models flag for test setup ([77c5d1d](https://github.com/RouHim/turbo-pix/commit/77c5d1d89d26398558345dae8fe6184730944128))
* add clickable logo with favicon in header ([a04c4a6](https://github.com/RouHim/turbo-pix/commit/a04c4a61e20ac231497644758c33116f3eab1a3a))
* Add comprehensive i18n system and enhance photo management ([b0e3a89](https://github.com/RouHim/turbo-pix/commit/b0e3a89e40936c5f9185ac72beec2f92abcbfa8c))
* add comprehensive i18n translations for UI strings ([90d8bda](https://github.com/RouHim/turbo-pix/commit/90d8bda38019eb4f5aca8ce52295e6c583eebb97))
* add comprehensive on-demand metadata viewer ([c1ea9ee](https://github.com/RouHim/turbo-pix/commit/c1ea9eec980f855c999d0408097266ae20f8fa55))
* Add comprehensive video support ([7ad0a31](https://github.com/RouHim/turbo-pix/commit/7ad0a3100b254ae87739cb1f584f11a662fb16c5))
* add disk cache limit with LRU eviction ([775ca69](https://github.com/RouHim/turbo-pix/commit/775ca69e72fb962fe2fee55f57b00323f2b1ecf9))
* add fast loading indicator to photo viewer ([6fd18d6](https://github.com/RouHim/turbo-pix/commit/6fd18d6a510fd771e4b0525c4a973ca4e1cc0fd7))
* add favicon and web manifest for improved site branding ([131c5cf](https://github.com/RouHim/turbo-pix/commit/131c5cffb62ce102e50bbb5b08afe681a9abb62d))
* Add file creation date fallback for photos without EXIF data ([3aa2354](https://github.com/RouHim/turbo-pix/commit/3aa23547004c9b89350be926674ac485e5df5626))
* add fullscreen support and improve UI consistency ([772c465](https://github.com/RouHim/turbo-pix/commit/772c465b1fa8149ac4763d7d597bea7d9f24aedc))
* add interactive timeline slider for date-based photo navigation ([5e40970](https://github.com/RouHim/turbo-pix/commit/5e409701cd5502f38fb6782c5dfc426ad369dbda))
* add RAW image format support - Phase 1: Foundation ([899ac25](https://github.com/RouHim/turbo-pix/commit/899ac251cedb9ef2645030b3ff56bbfeac25af1c)), closes [#5](https://github.com/RouHim/turbo-pix/issues/5)
* add RAW image format support - Phase 2-4: Integration & Testing ([bed810e](https://github.com/RouHim/turbo-pix/commit/bed810ea91fc11d5ec30bb58c177eb22f0a6e263))
* add semantic search with ML embeddings for natural language photo queries ([fad0bed](https://github.com/RouHim/turbo-pix/commit/fad0bed5fba51d481d17f6372a1ee06917dd1c66))
* add semantic-release automation and binary builds ([0f37fab](https://github.com/RouHim/turbo-pix/commit/0f37fab95c81aa1039f32cf18af6509706010014))
* add static ffmpeg/ffprobe binaries to container image ([083f641](https://github.com/RouHim/turbo-pix/commit/083f641b7c9fd77a024d84ed0849e467bffa7fd8))
* add TURBO_PIX_DATA_PATH parameter for centralized data storage ([50ff790](https://github.com/RouHim/turbo-pix/commit/50ff790b7dc73ff961ea5223155555891ac2e887))
* Add video play icon overlays to distinguish videos from photos ([4884de7](https://github.com/RouHim/turbo-pix/commit/4884de7a32a0eca6a1752540b345d7cce69e18d3))
* Complete module flattening project ([73abc0a](https://github.com/RouHim/turbo-pix/commit/73abc0ae68bc2ac947b314afa2b80fef00bfb550))
* enhance CI/CD pipeline with modern tooling and Docker Hub integration ([b519d23](https://github.com/RouHim/turbo-pix/commit/b519d23042fa2c21a7c352b8268b522bfd0437c1))
* enhance static file routing with dynamic content type handling ([c4f09b8](https://github.com/RouHim/turbo-pix/commit/c4f09b86f3382818196ccabe36fa45d0abfc29ad))
* Implement dark theme with CSS variables and localStorage persistence ([1aa3774](https://github.com/RouHim/turbo-pix/commit/1aa37745050f9bfde70b540293447a199cde7f44))
* implement modern mobile gesture system for photo viewer ([bc2c6ff](https://github.com/RouHim/turbo-pix/commit/bc2c6ff6e059d0a98cc3cb096a35b5e88d18f225))
* Implement pure hash-based thumbnail system ([42a29f4](https://github.com/RouHim/turbo-pix/commit/42a29f4b0da271e06df46d55dfb2569a46a80854))
* implement RAW image processing and orientation correction ([ac14a0f](https://github.com/RouHim/turbo-pix/commit/ac14a0fd004d5efbd0dc27750b91436bd963a250))
* improve metadata button visibility and layout on mobile ([b4f3315](https://github.com/RouHim/turbo-pix/commit/b4f3315749f6d063b4347bfb91030bec67574248))
* improve mobile UX with Instagram-style grid and enhanced navigation ([ae72680](https://github.com/RouHim/turbo-pix/commit/ae72680d823f1493282b3c266f06c33cc87b37b8))
* improve mobile UX with Instagram-style photo grid ([82f5cb4](https://github.com/RouHim/turbo-pix/commit/82f5cb40f825612474f18c99fb46c1e644a4d895))
* improve photo viewer sidebar UX and mobile layout ([2099f6f](https://github.com/RouHim/turbo-pix/commit/2099f6faef831c972a92ac273ad851f1b1d1e6a8))
* migrate from emoji icons to Feather Icons with semantic consistency ([37e8cd3](https://github.com/RouHim/turbo-pix/commit/37e8cd358985a6d8125dfa92f7e00a6fab37a0ea))
* migrate from integer IDs to SHA256 hash-based primary keys ([ff5ce2d](https://github.com/RouHim/turbo-pix/commit/ff5ce2d2ca3b1f196a2077d17fde9f7170af6bbe))
* move favorite button to viewer controls in detail view ([d13908f](https://github.com/RouHim/turbo-pix/commit/d13908fa7ab8d8d7edb3cc4678316543d9b15646))
* reduce bottom spacing of viewer controls on mobile ([112fbea](https://github.com/RouHim/turbo-pix/commit/112fbeae9a41c8428d496d2aa213824afe4b3f8b))
* refactor Photo entity with JSON metadata storage ([1ac7003](https://github.com/RouHim/turbo-pix/commit/1ac7003540347d0064d68891733bf2634e4cc1e5))
* store AI models in configurable data path and optimize musl performance ([891b71e](https://github.com/RouHim/turbo-pix/commit/891b71efa0f80531a963cdf84831bfc8d8309506))
* support separate caching for WebP and JPEG thumbnails ([d805831](https://github.com/RouHim/turbo-pix/commit/d80583181264afa858c6797822cb96af4e3dd85a))


### Performance Improvements

* Replace content-based hashing with path-based hashing for 26,000x speed improvement ([a6ea4d0](https://github.com/RouHim/turbo-pix/commit/a6ea4d03f2d57340b0209013f672c38c842c650b))


### Reverts

* restore object-fit cover and aspect-ratio preserving thumbnails ([1bfd1ea](https://github.com/RouHim/turbo-pix/commit/1bfd1ea2f73af4154a68eecdd9c962416c4ed2f7))


### BREAKING CHANGES

* Reduced Photo struct from 53 fields to 19 fields (64% reduction)
by moving all EXIF/camera/location/video metadata into a single JSON column with
nested structure. No backward compatibility layer - clean break for better architecture.

Backend changes:
- Photo struct: 19 core fields + metadata JSON column
- Database schema: Separated computational fields (width, height, orientation,
  duration, taken_at) from informational metadata
- JSON structure: Nested groups (camera, settings, location, video)
- Type-safe accessor methods for Rust code
- Search queries updated to use json_extract() for nested metadata

Frontend changes:
- Updated viewerMetadata.js to access photo.metadata.camera.make, etc.
- Updated photoCard.js to read nested metadata structure
- Direct breaking change with no compatibility wrapper

Benefits:
- Clear semantic boundary: computational vs informational data
- Reduced code complexity and database columns
- Type preservation in JSON (booleans, numbers, strings)
- Easier to extend with new metadata fields
- Better performance with WITHOUT ROWID optimization

Verified:
- E2E tests passing (photo grid, viewer, navigation, favorites, filters, search)
- Metadata display correct for all nested fields
- Search working with JSON queries
- Zero compiler warnings

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
* Photo identification changed from integer IDs to SHA256 hashes

Database Schema:
- Replace id INTEGER PRIMARY KEY with hash_sha256 TEXT PRIMARY KEY
- Add CHECK constraint for 64-character hex validation
- Implement WITHOUT ROWID optimization for ~15% storage reduction
- Remove redundant hash_sha256 field (now primary key)

Backend Changes:
- Update Photo struct to use hash_sha256 as primary field
- Modify all database methods (find_by_hash, create, update, delete)
- Update API handlers to accept String hash parameters
- Change route definitions from i64 to String parameters
- Fix column indexing in from_row() method for new schema

Frontend Changes:
- Update JavaScript to use photo.hash_sha256 instead of photo.id
- Modify URL helpers to use hash-based endpoints
- Update photo grid and viewer components for hash identification
- Change API client methods to work with hash parameters

Benefits:
- Content-addressable URLs enable better caching
- Natural deduplication prevents duplicate content storage
- Hash-based primary keys provide data integrity
- Unified addressing scheme across all endpoints

Tests: All 45 tests pass including hash validation constraints
