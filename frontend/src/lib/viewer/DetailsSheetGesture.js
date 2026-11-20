import { GESTURE_CLICK_SUPPRESS_MS } from '../gestures/panelSettle.js';
import { sheetHeaderSwipeDirection } from './sheetSwipe.js';

const AXIS_THRESHOLD = 10;
const HANDLE_ZONE_PX = 56;

/**
 * Downward drag on the open mobile details sheet.
 *
 * The sheet's own body scrolls; the drag only takes over at `scrollTop === 0`
 * (or when the gesture starts in the handle/header zone), which is the native
 * bottom-sheet handoff: scroll first, close at the top edge. The photo-area
 * gestures are handled by SwipeableViewer and reach the same viewer methods.
 */
export class DetailsSheetGesture {
  constructor(viewer) {
    this.viewer = viewer;
    this.element = null;
    this.axis = null;
    this.dragging = false;
    this.handleZone = false;
    this.horizontalTriggered = false;
    this.start = null;
    this.last = null;
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

  mount(element) {
    this.element = element;
    element.addEventListener('touchstart', this._onStart, { passive: false });
    element.addEventListener('touchmove', this._onMove, { passive: false });
    element.addEventListener('touchend', this._onEnd, { passive: false });
    element.addEventListener('touchcancel', this._onCancel);
    element.addEventListener('click', this._onClick, true);
  }

  destroy() {
    const element = this.element;
    if (element) {
      element.removeEventListener('touchstart', this._onStart);
      element.removeEventListener('touchmove', this._onMove);
      element.removeEventListener('touchend', this._onEnd);
      element.removeEventListener('touchcancel', this._onCancel);
      element.removeEventListener('click', this._onClick, true);
    }
    this.element = null;
    this.reset();
  }

  eventTime(event) {
    return event?.timeStamp > 0 ? event.timeStamp : Date.now();
  }

  handleStart(event) {
    this.reset();
    if (event.touches.length !== 1) return;
    if (!this.viewer.isOpen || !this.viewer.isSidebarOpen) return;
    if (!this.viewer.canDragDetailsSheet) return;

    const [touch] = event.touches;
    const rect = this.element.getBoundingClientRect();
    // The visible title/header sits below the 56 px grab strip, but it is the
    // natural horizontal handle. A press that starts on it owns both axes.
    const headerTarget = Boolean(event.target?.closest?.('.photo-info-header'));
    this.handleZone = headerTarget || touch.clientY - rect.top <= HANDLE_ZONE_PX;
    if (!this.handleZone && this.element.scrollTop > 0) return;

    const time = this.eventTime(event);
    this.start = { x: touch.clientX, y: touch.clientY, time };
    this.last = { x: touch.clientX, y: touch.clientY, time };
  }

  handleMove(event) {
    if (!this.start) return;
    if (event.touches.length !== 1) {
      if (this.dragging) this.handleCancel();
      else this.reset();
      return;
    }
    const [touch] = event.touches;

    const time = this.eventTime(event);
    const deltaTime = time - this.last.time;
    if (deltaTime > 0) {
      this.velocityFrames.push((touch.clientY - this.last.y) / deltaTime);
      if (this.velocityFrames.length > 5) this.velocityFrames.shift();
    }
    this.last = { x: touch.clientX, y: touch.clientY, time };

    const deltaX = touch.clientX - this.start.x;
    const deltaY = touch.clientY - this.start.y;
    if (!this.axis && Math.hypot(deltaX, deltaY) <= AXIS_THRESHOLD) return;

    if (!this.axis) {
      this.axis = Math.abs(deltaX) > Math.abs(deltaY) ? 'horizontal' : 'vertical';
      if (this.axis === 'horizontal') {
        // The body scrolls; only the handle/header owns horizontal navigation.
        if (!this.handleZone) {
          this.start = null;
          return;
        }
      } else {
        // Only a downward drag closes the sheet; upward drags scroll content.
        if (deltaY <= 0) {
          this.start = null;
          return;
        }
        if (!this.handleZone && this.element.scrollTop > 0) {
          this.start = null;
          return;
        }
        this.dragging = true;
        this.viewer.beginSheetDrag(true);
      }
    }

    if (this.axis === 'horizontal') {
      if (!this.handleZone || this.horizontalTriggered) return;
      const direction = sheetHeaderSwipeDirection(deltaX, deltaY);
      if (!direction) return;
      this.horizontalTriggered = true;
      this.suppressClicksUntil = performance.now() + GESTURE_CLICK_SUPPRESS_MS;
      event.preventDefault();
      this.viewer.slideTo(direction);
      return;
    }

    if (!this.dragging) return;
    event.preventDefault();
    this.viewer.updateSheetDrag(deltaY);
  }

  handleEnd() {
    if (this.dragging) {
      this.suppressClicksUntil = performance.now() + GESTURE_CLICK_SUPPRESS_MS;
      this.viewer.endSheetDrag(this.averageVelocity());
    } else if (this.horizontalTriggered) {
      this.suppressClicksUntil = performance.now() + GESTURE_CLICK_SUPPRESS_MS;
      this.viewer.noteGestureEnd?.();
    }
    this.reset();
  }

  handleCancel() {
    if (this.dragging) {
      this.suppressClicksUntil = performance.now() + GESTURE_CLICK_SUPPRESS_MS;
      this.viewer.cancelSheetDrag();
    } else if (this.horizontalTriggered) {
      this.suppressClicksUntil = performance.now() + GESTURE_CLICK_SUPPRESS_MS;
      this.viewer.noteGestureEnd?.();
    }
    this.reset();
  }

  averageVelocity() {
    if (this.velocityFrames.length === 0) return 0;
    return this.velocityFrames.reduce((sum, value) => sum + value, 0) / this.velocityFrames.length;
  }

  reset() {
    this.axis = null;
    this.dragging = false;
    this.handleZone = false;
    this.horizontalTriggered = false;
    this.start = null;
    this.last = null;
    this.velocityFrames = [];
  }
}
