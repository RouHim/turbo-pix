# TurboPix HTTP API

The reference for every route the `turbo-pix` binary serves. Every endpoint entry
names the handler module that defines it, so a claim here can be checked against
the code without guessing.

This document describes the current binary, not a versioned contract: TurboPix is
pre-production and breaking changes are allowed (see `AGENTS.md`).

## Deployment posture

- **Unauthenticated, LAN-only.** There is no login, no session, no API key. The
  server trusts every request that reaches it; do not expose it to an untrusted
  network or the public internet.
- **Bind address.** `TURBO_PIX_HOST` (default `127.0.0.1`) and `TURBO_PIX_PORT`
  (default `18473`).
- **Host pinning (DNS-rebinding hardening).** The router wraps every route in
  `require_same_origin` (`src/warp_helpers.rs`):
  - A request **without** an `Origin` header passes — curl, scripts and
    same-origin navigations are unaffected.
  - A request with an `Origin` header passes only when its hostname equals the
    `Host` header's hostname (hostname-only, so the Vite dev proxy on
    `localhost:5173` → `localhost:18473` still works).
  - `Origin: null` is always rejected.
  - When `TURBO_PIX_ALLOWED_HOSTS` is set (comma-separated), the `Host` header
    must additionally match one of those names. Without an explicit list, a
    loopback bind pins `Host` to `127.0.0.1`, `localhost` and `::1`; a
    non-loopback bind (`0.0.0.0` for LAN/Docker) **must** set
    `TURBO_PIX_ALLOWED_HOSTS`, otherwise every LAN host is rejected
    (`src/config.rs`).
- **No CORS allowance.** There is no `Access-Control-Allow-*` response at all;
  cross-origin browser reads are refused, not allowed with a wildcard.

## Conventions

### Photo hash

`{hash}` in every path is a photo's `hash_sha256` exactly as returned by a
listing or by `GET /api/photos/{hash}`. It is a hash of the file's path string (not
of its bytes), so it survives in-place metadata edits and only changes on
rotation; thumbnail and transcode caches fold in size+mtime on top of it.

### Error envelope

Every error produced by the shared rejection handler (`src/warp_helpers.rs`) has
this body:

```json
{
  "error": "Database error",
  "code": 500,
  "error_code": "invalid_date",
  "timestamp": "2026-11-14T10:00:00+00:00"
}
```

- `error` — human-readable message; database errors are sanitized (SQL text and
  server paths stay in the log).
- `code` — duplicate of the HTTP status.
- `error_code` — present **only** on the video-metadata refusals (machine-readable
  action key); omitted everywhere else.
- `timestamp` — RFC 3339, UTC.

Statuses any route can answer, before its handler runs:

| Status | Cause |
|---|---|
| `400` | Query string cannot be deserialized (`Invalid query parameters`) |
| `400` | JSON body cannot be deserialized (`Invalid request body`) |
| `403` | Cross-origin / `null` origin / `Host` not in `TURBO_PIX_ALLOWED_HOSTS` (`Cross-origin request rejected`) |
| `405` | Path matched, method did not (`Method not allowed`) |
| `411` | JSON route with a chunked or `Content-Length`-less body (`Content-Length required`) |
| `413` | Body larger than 1 MiB (`Payload too large`) |
| `415` | Body content type is not JSON (`Unsupported media type`) |
| `500` | Unhandled rejection (`Internal server error`) |

### Request bodies

- JSON only, capped at 1 MiB (`warp::body::content_length_limit`) on the routes
  that take a body. The cap is per-route rather than global middleware because
  warp's limit requires a `Content-Length` header, which plain GETs do not send —
  a global limit would answer `411` to every GET.
- Bodies that are too large, missing `Content-Length`, or not JSON are refused
  with `413` / `411` / `415` respectively, never a `500`.

### Batch size

The batch endpoints reject an empty array and more than 1000 items with `400`
(`hashes must not be empty`, `too many hashes (max 1000)`; collage batches say
`ids` instead of `hashes`). Album member lists are the exception: only the
>1000 bound applies there (`Too many hashes (max 1000)`), and an empty list is
accepted. Partial failures inside a valid batch are reported in the `200` body,
not by rejecting the request.

---

## Health and readiness

Mounted at the root, **not** under `/api`.

### `GET /health`

Liveness only; never touches the database.

- **Response `200`**: `{ "status": "healthy", "timestamp": "<rfc3339>" }`
- Source: `src/handlers_health.rs`

### `GET /ready`

Readiness: acquires one connection from the pool.

- **Response `200`**: `{ "status": "ready", "database": "connected", "timestamp": "<rfc3339>" }`
- **`500`** `{ "error": "Database error", ... }` when the pool cannot hand out a
  connection.
- Source: `src/handlers_health.rs`

## Config

### `GET /api/config`

Runtime configuration the frontend needs before it renders (locale, map tiles).

- **Response `200`**: `{ "default_locale": "en", "tile_url": "https://tile.openstreetmap.org/{z}/{x}/{y}.png" }`
  - `default_locale` ← `TURBO_PIX_LOCALE`; `tile_url` ← `TURBO_PIX_TILE_URL`.
- Source: `src/handlers_config.rs`

## Photos

### The photo object

Every photo in a response is the row plus one formatting step
(`src/photo_reply.rs`): the derived coordinate columns are **not** serialized at
the top level and appear instead as `metadata.location.latitude` /
`metadata.location.longitude`.

```json
{
  "hash_sha256": "…",
  "file_path": "/photos/2024/IMG_1234.jpg",
  "filename": "IMG_1234.jpg",
  "file_size": 4821304,
  "mime_type": "image/jpeg",
  "taken_at": "2024-07-04T12:00:00Z",
  "width": 6000,
  "height": 4000,
  "orientation": 1,
  "duration": null,
  "thumbnail_path": null,
  "has_thumbnail": true,
  "blurhash": "LKO2?U%2Tw=w]~RBVZRi};RPxuwH",
  "is_favorite": false,
  "semantic_vector_indexed": true,
  "metadata": {
    "camera": { "make": "Canon", "model": "EOS R5" },
    "settings": { "iso": 100 },
    "location": { "city": "Berlin", "latitude": 52.52, "longitude": 13.405 }
  },
  "file_modified": "2024-07-05T08:11:02Z",
  "date_indexed": "2024-07-05T08:11:05Z",
  "created_at": "2024-07-05T08:11:05Z",
  "updated_at": "2024-07-05T08:11:05Z"
}
```

`taken_at`, `metadata` and the two `file_modified`/`date_indexed` values are the
file's facts: no route writes them from client input, and the next scan
reconciles them.

### `GET /api/photos` — grid listing

Paginated listing over every indexed photo.

| Query param | Type | Meaning |
|---|---|---|
| `page` | u32 | 1-based; default `1`, values `< 1` clamp to `1` |
| `limit` | u32 | Page size; default `50`, clamped to `1..=100` |
| `sort` | string | `date` (default) \| `filename`/`name` \| `file_size`/`size` \| `created_at`; any other value falls back to `date` |
| `order` | string | `asc`; anything else (including absent) is `desc` |
| `q` | string | Token grammar below |
| `year`, `month` | i32 | Inclusive start bound on `taken_at` |
| `to_year`, `to_month` | i32 | Inclusive end bound on `taken_at` (absent = single period) |

The `q` grammar is whitespace-split and ANDed, sharing one builder with every
other listing (`src/db.rs::build_search_where`):

- `type:video` / `type:image` → filters on `mime_type`.
- `is_favorite:true` / `is_favorite:false` → filters on the favorite flag
  (`false` also matches `NULL`).
- `location:<city…>` → absorbs following words until the next prefix token, then
  matches `metadata.location.city`.
- any other token → case-insensitive `LIKE %token%` over `filename`,
  `metadata.camera.make`, `metadata.camera.model`, `metadata.location.city`.
  `%`, `_` and `\` are escaped, so `IMG_2024` matches literally.

Ordering is `"<column> <dir>, hash_sha256 <dir>"`, so pages are deterministic.

- **Response `200`**:
  `{ "photos": [...], "total": 1234, "page": 1, "limit": 50, "has_next": true, "has_prev": false }`
- **Statuses**: `200`; `400` for a non-numeric `page`/`limit`/`year`/`month`;
  `500` on a database failure.
- Source: `src/handlers_photo.rs` (`list_photos`)

### `GET /api/photos/map` — map listing

The whole filtered set, **unpaginated** (the map draws every marker it is given).

| Query param | Type | Meaning |
|---|---|---|
| `sort`, `order` | string | Same whitelist and default as the grid |
| `q`, `year`, `month`, `to_year`, `to_month` | — | Same semantics as the grid |
| `album` | i64 | Restricts to one album's members |

- **Response `200`**: `{ "photos": [...] }`
- **Statuses**: `200`; `400` for a malformed parameter; `500` on a database
  failure.
- The route extracts the query as `Option`, so a malformed query is answered by
  this handler rather than falling into the `{hash}` route's `404` (both routes
  match `/api/photos/map`).
- Source: `src/handlers_photo.rs` (`list_map_photos`)

### `GET /api/photos/timeline`

Date range and per-month density for the timeline scrubber. No parameters.

- **Response `200`**:
  `{ "min_date": "<rfc3339|null>", "max_date": "<rfc3339|null>", "density": [ { "year": 2024, "month": 7, "count": 83 } ] }`
  — `density` is ordered by year then month; undated rows contribute no bucket.
- **Statuses**: `200`; `500` on a database failure.
- Source: `src/handlers_photo.rs` (`get_timeline`), shape from `src/db_types.rs`

### `GET /api/photos/{hash}` — single photo

- **Response `200`**: the photo object above.
- **Statuses**: `200`; `404` `{ "error": "Photo not found" }`; `500`.
- Source: `src/handlers_photo.rs` (`get_photo`)

### `DELETE /api/photos/{hash}` — delete photo

Removes the DB row, the backing file and the hash-keyed caches.

- **Response `200`**: `{ "success": true, "message": "Photo deleted successfully" }`
- **Statuses**: `200`; `403` when the filesystem refuses the delete
  (`PermissionError`, message from the OS); `404`; `500`.
- Source: `src/handlers_photo.rs` (`delete_photo`)

### `GET /api/photos/{hash}/file` — original bytes

Full-size representation for the viewer. RAW sources (`.cr2`, `.nef`, …) are
decoded and re-encoded as JPEG on the fly, under a bounded decode semaphore; other
files are streamed as-is.

- **Response `200`**: file bytes with `content-type` (RAW → `image/jpeg`),
  `content-length`, `cache-control: public, max-age=31536000`.
- **Statuses**: `200`; `404` (no row, or backing file gone — a missing file is a
  `404`, never a lying `content-length`); `500` when RAW decode/encode fails.
- Byte ranges are **not** implemented on this route.
- Source: `src/handlers_photo.rs` (`get_photo_file`)

### `HEAD /api/photos/{hash}/file`

Headers only, no body, no transcoding triggered.

- **Response `200`**: `content-type`, `content-length` (actual on-disk size),
  `cache-control: public, max-age=31536000`.
- **Statuses**: `200`; `404`.
- For RAW sources `content-type` is reported as `image/jpeg` (what the GET would
  return) while `content-length` is the RAW source size — a deliberate divergence,
  because computing the decoded length would mean transcoding.
- Source: `src/handlers_photo.rs` (`head_photo_file`)

### `GET /api/photos/{hash}/thumbnail`

| Query param | Type | Meaning |
|---|---|---|
| `size` | string | `small` (200 px) \| `medium` (400 px, default) \| `large` (800 px); unknown → `medium` |
| `format` | string | `jpeg` (default) \| `webp`; unknown → `jpeg` |

- **Response `200`**: thumbnail bytes, `content-type: image/jpeg|image/webp`,
  `cache-control: public, max-age=86400`.
- **Statuses**: `200`; `404` (unknown hash **or** generation failure); `500`.
- Source: `src/handlers_thumbnail.rs` (`get_photo_thumbnail`)

### `GET /api/photos/{hash}/video` — video bytes

The video byte endpoint. It decides playback server-side
(`video_capability::plan()` over the file's capability record against the client's
declared codecs) and serves one of: the original, an already-cached faststart
remux sidecar, an already-cached whole-file conversion, or the original with a
warning header when a conversion failed.

| Query param | Type | Meaning |
|---|---|---|
| `client` | string | Client codec declaration (comma-separated tokens). Overridden by the `X-TurboPix-Codecs` header when both are present. Absent → conservative h264-8/no-audio plan. |
| `decision` | flag | Bare (`?decision`) or `=true` → return the playback decision as JSON instead of bytes |
| `metadata` | `true` | Return the video facts as JSON instead of bytes (`?metadata=true`) |
| `transcode` | `true` | Route a byte request to the cached conversion artifact (`?transcode=true`) |

**Codec precedence.** `headers["X-TurboPix-Codecs"]` wins over the `client` query
param; neither present means the conservative plan. The byte endpoint echoes the
resolved declaration into the returned decision URLs so the follow-up stream
re-decision sees the same declaration. A browser's own media-element request
cannot carry a request header, which is why the query param exists at all.

**Request headers read**: `X-TurboPix-Codecs`, `Range`.

**`?decision` response `200`**:

```json
{
  "action": "direct",
  "url": "/api/photos/<hash>/video?client=h264-8",
  "mode": null,
  "mime": null,
  "duration": 34.5,
  "cached": false,
  "reason": null,
  "codec": "h264",
  "bit_depth": 8,
  "audio_codec": "aac"
}
```

- `action` is `direct` (serve `url` as a file), `stream` (open `url` as an fMP4
  stream) or `empty` (0-byte backing file, likely a sync temp — `url`/`mode`/`mime`
  are `null` and the client should surface "still syncing").
- For `stream`, `mode` is `remux` | `audio` | `transcode` and `mime` is the MIME
  the client must create its `SourceBuffer` with.
- `cached: true` means a completed artifact is already on disk; for a cached
  whole-file conversion the `url` already carries `transcode=true`.
- `codec`, `bit_depth` and `audio_codec` describe the source file identically for
  every delivery (the client gates playback verification and audio handling on
  them).
- A `stream` decision's `url` carries neither `start` nor `mode`: the player
  appends `start=<seconds>` on every (re)start and `mode=<decision.mode>`.

**Byte response `200`**: the file or artifact, `content-type` = stored MIME for
the original / `video/mp4` for a remux sidecar or conversion, `accept-ranges:
bytes`, `cache-control: public, max-age=31536000`. `Range: bytes=…` is honored
with `206 Partial Content` + `content-range`; a multi-range or malformed value is
ignored and the full representation is served (RFC 9110 §14.2).

**Empty source**: `200` with `content-length: 0` and `x-transcode-warning: empty`
(the request is not an error; the client reports the file as unusable).

**`x-transcode-warning`** is attached only when a conversion attempt actually
failed and the original is being served instead:

- `empty` — 0-byte file
- `HEVC transcoding not available - serving original video`
- `Transcode worker pool busy - serving original video`

The warning is also set on the `206` arm.

- **Statuses**: `200`; `206`; `404` (unknown hash or missing backing file);
  `416` with `content-range: bytes */<size>` for an unsatisfiable range (including
  any range on a 0-byte file); `500`.
- A byte request for a **directly playable** source ignores `transcode=true` and
  serves the original.
- Source: `src/handlers_video.rs` (`get_video_file`)

### `HEAD /api/photos/{hash}/video`

Headers of the byte route without triggering conversion.

- **Response `200`**: `content-type`, `content-length` (actual on-disk size),
  `accept-ranges: bytes`, `cache-control: public, max-age=31536000`.
- **Statuses**: `200`; `404`.
- Source: `src/handlers_photo.rs` (`head_photo_video`)

### `GET /api/photos/{hash}/video/status`

Poll target after a whole-file conversion is spawned.

- **Response `200`**: the transcode status plus a deadline hint:

```json
{
  "state": "in_progress",
  "hash": "…",
  "started_at": "2026-11-14T10:00:00Z",
  "error": null,
  "percent": 42,
  "encoder": null,
  "deadline_ms": 258000
}
```

  `state` is one of `pending` | `in_progress` | `completed` | `failed` |
  `timeout`; `percent` is absent when no progress signal exists; `encoder`
  (`libx264` or a hardware encoder) is set on `completed`; `deadline_ms` is how
  long the server will keep the job running before giving up, absent when
  unknowable. The status store is in-memory, so a restart forgets it.
- **Statuses**: `200`; `404` when no status is retained for that hash.
- Source: `src/handlers_video.rs` (`get_video_status`)

### `GET /api/photos/{hash}/video/stream` — live fMP4 stream

Streams fragmented MP4 out of a live ffmpeg run. This is the endpoint the player
uses; the byte endpoint above serves whole files.

| Query param | Type | Meaning |
|---|---|---|
| `start` | f64 | Seconds into the source; clamped to `0..=86400`. Default `0`. The client re-requests with a new `start` on seek. |
| `mode` | string | `remux` \| `audio` \| `transcode`. Only ever **escalates** the plan: a client may ask for more work than the plan requires, never less. Unknown values are ignored rather than rejected. |
| `client` | string | Same tokens as `?client=` on the decision; overridden by `X-TurboPix-Codecs`. |

**Request headers read**: `X-TurboPix-Codecs`.

The mode is re-derived server-side from the same plan the decision used, then
raised by `?mode=` if that asks for more — a stream request cannot talk the
server down from the conversion the plan requires.

**Response `200`**: the fMP4 byte stream with

| Header | Value |
|---|---|
| `content-type` | `video/mp4` |
| `cache-control` | `no-store` |
| `x-turbopix-mode` | `remux` \| `audio` \| `transcode` (the mode this run is actually producing) |
| `x-turbopix-mime` | the MIME matching the codecs this run actually copies/encodes — use this for `MediaSource.isTypeSupported` / `addSourceBuffer` |
| `x-turbopix-encoder` | present only when video was actually encoded (`libx264` or a hardware encoder); **absent** on the copy modes and on audio-only runs |
| `x-turbopix-duration` | `"<seconds.3f>"`, when the duration is known |

**Statuses**:

| Status | Body | Meaning |
|---|---|---|
| `200` | stream | Streaming |
| `200` | empty, `content-length: 0`, `x-transcode-warning: empty` | 0-byte backing file |
| `415` | `{ "error": "source has no playable stream" }` | Neither a mappable video nor an audio track: refused without spawning ffmpeg or taking a permit. Deliberately **not** `503` — the client's saturation path keys on `503`, so a `415` climbs one rung of its ladder instead of retrying the same offset forever. |
| `503` + `Retry-After: 2` | `{ "error": "no conversion slot available" }` | Every transcode permit stayed taken for `TURBO_PIX_STREAM_QUEUE_WAIT_SECS` (default 20 s) — the request waits for a slot rather than failing at once. Retry the same request; the viewer keeps its waiting notice visible. |
| `503` + `Retry-After: 5` | `{ "error": "conversion disabled" }` | Transcoding is disabled (`TURBO_PIX_MAX_TRANSCODES=0`) |
| `404` | error envelope | Unknown hash or missing backing file |
| `500` | `{ "error": "conversion could not be started" }` | ffmpeg could not be spawned (the detail, including the configured path, stays in the server log) |

A run that starts at the head of the source and finishes cleanly fills the
conversion cache in the background, behind the stream the client is already
playing.

- Source: `src/handlers_video.rs` (`stream_video`)

### `PUT /api/photos/{hash}/favorite`

Body: `{ "is_favorite": true }` — an explicit set, not a toggle.

- **Response `200`**: the updated photo object (`is_favorite` and `updated_at`
  changed; every file fact preserved).
- **Statuses**: `200`; `404`; `400`/`411`/`413`/`415` per the body rules; `500`.
- Source: `src/handlers_photo.rs` (`toggle_favorite`)

### `GET /api/photos/{hash}/exif`

Raw EXIF read from the file on each request (not from the row).

- **Response `200`**:

```json
{
  "hash": "…",
  "filename": "IMG_1234.jpg",
  "exif": {
    "0x0110_Model": { "value": "EOS R5", "tag": "Model" }
  }
}
```

  Keys are `0x<tag hex>_<tag name>`. A file with no EXIF APP1/APP2 segment answers
  `404` (this is a normal condition, not a fault — treat it as "no EXIF"), not an
  empty object.
- **Statuses**: `200`; `404` (unknown hash, unreadable file, or no EXIF segment);
  `500` for any other EXIF read failure.
- Source: `src/handlers_photo.rs` (`get_photo_exif`)

### `PATCH /api/photos/{hash}/metadata`

Writes capture facts to the **file**, then records the read-back on the row — the
file is the authority.

Body (all fields optional):

```json
{ "taken_at": "2024-07-04T12:00:00Z", "latitude": 52.52, "longitude": 13.405 }
```

- Photos: an EXIF append (atomic sibling-temp + rename).
- Videos: a `moov` rewrite, serialized by a single video-edit lock.

**Response `200`**: the photo object as read back from the file.

**Statuses**:

| Status | `error_code` | Cause |
|---|---|---|
| `200` | — | Saved |
| `400` | `invalid_date` (video) / none (photo) | Unparsable `taken_at` |
| `400` | `invalid_coordinates` | Out-of-range or unpaired coordinates (photo path reports it as a plain validation message) |
| `403` | `file_read_only` | Read-only medium |
| `404` | `file_missing` | Backing file gone |
| `404` | — | Unknown hash |
| `415` | `unsupported_container` | Not a container this writer can edit |
| `422` | `no_writable_slot` | Fragmented file, or no room for the value |
| `422` | `no_location_carrier` | Container has no location carrier |
| `422` | `unrepresentable_value` | Value cannot be represented in the container |
| `500` | — | I/O failure or failed rollback (the file may differ from the row) |

- Source: `src/handlers_photo.rs` (`update_photo_metadata`)

### `POST /api/photos/{hash}/rotate`

Body: `{ "angle": 90 }` — only `90`, `180` or `270`.

Rotations are serialized server-wide. The content hash changes, so the response
carries a **new** hash; thumbnails and transcode artifacts for the old hash are
dropped.

- **Response `200`**: the updated photo object (with the new `hash_sha256`,
  `width`/`height` and `updated_at`).
- **Statuses**: `200`; `400` (`Invalid rotation angle: …, Must be 90, 180, or 270`);
  `404`; `500`.
- Source: `src/handlers_photo.rs` (`rotate_photo`)

### `POST /api/photos/batch/delete`

Body: `{ "hashes": ["…", "…"] }`

- **Response `200`**: `{ "applied": ["…"], "failed": [ { "id": "…", "error": "Photo not found" } ] }`
  — partial failure is a `200` with a per-item list; `failed` is omitted when empty.
- **Statuses**: `200`; `400` (empty or >1000 hashes); `411`/`413`/`415`; `500`.
- Source: `src/handlers_photo.rs` (`batch_delete`)

### `POST /api/photos/batch/favorite`

Body: `{ "hashes": ["…"], "is_favorite": true }` — explicit direction, never a
toggle, so a mixed selection resolves to one state.

- **Response `200`**: the batch result shape above.
- **Statuses**: `200`; `400`; `411`/`413`/`415`; `500`.
- Source: `src/handlers_photo.rs` (`batch_favorite`)

### `POST /api/photos/batch/export`

Body: `{ "hashes": ["…"] }`. Packs the selected **originals** into one ZIP
(`Stored`, no re-compression; duplicate names get `-2`, `-3`, … before the
extension).

- **Response `200`**: `content-type: application/zip`,
  `content-disposition: attachment; filename="turbo-pix-export-<timestamp>.zip"`,
  `cache-control: no-store`.
- **`400`**: `{ "error": "Some selected photos could not be exported", "failed": [ … ] }`
  — the one batch action that rejects wholesale, because a half-written archive is
  not useful. Missing rows and missing files are both listed per item.
- **Statuses**: `200`; `400` (unknown photo/file, or empty/>1000 hashes);
  `411`/`413`/`415`; `500`.
- Source: `src/handlers_photo.rs` (`batch_export`)

## Search

### `GET /api/search/semantic`

CLIP-embedding similarity search over indexed photos.

| Query param | Type | Meaning |
|---|---|---|
| `q` | string | **Required**; the search phrase |
| `limit` | usize | Default `50`, clamped to `1..=200` |
| `offset` | usize | Default `0`, capped at `1_000_000` |

- **Response `200`**:

```json
{
  "results": [ { "path": "/photos/a.jpg", "hash": "…", "score": 0.83 } ],
  "query": "a red bicycle",
  "total": 1
}
```

  Results are sorted by descending `score`; results whose path no longer matches a
  row are dropped. An empty match is `200` with `results: []`.
- **Statuses**: `200`; `400` when `q` is missing or the query fails to
  deserialize; `500` when the search engine or the follow-up lookup fails.
- Source: `src/handlers_search.rs` (`semantic_search`)

## Indexing

### `GET /api/indexing/status`

Progress of the background scan, polled by the UI.

- **Response `200`**:

```json
{
  "is_indexing": true,
  "is_complete": false,
  "started_at": "2026-11-14T09:58:00Z",
  "active_phase_id": "metadata",
  "phases": [
    {
      "id": "metadata",
      "state": "active",
      "kind": "determinate",
      "processed": 1200,
      "total": 4000,
      "errors": 0,
      "current_item": "/photos/2024/IMG_9999.jpg"
    }
  ],
  "photos_indexed": 4000
}
```

  - `phases` always carries the six canonical phases in order: `discovering`,
    `metadata`, `semantic_vectors`, `geo_resolution`, `collages`, `housekeeping`.
  - `state` is `pending` | `active` | `done` | `error`; `kind` is `determinate` |
    `indeterminate`; `total` is `null` for indeterminate phases; `current_item` is
    a raw path/basename, not user-facing text.
  - `active_phase_id` is `""` when not indexing; `photos_indexed` is the current
    row count.
- **Response headers**: `cache-control: no-store, no-cache, must-revalidate`,
  `pragma: no-cache`.
- **Statuses**: `200` (this handler is infallible).
- Source: `src/handlers_indexing.rs` (`get_indexing_status`)

## Albums

An album is `{ "id": 1, "name": "Trip", "created_at": "…", "updated_at": "…" }`.
Names are trimmed, must be non-empty and at most 200 characters (`400` otherwise).

### `GET /api/albums`

- **Response `200`**: `{ "albums": [ … ] }`
- Source: `src/handlers_albums.rs` (`list_albums`)

### `POST /api/albums`

Body: `{ "name": "Trip", "initial_hashes": ["…"] }` (`initial_hashes` optional).

- **Response `201`**: the created album object.
- **Statuses**: `201`; `400` (bad name, or more than 1000 `initial_hashes`);
  `411`/`413`/`415`; `500`.
- Source: `src/handlers_albums.rs` (`create_album`)

### `PATCH /api/albums/{id}`

Body: `{ "name": "New name" }`

- **Response `200`**: the renamed album object.
- **Statuses**: `200`; `400`; `404` (unknown album, JSON envelope); `500`.
- Source: `src/handlers_albums.rs` (`rename_album`)

### `DELETE /api/albums/{id}`

- **Response `204`**: no body.
- **Statuses**: `204`; `404`; `500`.
- Source: `src/handlers_albums.rs` (`delete_album`)

### `GET /api/albums/{id}/photos`

Members of one album, paginated.

| Query param | Type | Meaning |
|---|---|---|
| `page` | u32 | Default `1`, min `1` |
| `limit` | u32 | Default `50`, clamped to `1..=100` |
| `sort`, `order` | string | Same whitelist/default as the grid listing |

- **Response `200`**: `{ "photos": [...], "total": 12, "page": 1, "limit": 50, "has_next": false, "has_prev": false }`
- **Statuses**: `200`; `400`; `404` (album deleted concurrently is also reported
  as `404`, not an empty `200`); `500`.
- Source: `src/handlers_albums.rs` (`list_album_photos`)

### `POST /api/albums/{id}/members`

Body: `{ "hashes": ["…"] }`. Adding a member that is already there is a no-op.

- **Response `200`**: `{ "added": 3, "total": 12 }`
- **Statuses**: `200`; `400` (more than 1000 hashes); `404`; `411`/`413`/`415`;
  `500`.
- Source: `src/handlers_albums.rs` (`add_album_members`)

### `DELETE /api/albums/{id}/members`

Body: `{ "hashes": ["…"] }` — a `DELETE` **with** a JSON body (the 1 MiB cap and
`Content-Length` requirement apply).

- **Response `200`**: `{ "removed": 2, "total": 10 }`
- **Statuses**: `200`; `400`; `404`; `411`/`413`/`415`; `500`.
- Source: `src/handlers_albums.rs` (`remove_album_members`)

## Collages

A collage is
`{ "id": 7, "date": "2024-07-04", "file_path": "…", "thumbnail_path": "…|null", "photo_count": 5, "photo_hashes": ["…"], "accepted_at": "…|null", "rejected_at": "…|null", "created_at": "…" }`.

### `GET /api/collages/pending`

- **Response `200`**: an array of collage objects (may be empty).
- **Statuses**: `200`; `500`.
- Source: `src/handlers_collage.rs` (`list_pending_collages`)

### `GET /api/collages/{id}/image`

The rendered collage JPEG.

- **Response `200`**: `content-type: image/jpeg`, `content-length`,
  `cache-control: public, max-age=31536000` (streamed, not buffered).
- **Statuses**: `200`; `404` (unknown id or missing file); `500`.
- Source: `src/handlers_collage.rs` (`get_collage_image`)

### `POST /api/collages/generate`

Triggers collage generation for the current library (also a maintenance/testing
hook).

- **Response `200`**: `{ "success": true, "count": 2, "message": "2 collage(s) generated" }`
- **Statuses**: `200`; `500`.
- Source: `src/handlers_collage.rs` (`generate_collages_manual`)

### `POST /api/collages/{id}/accept`

Moves the collage into the photos directory and indexes it immediately.
Idempotent for settled collages.

- **Response `200`**: `{ "success": true, "message": "Collage accepted and added to 'All Photos'.", "path": "/…" }`
- **Statuses**: `200`; `500`.
- Source: `src/handlers_collage.rs` (`accept_collage`)

### `DELETE /api/collages/{id}/reject`

Deletes the collage files and marks the row rejected.

- **Response `200`**: `{ "success": true, "message": "Collage rejected and deleted" }`
- **Statuses**: `200`; `500`.
- Source: `src/handlers_collage.rs` (`reject_collage`)

### `POST /api/collages/batch-accept`

Body: `{ "ids": [1, 2] }`. Accept is idempotent, so a double submit is merely
counted as applied again.

- **Response `200`**: `{ "applied": ["1"], "failed": [ { "id": "2", "error": "…" } ] }`
- **Statuses**: `200`; `400` (empty or >1000 ids); `411`/`413`/`415`; `500`.
- Source: `src/handlers_collage.rs` (`batch_accept_collages`)

### `POST /api/collages/batch-reject`

Body: `{ "ids": [1, 2] }`

- **Response `200`**: the batch result shape above.
- **Statuses**: `200`; `400`; `411`/`413`/`415`; `500`.
- Source: `src/handlers_collage.rs` (`batch_reject_collages`)

## Saved searches

A saved search is
`{ "id": 1, "name": "Berlin 2024", "query": "…|null", "view": "all", "sort": "date_desc", "year": 2024, "month": null, "to_year": null, "to_month": null, "created_at": "…" }`.

- `view` must be one of `all`, `favorites`, `videos`, `collages`, `housekeeping`.
- `sort` must be one of `date_desc`, `date_asc`, `name_asc`, `name_desc`,
  `size_desc`, `size_asc`.
- Names are trimmed, non-empty, ≤200 characters.

### `GET /api/saved-searches`

- **Response `200`**: `{ "saved_searches": [ … ] }`
- Source: `src/handlers_saved_searches.rs` (`list_saved_searches`)

### `POST /api/saved-searches`

Body: `{ "name": "…", "view": "all", "sort": "date_desc", "query": "…", "year": 2024, "month": null, "to_year": null, "to_month": null }`
(`name`, `view`, `sort` required; the rest optional).

- **Response `201`**: the created saved search.
- **`409`**: `{ "error": "already_saved", "saved_search": { … } }` when an
  identical search already exists — the existing row is returned so the client can
  highlight it instead of duplicating.
- **Statuses**: `201`; `400` (bad name, view, sort, or date filter); `409`;
  `411`/`413`/`415`; `500`.
- Source: `src/handlers_saved_searches.rs` (`create_saved_search`)

### `PATCH /api/saved-searches/{id}`

Body: `{ "name": "…" }`

- **Response `200`**: the renamed saved search.
- **Statuses**: `200`; `400`; `404`; `500`.
- Source: `src/handlers_saved_searches.rs` (`rename_saved_search`)

### `DELETE /api/saved-searches/{id}`

- **Response `204`**: no body.
- **Statuses**: `204`; `404`; `500`.
- Source: `src/handlers_saved_searches.rs` (`delete_saved_search`)

## Housekeeping

Candidates are photos the housekeeping phase flagged (blurry, duplicates, …).

### `GET /api/housekeeping/candidates`

- **Response `200`**: `{ "candidates": [ { "photo": { …photo object… }, "reason": "…", "score": 0.81 } ] }`
  — at most the 100 highest-scoring candidates.
- **Statuses**: `200`; `500`.
- Source: `src/handlers_housekeeping.rs` (`list_housekeeping_candidates`)

### `DELETE /api/housekeeping/candidates/{hash}`

Dismisses one candidate. The photo itself stays in the library.

- **Response `200`**: `{ "success": true }`
- **Statuses**: `200`; `404` when no candidate row exists for that hash; `500`.
- Source: `src/handlers_housekeeping.rs` (`remove_housekeeping_candidate`)

### `POST /api/housekeeping/candidates/batch-remove`

Body: `{ "hashes": ["…"] }`. Dismiss, not delete.

- **Response `200`**: `{ "applied": ["…"], "failed": [ { "id": "…", "error": "Candidate not found" } ] }`
- **Statuses**: `200`; `400`; `411`/`413`/`415`; `500`.
- Source: `src/handlers_housekeeping.rs` (`batch_remove_candidates`)

## Static frontend

Everything that is not an API route is served from the frontend bundle embedded in
the binary (`src/handlers_static.rs`):

- Known assets (embedded at build time) are served with a `content-type` derived
  from the file extension and `cache-control: no-cache` — Vite emits unhashed
  filenames, so the browser must revalidate on every load. `GET` and `HEAD` are
  both supported (the `HEAD` mirror returns the same headers with an empty body
  and advertises no `accept-ranges`, because the static route ignores `Range`).
- Any other `GET` path returns `index.html` with `cache-control: no-cache` (SPA
  routing), **except** paths under `/api/`, `/assets/`, `/favicon*`,
  `/site.webmanifest` and `/fonts/`, which keep their real `404`.

## Route index

| Method | Path | Handler |
|---|---|---|
| GET | `/health` | `src/handlers_health.rs` |
| GET | `/ready` | `src/handlers_health.rs` |
| GET | `/api/config` | `src/handlers_config.rs` |
| GET | `/api/photos` | `src/handlers_photo.rs` |
| GET | `/api/photos/map` | `src/handlers_photo.rs` |
| GET | `/api/photos/timeline` | `src/handlers_photo.rs` |
| POST | `/api/photos/batch/delete` | `src/handlers_photo.rs` |
| POST | `/api/photos/batch/favorite` | `src/handlers_photo.rs` |
| POST | `/api/photos/batch/export` | `src/handlers_photo.rs` |
| GET | `/api/photos/{hash}` | `src/handlers_photo.rs` |
| DELETE | `/api/photos/{hash}` | `src/handlers_photo.rs` |
| GET | `/api/photos/{hash}/file` | `src/handlers_photo.rs` |
| HEAD | `/api/photos/{hash}/file` | `src/handlers_photo.rs` |
| GET | `/api/photos/{hash}/thumbnail` | `src/handlers_thumbnail.rs` |
| GET | `/api/photos/{hash}/video` | `src/handlers_video.rs` |
| HEAD | `/api/photos/{hash}/video` | `src/handlers_photo.rs` |
| GET | `/api/photos/{hash}/video/status` | `src/handlers_video.rs` |
| GET | `/api/photos/{hash}/video/stream` | `src/handlers_video.rs` |
| PUT | `/api/photos/{hash}/favorite` | `src/handlers_photo.rs` |
| GET | `/api/photos/{hash}/exif` | `src/handlers_photo.rs` |
| PATCH | `/api/photos/{hash}/metadata` | `src/handlers_photo.rs` |
| POST | `/api/photos/{hash}/rotate` | `src/handlers_photo.rs` |
| GET | `/api/search/semantic` | `src/handlers_search.rs` |
| GET | `/api/indexing/status` | `src/handlers_indexing.rs` |
| GET | `/api/albums` | `src/handlers_albums.rs` |
| POST | `/api/albums` | `src/handlers_albums.rs` |
| PATCH | `/api/albums/{id}` | `src/handlers_albums.rs` |
| DELETE | `/api/albums/{id}` | `src/handlers_albums.rs` |
| GET | `/api/albums/{id}/photos` | `src/handlers_albums.rs` |
| POST | `/api/albums/{id}/members` | `src/handlers_albums.rs` |
| DELETE | `/api/albums/{id}/members` | `src/handlers_albums.rs` |
| GET | `/api/collages/pending` | `src/handlers_collage.rs` |
| GET | `/api/collages/{id}/image` | `src/handlers_collage.rs` |
| POST | `/api/collages/generate` | `src/handlers_collage.rs` |
| POST | `/api/collages/{id}/accept` | `src/handlers_collage.rs` |
| DELETE | `/api/collages/{id}/reject` | `src/handlers_collage.rs` |
| POST | `/api/collages/batch-accept` | `src/handlers_collage.rs` |
| POST | `/api/collages/batch-reject` | `src/handlers_collage.rs` |
| GET | `/api/saved-searches` | `src/handlers_saved_searches.rs` |
| POST | `/api/saved-searches` | `src/handlers_saved_searches.rs` |
| PATCH | `/api/saved-searches/{id}` | `src/handlers_saved_searches.rs` |
| DELETE | `/api/saved-searches/{id}` | `src/handlers_saved_searches.rs` |
| GET | `/api/housekeeping/candidates` | `src/handlers_housekeeping.rs` |
| POST | `/api/housekeeping/candidates/batch-remove` | `src/handlers_housekeeping.rs` |
| DELETE | `/api/housekeeping/candidates/{hash}` | `src/handlers_housekeeping.rs` |

Route composition (which filter `.or`s which, and why literal paths must come
before `{hash}`) lives in `build_*_routes` in each handler module, mounted under
`src/main.rs`.
