// Gesture Manager - Modern touch gesture handling
/* global GestureRecognizers */

class GestureManager {
  constructor(element, options = {}) {
    this.element = element;
    this.options = {
      enablePinch: true,
      enableSwipe: true,
      enableDoubleTap: true,
      enablePan: true,
      ...options,
    };

    // Touch tracking
    this.touches = new Map();
    this.touchStartTime = 0;
    this.velocityFrames = [];

    // Gesture state
    this.activeGesture = null;
    this.gestureState = 'idle'; // idle, recognizing, active
    this.gestureAxis = null;

    // Callbacks
    this.callbacks = {
      onPinch: null,
      onPinchEnd: null,
      onSwipe: null,
      onDoubleTap: null,
      onPan: null,
      onPanEnd: null,
    };

    // Recognizers
    this.recognizers = null;

    this.init();
  }

  init() {
    // Wait for GestureRecognizers to be available
    if (typeof GestureRecognizers !== 'undefined') {
      this.initRecognizers();
    } else {
      // Retry after a short delay
      setTimeout(() => this.init(), 50);
      return;
    }

    this.bindEvents();
  }

  initRecognizers() {
    this.recognizers = {
      pinch: new GestureRecognizers.PinchRecognizer(),
      swipe: new GestureRecognizers.SwipeRecognizer(),
      doubleTap: new GestureRecognizers.DoubleTapRecognizer(),
      pan: new GestureRecognizers.PanRecognizer(),
    };
  }

  bindEvents() {
    if (!this.element) return;

    this.element.addEventListener('touchstart', (e) => this.handleTouchStart(e), {
      passive: false,
    });
    this.element.addEventListener('touchmove', (e) => this.handleTouchMove(e), { passive: false });
    this.element.addEventListener('touchend', (e) => this.handleTouchEnd(e), { passive: false });
    this.element.addEventListener('touchcancel', (e) => this.handleTouchCancel(e));
  }

  handleTouchStart(e) {
    const timestamp = Date.now();

    if (this.touches.size === 0) {
      this.resetGestureTracking();
    }

    this.touchStartTime = timestamp;

    // Track all touches
    Array.from(e.touches).forEach((touch) => {
      this.touches.set(touch.identifier, {
        id: touch.identifier,
        startX: touch.clientX,
        startY: touch.clientY,
        currentX: touch.clientX,
        currentY: touch.clientY,
        startTime: timestamp,
        lastTime: timestamp,
        velocityX: 0,
        velocityY: 0,
      });
    });

    // Detect gesture type
    if (this.touches.size === 2 && this.options.enablePinch) {
      this.startPinchGesture();
    } else if (this.touches.size === 1) {
      // Could be tap, swipe, or pan - wait for movement
      this.gestureState = 'recognizing';
    }
  }

  handleTouchMove(e) {
    if (this.touches.size === 0) return;

    const timestamp = Date.now();

    // Update touch positions and velocities
    Array.from(e.touches).forEach((touch) => {
      const tracked = this.touches.get(touch.identifier);
      if (!tracked) return;

      const deltaTime = timestamp - tracked.lastTime;
      if (deltaTime > 0) {
        tracked.velocityX = (touch.clientX - tracked.currentX) / deltaTime;
        tracked.velocityY = (touch.clientY - tracked.currentY) / deltaTime;
      }

      tracked.currentX = touch.clientX;
      tracked.currentY = touch.clientY;
      tracked.lastTime = timestamp;
    });

    if (this.touches.size === 1) {
      const touch = Array.from(this.touches.values())[0];
      this.updateGestureAxis(touch);
      this.recordVelocityFrame(touch, timestamp);
    }

    // Process active gesture or recognize new one
    if (this.activeGesture === 'pinch' && this.touches.size === 2) {
      e.preventDefault();
      this.processPinch();
    } else if (this.activeGesture === 'pan' && this.touches.size === 1) {
      e.preventDefault();
      this.processPan();
    } else if (this.gestureState === 'recognizing' && this.touches.size === 1) {
      // Try to recognize gesture based on movement
      const touch = Array.from(this.touches.values())[0];
      const deltaX = touch.currentX - touch.startX;
      const deltaY = touch.currentY - touch.startY;
      const distance = Math.sqrt(deltaX * deltaX + deltaY * deltaY);

      if (distance > 10) {
        // Movement detected - determine gesture type
        // Check if we should pan (will be determined by viewer zoom state)
        // For now, assume it's a potential swipe
        this.gestureState = 'active';
      }
    }
  }

  handleTouchEnd(e) {
    const timestamp = Date.now();

    // Get remaining touches
    const remainingTouches = new Set(Array.from(e.touches).map((t) => t.identifier));

    // Get ended touches
    const endedTouches = Array.from(this.touches.keys()).filter((id) => !remainingTouches.has(id));

    // Process gesture completion
    if (this.activeGesture === 'pinch' && endedTouches.length > 0) {
      this.endPinchGesture();
    } else if (this.activeGesture === 'pan' && endedTouches.length > 0) {
      this.endPanGesture();
    } else if (this.touches.size === 1 && endedTouches.length === 1) {
      // Single touch ended - check for tap or swipe
      const touch = this.touches.get(endedTouches[0]);
      if (!touch) return;

      touch.lastTime = timestamp;

      const deltaX = touch.currentX - touch.startX;
      const deltaY = touch.currentY - touch.startY;
      const distance = Math.sqrt(deltaX * deltaX + deltaY * deltaY);
      const duration = timestamp - touch.startTime;

      // Check for tap
      if (distance < 10 && duration < 300) {
        this.handleTap(touch);
      }
      // Check for swipe
      else if (this.options.enableSwipe) {
        const swipeTouch = this.createTouchWithSmoothedVelocity(touch);
        const swipe = this.recognizers.swipe.recognize(swipeTouch, this.getSwipeOptions());

        if (swipe?.type === 'swipe' && this.callbacks.onSwipe) {
          this.callbacks.onSwipe(swipe.data);
        }
      }
    }

    // Clean up ended touches
    endedTouches.forEach((id) => {
      this.touches.delete(id);
    });

    // Reset state if no touches remain
    if (this.touches.size === 0) {
      this.resetGestureTracking();
    }
  }

  handleTouchCancel() {
    // Clean up on cancel
    this.touches.clear();
    this.resetGestureTracking();
  }

  handleTap(touch) {
    const tap = this.recognizers.doubleTap.recognize(touch);
    if (tap?.type === 'doubleTap' && this.options.enableDoubleTap && this.callbacks.onDoubleTap) {
      this.callbacks.onDoubleTap(tap.data);
    }
  }

  startPinchGesture() {
    this.activeGesture = 'pinch';
    this.gestureState = 'active';

    const touches = Array.from(this.touches.values());
    if (touches.length !== 2) return;

    const dx = touches[1].currentX - touches[0].currentX;
    const dy = touches[1].currentY - touches[0].currentY;
    this.pinchStartDistance = Math.sqrt(dx * dx + dy * dy);
    this.pinchLastDistance = this.pinchStartDistance;
    this.pinchCenterX = (touches[0].currentX + touches[1].currentX) / 2;
    this.pinchCenterY = (touches[0].currentY + touches[1].currentY) / 2;
  }

  processPinch() {
    const touches = Array.from(this.touches.values());
    if (touches.length !== 2) return;

    const dx = touches[1].currentX - touches[0].currentX;
    const dy = touches[1].currentY - touches[0].currentY;
    const distance = Math.sqrt(dx * dx + dy * dy);

    const scale = distance / this.pinchStartDistance;
    const deltaScale = distance / this.pinchLastDistance;
    this.pinchLastDistance = distance;

    const centerX = (touches[0].currentX + touches[1].currentX) / 2;
    const centerY = (touches[0].currentY + touches[1].currentY) / 2;

    if (this.callbacks.onPinch) {
      this.callbacks.onPinch({
        scale,
        deltaScale,
        centerX,
        centerY,
        initialCenterX: this.pinchCenterX,
        initialCenterY: this.pinchCenterY,
      });
    }
  }

  endPinchGesture() {
    if (this.callbacks.onPinchEnd) {
      this.callbacks.onPinchEnd();
    }
    this.activeGesture = null;
  }

  processPan() {
    const touch = Array.from(this.touches.values())[0];
    if (!touch) return;

    const gestureTouch = this.applyAxisLock({
      deltaX: touch.currentX - touch.startX,
      deltaY: touch.currentY - touch.startY,
      velocityX: touch.velocityX,
      velocityY: touch.velocityY,
    });

    if (this.callbacks.onPan) {
      this.callbacks.onPan({
        deltaX: gestureTouch.deltaX,
        deltaY: gestureTouch.deltaY,
        velocityX: gestureTouch.velocityX,
        velocityY: gestureTouch.velocityY,
      });
    }
  }

  endPanGesture() {
    const touch = Array.from(this.touches.values())[0];
    const averagedVelocity = this.getAverageVelocity(touch);

    if (!touch && this.callbacks.onPanEnd) {
      this.callbacks.onPanEnd({
        velocityX: averagedVelocity.velocityX,
        velocityY: averagedVelocity.velocityY,
      });
      return;
    }

    if (this.callbacks.onPanEnd) {
      this.callbacks.onPanEnd({
        velocityX: averagedVelocity.velocityX,
        velocityY: averagedVelocity.velocityY,
      });
    }
    this.activeGesture = null;
  }

  updateGestureAxis(touch) {
    if (!touch || this.gestureAxis) return;

    const deltaX = touch.currentX - touch.startX;
    const deltaY = touch.currentY - touch.startY;
    const distance = Math.sqrt(deltaX * deltaX + deltaY * deltaY);
    if (distance <= 10) return;

    this.gestureAxis = Math.abs(deltaX) > Math.abs(deltaY) ? 'horizontal' : 'vertical';
  }

  recordVelocityFrame(touch, timestamp) {
    if (!touch) return;

    this.velocityFrames.push({
      vx: touch.velocityX,
      vy: touch.velocityY,
      time: timestamp,
    });

    if (this.velocityFrames.length > 5) {
      this.velocityFrames.shift();
    }
  }

  applyAxisLock({ deltaX, deltaY, velocityX, velocityY }) {
    if (this.gestureAxis === 'horizontal') {
      return {
        deltaX,
        deltaY: 0,
        velocityX,
        velocityY: 0,
      };
    }

    if (this.gestureAxis === 'vertical') {
      return {
        deltaX: 0,
        deltaY,
        velocityX: 0,
        velocityY,
      };
    }

    return {
      deltaX,
      deltaY,
      velocityX,
      velocityY,
    };
  }

  getAverageVelocity(touch) {
    const frameCount = this.velocityFrames.length;

    if (frameCount === 0) {
      return this.applyAxisLock({
        deltaX: 0,
        deltaY: 0,
        velocityX: touch?.velocityX || 0,
        velocityY: touch?.velocityY || 0,
      });
    }

    const totals = this.velocityFrames.reduce(
      (sum, frame) => ({
        velocityX: sum.velocityX + frame.vx,
        velocityY: sum.velocityY + frame.vy,
      }),
      { velocityX: 0, velocityY: 0 }
    );

    return this.applyAxisLock({
      deltaX: 0,
      deltaY: 0,
      velocityX: totals.velocityX / frameCount,
      velocityY: totals.velocityY / frameCount,
    });
  }

  createTouchWithSmoothedVelocity(touch) {
    const averagedVelocity = this.getAverageVelocity(touch);

    return {
      ...touch,
      velocityX: averagedVelocity.velocityX,
      velocityY: averagedVelocity.velocityY,
    };
  }

  getSwipeOptions() {
    return {
      allowHorizontal: this.gestureAxis !== 'vertical',
      allowVertical: this.gestureAxis !== 'horizontal',
    };
  }

  resetGestureTracking() {
    this.activeGesture = null;
    this.gestureState = 'idle';
    this.gestureAxis = null;
    this.velocityFrames = [];
  }

  // Public API
  on(event, callback) {
    const eventMap = {
      pinch: 'onPinch',
      pinchEnd: 'onPinchEnd',
      swipe: 'onSwipe',
      doubleTap: 'onDoubleTap',
      pan: 'onPan',
      panEnd: 'onPanEnd',
    };

    if (eventMap[event]) {
      this.callbacks[eventMap[event]] = callback;
    }
  }

  enablePan() {
    this.activeGesture = 'pan';
  }

  disablePan() {
    if (this.activeGesture === 'pan') {
      this.activeGesture = null;
    }
  }

  destroy() {
    // Clean up event listeners
    if (this.element) {
      this.element.removeEventListener('touchstart', this.handleTouchStart);
      this.element.removeEventListener('touchmove', this.handleTouchMove);
      this.element.removeEventListener('touchend', this.handleTouchEnd);
      this.element.removeEventListener('touchcancel', this.handleTouchCancel);
    }

    this.touches.clear();
    this.callbacks = {};
    this.recognizers = null;
  }
}

window.GestureManager = GestureManager;
