/**
 * Svelte action: touch long-press detection for entering selection mode.
 *
 * Only `pointerType === 'touch'` long-presses trigger (mouse/pen keep their
 * native behaviors). While a long-press is armed the browser's context menu
 * is suppressed (one-shot `contextmenu` listener) and the click that follows
 * the long-press is swallowed (capture-phase click listener) so it cannot
 * also open the viewer or toggle the card twice.
 */
export function longpress(node, { onLongPress, delay = 500, threshold = 10 }) {
  let timer = null;
  let startX = 0;
  let startY = 0;
  let suppressClick = false;

  function clearTimer() {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  }

  function suppressContextMenu(e) {
    e.preventDefault();
  }

  function disarm() {
    clearTimer();
    if (node.__longpressContextMenuBound) {
      node.removeEventListener('contextmenu', suppressContextMenu);
      node.__longpressContextMenuBound = false;
    }
  }

  function onPointerDown(e) {
    if (e.pointerType !== 'touch') return;
    // A new gesture always clears a stale suppression flag from a previous
    // long-press whose click never arrived (finger released outside).
    suppressClick = false;
    disarm();
    startX = e.clientX;
    startY = e.clientY;
    timer = setTimeout(() => {
      timer = null;
      suppressClick = true;
      node.addEventListener('contextmenu', suppressContextMenu);
      node.__longpressContextMenuBound = true;
      onLongPress?.();
    }, delay);
  }

  function onPointerMove(e) {
    // Only a pending (not yet fired) press is a scroll gesture candidate;
    // once armed, small finger jitter must not cancel the long-press.
    if (timer === null) return;
    const dx = e.clientX - startX;
    const dy = e.clientY - startY;
    if (Math.hypot(dx, dy) > threshold) disarm();
  }

  function onPointerUp(e) {
    if (e.pointerType !== 'touch') return;
    disarm();
    // suppressClick stays set: the click after a fired long-press must be
    // swallowed. If no long-press fired, the flag is false and clicks pass.
  }

  function onPointerCancel(e) {
    if (e.pointerType !== 'touch') return;
    disarm();
    suppressClick = false;
  }

  function onClick(e) {
    if (!suppressClick) return;
    e.stopPropagation();
    e.preventDefault();
    suppressClick = false;
    disarm();
  }

  node.addEventListener('pointerdown', onPointerDown);
  node.addEventListener('pointermove', onPointerMove);
  node.addEventListener('pointerup', onPointerUp);
  node.addEventListener('pointercancel', onPointerCancel);
  node.addEventListener('click', onClick, { capture: true });

  return {
    update(newOptions) {
      onLongPress = newOptions?.onLongPress;
      delay = newOptions?.delay ?? 500;
      threshold = newOptions?.threshold ?? 10;
    },
    destroy() {
      disarm();
      suppressClick = false;
      node.removeEventListener('pointerdown', onPointerDown);
      node.removeEventListener('pointermove', onPointerMove);
      node.removeEventListener('pointerup', onPointerUp);
      node.removeEventListener('pointercancel', onPointerCancel);
      node.removeEventListener('click', onClick, { capture: true });
      node.__longpressContextMenuBound = false;
    },
  };
}
