import { api } from './api.js';

export const appState = $state({
  sidebarOpen: false,
  mobileSearchOpen: false,
});

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
  busy: null, // 'delete' | 'keep' | 'favorite' | 'unfavorite' | 'dateShift' | 'export' | 'accept' | 'reject' | null
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

export const savedSearches = $state([]);

export async function loadSavedSearches() {
  try {
    const data = await api.getSavedSearches();
    savedSearches.splice(0, savedSearches.length, ...(data?.saved_searches || []));
  } catch (error) {
    console.error('Failed to load saved searches', error);
  }
}

export const albums = $state([]);

export async function loadAlbums() {
  try {
    const data = await api.getAlbums();
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

export function addToast(title, message = '', type = 'info', duration = 4000) {
  const id = ++toastId;
  toasts.push({ id, title, message, type, leaving: false });
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
