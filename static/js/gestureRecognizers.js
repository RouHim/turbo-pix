// Gesture Recognizers - Individual gesture pattern recognizers

class PinchRecognizer {
  constructor() {
    this.minPinchDistance = 20;
  }

  recognize(touches) {
    if (touches.length !== 2) return null;

    const [touch1, touch2] = touches;
    const dx = touch2.currentX - touch1.currentX;
    const dy = touch2.currentY - touch1.currentY;
    const distance = Math.sqrt(dx * dx + dy * dy);

    if (distance < this.minPinchDistance) return null;

    return {
      type: 'pinch',
      confidence: 1.0,
      data: {
        distance,
        centerX: (touch1.currentX + touch2.currentX) / 2,
        centerY: (touch1.currentY + touch2.currentY) / 2,
      },
    };
  }
}

class SwipeRecognizer {
  constructor() {
    this.minDistance = 50;
    this.maxDuration = 1000;
    this.velocityThreshold = 0.3; // px/ms
  }

  recognize(touch, options = {}) {
    const { allowVertical = true, allowHorizontal = true } = options;

    if (!touch) return null;

    const deltaX = touch.currentX - touch.startX;
    const deltaY = touch.currentY - touch.startY;
    const absDeltaX = Math.abs(deltaX);
    const absDeltaY = Math.abs(deltaY);
    const duration = touch.lastTime - touch.startTime;

    if (duration > this.maxDuration) return null;

    const velocity = Math.sqrt(
      touch.velocityX * touch.velocityX + touch.velocityY * touch.velocityY
    );

    // Determine primary direction
    if (absDeltaX > absDeltaY) {
      // Horizontal swipe
      if (!allowHorizontal) return null;

      const threshold = window.innerWidth * 0.2;
      if (absDeltaX > threshold || Math.abs(touch.velocityX) > this.velocityThreshold) {
        return {
          type: 'swipe',
          confidence: Math.min(velocity / 2, 1.0),
          data: {
            direction: deltaX > 0 ? 'right' : 'left',
            distance: absDeltaX,
            velocity: Math.abs(touch.velocityX),
          },
        };
      }
    } else {
      // Vertical swipe
      if (!allowVertical) return null;

      const threshold = window.innerHeight * 0.2;
      if (absDeltaY > threshold || Math.abs(touch.velocityY) > this.velocityThreshold) {
        return {
          type: 'swipe',
          confidence: Math.min(velocity / 2, 1.0),
          data: {
            direction: deltaY > 0 ? 'down' : 'up',
            distance: absDeltaY,
            velocity: Math.abs(touch.velocityY),
          },
        };
      }
    }

    return null;
  }
}

class DoubleTapRecognizer {
  constructor() {
    this.maxDoubleTapDelay = 300; // ms
    this.maxTapDistance = 10; // px
    this.maxTapDuration = 300; // ms
    this.lastTapTime = 0;
    this.lastTapX = 0;
    this.lastTapY = 0;
  }

  recognize(touch) {
    if (!touch) return null;

    const deltaX = touch.currentX - touch.startX;
    const deltaY = touch.currentY - touch.startY;
    const distance = Math.sqrt(deltaX * deltaX + deltaY * deltaY);
    const duration = touch.lastTime - touch.startTime;

    // Must be a tap (short duration, small movement)
    if (distance > this.maxTapDistance || duration > this.maxTapDuration) {
      return null;
    }

    const now = touch.lastTime;
    const timeSinceLastTap = now - this.lastTapTime;
    const distanceFromLastTap = Math.sqrt(
      Math.pow(touch.currentX - this.lastTapX, 2) + Math.pow(touch.currentY - this.lastTapY, 2)
    );

    // Check if this is a double tap
    if (
      timeSinceLastTap < this.maxDoubleTapDelay &&
      distanceFromLastTap < this.maxTapDistance * 2
    ) {
      // Double tap detected
      this.lastTapTime = 0; // Reset to prevent triple tap
      return {
        type: 'doubleTap',
        confidence: 1.0,
        data: {
          x: touch.currentX,
          y: touch.currentY,
        },
      };
    }

    // Store this tap for potential future double tap
    this.lastTapTime = now;
    this.lastTapX = touch.currentX;
    this.lastTapY = touch.currentY;

    return {
      type: 'tap',
      confidence: 1.0,
      data: {
        x: touch.currentX,
        y: touch.currentY,
      },
    };
  }

  reset() {
    this.lastTapTime = 0;
    this.lastTapX = 0;
    this.lastTapY = 0;
  }
}

class PanRecognizer {
  constructor() {
    this.minDistance = 10;
  }

  recognize(touch) {
    if (!touch) return null;

    const deltaX = touch.currentX - touch.startX;
    const deltaY = touch.currentY - touch.startY;
    const distance = Math.sqrt(deltaX * deltaX + deltaY * deltaY);

    if (distance < this.minDistance) return null;

    return {
      type: 'pan',
      confidence: Math.min(distance / 50, 1.0),
      data: {
        deltaX,
        deltaY,
        velocityX: touch.velocityX,
        velocityY: touch.velocityY,
      },
    };
  }
}

class LongPressRecognizer {
  constructor() {
    this.minDuration = 500; // ms
    this.maxMovement = 10; // px
  }

  recognize(touch, isStillActive = true) {
    if (!touch || !isStillActive) return null;

    const deltaX = touch.currentX - touch.startX;
    const deltaY = touch.currentY - touch.startY;
    const distance = Math.sqrt(deltaX * deltaX + deltaY * deltaY);
    const duration = Date.now() - touch.startTime;

    if (distance > this.maxMovement) return null;
    if (duration < this.minDuration) return null;

    return {
      type: 'longPress',
      confidence: 1.0,
      data: {
        x: touch.currentX,
        y: touch.currentY,
        duration,
      },
    };
  }
}

// Export recognizers
window.GestureRecognizers = {
  PinchRecognizer,
  SwipeRecognizer,
  DoubleTapRecognizer,
  PanRecognizer,
  LongPressRecognizer,
};
