// SwipeableViewer - handles swipe navigation, vertical dismiss, and rubber-band effects
// Ported from static/js/viewer.js

/** Length of the slide that follows a completed horizontal swipe, in ms. */
const SLIDE_MS = 300;

export class SwipeableViewer {
  constructor(viewer) {
    this.viewer = viewer;
    this.animationFrame = null;
    this.currentTranslateX = 0;
    this.dragBaseTranslateX = 0;
    this.isDragging = false;
    this.isAnimating = false;
    this.touchStartPoint = null;
    // A swipe's navigation is decided in `handlePanEnd` and performed when the
    // slide ends; `flushPendingNavigation` is the one place that swap happens.
    this.pendingNavigation = null;
    this.pendingFromHash = null;
    this.navigationTimer = null;

    this.currentTranslateY = 0;
    this.isDraggingDown = false;
    this.currentScale = 1;
    this.currentOpacity = 1;
    this.rubberBandScale = 1;

    this.adjacent = {
      previous: null,
      next: null,
    };
  }

  /** Call after the viewer DOM elements are mounted */
  mount(elements) {
    this.elements = elements;
    this.adjacent.previous = this.createAdjacentImage('previous');
    this.adjacent.next = this.createAdjacentImage('next');
    this._onMainTouchStart = (event) => {
      if (!this.viewer.isOpen || event.touches.length !== 1 || !this.viewer.gestureManager) {
        return;
      }

      this.interruptAnimation();
      // A slide interrupted here will never reach its completion callback, so
      // the decision it parked has to be dropped with it: the armed timer would
      // otherwise fire mid-gesture, navigate, and reset `dragBaseTranslateX`
      // under the finger — the next move would then render `0 + deltaX`.
      this.cancelPendingNavigation();
      this.dragBaseTranslateX = this.currentTranslateX;
      const [touch] = event.touches;
      this.touchStartPoint = { x: touch.clientX, y: touch.clientY };
    };
    this._onMainTouchMove = (event) => {
      if (
        !this.viewer.isOpen ||
        event.touches.length !== 1 ||
        !this.viewer.gestureManager ||
        !this.touchStartPoint
      ) {
        return;
      }

      const [touch] = event.touches;
      const deltaX = touch.clientX - this.touchStartPoint.x;
      const deltaY = touch.clientY - this.touchStartPoint.y;
      const distance = Math.hypot(deltaX, deltaY);
      if (distance <= 10) {
        return;
      }

      if (this.viewer.controls?.isZoomed() || Math.abs(deltaX) > Math.abs(deltaY)) {
        this.viewer.gestureManager.startPan();
        // The GestureManager's touchmove listener runs BEFORE this one (it is
        // registered first via the Svelte action), so on this event it saw
        // activeGesture still null and could not prevent the native scroll.
        // Cancel it here — the pan-initiating move must not scroll the page.
        event.preventDefault();
      } else if (Math.abs(deltaY) > Math.abs(deltaX) && deltaY > 0) {
        this.viewer.gestureManager.startPan();
        event.preventDefault();
      }
    };
    this._onMainTouchEnd = () => {
      this.touchStartPoint = null;
    };
    this._onMainTouchCancel = () => {
      this.touchStartPoint = null;
      if (this.isDraggingDown) {
        this.snapBackVertical();
        return;
      }
      this.handleTouchCancel();
    };
    this.bindEvents();
    this.reset();
  }

  bindEvents() {
    if (!this.elements.main) return;

    this.elements.main.addEventListener('touchstart', this._onMainTouchStart, { passive: false });
    this.elements.main.addEventListener('touchmove', this._onMainTouchMove, { passive: false });
    this.elements.main.addEventListener('touchend', this._onMainTouchEnd, { passive: false });
    this.elements.main.addEventListener('touchcancel', this._onMainTouchCancel);
  }

  destroy() {
    this.interruptAnimation();
    // A slide that was still running carried a decided navigation. `$effect`
    // re-runs (a re-bound media element, a photos refresh) tear this viewer down
    // inside that window, and cancelling the frame alone would swallow a swipe
    // the user already made — the photo simply never changes. Flushing through
    // the same path as the slide's completion is what keeps it honest: it drops
    // a decision the user already overrode (a teardown caused by their own
    // navigation) instead of advancing a second time.
    void this.flushPendingNavigation();
    const main = this.elements?.main;
    if (main) {
      main.removeEventListener('touchstart', this._onMainTouchStart);
      main.removeEventListener('touchmove', this._onMainTouchMove);
      main.removeEventListener('touchend', this._onMainTouchEnd);
      main.removeEventListener('touchcancel', this._onMainTouchCancel);
    }
    this.adjacent.previous?.remove();
    this.adjacent.next?.remove();
    this.adjacent = { previous: null, next: null };
    this.elements = null;
  }

  createAdjacentImage(direction) {
    const image = document.createElement('img');
    image.className = `viewer-adjacent viewer-adjacent-${direction}`;
    image.alt = '';
    image.draggable = false;
    image.setAttribute('aria-hidden', 'true');
    this.elements.main?.prepend(image);
    return image;
  }

  reset() {
    // A reset does NOT drop a parked decision in general: this also runs when a
    // photo's media finishes loading, which can land inside the slide's window
    // for a slow decode, and cancelling there would throw away a swipe the user
    // already made. Closing the viewer is the exception — `close()` flips
    // `isOpen` before calling this, so a session that ended this way must not
    // let its decision fire if the same photo is reopened within the window.
    // Everything else is validated in `flushPendingNavigation`.
    if (!this.viewer.isOpen) {
      this.cancelPendingNavigation();
    } else if (
      this.pendingNavigation &&
      this.viewer.getCurrentPhoto()?.hash_sha256 !== this.pendingFromHash
    ) {
      // The photo changed while a decision was parked, so the user navigated on
      // their own (an arrow click, ArrowRight). Cancel now rather than at flush
      // time: navigating back to the source photo inside the same window would
      // satisfy the flush-time hash guard and replay a swipe already overridden.
      this.cancelPendingNavigation();
    }
    this.interruptAnimation();
    this.isDragging = false;
    this.isAnimating = false;
    this.dragBaseTranslateX = 0;
    this.isDraggingDown = false;
    this.currentTranslateY = 0;
    this.currentScale = 1;
    this.currentOpacity = 1;
    this.rubberBandScale = 1;
    this.render(0);
    this.hideAdjacent();
    this.updateAdjacentSources();
    this.toggleSwipeClass(false);
    this.viewer.gestureManager?.resetRecognizers?.();
  }

  handlePan(data) {
    if (!this.canHandleHorizontalPan()) {
      return false;
    }

    this.isDragging = true;
    this.toggleSwipeClass(true);
    this.updateAdjacentSources();

    const translateX = this.getDragTranslateX(data.deltaX);
    this.render(translateX);
    return true;
  }

  handlePanEnd(data) {
    if (!this.isDragging && this.currentTranslateX === 0) {
      return false;
    }

    this.toggleSwipeClass(false);

    const direction = this.getNavigationDirection(this.currentTranslateX);
    const shouldNavigate = this.shouldNavigate(this.currentTranslateX, data.velocityX);

    if (!direction || !shouldNavigate || !this.canNavigate(direction)) {
      this.snapBack();
      return true;
    }

    const targetX = direction === 'next' ? -this.getViewportWidth() : this.getViewportWidth();
    // The decision is taken here, but the photo swap waits for the slide. Two
    // things can swallow that window: a teardown cancels the frame (see
    // `destroy`), and a throttled frame loop never reaches the callback at all —
    // Chromium stops `requestAnimationFrame` entirely for a hidden or occluded
    // page, and the slide is the only thing that would have run. So the decision
    // is parked and flushed by whichever comes first, the slide's completion or
    // a timer just past its duration; `flushPendingNavigation` runs it once.
    this.pendingNavigation = direction;
    this.pendingFromHash = this.viewer.getCurrentPhoto()?.hash_sha256 ?? null;
    clearTimeout(this.navigationTimer);
    this.navigationTimer = setTimeout(() => void this.flushPendingNavigation(), SLIDE_MS + 50);
    this.animateTo(
      targetX,
      async () => {
        await this.flushPendingNavigation();
      },
      SLIDE_MS
    );

    return true;
  }

  handleTouchCancel() {
    if (!this.isDragging && this.currentTranslateX === 0 && !this.isAnimating) {
      return;
    }

    this.interruptAnimation();
    this.cancelPendingNavigation();
    this.toggleSwipeClass(false);
    this.finishInteraction();
    this.render(0);
  }

  handleVerticalPan(data) {
    if (!this.viewer.isOpen || !this.viewer.getCurrentPhoto()) {
      return false;
    }

    if (this.viewer.gestureManager?.gestureAxis !== 'vertical') {
      return false;
    }

    if (data.deltaY <= 0) {
      return false;
    }

    this.isDraggingDown = true;
    this.currentTranslateY = data.deltaY;
    this.currentScale = Math.max(0.7, 1 - Math.abs(data.deltaY) / 1000);
    this.currentOpacity = Math.max(0.5, 1 - Math.abs(data.deltaY) / 800);
    this.renderVertical();
    return true;
  }

  handleVerticalPanEnd(data) {
    if (!this.isDraggingDown) {
      return false;
    }

    const shouldDismiss = this.currentTranslateY >= 150 || data.velocityY > 0.5;

    if (shouldDismiss) {
      this.animateDismiss();
    } else {
      this.snapBackVertical();
    }

    return true;
  }

  animateDismiss() {
    this.interruptAnimation();

    const startY = this.currentTranslateY;
    const startScale = this.currentScale;
    const startOpacity = this.currentOpacity;
    const targetY = window.innerHeight;
    const duration = 250;
    const startTime = Date.now();
    this.isAnimating = true;

    const animate = () => {
      const elapsed = Date.now() - startTime;
      const progress = Math.min(elapsed / duration, 1);
      const eased = progress * progress;

      this.currentTranslateY = startY + (targetY - startY) * eased;
      this.currentScale = startScale + (0.7 - startScale) * eased;
      this.currentOpacity = startOpacity + (0 - startOpacity) * eased;
      this.renderVertical();

      if (progress < 1) {
        this.animationFrame = requestAnimationFrame(animate);
        return;
      }

      this.animationFrame = null;
      this.isAnimating = false;
      this.resetVerticalState();
      this.viewer.triggerHapticFeedback('medium');
      this.viewer.close();
    };

    animate();
  }

  snapBackVertical() {
    this.interruptAnimation();

    const startY = this.currentTranslateY;
    const startScale = this.currentScale;
    const startOpacity = this.currentOpacity;
    const duration = 300;
    const startTime = Date.now();
    this.isAnimating = true;

    const animate = () => {
      const elapsed = Date.now() - startTime;
      const progress = Math.min(elapsed / duration, 1);
      const eased = 1 - Math.pow(1 - progress, 3);

      this.currentTranslateY = startY + (0 - startY) * eased;
      this.currentScale = startScale + (1 - startScale) * eased;
      this.currentOpacity = startOpacity + (1 - startOpacity) * eased;
      this.renderVertical();

      if (progress < 1) {
        this.animationFrame = requestAnimationFrame(animate);
        return;
      }

      this.animationFrame = null;
      this.isAnimating = false;
      this.resetVerticalState();
    };

    animate();
  }

  resetVerticalState() {
    this.isDraggingDown = false;
    this.currentTranslateY = 0;
    this.currentScale = 1;
    this.currentOpacity = 1;
    this.rubberBandScale = 1;
    this.renderVertical();
  }

  renderVertical() {
    const activeMedia = this.getActiveMediaElement();
    if (!activeMedia) {
      return;
    }

    const combinedScale = this.currentScale * this.rubberBandScale;
    activeMedia.style.transform = `translateX(${this.currentTranslateX}px) translateY(${this.currentTranslateY}px) scale(${combinedScale})`;
    activeMedia.style.opacity = `${this.currentOpacity}`;
  }

  startZoomEdgeSwipe(data) {
    this.viewer.controls.reset();
    this.dragBaseTranslateX = 0;
    return this.handlePan(data);
  }

  canHandleHorizontalPan() {
    if (!this.viewer.isOpen || !this.viewer.getCurrentPhoto()) {
      return false;
    }

    return this.viewer.gestureManager?.gestureAxis === 'horizontal';
  }

  shouldNavigate(translateX, velocityX) {
    const threshold = this.getViewportWidth() * 0.3;
    return Math.abs(translateX) >= threshold || Math.abs(velocityX) > 0.3;
  }

  getDragTranslateX(deltaX) {
    const translateX = this.dragBaseTranslateX + deltaX;

    if (translateX > 0 && !this.canNavigate('previous')) {
      const resistedX = translateX * 0.3;
      this.rubberBandScale = Math.max(0.9, 1 - Math.abs(resistedX) * 0.0005);
      return resistedX;
    }

    if (translateX < 0 && !this.canNavigate('next')) {
      const resistedX = translateX * 0.3;
      this.rubberBandScale = Math.max(0.9, 1 - Math.abs(resistedX) * 0.0005);
      return resistedX;
    }

    this.rubberBandScale = 1;
    return translateX;
  }

  getNavigationDirection(translateX) {
    if (translateX < 0) {
      return 'next';
    }

    if (translateX > 0) {
      return 'previous';
    }

    return null;
  }

  canNavigate(direction) {
    if (direction === 'previous') {
      return this.viewer.currentIndex > 0;
    }

    if (direction === 'next') {
      return this.viewer.currentIndex < this.viewer.photos.length - 1;
    }

    return false;
  }

  getViewportWidth() {
    return window.innerWidth || this.elements.main?.clientWidth || 0;
  }

  snapBack() {
    const isRubberBand = this.rubberBandScale !== 1;
    const duration = isRubberBand ? 400 : 300;
    this.animateTo(0, () => this.finishInteraction(), duration);
  }

  animateTo(targetX, onComplete = null, duration = 300) {
    this.interruptAnimation();

    const startX = this.currentTranslateX;
    const startRubberBandScale = this.rubberBandScale;
    const startTime = Date.now();
    this.isAnimating = true;

    const animate = () => {
      const elapsed = Date.now() - startTime;
      const progress = Math.min(elapsed / duration, 1);
      const eased = 1 - Math.pow(1 - progress, 3);
      const translateX = startX + (targetX - startX) * eased;

      this.rubberBandScale = startRubberBandScale + (1 - startRubberBandScale) * eased;
      this.render(translateX);

      if (progress < 1) {
        this.animationFrame = requestAnimationFrame(animate);
        return;
      }

      this.animationFrame = null;
      this.isAnimating = false;

      if (onComplete) {
        onComplete();
      }
    };

    animate();
  }

  interruptAnimation() {
    if (!this.animationFrame) {
      return;
    }

    cancelAnimationFrame(this.animationFrame);
    this.animationFrame = null;
    this.isAnimating = false;
  }

  async navigate(direction) {
    this.finishInteraction();

    if (direction === 'next') {
      await this.viewer.showNext();
    }

    if (direction === 'previous') {
      await this.viewer.showPrevious();
    }

    // destroy() nulls this.elements/this.adjacent while the media load is
    // pending; the continuation below must not dereference them.
    if (!this.elements) return;

    this.viewer.triggerHapticFeedback('light');
    this.render(0);
    this.updateAdjacentSources();
  }

  /**
   * Runs the navigation a finished swipe decided on, exactly once.
   *
   * Two callers race for it: the slide's completion and `destroy()`. Whoever
   * gets there first clears the parked decision, so a teardown can never both
   * flush it and let the animation flush it again.
   */
  async flushPendingNavigation() {
    const direction = this.pendingNavigation;
    const fromHash = this.pendingFromHash;
    this.cancelPendingNavigation();
    if (!direction) return;
    // A decision belongs to the interaction that produced it. The viewer may
    // have closed, or another navigation may have landed (an arrow click or a
    // key press inside the slide's window); navigating now would advance from a
    // photo the user already left, or rewrite the URL behind a closed viewer.
    if (!this.viewer.isOpen || this.viewer.getCurrentPhoto()?.hash_sha256 !== fromHash) {
      // Stale decision: the viewer closed, or another navigation landed inside
      // the slide's window. The slide already rendered the off-screen transform
      // and made the neighbour visible, and nothing else undoes that until the
      // next photo's media loads (never, if that load errors) — so drop the
      // transform with the decision.
      this.interruptAnimation();
      this.finishInteraction();
      this.render(0);
      return;
    }
    // The timer exists for the race the slide can lose — a throttled or
    // occluded frame loop, or a stall near the end. When it wins, the
    // still-queued frame would land after `navigate()`'s `render(0)` with
    // `progress === 1` and call `render(targetX)` again, leaving the viewer
    // translated off-screen with the next photo's adjacent element showing.
    // A no-op on the normal path, where completion already cleared the frame.
    this.interruptAnimation();
    await this.navigate(direction);
  }

  /**
   * Drops a parked decision and its timer.
   *
   * Only call this when the slide carrying the decision is not going to
   * complete — an interrupted animation, a touch cancel, or a flush that is
   * taking the decision now. On the normal completion path the flush itself
   * consumes it.
   */
  cancelPendingNavigation() {
    clearTimeout(this.navigationTimer);
    this.navigationTimer = null;
    this.pendingNavigation = null;
    this.pendingFromHash = null;
  }

  finishInteraction() {
    this.isDragging = false;
    this.dragBaseTranslateX = 0;
    this.isAnimating = false;
    this.isDraggingDown = false;
    this.currentTranslateY = 0;
    this.currentScale = 1;
    this.currentOpacity = 1;
    this.rubberBandScale = 1;
    this.toggleSwipeClass(false);
    this.hideAdjacent();
  }

  render(translateX) {
    this.currentTranslateX = translateX;

    const activeMedia = this.getActiveMediaElement();
    if (activeMedia) {
      const combinedScale = this.currentScale * this.rubberBandScale;
      activeMedia.style.transform = `translateX(${translateX}px) translateY(${this.currentTranslateY}px) scale(${combinedScale})`;
      activeMedia.style.opacity = `${this.currentOpacity}`;
    }

    this.renderAdjacent(translateX);
  }

  renderAdjacent(translateX) {
    const width = this.getViewportWidth();
    const direction = this.getNavigationDirection(translateX);

    if (this.adjacent.previous) {
      this.adjacent.previous.style.transform = `translateX(${translateX - width}px)`;
      this.adjacent.previous.classList.toggle(
        'visible',
        translateX > 0 &&
          direction === 'previous' &&
          this.adjacent.previous.dataset.available === 'true'
      );
    }
    if (this.adjacent.next) {
      this.adjacent.next.style.transform = `translateX(${translateX + width}px)`;
      this.adjacent.next.classList.toggle(
        'visible',
        translateX < 0 && direction === 'next' && this.adjacent.next.dataset.available === 'true'
      );
    }
  }

  updateAdjacentSources() {
    this.updateAdjacentSource('previous', this.viewer.photos[this.viewer.currentIndex - 1]);
    this.updateAdjacentSource('next', this.viewer.photos[this.viewer.currentIndex + 1]);
  }

  updateAdjacentSource(direction, photo) {
    const element = this.adjacent[direction];
    const source = this.getAdjacentSource(photo);

    element.dataset.available = source ? 'true' : 'false';
    element.src = source || '';
    element.classList.remove('visible');
  }

  getAdjacentSource(photo) {
    if (!photo || this.viewer.isVideoFile(photo.filename)) {
      return null;
    }

    const preloaded = this.viewer.preloadedImages.get(photo.hash_sha256);
    return preloaded?.src || this.viewer.getMediaUrl(photo);
  }

  getActiveMediaElement() {
    if (this.elements.image?.style.display !== 'none') {
      return this.elements.image;
    }

    if (this.elements.video?.style.display !== 'none') {
      return this.elements.video;
    }

    return this.elements.image || this.elements.video;
  }

  hideAdjacent() {
    this.adjacent.previous?.classList.remove('visible');
    this.adjacent.next?.classList.remove('visible');
  }

  toggleSwipeClass(active) {
    this.elements.image?.classList.toggle('swiping', active);
    this.elements.video?.classList.toggle('swiping', active);
  }
}
