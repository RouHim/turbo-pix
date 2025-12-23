/* global requestAnimationFrame, cancelAnimationFrame */

class ViewerControls {
  constructor(viewer, elements) {
    this.viewer = viewer;
    this.elements = elements;

    this.zoomLevel = 1;
    this.maxZoom = 5;
    this.minZoom = 0.5;
    this.isDragging = false;
    this.dragStart = { x: 0, y: 0 };
    this.imagePosition = { x: 0, y: 0 };

    // Gesture-based zoom state
    this.gestureZoomLevel = 1;
    this.gestureBaseZoom = 1;
    this.zoomCenter = { x: 0, y: 0 };

    // Animation state
    this.animationFrame = null;
    this.isAnimating = false;

    this.init();
  }

  init() {
    this.bindZoomControls();
    this.bindFullscreenControls();
    this.bindDragControls();
  }

  bindZoomControls() {
    if (this.elements.zoomIn) {
      utils.on(this.elements.zoomIn, 'click', () => this.zoomIn());
    }

    if (this.elements.zoomOut) {
      utils.on(this.elements.zoomOut, 'click', () => this.zoomOut());
    }

    if (this.elements.zoomFit) {
      utils.on(this.elements.zoomFit, 'click', () => this.fitToScreen());
    }
  }

  bindFullscreenControls() {
    if (this.elements.fullscreenBtn) {
      utils.on(this.elements.fullscreenBtn, 'click', () => this.toggleFullscreen());
    }
  }

  bindDragControls() {
    if (this.elements.image) {
      utils.on(this.elements.image, 'mousedown', (e) => this.startDrag(e));
      utils.on(document, 'mousemove', (e) => this.drag(e));
      utils.on(document, 'mouseup', () => this.endDrag());
    }
  }

  zoomIn() {
    if (this.zoomLevel < this.maxZoom) {
      this.zoomLevel = Math.min(this.zoomLevel * 1.5, this.maxZoom);
      this.applyZoom();
    }
  }

  zoomOut() {
    if (this.zoomLevel > this.minZoom) {
      this.zoomLevel = Math.max(this.zoomLevel / 1.5, this.minZoom);
      this.applyZoom();
    }
  }

  fitToScreen() {
    this.zoomLevel = 1;
    this.imagePosition = { x: 0, y: 0 };
    this.applyZoom();
  }

  updateZoomButtonStates() {
    // Disable zoom buttons for videos (zooming not supported for videos)
    const isVideo = this.viewer.isVideoFile(this.viewer.getCurrentPhoto()?.filename);

    if (this.elements.zoomIn) {
      this.elements.zoomIn.disabled = isVideo;
      this.elements.zoomIn.classList.toggle('btn-disabled', isVideo);
      this.elements.zoomIn.title = isVideo ? 'Zoom not supported for videos' : 'Zoom In';
    }

    if (this.elements.zoomOut) {
      this.elements.zoomOut.disabled = isVideo;
      this.elements.zoomOut.classList.toggle('btn-disabled', isVideo);
      this.elements.zoomOut.title = isVideo ? 'Zoom not supported for videos' : 'Zoom Out';
    }
  }

  applyZoom() {
    if (!this.elements.image) return;

    const transform = `scale(${this.zoomLevel}) translate(${this.imagePosition.x}px, ${this.imagePosition.y}px)`;
    this.elements.image.style.transform = transform;

    this.elements.image.classList.toggle('zoomed', this.zoomLevel > 1);
  }

  startDrag(e) {
    if (this.zoomLevel <= 1) return;

    e.preventDefault();
    this.isDragging = true;
    this.dragStart = {
      x: e.clientX - this.imagePosition.x,
      y: e.clientY - this.imagePosition.y,
    };
    this.elements.image.style.cursor = 'grabbing';
  }

  drag(e) {
    if (!this.isDragging || this.zoomLevel <= 1) return;

    e.preventDefault();
    this.imagePosition = {
      x: e.clientX - this.dragStart.x,
      y: e.clientY - this.dragStart.y,
    };
    this.applyZoom();
  }

  endDrag() {
    if (!this.isDragging) return;

    this.isDragging = false;
    if (this.elements.image) {
      this.elements.image.style.cursor = this.zoomLevel > 1 ? 'grab' : 'default';
    }
  }

  toggleFullscreen() {
    if (!document.fullscreenElement) {
      if (this.elements.viewer) {
        if (this.elements.viewer.requestFullscreen) {
          this.elements.viewer.requestFullscreen();
        } else if (this.elements.viewer.mozRequestFullScreen) {
          this.elements.viewer.mozRequestFullScreen();
        } else if (this.elements.viewer.webkitRequestFullscreen) {
          this.elements.viewer.webkitRequestFullscreen();
        } else if (this.elements.viewer.msRequestFullscreen) {
          this.elements.viewer.msRequestFullscreen();
        }
      }
    } else {
      if (document.exitFullscreen) {
        document.exitFullscreen();
      } else if (document.mozCancelFullScreen) {
        document.mozCancelFullScreen();
      } else if (document.webkitExitFullscreen) {
        document.webkitExitFullscreen();
      } else if (document.msExitFullscreen) {
        document.msExitFullscreen();
      }
    }
  }

  reset() {
    this.fitToScreen();
    this.gestureZoomLevel = 1;
    this.gestureBaseZoom = 1;
    this.zoomCenter = { x: 0, y: 0 };
  }

  getZoomLevel() {
    return this.zoomLevel;
  }

  // Gesture-based zoom methods
  startPinchZoom(scale, centerX, centerY) {
    this.gestureBaseZoom = this.zoomLevel;
    this.zoomCenter = { x: centerX, y: centerY };
  }

  updatePinchZoom(scale, deltaScale, centerX, centerY) {
    // Calculate new zoom level based on pinch scale
    const newZoom = this.gestureBaseZoom * scale;
    this.zoomLevel = Math.max(this.minZoom, Math.min(this.maxZoom, newZoom));

    // Update zoom center for this frame
    this.zoomCenter = { x: centerX, y: centerY };

    this.applyZoom();
  }

  endPinchZoom() {
    this.gestureBaseZoom = this.zoomLevel;
  }

  // Double-tap zoom
  doubleTapZoom(x, y) {
    const targetZoom = this.zoomLevel > 1 ? 1 : 2.5;
    this.animateZoomTo(targetZoom, x, y);
  }

  // Smooth zoom animation
  animateZoomTo(targetZoom, centerX = null, centerY = null) {
    if (this.isAnimating && this.animationFrame) {
      cancelAnimationFrame(this.animationFrame);
    }

    const startZoom = this.zoomLevel;
    const startX = this.imagePosition.x;
    const startY = this.imagePosition.y;
    const duration = 300; // ms
    const startTime = Date.now();

    // If center point provided, calculate target position
    let targetX = 0;
    let targetY = 0;

    if (centerX !== null && centerY !== null && targetZoom > 1) {
      const rect = this.elements.image.getBoundingClientRect();
      const relX = (centerX - rect.left) / rect.width;
      const relY = (centerY - rect.top) / rect.height;
      targetX = -relX * rect.width * (targetZoom - 1) * 0.5;
      targetY = -relY * rect.height * (targetZoom - 1) * 0.5;
    }

    this.isAnimating = true;

    const animate = () => {
      const elapsed = Date.now() - startTime;
      const progress = Math.min(elapsed / duration, 1);

      // Ease-out cubic
      const eased = 1 - Math.pow(1 - progress, 3);

      this.zoomLevel = startZoom + (targetZoom - startZoom) * eased;
      this.imagePosition.x = startX + (targetX - startX) * eased;
      this.imagePosition.y = startY + (targetY - startY) * eased;

      this.applyZoom();

      if (progress < 1) {
        this.animationFrame = requestAnimationFrame(animate);
      } else {
        this.isAnimating = false;
        this.gestureBaseZoom = this.zoomLevel;
      }
    };

    animate();
  }

  // Touch-based pan
  updateTouchPan(deltaX, deltaY) {
    if (this.zoomLevel <= 1) return;

    // Apply pan with boundary constraints
    const maxPanX = (this.elements.image.width * this.zoomLevel - this.elements.image.width) / 2;
    const maxPanY = (this.elements.image.height * this.zoomLevel - this.elements.image.height) / 2;

    this.imagePosition.x = Math.max(-maxPanX, Math.min(maxPanX, deltaX / this.zoomLevel));
    this.imagePosition.y = Math.max(-maxPanY, Math.min(maxPanY, deltaY / this.zoomLevel));

    this.applyZoom();
  }

  // Apply momentum to pan after release
  applyMomentum(velocityX, velocityY) {
    if (this.zoomLevel <= 1) return;

    const friction = 0.95;
    const minVelocity = 0.01;
    let vx = velocityX * 100; // Scale velocity
    let vy = velocityY * 100;

    const animate = () => {
      if (Math.abs(vx) < minVelocity && Math.abs(vy) < minVelocity) {
        return;
      }

      vx *= friction;
      vy *= friction;

      this.imagePosition.x += vx;
      this.imagePosition.y += vy;

      // Apply boundary constraints
      const maxPanX = (this.elements.image.width * this.zoomLevel - this.elements.image.width) / 2;
      const maxPanY =
        (this.elements.image.height * this.zoomLevel - this.elements.image.height) / 2;

      this.imagePosition.x = Math.max(-maxPanX, Math.min(maxPanX, this.imagePosition.x));
      this.imagePosition.y = Math.max(-maxPanY, Math.min(maxPanY, this.imagePosition.y));

      // Rubber band at edges
      if (Math.abs(this.imagePosition.x) >= maxPanX) {
        vx *= 0.5;
      }
      if (Math.abs(this.imagePosition.y) >= maxPanY) {
        vy *= 0.5;
      }

      this.applyZoom();
      this.animationFrame = requestAnimationFrame(animate);
    };

    animate();
  }

  isZoomed() {
    return this.zoomLevel > 1;
  }
}

window.ViewerControls = ViewerControls;
