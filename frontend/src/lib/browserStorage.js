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
