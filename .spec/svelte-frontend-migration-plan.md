# Implementation Plan: Svelte Frontend Migration

## Context

Replace the TurboPix vanilla-JS frontend (~10,900 lines of vanilla JavaScript across 24 script files, no bundler, embedded into the Rust binary via `include_static!` in `src/handlers_static.rs`) with a Svelte 5 (runes) + Vite build pipeline. Every user-facing feature, behavior, DOM hook, and visual detail is preserved identically; the Rust backend, REST API, and database are untouched. Approved spec: `.spec/svelte-frontend-migration.md`.

Locked decisions (from spec, do not reopen): Svelte 5 runes, Vite, no SvelteKit/SSR; Vite outputs to `dist/` at repo root, Rust embeds from there; full CSS migration to component-scoped `<style>` blocks plus one global token stylesheet; `svelte-i18n` as the i18n library (plan decision — the spec leaves the library open); big-bang cutover, no hybrid mode.

## Approach

Phases are sequential. After Phase 1 the tree builds and the binary serves the Svelte shell; each later phase adds one behavior group. E2E tests are the final gate (Phase 9) — they cannot pass mid-migration because they exercise the real UI. `static/` is deleted only in Phase 10, after E2E is green.

### Phase 1: Vite + Svelte scaffold and Rust build integration

**1a. npm setup — single `package.json` at repo root (do NOT create `frontend/package.json`)**

Run at repo root:
```bash
npm install svelte@^5 svelte-i18n feather-icons
npm install -D vite @sveltejs/vite-plugin-svelte eslint-plugin-svelte prettier-plugin-svelte
```

Edit root `package.json`:
- Add scripts: `"dev": "vite"`, `"build": "vite build"`.
- Repoint lint/format globs from `static/` to the new sources: `format`/`format:check` → `frontend/**/*.{js,css,svelte,html}`; `lint:js` → `frontend/src/**/*.{js,svelte}`; `lint:css` → `frontend/src/**/*.css`.

**1b. Build config — exact file contents**

`vite.config.js` at repo root:
```js
import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  root: 'frontend',
  base: '/',
  plugins: [svelte()],
  build: {
    outDir: '../dist',       // resolves to <repo>/dist (outDir is relative to `root`)
    emptyOutDir: true,       // required: outDir is outside `root`
    sourcemap: false,
    rollupOptions: {
      output: {
        entryFileNames: 'assets/[name].js',
        chunkFileNames: 'assets/[name].js',
        assetFileNames: 'assets/[name].[ext]',
      },
    },
  },
});
```
Hashless filenames are deliberate: this is a locally served desktop app, there is no CDN cache to bust, and it keeps embedded paths stable.

`frontend/svelte.config.js` (the Vite root is `frontend/`):
```js
export default { compilerOptions: { runes: true } };
```

`frontend/index.html` — minimal shell. The static markup of the old `static/index.html` (header, sidebar, viewer, modals) is NOT copied; it becomes Svelte components in later phases. Exact contents:
```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0, viewport-fit=cover" />
    <title>TurboPix</title>
    <link rel="icon" type="image/svg+xml" href="/favicon.svg" />
    <link rel="manifest" href="/site.webmanifest" />
    <script>
      // Theme FOUC prevention: apply persisted/system theme before first paint.
      (function () {
        var theme = null;
        try { theme = JSON.parse(localStorage.getItem('theme')); } catch (e) { /* ignore */ }
        if (theme !== 'light' && theme !== 'dark') {
          theme = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
        }
        document.documentElement.classList.add(theme + '-theme');
      })();
    </script>
  </head>
  <body>
    <div id="app"></div>
    <script type="module" src="/src/main.js"></script>
  </body>
</html>
```
Note: `theme` is stored JSON-encoded by the current `utils.storage` (i.e. `"\"dark\""`), hence `JSON.parse`.

`frontend/src/main.js`:
```js
import { mount } from 'svelte';
import App from './App.svelte';
import './app.css';

const app = mount(App, { target: document.getElementById('app') });
export default app;
```

`frontend/src/App.svelte`: placeholder `<h1>TurboPix</h1>` for now; `frontend/src/app.css`: empty for now (filled in Phase 2).

**1c. Static assets moved into the Vite project**

Move these from `static/` into `frontend/public/` (Vite copies `public/` verbatim into `dist/`):
- `static/favicon.svg` → `frontend/public/favicon.svg`
- `static/site.webmanifest` → `frontend/public/site.webmanifest`
- The 3 frontend fonts → `frontend/public/fonts/`: `PlayfairDisplay-Bold.woff2`, `DMSans-Regular.woff2`, `DMSans-Medium.woff2` (the `@font-face` rules in the CSS reference `/fonts/<name>.woff2`, which keeps working unchanged; the old `PlayfairDisplay-Regular.woff2` was never referenced by a `@font-face` rule and is not ported).

**1d. Rust embedding: replace the manual file lists with `build.rs` codegen**

Reason: Vite may emit unpredictable chunk files (`assets/vendor.js` etc.) as dependencies grow, so a hand-maintained `STATIC_FILES` list breaks silently. A build script globs `dist/` at compile time — no list to maintain, no new crate dependencies.

Create `build.rs` at repo root (none exists today):
```rust
use std::{env, fs, path::Path};

const TEXT_EXTS: &[&str] = &["html", "js", "css", "svg", "json", "txt", "map", "webmanifest"];

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let dist = Path::new(&manifest_dir).join("dist");

    if !dist.join("index.html").exists() {
        panic!("dist/index.html not found — run `npm run build` before `cargo build`");
    }

    let mut text_entries = Vec::new();
    let mut binary_entries = Vec::new();
    collect(&dist, &dist, &mut text_entries, &mut binary_entries);

    let mut code = String::from("const STATIC_FILES: &[(&str, &str)] = &[\n");
    for (rel, abs) in &text_entries {
        code.push_str(&format!("    ({rel:?}, include_str!({abs:?})),\n"));
    }
    code.push_str("];\n\nconst STATIC_BINARY_FILES: &[(&str, &[u8])] = &[\n");
    for (rel, abs) in &binary_entries {
        code.push_str(&format!("    ({rel:?}, include_bytes!({abs:?}) as &[u8]),\n"));
    }
    code.push_str("];\n");

    let out_dir = env::var("OUT_DIR").unwrap();
    fs::write(Path::new(&out_dir).join("embedded_static.rs"), code).unwrap();
}

fn collect(dir: &Path, root: &Path, text: &mut Vec<(String, String)>, binary: &mut Vec<(String, String)>) {
    println!("cargo:rerun-if-changed={}", dir.display());
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, root, text, binary);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            let abs = path.to_string_lossy().replace('\\', "/");
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if TEXT_EXTS.contains(&ext) {
                text.push((rel, abs));
            } else {
                binary.push((rel, abs));
            }
        }
    }
}
```

Edit `src/handlers_static.rs`:
- Delete the `include_static!` and `include_static_binary!` macro definitions and both const arrays (`STATIC_FILES`, `STATIC_BINARY_FILES`).
- Add at module level: `include!(concat!(env!("OUT_DIR"), "/embedded_static.rs"));`
- Keep `content_type_from_path`, `build_route_for_file`, `build_route_for_binary_file` unchanged — the generated arrays have the same shapes as the old ones.
- In `build_static_routes()`'s SPA fallback, replace the rejected-prefix list. Old: `/api/`, `/css/`, `/js/`, `/i18n/`, `/favicon`, `/site.webmanifest`, `/fonts/`. New exact list:
  ```rust
  if path_str.starts_with("/api/")
      || path_str.starts_with("/assets/")
      || path_str.starts_with("/favicon")
      || path_str.starts_with("/site.webmanifest")
      || path_str.starts_with("/fonts/")
  ```
  (`/css/`, `/js/`, `/i18n/` no longer exist; Vite serves bundles under `/assets/`.)

**1e. E2E build step**

Edit `tests/e2e/setup/global-setup.js` → `buildBinary()`: run the frontend build first so `dist/` exists before cargo embeds it. Exact change — replace the body of `buildBinary()`'s try block with:
```js
await execAsync('npm run build');
const { stdout, stderr } = await execAsync('cargo build --bin turbo-pix');
```

**1f. Smoke test (gate for Phase 1)**

```bash
npm run build && cargo build --bin turbo-pix
TURBO_PIX_DATA_PATH=/tmp/tp-smoke TURBO_PIX_PHOTO_PATHS=/tmp/tp-photos TURBO_PIX_PORT=18473 ./target/debug/turbo-pix &
curl -s http://localhost:18473/ | grep -q '<div id="app">' && echo OK
curl -s http://localhost:18473/assets/index.js | head -c 100   # returns JS bundle
curl -s -o /dev/null -w '%{http_code}' http://localhost:18473/favicon.svg   # 200
```
Kill the server afterwards.

### Phase 2: App shell — ported core modules, state, router, i18n, layout

**2a. Port shared modules (plain ESM, no `window.*` globals)**

| New file | Port source | Port exactly |
|---|---|---|
| `frontend/src/lib/api.js` | `static/js/api.js` | Entire `TurboPixAPI` class, all methods/URLs unchanged. Export `export const api = new TurboPixAPI();`. Remove `window.api` assignment. Logger/perf calls now import from `./logger.js` / `./utils.js`. |
| `frontend/src/lib/utils.js` | `static/js/utils.js` | `formatFileSize`, `formatDate`, `formatDuration`, `formatCollageDate`, `throttle`, `debounce`, `storage` (localStorage JSON wrapper — keep the same JSON encoding for `theme`, `viewSettings`, `searchHistory`), `handleError`, `showToast` logic (see 2e), `performance` marks, `getThumbnailUrl`, `getPhotoUrl`, `getVideoUrl`, `videoCodecSupport`. Do NOT port: `$`, `$$`, `createElement`, `createElementWithAttrs`, `setSafeAttributes`, `on`, `off`, `emit`, `SimpleState` (DOM/Svelte replaces them). |
| `frontend/src/lib/constants.js` | `static/js/constants.js` | `APP_CONSTANTS` object as a named export, values unchanged. |
| `frontend/src/lib/logger.js` | `static/js/logger.js` | `TurboPixLogger` class, `export const logger = new TurboPixLogger(...)`. Keep `turbopix_logs` localStorage persistence as-is. |
| `frontend/src/lib/blurhash.js` | `static/js/blurhash.js` | `decode`, `createCanvas`, `toDataURL` as named exports. |

`static/js/i18n.js` is dead code — it is embedded in `STATIC_FILES` but no `<script>` tag in `static/index.html` loads it (verified). Do not port it.

**2b. Reactive state modules (Svelte 5 runes)**

Create `frontend/src/lib/state.svelte.js`:
```js
export const appState = $state({
  currentView: 'all',
  searchQuery: '',
  sortOrder: 'date_desc',
  selectedYear: null,
  selectedMonth: null,
  isLoading: false,
  isMobile: false,
  sidebarOpen: false,
  totalPhotos: 0,
});

export const indexingState = $state({
  isIndexing: false,
  currentPhase: null,
  phases: [],
  photosIndexed: 0,
  currentItem: '',
  sheetOpen: false,
});

export const photoGridState = $state({
  photos: [],
  currentPage: 1,
  loading: false,
  hasMore: true,
  currentQuery: '',
  semanticSearchMode: false,
});

export const themeState = $state({ theme: 'light' });
```

**2c. Router — verbatim logic port**

Create `frontend/src/lib/router.svelte.js`. Copy these methods verbatim from `static/js/router.js` (they are pure and already correct): `parseUrl`, `normalizeState`, `parsePositiveInteger`, `normalizeString`, `buildUrl` (including the month-nested-inside-year guard). Wrap them around a reactive object:
```js
export const route = $state({ view: 'all', photo: null, query: null, sort: 'date_desc', year: null, month: null });
```
- `pushState(changes)` / `replaceState(changes)`: merge → normalize → `buildUrl` → `history.pushState/replaceState(state, '', url)` → assign to `route`. Guarded by an `updatingFromPopstate` flag that skips the history write when the update originates from the popstate handler (the anti-loop pattern from AGENTS.md).
- `init()`: parse `window.location` into `route`, register the `popstate` listener (once), which sets the flag, re-parses into `route`, clears the flag.
- Valid views `['all','favorites','videos','collages','housekeeping']`, valid sorts `['date_desc','date_asc','name_asc','name_desc','size_desc','size_asc']` — same literals as the source.
- Components never listen to router callbacks; they read `route` reactively (runes re-render). This replaces `onStateChange`.

**2d. i18n via svelte-i18n**

Port dictionaries: `static/i18n/en/index.js` → `frontend/src/i18n/en.json`, `static/i18n/de/index.js` → `frontend/src/i18n/de.json`. Same nested object shape (two-level `namespace.key`), with ONE transform: every interpolation placeholder `{{name}}` becomes `{name}` (svelte-i18n syntax). ~15 occurrences (`photos_count`, `collage_for`, `search_results`, `photos_from_year`, `indexing_counter`, `indexing_sheet_errors`, `indexing_ring_tooltip`, `collage_photos`, `collagesGenerated`, `no_photos_match_search`, `edit_unsupported_format`, etc. — grep `{{` in `static/i18n/` for the complete list).

Create `frontend/src/lib/i18n.js`:
```js
import { init, register, locale, _ } from 'svelte-i18n';
import en from '../i18n/en.json';
import de from '../i18n/de.json';

register('en', () => Promise.resolve(en));
register('de', () => Promise.resolve(de));

export function initI18n(defaultLocale) {
  const saved = localStorage.getItem('turbo-pix-locale');   // raw string, NOT JSON
  const initial = ['en', 'de'].includes(saved) ? saved
    : ['en', 'de'].includes(defaultLocale) ? defaultLocale : 'en';
  init({ fallbackLocale: 'en', initialLocale: initial });
}

export function setLocale(l) {
  if (!['en', 'de'].includes(l)) l = 'en';
  locale.set(l);
  localStorage.setItem('turbo-pix-locale', l);
}

export { _ as t };
```
Resolution order (localStorage → `/api/config` `default_locale` → browser language → `en`) is required by spec FR-015: the operator config beats the browser, and the browser language is kept as a fallback (config value wins when it differs). Note the current vanilla code never reads `turbo-pix-locale` at startup — this is a deliberate spec-mandated fix, not a regression. Also port `translateError(errorMessage)` from `static/i18n/i18nManager.js` (backend error-string → translation-key mapping) into this module.

In templates use `{$t('ui.appTitle')}` and `{$t('ui.photos_count', { values: { count } })}`. This replaces every `data-i18n` / `data-i18n-placeholder` / `data-i18n-title` / `data-i18n-alt` attribute from the old HTML — no test selects on `data-i18n` (verified by grep), so removal is safe.

**2e. Cross-component events and test-required globals**

Preserve these exact window touchpoints (E2E tests depend on them — verified in `tests/e2e/specs/indexing-orbit.e2e.spec.js`):
- `window.indexingStatus` — an object exposing at minimum `async checkStatus()` and `isIndexing`. Assigned by `IndexingOrbit.svelte` in `onMount` (Phase 6).
- CustomEvents dispatched on `window` with the same names/details as today: `indexingStatusChanged` (detail = normalized status), `favoriteToggled` (`{photoHash, isFavorite}`), `collageAccepted`/`collageRejected` (`{collageId}`), `housekeepingCandidateRemoved` (`{hash}`). Svelte state is the source of truth; events are dispatched alongside for tests and cross-component signaling.
- localStorage keys with exact encodings: `theme` (JSON string via `utils.storage`), `turbo-pix-locale` (raw string), `turbopix_has_indexed` (string `'true'`), `viewSettings` (JSON), `searchHistory` (JSON array, max 20), `turbopix_logs` (JSON).

Toasts: port `utils.showToast` as a reactive `toasts` array in `state.svelte.js` plus a `ToastContainer.svelte` rendered by `App.svelte`; same visual behavior as the current implementation.

**2f. Shell components and selector contract**

Build `App.svelte` + `Header.svelte`, `Sidebar.svelte`, `SearchBar.svelte`, `ThemeToggle.svelte`, `Icon.svelte`. Layout structure mirrors the old `static/index.html`:

```
<div id="app">
  <Header />            <!-- .header: logo link (#logo-link, href "/"), SearchBar, ThemeToggle, .menu-btn (mobile) -->
  <Sidebar />           <!-- .sidebar: nav buttons, .sidebar-overlay -->
  <main class="main-content">   <!-- scroll container: overflow-y: auto — AGENTS.md constraint -->
    <h1 id="current-view-title">…</h1>
    <SortControls />    <!-- <select id="sort-select"> with the 6 sort options -->
    {#if route.view === 'collages'}<CollagesView />
    {:else if route.view === 'housekeeping'}<HousekeepingView />
    {:else}<PhotoGrid />{/if}
  </main>
  <IndexingOrbit />
  <PhotoViewer />
  <ToastContainer />
</div>
```

Selector contract — these ids/classes/attributes MUST exist with the exact names below (from `tests/e2e/setup/test-helpers.js` and spec files; E2E depends on every one):
- Nav: `button[data-view="all|favorites|videos|collages|housekeeping"]`, active item gets class `active`.
- Grid: `#photo-grid` AND class `.photo-grid` on the same container; `.photo-card` per card; `[data-photo-id="<hash>"]`; action buttons `[data-action="favorite|download|keep|delete-housekeeping|accept-collage|reject-collage"]`.
- Load-more: `#load-more-container` with class `.load-more-container`, containing `#load-more-btn` (class `.load-more-btn`, label `{$t('ui.load_more')}`).
- Viewer: `#photo-viewer` (class `active` when open; hidden/removed when closed), `.viewer-overlay`, `.viewer-content`, `.viewer-main`, `.viewer-close`, `.close-viewer`, `.viewer-prev`, `.viewer-next`, `#viewer-image`, `#viewer-video`, `.viewer-controls`, `.zoom-btn`, `.viewer-sidebar`, `.viewer-loading-indicator`, `.favorite-btn`, `.download-btn`, `.metadata-btn`.
- Header/misc: `.header`, `#search-input`, `#search-btn`, `#sort-select`, `#current-view-title`, `.menu-btn`, `.sidebar`, `.sidebar-overlay`.
- Indexing (Phase 6): `[data-phase-ring]`, `[data-ring-mode]`, `[data-bottom-sheet]`, `[data-sheet-close]`, `[data-sheet-photos-count]`, `[data-phase-id]`, `[data-phase-count]`, `[data-phase-fill]`, `[data-phase-errors]`, `[data-sheet-current-item]`.

`Icon.svelte`: wraps `feather-icons` — `import { icons } from 'feather-icons'`; renders `{@html icons[name].toSvg({ width, height })}`. Trusted static icon set, `{@html}` is safe here. Replaces `window.feather`/`iconHelper`.

`ThemeToggle.svelte`: reads `themeState.theme`; toggling sets `document.documentElement.classList` (`light-theme`/`dark-theme`) and persists via `utils.storage.set('theme', theme)` — same behavior as `app.js:initTheme/setTheme`.

`App.svelte` `onMount`: fetch `/api/config` → `initI18n(config.default_locale)` → `router.init()` → set up `window.resize` listener (throttled) driving `appState.isMobile` (< 768px). Navigation clicks call `router.pushState({ view })`; `appState.currentView` derives from `route.view`.

**2g. Smoke test**: `npm run build && cargo build` → header with logo/search/theme toggle, sidebar with 5 nav buttons, view title. Clicking nav updates URL (`/favorites` etc.) and `.active` moves. Theme toggle switches `html` class and survives reload with no flash. German locale: set `TURBO_PIX_LOCALE=de` env → UI renders German strings.

### Phase 3: Photo grid, cards, infinite scroll

**3a. `PhotoCard.svelte`** — props `{ photo, context }` where `context` is `'default' | 'collage' | 'housekeeping'`. Reproduce `static/js/photoCard.js` `create()` DOM exactly: `.photo-card` with `data-photo-id={photo.hash_sha256}`; image container with blurhash canvas (decode `photo.blurhash` → `toDataURL` → shown until `<img>` loads); `<picture>` with WebP `<source>` + JPEG fallback using `getThumbnailUrl(hash, size)` srcset (small/medium/large); overlay with title + meta line (date • camera • size via `formatDate`/`formatFileSize`); `.video-play-icon` for videos (`APP_CONSTANTS.VIDEO_EXTENSIONS` check); action buttons per context (favorite/download; accept/reject for collages; keep/delete for housekeeping). Favorite toggle: optimistic UI flip → `api.addToFavorites`/`removeFromFavorites` → on success update `photoGridState.photos` entry + dispatch `favoriteToggled`. Card click (not on action buttons) → open viewer via callback prop.

**3b. `PhotoGrid.svelte`** — port `static/js/photoGrid.js` logic into the component using `photoGridState`:
- `loadPhotos(query, filters, reset)`: abort previous `AbortController`, call `api.getPhotos` (or `loadSemanticSearch` when `semanticSearchMode`), minimum 300 ms loading display, error → error state with retry button.
- Render `{#each photoGridState.photos as photo (photo.hash_sha256)}<PhotoCard …/>{/each}` — keyed each, no manual DocumentFragment.
- Loading: 6 `.skeleton-item` divs (same markup as `static/index.html`'s skeleton).
- Empty state (port `showEmptyState()` exactly): when `photoGridState.photos.length === 0 && indexingState.isIndexing && !photoGridState.currentQuery` → show the indexing-in-progress message AND keep `#load-more-container` hidden (asserted by `indexing-empty-state.e2e.spec.js`); otherwise "No Photos Found".
- React to `route` changes (`$effect`): view/query/sort/year/month changes → `loadPhotos` with mapped params. Favorites view → favorites filter; Videos view → video type filter.
- Listen for `favoriteToggled` to update card state; `indexingStatusChanged` to re-evaluate empty state.

**3c. Infinite scroll — keep the scroll-listener design (do NOT switch to IntersectionObserver)**

Port `static/js/infiniteScroll.js` behavior into `PhotoGrid.svelte` `onMount`: `scroll` listener on the `.main-content` element, throttled 250 ms, trigger `loadMore()` when `scrollHeight - (scrollTop + clientHeight) <= 800` and `!loading && hasMore`. Keep `#load-more-container` with the exact indicator markup: `.dot-wave` with 3 `.dot-wave-dot` while loading (and photos exist), `.end-dots` with 3 `.end-dot` when `!hasMore` and photos exist, hidden otherwise. `#load-more-btn` click → `loadMore()`. Recheck position after each load via `requestAnimationFrame` + 50 ms timeout (port `recheckAfterLoad`).

**3d. Smoke test**: grid renders cards from the test backend; scrolling near bottom loads the next batch of 50; empty+indexing state shows the indexing message; `#load-more-container` visibility matches the old logic.

### Phase 4: Photo viewer

**4a. `PhotoViewer.svelte`** — port `static/js/viewer.js` `PhotoViewer`: modal `#photo-viewer` (`class:active` when open, `{#if}`-rendered or `display:none` when closed — tests accept both `hidden` and detached). State: `currentPhoto`, `photos[]`, `currentIndex`, `preloadedImages` Map.
- `open(photo, allPhotos)`: set array/index, `document.body.style.overflow = 'hidden'`, View Transitions API entry when supported, `router.replaceState({ photo: hash })`, `displayPhoto`.
- `close(updateUrl = true)`: clear preload Map, restore overflow, `router.replaceState({ photo: null })` unless `updateUrl === false`.
- `displayPhoto(photo)`: loading indicator → `isVideoFile()` branch. Images: check `preloadedImages`, else `new Image()` with onload; blurhash shown while loading. Videos: port the HEVC/`videoCodecSupport` check and transcoding-status polling (2 s interval, 5 min timeout) verbatim.
- `preloadAdjacentPhotos()`: preload index ±1 after each display.
- Keyboard via `<svelte:window on:keydown>`: Escape/ArrowLeft/ArrowRight/Space/`f`/`d` — same mapping as `setupKeyboardNavigation()`.
- Prev/next buttons; `updateNavigation()` hides at boundaries. Haptic `navigator.vibrate` on heavy actions. URL stays in sync on every navigation (`?photo=<hash>`).
- Deep-link: on app init, if `route.photo` set → `api.getPhoto(hash)` → open (port `openByHash`).

**4b. Gestures — port the existing classes, integrate via a Svelte action**

Copy `static/js/gestureRecognizers.js` → `frontend/src/lib/gestures/recognizers.js` and `static/js/gestureManager.js` → `frontend/src/lib/gestures/GestureManager.js` (mechanical ESM export changes only; touch math stays identical). Create `frontend/src/lib/gestures/action.js`:
```js
import { GestureManager } from './GestureManager.js';
export function gestures(node, handlers) {
  const manager = new GestureManager(node, { enablePinch: true, enableSwipe: true, enableDoubleTap: true, enablePan: true });
  for (const [event, cb] of Object.entries(handlers)) manager.on(event, cb);
  manager.init();
  return { destroy: () => manager.destroy() };
}
```
Apply `use:gestures` on `.viewer-main`. Port `SwipeableViewer` (from `static/js/viewer.js`) into `frontend/src/lib/viewer/SwipeableViewer.js` unchanged in behavior — rubber-banding (0.3 factor at boundaries), 30 %-of-viewport or velocity > 0.3 navigation threshold, adjacent-image rendering with `requestAnimationFrame`. It manipulates elements the component provides; wire via callback props (`onNavigate(direction)`, `onDismiss()`).

**4c. `ViewerControls.svelte` + `ViewerMetadata.svelte`**

Port `static/js/viewerControls.js`: zoom in/out/fit buttons (`.zoom-btn`), CSS-transform zoom state, mouse-drag pan when zoomed with boundary checks, pinch zoom via gesture callbacks, double-tap zoom (300 ms ease-out cubic animation), momentum pan (0.95 friction), fullscreen toggle, zoom button disabled states. Port `static/js/viewerMetadata.js`: sidebar sections (basic info, camera, settings, location, video) with the same field ids (`#photo-title`, `#photo-date`, `#photo-size`, `#photo-camera`, `#photo-location`, …), favorite button (`.favorite-btn`) state sync, show/hide sidebar toggle. Favorite in viewer dispatches `favoriteToggled` and updates its own button. Rotate (`api.rotatePhoto`), delete (`api.deletePhoto` with `confirm()`), download (temporary `<a download>`) — port as-is. Collage photos show accept/reject (`data-action="accept-collage"`).

**4d. `ViewerMetadataEdit.svelte`** — port `static/js/viewerMetadataEdit.js`: modal `#metadata-edit-modal`, form fields for title/description/taken_at, same validation, `api.updatePhotoMetadata(hash, updates)` PATCH, success → update displayed metadata + toast.

**4e. Smoke test**: open from card click → `#photo-viewer.active` attached; arrows/swipe navigate with URL updates; pinch/double-tap zoom; sidebar shows metadata; Escape closes and restores scroll position; direct URL `/?photo=<hash>` opens the viewer on load.

### Phase 5: Search, timeline, view switching

**5a. Search in `SearchBar.svelte`** — port `static/js/search.js`: 300 ms debounced input; Enter/button → `router.pushState({ query })`; semantic search by default — strip optional `@` prefix, `api.semanticSearch(query, batchSize, offset)` → fetch full photos per hash → display; prefix queries (`type:`, `location:`, `is_favorite:`) → regular `api.getPhotos` filter path (port `parseSearchQuery`/`buildSearchFilters`); Escape → clear (`router.pushState({ query: null })`, reset `semanticSearchMode`, reload unfiltered grid). Suggestions dropdown `#search-suggestions` + `.search-hint`: recent searches from `searchHistory` storage + quick filters (camera, GPS, video), max 8. View title `#current-view-title` shows `{$t('ui.search_results', { values: { query } })}` while searching. Update `searchHistory` via `utils.storage` (max 20, most-recent-first — same as `api.addToSearchHistory`).

**5b. `TimelineSlider.svelte`** — port `static/js/timeline.js`: fetch `/api/photos/timeline`; canvas heatmap (bar per month, density-based opacity 0.3–1.0, hover highlight, selected stroke glow, year markers via `drawYearMarkers`); range slider input (300 ms debounce) mapping index → year/month; `#timeline-year-select`/`#timeline-month-select` dropdowns (months from `APP_CONSTANTS.MONTH_KEYS`); reset buttons (`.timeline-reset`) and slider double-click reset; `applyFilter(updateUrl = true)` → `router.pushState({ year, month })` — never writes `month` without `year` (guarded in `buildUrl`); `setFilterFromState(year, month)` for popstate restores without URL push.

**5c. View switching**: `route.view` drives which component renders (Phase 2f). Favorites: `getPhotos` with favorites filter (same params as `api.getFavoritePhotos` uses today); Videos: video-extension filter. Sort: `#sort-select` change → `router.pushState({ sort })` → grid reloads. View title text per view via the `titleKeys` mapping in `app.js` (`ui.all_photos`, `ui.favorites`, …).

**5d. Smoke test**: `?q=sunset` in URL on load → semantic results; timeline click filters grid and sets `?year=…&month=…`; `/favorites` shows only favorited; sort select reorders.

### Phase 6: Collages, housekeeping, indexing UI

**6a. `CollagesView.svelte`** — port `static/js/collages.js`: `api.getPendingCollages()` on mount; large preview cards with accept (`api.acceptCollage`) / reject (`confirm()` → `api.rejectCollage`) buttons (`data-action` attrs); remove acted card from list; "generate" button → `api.generateCollages()` → refresh; empty state text; dispatch `collageAccepted`/`collageRejected`. Clicking a collage image opens `PhotoViewer` with the collage photo (accept/reject available in viewer too).

**6b. `HousekeepingView.svelte`** — port `static/js/housekeeping.js`: `api.getHousekeepingCandidates()`; cards with keep (`api.removeHousekeepingCandidate`) / delete (`confirm()` → `api.deletePhoto`); remove from list on action; dispatch `housekeepingCandidateRemoved`.

**6c. `IndexingOrbit.svelte`** — port `static/js/indexingOrbit.js` `IndexingOrbitManager` fully:
- Poll `api.getIndexingStatus()`: 1 s while indexing, 30 s idle; normalize status (port `normalizeStatus`); write to `indexingState`; dispatch `indexingStatusChanged` CustomEvent (detail = normalized status).
- `onMount`: assign `window.indexingStatus = { checkStatus: this.checkStatus, isIndexing: … }` — REQUIRED by `indexing-orbit.e2e.spec.js` (`await window.indexingStatus.checkStatus()`).
- SVG orbit: `viewBox="0 0 280 280"`, 6 phase segments (60° arc, 4° gap, radius 120) in the exact phase order `discovering, metadata, semantic_vectors, geo_resolution, collages, housekeeping` with their feather icons; port `describeArc`/`polarToCartesian`; determinate phases via stroke-dashoffset progress; indeterminate phases via animated orbit dot (`.orbit-dot`); `data-ring-mode` = `large | compact | hidden` per `determineMode()`; completion pulse → hide after 2 s; click ring toggles sheet; `prefers-reduced-motion` respected.
- First-visit behavior: `turbopix_has_indexed` localStorage gate — auto-open sheet on first indexing, set key after completion (port `hasIndexedBefore`/`markIndexingCompleted` verbatim).

**6d. Indexing bottom sheet** (implemented inside `IndexingOrbit.svelte` — no separate `IndexingSheet.svelte` file) — `[data-bottom-sheet]` container: per-phase rows `[data-phase-id]` with `[data-phase-fill]` progress bars (determinate width %, indeterminate animation), `[data-phase-count]`, `[data-phase-errors]`; `[data-sheet-current-item]`; `[data-sheet-photos-count]`; `[data-sheet-close]` button; Escape closes; backdrop element; `aria-hidden`/`aria-expanded` managed as in the source.

**6e. Smoke test**: with the backend indexing, ring segments animate and sheet shows live progress; `window.indexingStatus.checkStatus()` resolves in devtools; completion hides the ring after pulse.

### Phase 7: Responsive layout

- Breakpoints via the `appState.isMobile` listener (768 px) plus CSS media queries at 768/1024/1200 — port all rules from `static/css/responsive.css` into the relevant components' `<style>` blocks.
- Mobile sidebar: `.sidebar` off-canvas transform + `.sidebar-overlay` backdrop; `.menu-btn` in header toggles; close on backdrop click / Escape / nav click.
- Mobile search: search icon in header opens a full-screen `.mobile-search` overlay with autofocus; close on submit/Escape.
- Mobile viewer metadata: `.viewer-sidebar` becomes a bottom sheet with a drag handle; port the touch-drag translateY behavior from `responsive.css`+viewer code; desktop keeps right sidebar. Preserve asserted computed styles: `.viewer-sidebar` `z-index: 15` on mobile, above `.viewer-controls` (asserted by `viewer-mobile-sidebar.e2e.spec.js`).
- Smoke test: 375×667 viewport — hamburger opens sidebar, search overlay works, viewer metadata sheet drags; 1920×1080 unchanged.

### Phase 8: CSS migration (executed inside Phases 2–7, token file first)

- `frontend/src/app.css` (global, imported in `main.js`): copy VERBATIM from `static/css/main.css` — `@font-face` rules, the `@layer tokens` block (`:root`, `html.light-theme`, `html.dark-theme` custom properties), `@layer base` (box-sizing, scrollbar, body, noise-texture `body::after`). Keep `@layer` declarations so cascade order is unchanged.
- Per component: move that component's rules from `main.css`/`components.css`/`responsive.css` into its `<style>` block, copying declarations VERBATIM (values, transitions, shadows, z-indexes — glassmorphism and mobile-sidebar tests assert computed styles like `backdrop-filter` containing `saturate` and `z-index: 15`). Replace hardcoded values with `var(--token)` only where the source already uses tokens. Use `:global()` only for selectors that must escape scoping (e.g., `html.dark-theme`, `body`).
- Glassmorphism: `.header`, `.sidebar`, `[data-phase-ring]`, viewer controls/buttons keep `position: fixed`/absolute + `backdrop-filter: blur(16px) saturate(1.5)` with `-webkit-` prefix, content scrolling beneath (AGENTS.md constraint).
- View Transitions: viewer open/close wrapped in `document.startViewTransition()` when available; `@supports` guard; `view-transition-name` on the active image.

### Phase 9: E2E gate

1. Kill any stale dev server with the narrow pattern first (AGENTS.md: a broad `pkill -9 -f turbo-pix` also kills the Playwright runner itself, since its argv contains the repo path via node_modules):
   `pkill -9 -f 'target/(debug|release)/turbo-pix'`
2. `npm run test:e2e` from repo root. Global setup now runs `npm run build` + `cargo build` + `cargo run` with `TURBO_PIX_DATA_PATH=test-e2e-data` etc. (unchanged from current `global-setup.js`).
3. Fix failures by aligning Svelte output with the selector contract (Phase 2f) and behavior — never by weakening test assertions. Selector updates inside tests are allowed only where the old markup itself changed shape (should be none if the contract holds).
4. If `TestHelpers` needs changes: it operates on selectors/URLs only (verified — the only `window.*` app global tests touch is `window.indexingStatus`, preserved in Phase 6c), so no helper rewrite should be needed.

### Phase 10: Remove the old frontend (last, after E2E is green)

1. Move backend collage fonts out of `static/`: `static/fonts/Questrial-Regular.ttf` and `static/fonts/JetBrainsMono-Regular.ttf` → new repo-root `assets/fonts/`. Update `src/collage_generator.rs` `load_font()` (currently lines ~894–901): `include_bytes!("../static/fonts/Questrial-Regular.ttf")` → `include_bytes!("../assets/fonts/Questrial-Regular.ttf")`, same for JetBrainsMono. These fonts render collage date labels server-side — deleting them breaks collage generation, not the frontend.
2. Delete the entire `static/` directory (all JS, CSS, i18n, index.html, remaining fonts). Nothing else references it: verified by grep — the only `../static` references in `src/` are `handlers_static.rs` (rewritten in Phase 1) and `collage_generator.rs` (fixed in step 1).
3. `cargo clippy` and `cargo test` must pass with zero warnings.
4. Root `eslint.config` / stylelint config: add `eslint-plugin-svelte` + `prettier-plugin-svelte` setup for `.svelte` files (drop stylelint for `.svelte` scoped styles; keep it for `frontend/src/app.css` only).

## Critical files & anchors

- `src/handlers_static.rs` — macro-defined `STATIC_FILES`/`STATIC_BINARY_FILES` and the SPA-fallback prefix list: replaced by `build.rs`-generated arrays; fallback prefixes change to `/api/`, `/assets/`, `/favicon`, `/site.webmanifest`, `/fonts/`.
- `build.rs` (new) — globs `dist/`, generates `embedded_static.rs` into `OUT_DIR`, panics with "run `npm run build` first" when `dist/index.html` is missing; the whole Rust↔Vite contract lives here.
- `src/collage_generator.rs` `load_font()` (~:894) — backend-only TTF fonts embedded from `../static/fonts/`; must move to `assets/fonts/` BEFORE `static/` is deleted.
- `static/js/viewer.js` — `PhotoViewer` + `SwipeableViewer`: the behavioral source of truth for the viewer port (gesture thresholds, preload ±1, URL sync, keyboard map, transcoding poll).
- `tests/e2e/setup/global-setup.js` + `tests/e2e/setup/test-helpers.js` — `buildBinary()` needs the `npm run build` prepend; `TestHelpers.selectors` is the authoritative DOM selector contract.

## Verification

Per-phase smoke tests are listed at each phase's end. Final end-to-end proof (from repo root):

```bash
npm run build && cargo build --bin turbo-pix        # tree builds
pkill -9 -f 'target/(debug|release)/turbo-pix'; npm run test:e2e   # full Playwright suite green
```

Manual new-behavior checks (server: `TURBO_PIX_DATA_PATH=/tmp/tp TURBO_PIX_PHOTO_PATHS=<photos> TURBO_PIX_PORT=18473 ./target/debug/turbo-pix`, open `http://localhost:18473`):
1. **Svelte app serves**: `view-source` shows `<div id="app">` + `/assets/index.js`; grid renders photo cards with `[data-photo-id]` — proves Rust embeds Vite output.
2. **Viewer deep-link**: open `/?photo=<hash>` directly → viewer opens on that photo — proves router + viewer wiring.
3. **Search round-trip**: type `receipt`, Enter → URL gains `?q=receipt`, grid shows semantic results — proves search path.
4. **Indexing test hook**: devtools `await window.indexingStatus.checkStatus()` resolves and fires `indexingStatusChanged` — proves the E2E-required global.
5. **Theme persistence**: toggle dark → reload → dark without flash; `localStorage.theme` is `"dark"` (JSON) — proves FOUC script + storage encoding.
6. **Locale switch**: `TURBO_PIX_LOCALE=de` restart → nav shows "Favoriten"/"Videos" etc.; switch back via UI persists `turbo-pix-locale`.
7. **Mobile**: 375×667 — hamburger sidebar, search overlay, viewer bottom sheet.

## Assumptions & contingencies

- **Vite chunk names are unpredictable** — that is why embedding goes through `build.rs` globbing, not a file list. If the codegen approach hits an unforeseen blocker (e.g., path escaping on exotic filenames), fall back to the `rust-embed` crate: `#[derive(RustEmbed)] #[folder = "dist/"]` and rewrite `build_static_routes()` to iterate `Assets::iter()`.
- **svelte-i18n on Svelte 5**: store-based API (`$_`, `$locale`) is supported by Svelte 5's store compatibility, and the two-level nested dictionaries match its dot-path lookup. If it misbehaves at runtime, the spec explicitly allows an equivalent: replace it with a ~50-line `lib/i18n.svelte.js` holding `let currentLocale = $state('en')` + dictionaries + a `t(key, params)` doing the same lookup/`{param}` interpolation; templates keep calling `t(...)`.
- **Locale resolution order** (localStorage → config → browser language → `en`): the current vanilla code ignores `turbo-pix-locale` at startup (verified — `initializeI18n` only uses the config value). The spec (FR-015) mandates persistence; the browser-language fallback is a user decision (config wins over browser). The new behavior is deliberate, not a drift.
- **`static/js/i18n.js` is dead code** (embedded but never loaded by `index.html` — verified) and is not ported. If a hidden consumer surfaces during E2E, port it as `lib/i18n-legacy.js` instead of reviving `static/`.
- **Build order**: `npm run build` must precede `cargo build`; `build.rs` enforces this with a clear panic. `cargo:rerun-if-changed` per dist file keeps cargo rebuilds correct when the frontend rebuilds.
