/**
 * Svelte action: touch long-press detection for entering selection mode.
 *
 * Only `pointerType === 'touch'` long-presses trigger (mouse/pen keep their
 * native behaviors). While a long-press is armed the browser's context menu
 * is suppressed (one-shot `contextmenu` listener) and the click that follows
 * the long-press is swallowed (capture-phase click listener) so it cannot
 * also open the viewer or toggle the card twice.
 *
 * When `onRangeMove` is supplied, the same finger can drag across cards after
 * the press fires. The pointer is tracked at document level because it leaves
 * the origin card immediately; a nearest scroll container follows the finger
 * near its top/bottom edge so a range larger than one viewport stays
 * selectable. Every card exposes `data-select-key`, which is the surface's
 * stable key (`hash_sha256` for photos, a collage id as a string otherwise).
 */

const RANGE_EDGE_PX = 48;
const RANGE_SCROLL_PX = 8;
const RANGE_CLICK_GUARD_MS = 1000;
const RANGE_RELEASE_GUARD_MS = 350;

export function longpress(node, options) {
  let onLongPress = options?.onLongPress;
  let onRangeMove = options?.onRangeMove;
  let onRangeEnd = options?.onRangeEnd;
  let delay = options?.delay ?? 500;
  let threshold = options?.threshold ?? 10;

  let timer = null;
  let startX = 0;
  let startY = 0;
  let suppressClick = false;

  let rangeActive = false;
  let rangePointer = null;
  let rangeScrollEl = null;
  let rangeScrollFrame = null;
  let rangeScrollDirection = 0;
  let lastRangeKey = null;

  let clickGuardHandler = null;
  let clickGuardTimer = null;
  let clickGuardUntil = 0;

  function clearTimer() {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  }

  function suppressContextMenu(e) {
    e.preventDefault();
  }

  function findScrollParent(element) {
    let parent = element?.parentElement;
    while (parent) {
      const style = getComputedStyle(parent);
      if (/(auto|scroll)/.test(style.overflowY) && parent.scrollHeight > parent.clientHeight) {
        return parent;
      }
      parent = parent.parentElement;
    }
    return document.scrollingElement;
  }

  /**
   * A click can land on a different card than the origin when the finger is
   * released after crossing the grid. The origin card's capture listener
   * never sees that click, so the whole window has to reject it for the short
   * tail of the gesture.
   */
  function armGlobalClickGuard(duration) {
    clickGuardUntil = performance.now() + duration;
    if (clickGuardHandler === null) {
      clickGuardHandler = (event) => {
        if (performance.now() >= clickGuardUntil) return;
        event.preventDefault();
        event.stopPropagation();
      };
      window.addEventListener('click', clickGuardHandler, true);
    }
    clearTimeout(clickGuardTimer);
    clickGuardTimer = setTimeout(() => {
      clickGuardTimer = null;
      disarmGlobalClickGuard();
    }, duration + 50);
  }

  function disarmGlobalClickGuard() {
    if (clickGuardTimer !== null) {
      clearTimeout(clickGuardTimer);
      clickGuardTimer = null;
    }
    if (clickGuardHandler !== null) {
      window.removeEventListener('click', clickGuardHandler, true);
      clickGuardHandler = null;
    }
  }

  function updateRangeAt(x, y) {
    const element = document.elementFromPoint(x, y);
    const card = element?.closest?.('[data-select-key]');
    const key = card?.dataset?.selectKey;
    if (!key || key === lastRangeKey) return;
    lastRangeKey = key;
    onRangeMove?.(key);
  }

  function scrollRect() {
    if (!rangeScrollEl) return null;
    if (rangeScrollEl === document.scrollingElement || rangeScrollEl === document.documentElement) {
      return { top: 0, bottom: window.innerHeight };
    }
    return rangeScrollEl.getBoundingClientRect();
  }

  function updateScrollDirection() {
    if (!rangeActive || !rangeScrollEl || !rangePointer) return;
    const rect = scrollRect();
    if (!rect) return;

    let direction = 0;
    if (rangePointer.y < rect.top + RANGE_EDGE_PX) direction = -1;
    else if (rangePointer.y > rect.bottom - RANGE_EDGE_PX) direction = 1;
    rangeScrollDirection = direction;

    if (direction !== 0 && rangeScrollFrame === null) {
      rangeScrollFrame = requestAnimationFrame(autoScrollStep);
    }
  }

  function autoScrollStep() {
    rangeScrollFrame = null;
    if (!rangeActive || rangeScrollDirection === 0 || !rangeScrollEl || !rangePointer) return;

    rangeScrollEl.scrollTop += rangeScrollDirection * RANGE_SCROLL_PX;
    updateRangeAt(rangePointer.x, rangePointer.y);
    rangeScrollFrame = requestAnimationFrame(autoScrollStep);
  }

  function onRangePointerMove(e) {
    if (!rangeActive) return;
    rangePointer = { x: e.clientX, y: e.clientY };
    updateRangeAt(e.clientX, e.clientY);
    updateScrollDirection();
  }

  function onRangeTouchMove(e) {
    if (!rangeActive) return;
    if (e.cancelable) e.preventDefault();
    const touch = e.touches[0];
    if (!touch) return;
    rangePointer = { x: touch.clientX, y: touch.clientY };
    updateRangeAt(touch.clientX, touch.clientY);
    updateScrollDirection();
  }

  function finishRangeDrag() {
    if (!rangeActive) return;
    rangeActive = false;
    rangePointer = null;
    lastRangeKey = null;
    rangeScrollDirection = 0;
    if (rangeScrollFrame !== null) {
      cancelAnimationFrame(rangeScrollFrame);
      rangeScrollFrame = null;
    }

    window.removeEventListener('pointermove', onRangePointerMove);
    window.removeEventListener('pointerup', onRangePointerUp, true);
    window.removeEventListener('pointercancel', onRangePointerCancel, true);
    window.removeEventListener('touchmove', onRangeTouchMove);
    window.removeEventListener('touchend', onRangeTouchEnd, true);
    window.removeEventListener('touchcancel', onRangeTouchCancel, true);

    onRangeEnd?.();
    // The click the browser synthesizes after the release is the last event
    // of the gesture; keep the guard through that window only.
    armGlobalClickGuard(RANGE_RELEASE_GUARD_MS);
  }

  function onRangePointerUp() {
    finishRangeDrag();
  }

  function onRangePointerCancel() {
    finishRangeDrag();
  }

  function onRangeTouchEnd() {
    finishRangeDrag();
  }

  function onRangeTouchCancel() {
    finishRangeDrag();
  }

  function startRangeDrag() {
    if (rangeActive) return;
    rangeActive = true;
    rangePointer = { x: startX, y: startY };
    lastRangeKey = null;
    rangeScrollEl = findScrollParent(node);

    window.addEventListener('pointermove', onRangePointerMove, { passive: true });
    window.addEventListener('pointerup', onRangePointerUp, true);
    window.addEventListener('pointercancel', onRangePointerCancel, true);
    window.addEventListener('touchmove', onRangeTouchMove, { passive: false });
    window.addEventListener('touchend', onRangeTouchEnd, true);
    window.addEventListener('touchcancel', onRangeTouchCancel, true);

    armGlobalClickGuard(RANGE_CLICK_GUARD_MS);
  }

  function disarm() {
    clearTimer();
    finishRangeDrag();
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
      if (onRangeMove) startRangeDrag();
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
      onRangeMove = newOptions?.onRangeMove;
      onRangeEnd = newOptions?.onRangeEnd;
      delay = newOptions?.delay ?? 500;
      threshold = newOptions?.threshold ?? 10;
    },
    destroy() {
      disarm();
      disarmGlobalClickGuard();
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
