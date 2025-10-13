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
    this.lastTapTime = 0;
    this.tapCount = 0;

    // Gesture state
    this.activeGesture = null;
    this.gestureState = 'idle'; // idle, recognizing, active

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
        this.recognizeSwipe(touch);
      }
    }

    // Clean up ended touches
    endedTouches.forEach((id) => this.touches.delete(id));

    // Reset state if no touches remain
    if (this.touches.size === 0) {
      this.activeGesture = null;
      this.gestureState = 'idle';
    }
  }

  handleTouchCancel() {
    // Clean up on cancel
    this.touches.clear();
    this.activeGesture = null;
    this.gestureState = 'idle';
  }

  handleTap(touch) {
    const now = Date.now();
    const timeSinceLastTap = now - this.lastTapTime;

    if (timeSinceLastTap < 300 && this.options.enableDoubleTap) {
      // Double tap detected
      this.tapCount = 0;
      this.lastTapTime = 0;

      if (this.callbacks.onDoubleTap) {
        this.callbacks.onDoubleTap({
          x: touch.currentX,
          y: touch.currentY,
        });
      }
    } else {
      // Single tap
      this.tapCount = 1;
      this.lastTapTime = now;
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

    const deltaX = touch.currentX - touch.startX;
    const deltaY = touch.currentY - touch.startY;

    if (this.callbacks.onPan) {
      this.callbacks.onPan({
        deltaX,
        deltaY,
        velocityX: touch.velocityX,
        velocityY: touch.velocityY,
      });
    }
  }

  endPanGesture() {
    const touch = Array.from(this.touches.values())[0];
    if (!touch && this.callbacks.onPanEnd) {
      this.callbacks.onPanEnd({
        velocityX: 0,
        velocityY: 0,
      });
      return;
    }

    if (this.callbacks.onPanEnd) {
      this.callbacks.onPanEnd({
        velocityX: touch.velocityX || 0,
        velocityY: touch.velocityY || 0,
      });
    }
    this.activeGesture = null;
  }

  recognizeSwipe(touch) {
    const deltaX = touch.currentX - touch.startX;
    const deltaY = touch.currentY - touch.startY;
    const absDeltaX = Math.abs(deltaX);
    const absDeltaY = Math.abs(deltaY);

    // Determine swipe direction (more horizontal or vertical)
    if (absDeltaX > absDeltaY) {
      // Horizontal swipe
      const threshold = window.innerWidth * 0.2; // 20% of viewport width
      const velocityThreshold = 0.3; // px/ms

      if (absDeltaX > threshold || Math.abs(touch.velocityX) > velocityThreshold) {
        const direction = deltaX > 0 ? 'right' : 'left';

        if (this.callbacks.onSwipe) {
          this.callbacks.onSwipe({
            direction,
            distance: absDeltaX,
            velocity: Math.abs(touch.velocityX),
          });
        }
      }
    } else {
      // Vertical swipe
      const threshold = window.innerHeight * 0.2; // 20% of viewport height
      const velocityThreshold = 0.3; // px/ms

      if (absDeltaY > threshold || Math.abs(touch.velocityY) > velocityThreshold) {
        const direction = deltaY > 0 ? 'down' : 'up';

        if (this.callbacks.onSwipe) {
          this.callbacks.onSwipe({
            direction,
            distance: absDeltaY,
            velocity: Math.abs(touch.velocityY),
          });
        }
      }
    }
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
