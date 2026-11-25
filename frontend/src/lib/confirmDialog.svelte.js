import { get } from 'svelte/store';
import { t } from './i18n.js';

/**
 * Promise-based confirmation bus. A single `<ConfirmDialog>` mounted in
 * App.svelte renders the request; every caller awaits the answer. Replaces
 * `window.confirm()`, which is blocking browser chrome outside the design
 * language and cannot mark the destructive choice.
 */
export const confirmState = $state({
  open: false,
  title: '',
  message: '',
  confirmLabel: '',
  danger: true,
  slide: false,
  onConfirm: null,
  requestId: 0,
});

let resolveFn = null;

/**
 * @param {string} message
 * @param {{
 *   title?: string,
 *   confirmLabel?: string,
 *   danger?: boolean,
 *   slide?: { onConfirm?: () => Promise<void> | void },
 * }} [options]
 * @returns {Promise<boolean>} true when confirmed, false on cancel/Escape
 */
export function askConfirm(message, options = {}) {
  // A second request supersedes the first: the previous caller resolves false.
  if (resolveFn) resolveFn(false);
  confirmState.requestId += 1;
  confirmState.title = options.title ?? '';
  confirmState.message = message;
  confirmState.confirmLabel = options.confirmLabel ?? get(t)('ui.delete', { default: 'Delete' });
  confirmState.danger = options.danger ?? true;
  confirmState.slide = Boolean(options.slide);
  confirmState.onConfirm = options.slide?.onConfirm ?? null;
  confirmState.open = true;
  return new Promise((resolve) => {
    resolveFn = resolve;
  });
}

export function resolveConfirm(answer) {
  const fn = resolveFn;
  resolveFn = null;
  confirmState.open = false;
  // The destructive callback belongs to this request only; a closed dialog
  // must not be able to replay it from a stale pointer release.
  confirmState.onConfirm = null;
  confirmState.title = '';
  fn?.(answer);
}
