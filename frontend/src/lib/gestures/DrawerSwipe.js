import { drawerOpenProgress, GESTURE_CLICK_SUPPRESS_MS, shouldSettleOpen } from './panelSettle.js';

const AXIS_THRESHOLD = 10;

// Surfaces that own a horizontal pointer interaction themselves. Starting a
// drawer swipe on one of them would fight that control, so the gesture never
// begins there.
const EXCLUDED_SELECTOR = [
  '.map-view',
  '.timeline-selector',
  '.timeline-slider',
  '.date-rail',
  'input',
  'textarea',
  'select',
  '[contenteditable="true"]',
  '[role="slider"]',
].join(', ');

function hasBlockingOverlay() {
  return Boolean(
    document.querySelector('dialog[open]') ||
    document.querySelector('.photo-viewer.active') ||
    document.querySelector('.indexing-bottom-sheet:not([aria-hidden="true"])')
  );
}

/**
 * Full-surface horizontal drawer gesture.
 *
 * The drawer's transform is owned by the caller: this controller reports the
 * finger position while dragging and the final open state on release. It binds
 * to `window` only because the drawer's own DOM is off-screen while closed;
 * every gesture is filtered back to `.main-content` (open) or the drawer/scrim
 * (close), so the viewer and the map never reach it.
 */
export class DrawerSwipe {
  constructor({ isEnabled, isOpen, onDrag, onSettle }) {
    this.isEnabled = isEnabled;
    this.isOpen = isOpen;
    this.onDrag = onDrag;
    this.onSettle = onSettle;

    this.mode = null;
    this.axis = null;
    this.dragging = false;
    this.start = null;
    this.last = null;
    this.translateX = 0;
    this.baseTranslateX = 0;
    this.drawer = null;
    this.width = 0;
    this.velocityFrames = [];
    this.suppressClicksUntil = 0;

    this._onStart = (e) => this.handleStart(e);
    this._onMove = (e) => this.handleMove(e);
    this._onEnd = (e) => this.handleEnd(e);
    this._onCancel = () => this.handleCancel();
    this._onClick = (e) => {
      if (performance.now() >= this.suppressClicksUntil) return;
      e.preventDefault();
      e.stopPropagation();
    };
  }

  attach() {
    window.addEventListener('touchstart', this._onStart, { passive: false });
    window.addEventListener('touchmove', this._onMove, { passive: false });
    window.addEventListener('touchend', this._onEnd, { passive: false });
    window.addEventListener('touchcancel', this._onCancel);
    window.addEventListener('click', this._onClick, true);
  }

  detach() {
    window.removeEventListener('touchstart', this._onStart);
    window.removeEventListener('touchmove', this._onMove);
    window.removeEventListener('touchend', this._onEnd);
    window.removeEventListener('touchcancel', this._onCancel);
    window.removeEventListener('click', this._onClick, true);
  }

  eventTime(event) {
    return event?.timeStamp > 0 ? event.timeStamp : Date.now();
  }

  handleStart(event) {
    this.reset();
    if (event.touches.length !== 1) return;
    if (!this.isEnabled() || hasBlockingOverlay()) return;

    const target = event.target;
    if (!target || typeof target.closest !== 'function') return;
    if (target.closest('input, textarea, select, [contenteditable="true"]')) return;

    const open = this.isOpen();
    if (open) {
      if (!target.closest('#sidebar-nav, .sidebar-overlay')) return;
      this.mode = 'close';
    } else {
      if (!target.closest('.main-content')) return;
      if (target.closest(EXCLUDED_SELECTOR)) return;
      this.mode = 'open';
    }

    const [touch] = event.touches;
    const time = this.eventTime(event);
    this.start = { x: touch.clientX, y: touch.clientY, time };
    this.last = { x: touch.clientX, y: touch.clientY, time };
    this.drawer = document.getElementById('sidebar-nav');
    this.width = this.drawer?.offsetWidth || 0;
    // A gesture can begin while the drawer is still settling. Read the live
    // transform so the finger takes over from where the pixels are, not from
    // an assumed end state (that jump was the same class of bug as the CSS
    // transition fighting the rAF slide).
    this.baseTranslateX = this.currentTranslateX();
  }

  currentTranslateX() {
    if (!this.drawer) return this.isOpen() ? 0 : -this.width;
    const transform = getComputedStyle(this.drawer).transform;
    if (!transform || transform === 'none') return this.isOpen() ? 0 : -this.width;
    return Math.min(0, Math.max(-this.width, new window.DOMMatrixReadOnly(transform).m41));
  }

  handleMove(event) {
    if (!this.mode) return;
    if (event.touches.length !== 1) {
      if (this.dragging) this.handleCancel();
      else this.reset();
      return;
    }
    const [touch] = event.touches;

    const time = this.eventTime(event);
    const deltaTime = time - this.last.time;
    if (deltaTime > 0) {
      this.velocityFrames.push((touch.clientX - this.last.x) / deltaTime);
      if (this.velocityFrames.length > 5) this.velocityFrames.shift();
    }
    this.last = { x: touch.clientX, y: touch.clientY, time };

    const deltaX = touch.clientX - this.start.x;
    const deltaY = touch.clientY - this.start.y;
    if (Math.hypot(deltaX, deltaY) <= AXIS_THRESHOLD && !this.axis) return;

    if (!this.axis) {
      this.axis = Math.abs(deltaX) > Math.abs(deltaY) ? 'horizontal' : 'vertical';
      if (this.axis !== 'horizontal') {
        this.mode = null;
        return;
      }
      // The closed drawer only opens rightwards; the open drawer only closes
      // leftwards. The opposite direction is not a gesture this panel owns.
      if ((this.mode === 'open' && deltaX <= 0) || (this.mode === 'close' && deltaX >= 0)) {
        this.mode = null;
        return;
      }
      this.dragging = true;
    }

    if (!this.dragging) return;

    event.preventDefault();
    this.translateX = Math.min(0, Math.max(-this.width, this.baseTranslateX + deltaX));
    this.onDrag({
      translateX: this.translateX,
      progress: drawerOpenProgress(this.translateX, this.width),
    });
  }

  handleEnd() {
    if (!this.dragging) {
      this.reset();
      return;
    }

    const velocity = this.averageVelocity();
    const open = shouldSettleOpen({
      startedOpen: this.mode === 'close',
      progress: drawerOpenProgress(this.translateX, this.width),
      velocity,
    });
    this.suppressClicksUntil = performance.now() + GESTURE_CLICK_SUPPRESS_MS;
    this.onSettle(open);
    this.reset();
  }

  handleCancel() {
    if (this.dragging) {
      this.suppressClicksUntil = performance.now() + GESTURE_CLICK_SUPPRESS_MS;
      // A cancelled gesture returns to the state it started from, never to
      // wherever the interrupted finger left the panel.
      this.onSettle(this.mode === 'close');
    }
    this.reset();
  }

  averageVelocity() {
    if (this.velocityFrames.length === 0) return 0;
    const total = this.velocityFrames.reduce((sum, value) => sum + value, 0);
    return total / this.velocityFrames.length;
  }

  reset() {
    this.mode = null;
    this.axis = null;
    this.dragging = false;
    this.start = null;
    this.last = null;
    this.translateX = 0;
    this.baseTranslateX = 0;
    this.velocityFrames = [];
  }
}
