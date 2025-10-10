class ViewerControls {
  constructor(viewer, elements) {
    this.viewer = viewer;
    this.elements = elements;

    this.zoomLevel = 1;
    this.maxZoom = 3;
    this.minZoom = 0.5;
    this.isDragging = false;
    this.dragStart = { x: 0, y: 0 };
    this.imagePosition = { x: 0, y: 0 };

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
  }

  getZoomLevel() {
    return this.zoomLevel;
  }
}

window.ViewerControls = ViewerControls;
