/**
 * The desktop sidebar preference. Storage is injected so the module stays
 * node-testable: `frontend/src/lib/utils.js` pulls in JSON dictionaries and
 * cannot be imported by `node --test`.
 */
export const SIDEBAR_COLLAPSED_STORAGE_KEY = 'turbopix_sidebar_collapsed';

/** Only the written marker counts as collapsed; anything else is expanded. */
export const readSidebarCollapsed = (storage) => {
  try {
    return storage?.getItem(SIDEBAR_COLLAPSED_STORAGE_KEY) === 'true';
  } catch {
    return false;
  }
};

/** A denied write keeps the in-memory choice; it must never throw. */
export const writeSidebarCollapsed = (storage, collapsed) => {
  try {
    storage?.setItem(SIDEBAR_COLLAPSED_STORAGE_KEY, String(collapsed));
  } catch {
    // Storage unavailable or full: nothing to remember.
  }
};

/**
 * The real browser store, or null where it is unavailable. Some browsers
 * throw on the accessor itself (sandboxed/blocked storage), so `typeof` alone
 * is not enough.
 */
export const browserStorage = () => {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
};
