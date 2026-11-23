/**
 * The desktop timeline card preference. Storage is injected so the module
 * stays node-testable, mirroring `sidebarPreference.js`.
 */
export const TIMELINE_COLLAPSED_STORAGE_KEY = 'turbopix_timeline_collapsed';

/** Only the written marker counts as collapsed; anything else is expanded. */
export const readTimelineCollapsed = (storage) => {
  try {
    return storage?.getItem(TIMELINE_COLLAPSED_STORAGE_KEY) === 'true';
  } catch {
    return false;
  }
};

/** A denied write keeps the in-memory choice; it must never throw. */
export const writeTimelineCollapsed = (storage, collapsed) => {
  try {
    storage?.setItem(TIMELINE_COLLAPSED_STORAGE_KEY, String(collapsed));
  } catch {
    // Storage unavailable or full: nothing to remember.
  }
};
