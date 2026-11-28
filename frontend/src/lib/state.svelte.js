import { api } from './api.js';
import { browserStorage } from './browserStorage.js';
import { readSidebarCollapsed, writeSidebarCollapsed } from './sidebarPreference.js';
import { readTimelineCollapsed, writeTimelineCollapsed } from './timelinePreference.js';

export const appState = $state({
  sidebarOpen: false,
  // Desktop-only: the mobile drawer keeps its own `sidebarOpen` lifecycle.
  sidebarCollapsed: readSidebarCollapsed(browserStorage()),
  // Desktop-only fold of the timeline card; presentational, survives remounts.
  timelineCollapsed: readTimelineCollapsed(browserStorage()),
  mobileSearchOpen: false,
  // Current-date hint: the rail caption pauses while it is visible, while the
  // rail bubble and the top loading indicator pause the hint in return.
  dateHintVisible: false,
  dateRailBubbleVisible: false,
  dateRailLoading: false,
  // Raster tile endpoint served by GET /api/config (TURBO_PIX_TILE_URL).
  // null = not configured → the map renders without a tile layer.
  tileUrl: null,
  // GET /api/config: false hides the People nav and turns a deep-linked
  // People route back into All Photos.
  faceRecognitionEnabled: true,
});

/**
 * Applies the desktop sidebar choice and remembers it. A denied write must
 * not discard the state the user just chose.
 */
export function setSidebarCollapsed(collapsed) {
  appState.sidebarCollapsed = collapsed;
  writeSidebarCollapsed(browserStorage(), collapsed);
}

/**
 * Applies the desktop timeline fold and remembers it. A denied write must not
 * discard the state the user just chose.
 */
export function setTimelineCollapsed(collapsed) {
  appState.timelineCollapsed = collapsed;
  writeTimelineCollapsed(browserStorage(), collapsed);
}

export const indexingState = $state({
  isIndexing: false,
  currentPhase: null,
  phases: [],
  photosIndexed: 0,
  currentItem: '',
});

export const photoGridState = $state({
  photos: [],
  currentPage: 1,
  loading: false,
  hasMore: true,
  currentQuery: '',
  semanticSearchMode: false,
  // Bumped whenever the date-scrubber window replaces the listing or resets
  // `currentPage`; the grid's load dedupe includes it so re-anchoring can never
  // be swallowed by a previous signature.
  windowEpoch: 0,
  // A date-scrubber jump or earlier load is in flight; the grid mirrors this
  // into aria-busy so the jump is announced as busy.
  windowBusy: false,
  // Exact result count of the current regular load; null while it is unknown
  // (semantic search and album pages do not report one).
  total: null,
});

/**
 * Per-surface selection mode state. `selected` is a plain-object map (key →
 * true) rather than a Set: Svelte 5 runes instrument `Object.keys`/`delete`/
 * index-assign reliably, while Set `has()` tracking is not worth the doubt.
 * Keys are photo `hash_sha256` strings, or `String(collage.id)` on the
 * collages surface. `orderedKeys` is the current surface's visible keys in
 * display order (maintained by each view) — the range-selection and
 * select-all-visible contract.
 */
export const selectionState = $state({
  active: false,
  selected: {},
  anchorKey: null,
  orderedKeys: [],
  busy: null, // 'delete' | 'keep' | 'favorite' | 'unfavorite' | 'export' | 'accept' | 'reject' | null
});

export function enterSelectionMode() {
  selectionState.active = true;
}

export function exitSelectionMode() {
  selectionState.active = false;
  selectionState.selected = {};
  selectionState.anchorKey = null;
}

export function toggleSelected(key) {
  if (selectionState.selected[key]) delete selectionState.selected[key];
  else selectionState.selected[key] = true;
  selectionState.anchorKey = key;
}

export function selectRange(anchorKey, targetKey, orderedKeys) {
  const a = orderedKeys.indexOf(anchorKey);
  const b = orderedKeys.indexOf(targetKey);
  if (a === -1 || b === -1) {
    toggleSelected(targetKey);
    return;
  }
  const [from, to] = a <= b ? [a, b] : [b, a];
  for (let i = from; i <= to; i++) selectionState.selected[orderedKeys[i]] = true;
  selectionState.anchorKey = targetKey;
}

export function selectAllVisible() {
  const keys = selectionState.orderedKeys;
  const allSelected = keys.length > 0 && keys.every((k) => selectionState.selected[k]);
  keys.forEach((k) => {
    if (allSelected) delete selectionState.selected[k];
    else selectionState.selected[k] = true;
  });
  if (!allSelected) selectionState.anchorKey = keys[keys.length - 1] ?? null;
}

/** Drop selected keys that are no longer part of the surface's loaded set. */
export function pruneSelection(keys) {
  const keep = {};
  for (const k of keys) keep[k] = true;
  for (const k of Object.keys(selectionState.selected)) {
    if (!keep[k]) delete selectionState.selected[k];
  }
  if (selectionState.anchorKey != null && !keep[selectionState.anchorKey]) {
    selectionState.anchorKey = null;
  }
}

export const people = $state([]);
/** The merge that can still be undone; `null` once consumed or replaced. */
export const peopleMerge = $state({ slot: null });
let peopleLoadToken = 0;

export async function loadPeople(includeHidden = false) {
  const token = ++peopleLoadToken;
  try {
    const data = await api.getPeople({ includeHidden });
    if (token !== peopleLoadToken) return true;
    people.splice(0, people.length, ...(data?.people || []));
    peopleMerge.slot = data?.last_merge ?? null;
    return true;
  } catch (error) {
    // A superseded request that fails must not surface its error over the
    // fresher list a newer request already applied.
    if (token !== peopleLoadToken) return true;
    console.error('Failed to load people', error);
    return false;
  }
}

export const albums = $state([]);

// Monotonic load token: App, AlbumsView, and AlbumPicker can all have a
// GET /api/albums in flight at once, and a slow (stale) response must not
// splice over a newer list — or over a create/rename/delete that landed
// meanwhile (the mutating call sites reconcile with a fresh loadAlbums
// below, so dropping the stale snapshot converges on server truth).
let albumsLoadToken = 0;

export async function loadAlbums() {
  const token = ++albumsLoadToken;
  try {
    const data = await api.getAlbums();
    if (token !== albumsLoadToken) return;
    albums.splice(0, albums.length, ...(data?.albums || []));
  } catch (error) {
    console.error('Failed to load albums', error);
  }
}

export const themeState = $state({
  theme: document.documentElement.classList.contains('dark-theme') ? 'dark' : 'light',
});

export const toasts = $state([]);

let toastId = 0;
const TOAST_LEAVE_MS = 300;

export function addToast(title, message = '', type = 'info', duration = 4000, action = null) {
  const id = ++toastId;
  toasts.push({ id, title, message, type, leaving: false, action });
  if (duration > 0) {
    setTimeout(() => {
      removeToast(id);
    }, duration);
  }
}

export function removeToast(id) {
  const idx = toasts.findIndex((t) => t.id === id);
  if (idx === -1 || toasts[idx].leaving) return;
  // Play the leave animation, then actually remove.
  toasts[idx].leaving = true;
  setTimeout(() => {
    const i = toasts.findIndex((t) => t.id === id);
    if (i !== -1) toasts.splice(i, 1);
  }, TOAST_LEAVE_MS);
}
