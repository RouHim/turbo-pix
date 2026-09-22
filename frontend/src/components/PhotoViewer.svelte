<script>
  import { get } from 'svelte/store';
  import { t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import { route, pushState, replaceState } from '../lib/router.svelte.js';
  import { photoGridState } from '../lib/state.svelte.js';
  import { addToast } from '../lib/state.svelte.js';
  import {
    getPhotoUrl,
    getVideoUrl,
    isCollagePhoto,
    isRawFile,
    isVideoFile,
    showToast,
    videoCodecSupport,
  } from '../lib/utils.js';
  import { logger } from '../lib/logger.js';
  import { createStreamPlayer, mseSupported } from '../lib/video/msePlayer.js';
  import { isHardwareEncoder } from '../lib/video/encoderHint.js';
  import { gestures } from '../lib/gestures/action.js';
  import { SwipeableViewer } from '../lib/viewer/SwipeableViewer.js';
  import Icon from './Icon.svelte';
  import AlbumPicker from './AlbumPicker.svelte';
  import ViewerControls from './ViewerControls.svelte';
  import ViewerMetadata from './ViewerMetadata.svelte';
  import ViewerMetadataEdit from './ViewerMetadataEdit.svelte';

  // ── State ──────────────────────────────────────────────────────────────────
  let isOpen = $state(false);
  let currentPhoto = $state(null);
  let photos = $state([]);
  let currentIndex = $state(0);
  const preloadedImages = $state(new Map());
  let showSidebar = $state(false);
  let updateUrlEnabled = $state(true);
  // True while the viewer's own history entry (pushed by open()) is the
  // current entry; close() then pops it via history.back() instead of
  // replaceState. Plain field — never read in the template.
  let viewerPushedEntry = false;

  // Loading / video
  let isLoading = $state(false);
  let transcodeMessage = $state('');
  let transcodeError = $state(false);
  // The encoder serving the current playback, when the server video-encodes it.
  // `null` means "not video-encoded" (direct play, remux, or a copied video
  // track), which shows no hint at all.
  let activeEncoder = $state(null);
  const encoderIsHardware = $derived(isHardwareEncoder(activeEncoder));
  const encoderHintLabel = $derived(
    activeEncoder
      ? encoderIsHardware
        ? get(t)('video.encoder.gpu', { values: { encoder: activeEncoder } })
        : get(t)('video.encoder.cpu', { values: { encoder: activeEncoder } })
      : ''
  );
  // A stream run is waiting for a free conversion slot (503): not an error, but
  // the user must keep the "play original anyway" escape hatch. Rendered, so a
  // reactive state.
  let streamWaiting = $state(false);
  // Set when the user selected "Play original anyway" after a transcode
  // failure; suppresses the onerror transcode-retry so a failing original
  // cannot loop back into the transcode decision. Logic-only (never rendered).
  let hasUserChosenOriginal = false;
  // Active MSE stream playback (Task 3). A plain field, not `$state`: the
  // object holds MediaSource/DOM references and closures that must never be
  // proxied, and nothing renders from it.
  let streamPlayer = null;
  // Single-flight timer for retrying a stream run the server refused because
  // every conversion slot was taken. Plain field, torn down with the player and
  // by any newer run or user seek (see `disarmStreamRetry`).
  let streamRetryTimer = null;
  // The newest position the user asked for, in seconds: the offset of every run
  // the player begins (`onRunStart`) and every position picked on the element
  // itself (`onUserSeek`), whichever came last. A refusal re-arms the retry
  // here rather than at the refused run's own offset, because a run can be held
  // by the server for its whole `TURBO_PIX_STREAM_QUEUE_WAIT_SECS` (20 s) while
  // the user scrubs somewhere else, and that seek reaches the viewer only
  // through these callbacks (see `rememberStreamIntent`).
  let streamIntentOffset = 0;

  // Collage
  let isPendingCollage = $state(false);
  let isAcceptingCollage = $state(false);

  // Album picker: hashes are captured synchronously at open time so later
  // navigation cannot retarget the add (viewer staleness rule).
  let pickerOpen = $state(false);
  let pickerHashes = $state([]);

  function openAlbumPicker() {
    if (!currentPhoto || isCollagePhoto(currentPhoto)) return;
    pickerHashes = [currentPhoto.hash_sha256];
    pickerOpen = true;
  }

  // Rotation state (raw/video disable)
  let rotationDisabled = $state(false);
  let rotationDisabledTitle = $state('');

  // ── DOM refs ───────────────────────────────────────────────────────────────
  let viewerEl = $state(null);
  let imageEl = $state(null);
  let videoEl = $state(null);
  let mainEl = $state(null);
  // Element focused before the viewer opened; restored on close.
  let previouslyFocusedElement = null;

  // ── Zoom / Pan state (ported from ViewerControls) ──────────────────────────
  let zoomLevel = $state(1);
  const maxZoom = 5;
  const minZoom = 0.5;
  let isDragging = $state(false);
  let dragStart = $state({ x: 0, y: 0 });
  let imagePosition = $state({ x: 0, y: 0 });
  let gestureBaseZoom = $state(1);
  let zoomAnimFrame = null;
  let isZoomAnimating = false;
  let transcodePollTimer = null;
  let pinchStarted = false;

  // ── Gesture manager (created by use:gestures action) ───────────────────────
  let swipeableViewer = null;
  // gestureManager is set on mainEl.__gestureManager by the action; accessed via getter
  let metadataEditRef = $state(null);

  // ── Helpers ────────────────────────────────────────────────────────────────
  function getMediaUrl(photo) {
    if (!photo) return null;
    if (isCollagePhoto(photo)) {
      return photo.path || photo.thumbnail_path || null;
    }
    return getPhotoUrl(photo.hash_sha256);
  }

  const hasPrev = $derived(currentIndex > 0);
  const hasNext = $derived(currentIndex < photos.length - 1);
  const isVideo = $derived(currentPhoto ? isVideoFile(currentPhoto.filename) : false);
  const isCollage = $derived(currentPhoto ? isCollagePhoto(currentPhoto) : false);
  const isFavorite = $derived(currentPhoto ? api.isFavorite(currentPhoto) : false);

  // ── Zoom helpers ───────────────────────────────────────────────────────────
  function applyZoom() {
    if (!imageEl) return;
    const transform = `scale(${zoomLevel}) translate(${imagePosition.x}px, ${imagePosition.y}px)`;
    imageEl.style.transform = transform;
    imageEl.classList.toggle('zoomed', zoomLevel > 1);
  }

  function zoomIn() {
    if (isVideo) return;
    if (zoomLevel < maxZoom) {
      zoomLevel = Math.min(zoomLevel * 1.5, maxZoom);
      applyZoom();
    }
  }

  function zoomOut() {
    if (isVideo) return;
    if (zoomLevel > minZoom) {
      zoomLevel = Math.max(zoomLevel / 1.5, minZoom);
      applyZoom();
    }
  }

  function fitToScreen() {
    zoomLevel = 1;
    imagePosition = { x: 0, y: 0 };
    applyZoom();
  }

  function resetZoom() {
    // Stop any in-flight zoom/momentum animation: displayPhoto calls resetZoom
    // for the new photo, and the rAF loop must not keep writing stale zoom/
    // transform state afterwards.
    if (zoomAnimFrame) {
      cancelAnimationFrame(zoomAnimFrame);
      zoomAnimFrame = null;
      isZoomAnimating = false;
    }
    fitToScreen();
    gestureBaseZoom = 1;
  }

  function isZoomed() {
    return zoomLevel > 1;
  }

  function isAtPanBoundary() {
    if (!imageEl || zoomLevel <= 1) return { left: false, right: false };
    const maxPanX = (imageEl.width * zoomLevel - imageEl.width) / 2;
    const tolerance = 0.5;
    if (maxPanX <= 0) return { left: true, right: true };
    return {
      left: imagePosition.x >= maxPanX - tolerance,
      right: imagePosition.x <= -maxPanX + tolerance,
    };
  }

  // ── Mouse drag ─────────────────────────────────────────────────────────────
  function startDrag(e) {
    if (zoomLevel <= 1) return;
    e.preventDefault();
    isDragging = true;
    dragStart = {
      x: e.clientX - imagePosition.x,
      y: e.clientY - imagePosition.y,
    };
    if (imageEl) imageEl.style.cursor = 'grabbing';
  }

  function onDrag(e) {
    if (!isDragging || zoomLevel <= 1) return;
    e.preventDefault();
    imagePosition = {
      x: e.clientX - dragStart.x,
      y: e.clientY - dragStart.y,
    };
    applyZoom();
  }

  function endDrag() {
    if (!isDragging) return;
    isDragging = false;
    if (imageEl) imageEl.style.cursor = zoomLevel > 1 ? 'grab' : 'default';
  }

  // ── Fullscreen ─────────────────────────────────────────────────────────────
  function toggleFullscreen() {
    if (!viewerEl) return;
    if (!document.fullscreenElement) {
      if (viewerEl.requestFullscreen) viewerEl.requestFullscreen();
      else if (viewerEl.webkitRequestFullscreen) viewerEl.webkitRequestFullscreen();
    } else {
      if (document.exitFullscreen) document.exitFullscreen();
      else if (document.webkitExitFullscreen) document.webkitExitFullscreen();
    }
  }

  // ── Pinch zoom (called by gesture callbacks) ───────────────────────────────
  function startPinchZoom() {
    gestureBaseZoom = zoomLevel;
  }

  function updatePinchZoom(scale) {
    const newZoom = gestureBaseZoom * scale;
    zoomLevel = Math.max(minZoom, Math.min(maxZoom, newZoom));
    applyZoom();
  }

  function endPinchZoom() {
    gestureBaseZoom = zoomLevel;
  }

  // ── Double-tap zoom ────────────────────────────────────────────────────────
  function doubleTapZoom(x, y) {
    if (isVideo) return;
    const targetZoom = zoomLevel > 1 ? 1 : 2.5;
    animateZoomTo(targetZoom, x, y);
  }

  function animateZoomTo(targetZoom, centerX = null, centerY = null) {
    if (isZoomAnimating && zoomAnimFrame) {
      cancelAnimationFrame(zoomAnimFrame);
    }
    const startZoom = zoomLevel;
    const startX = imagePosition.x;
    const startY = imagePosition.y;
    const duration = 300;
    const startTime = Date.now();
    let targetX = 0;
    let targetY = 0;

    if (centerX !== null && centerY !== null && targetZoom > 1 && imageEl) {
      const rect = imageEl.getBoundingClientRect();
      const relX = (centerX - rect.left) / rect.width;
      const relY = (centerY - rect.top) / rect.height;
      targetX = -relX * rect.width * (targetZoom - 1) * 0.5;
      targetY = -relY * rect.height * (targetZoom - 1) * 0.5;
    }

    isZoomAnimating = true;

    const animate = () => {
      const elapsed = Date.now() - startTime;
      const progress = Math.min(elapsed / duration, 1);
      const eased = 1 - Math.pow(1 - progress, 3);

      zoomLevel = startZoom + (targetZoom - startZoom) * eased;
      imagePosition = {
        x: startX + (targetX - startX) * eased,
        y: startY + (targetY - startY) * eased,
      };
      applyZoom();

      if (progress < 1) {
        zoomAnimFrame = requestAnimationFrame(animate);
      } else {
        isZoomAnimating = false;
        gestureBaseZoom = zoomLevel;
      }
    };
    animate();
  }

  // ── Touch-based pan ────────────────────────────────────────────────────────
  function updateTouchPan(deltaX, deltaY) {
    if (zoomLevel <= 1 || !imageEl) return;
    const maxPanX = (imageEl.width * zoomLevel - imageEl.width) / 2;
    const maxPanY = (imageEl.height * zoomLevel - imageEl.height) / 2;
    imagePosition = {
      x: Math.max(-maxPanX, Math.min(maxPanX, deltaX / zoomLevel)),
      y: Math.max(-maxPanY, Math.min(maxPanY, deltaY / zoomLevel)),
    };
    applyZoom();
  }

  function applyMomentum(velocityX, velocityY) {
    if (zoomLevel <= 1 || !imageEl) return;
    const friction = 0.95;
    const minVelocity = 0.01;
    let vx = velocityX * 100;
    let vy = velocityY * 100;

    const animate = () => {
      if (Math.abs(vx) < minVelocity && Math.abs(vy) < minVelocity) return;

      vx *= friction;
      vy *= friction;

      imagePosition = {
        x: imagePosition.x + vx,
        y: imagePosition.y + vy,
      };

      const maxPanX = (imageEl.width * zoomLevel - imageEl.width) / 2;
      const maxPanY = (imageEl.height * zoomLevel - imageEl.height) / 2;
      imagePosition = {
        x: Math.max(-maxPanX, Math.min(maxPanX, imagePosition.x)),
        y: Math.max(-maxPanY, Math.min(maxPanY, imagePosition.y)),
      };

      if (Math.abs(imagePosition.x) >= maxPanX) vx *= 0.5;
      if (Math.abs(imagePosition.y) >= maxPanY) vy *= 0.5;

      applyZoom();
      zoomAnimFrame = requestAnimationFrame(animate);
    };
    animate();
  }

  // ── Gesture handlers ───────────────────────────────────────────────────────
  function onPinch(data) {
    const { scale } = data;
    if (!pinchStarted) {
      startPinchZoom();
      pinchStarted = true;
      if (imageEl) imageEl.classList.add('gesture-active');
      if (videoEl) videoEl.classList.add('gesture-active');
    }
    updatePinchZoom(scale);
  }

  function onPinchEnd() {
    endPinchZoom();
    pinchStarted = false;
    if (imageEl) imageEl.classList.remove('gesture-active');
    if (videoEl) videoEl.classList.remove('gesture-active');
  }

  function onDoubleTap(data) {
    const { x, y } = data;
    doubleTapZoom(x, y);
  }

  /**
   * Handles a pan while zoomed: pans the image, but at the zoom edge the
   * pan becomes a horizontal swipe to the adjacent photo. Returns true when
   * the pan was consumed (zoomed state), false to let the swipe viewer
   * handle it.
   */
  function handleZoomedPan(data) {
    const { deltaX, deltaY } = data;
    const panBoundary = isAtPanBoundary();
    const isHorizontalSwipe = mainEl?.__gestureManager?.gestureAxis === 'horizontal';
    const isSwipeToPrevious = isHorizontalSwipe && deltaX > 0 && panBoundary.left;
    const isSwipeToNext = isHorizontalSwipe && deltaX < 0 && panBoundary.right;

    if (isSwipeToPrevious || isSwipeToNext) {
      if (swipeableViewer?.startZoomEdgeSwipe(data)) {
        if (imageEl) imageEl.classList.remove('gesture-active');
        return true;
      }
    }

    updateTouchPan(deltaX, deltaY);
    if (imageEl) imageEl.classList.add('gesture-active');
    return true;
  }

  function onPan(data) {
    if (isZoomed()) {
      handleZoomedPan(data);
      return;
    }

    if (swipeableViewer?.handleVerticalPan(data)) {
      return;
    }

    swipeableViewer?.handlePan(data);
  }

  function onPanEnd(data) {
    const { velocityX, velocityY } = data;

    if (imageEl) imageEl.classList.remove('gesture-active');

    if (swipeableViewer?.handleVerticalPanEnd(data)) {
      return;
    }

    if (swipeableViewer?.handlePanEnd(data)) {
      return;
    }

    if (isZoomed()) {
      applyMomentum(velocityX, velocityY);
    }
  }

  // ── Haptic ─────────────────────────────────────────────────────────────────
  function triggerHapticFeedback(intensity = 'light') {
    if ('vibrate' in navigator) {
      const patterns = { light: 10, medium: 20, heavy: 50 };
      navigator.vibrate(patterns[intensity] || 10);
    }
  }

  // ── Core viewer API ────────────────────────────────────────────────────────
  async function open(photo, allPhotos = [], pushEntry = true) {
    photos = allPhotos;
    currentIndex = photos.findIndex((p) => p.hash_sha256 === photo.hash_sha256);
    if (currentIndex === -1) {
      photos = [photo];
      currentIndex = 0;
    }

    currentPhoto = photo;
    isPendingCollage = isCollagePhoto(photo) && isPendingCollagePhoto(photo);
    isAcceptingCollage = false;
    isOpen = true;
    updateUrlEnabled = !isCollagePhoto(photo);

    if (viewerEl) {
      if (imageEl) imageEl.style.viewTransitionName = 'viewer-image';
      const openAction = () => {
        // The callback runs on the next frame; if the viewer was already
        // closed in between (Escape within the deferral window), don't reopen.
        if (!isOpen) return;
        viewerEl.classList.add('active', 'fade-in');
        document.body.style.overflow = 'hidden';
      };
      if (document.startViewTransition) {
        document.startViewTransition(openAction);
      } else {
        openAction();
      }
    }

    // Dialog semantics: move focus into the viewer on open, restore on close.
    previouslyFocusedElement =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (viewerEl) viewerEl.focus({ preventScroll: true });

    showSidebar = false;

    if (updateUrlEnabled) {
      viewerPushedEntry = pushEntry;
      if (pushEntry) {
        pushState({ photo: photo.hash_sha256 });
      }
    }

    await displayPhoto(photo);
    updateRotationState();

    preloadAdjacentPhotos();
  }

  function close(updateUrl = true) {
    if (zoomAnimFrame) {
      cancelAnimationFrame(zoomAnimFrame);
      zoomAnimFrame = null;
      isZoomAnimating = false;
    }
    // Stop the transcode poll immediately instead of waiting for the next
    // tick's bailIfStale: reopening the same photo within that window would
    // let the old poll run alongside the new one (same hash + viewer open
    // again means it is not stale) and act on the new session. The interval
    // id is shared across polls — clear only the interval this viewer owns,
    // and only null the shared field / hide the shared toast while it still
    // points at us.
    const pollTimer = transcodePollTimer;
    if (pollTimer !== null) {
      clearInterval(pollTimer);
      if (transcodePollTimer === pollTimer) {
        transcodePollTimer = null;
        hideTranscodeToast();
      }
    } else {
      hideTranscodeToast();
    }
    // Stop the MSE stream (and its fetch) too: a hidden viewer must not keep
    // pulling conversion bytes, and the blob URL must be released.
    destroyStreamPlayer();
    metadataEditRef?.close?.();
    isOpen = false;
    isPendingCollage = false;
    isAcceptingCollage = false;
    // Nothing plays behind a closed viewer, so nothing is being encoded.
    activeEncoder = null;

    if (viewerEl) {
      if (imageEl) imageEl.style.viewTransitionName = '';
      viewerEl.classList.remove('active', 'fade-in');
      document.body.style.overflow = '';
    }

    if (previouslyFocusedElement?.isConnected) {
      previouslyFocusedElement.focus({ preventScroll: true });
    }
    previouslyFocusedElement = null;

    showSidebar = false;
    swipeableViewer?.reset();

    if (videoEl) videoEl.pause();

    preloadedImages.clear();

    if (updateUrl) {
      if (viewerPushedEntry) {
        viewerPushedEntry = false;
        window.history.back();
      } else {
        replaceState({ photo: null });
      }
    } else {
      viewerPushedEntry = false;
    }
  }

  async function showPrevious() {
    if (currentIndex > 0) {
      currentIndex--;
      await showPhotoAtIndex(currentIndex);
    }
  }

  async function showNext() {
    if (currentIndex < photos.length - 1) {
      currentIndex++;
      await showPhotoAtIndex(currentIndex);
    }
  }

  async function showPhotoAtIndex(index, updateUrl = true) {
    if (index < 0 || index >= photos.length) return;
    currentIndex = index;
    currentPhoto = photos[index];
    isPendingCollage = isCollagePhoto(currentPhoto) && isPendingCollagePhoto(currentPhoto);
    isAcceptingCollage = false;

    if (updateUrl && updateUrlEnabled) {
      replaceState({ photo: currentPhoto.hash_sha256 });
    }

    await displayPhoto(currentPhoto);
    updateRotationState();
    preloadAdjacentPhotos();
  }

  async function displayPhoto(photo) {
    // Every photo change invalidates the previous photo's stream state. The
    // MSE player's pump would otherwise keep pulling conversion bytes — the
    // server keeps an ffmpeg process holding a conversion permit for a video
    // no longer on screen — and its waiting notice would hover over an
    // unrelated photo, whose "play original" button would then play the
    // image's URL in the <video> element. displayVideo's own teardown cannot
    // run for the OLD video: its staleness guard bails. Both calls are
    // idempotent, so the video paths (which also tear down) stay safe.
    destroyStreamPlayer();
    hideTranscodeToast();
    // The previous photo's encoder claim dies with its playback: an image (or
    // any later branch) must never inherit it, and every video path below
    // states its own answer.
    activeEncoder = null;

    resetZoom();
    isLoading = true;

    if (imageEl) {
      imageEl.classList.remove('loaded');
      imageEl.style.display = 'none';
    }
    if (videoEl) {
      videoEl.pause();
      videoEl.classList.remove('loaded');
      videoEl.style.display = 'none';
      // The src stays (cleared only by setVideoSource); mark it stale so the
      // Space shortcut never plays the previous photo's video over this one.
      delete videoEl.dataset.photoHash;
    }

    try {
      if (isVideoFile(photo.filename)) {
        await displayVideo(photo);
      } else {
        await displayImage(photo);
      }
    } catch (error) {
      logger?.error('Error displaying photo', error, {
        component: 'PhotoViewer',
        photoHash: photo.hash_sha256,
        filename: photo.filename,
      });
      showToast(
        get(t)('notifications.error', { default: 'Error' }),
        get(t)('errors.failedToLoadPhoto', { default: 'Failed to load photo' }),
        'error'
      );
    } finally {
      // A stale poll (photo already superseded) must not hide the new photo's spinner.
      if (currentPhoto?.hash_sha256 === photo.hash_sha256) {
        isLoading = false;
      }
    }
  }

  async function displayImage(photo) {
    const imageUrl = getMediaUrl(photo);
    if (!imageUrl) {
      showToast(
        get(t)('notifications.error', { default: 'Error' }),
        get(t)('errors.failedToLoadImage', { default: 'Failed to load image' }),
        'error'
      );
      return;
    }

    if (preloadedImages.has(photo.hash_sha256)) {
      const img = preloadedImages.get(photo.hash_sha256);
      showImage(img.src);
      return;
    }

    const img = new Image();
    img.onload = () => {
      preloadedImages.set(photo.hash_sha256, img);
      // A newer photo may have been requested while this image was loading;
      // only display it if it is still the current one.
      if (currentPhoto?.hash_sha256 === photo.hash_sha256) showImage(img.src);
    };
    img.onerror = () => {
      // A newer photo may have been requested while this image was loading.
      if (currentPhoto?.hash_sha256 !== photo.hash_sha256) return;
      showToast(
        get(t)('notifications.error', { default: 'Error' }),
        get(t)('errors.failedToLoadImage', { default: 'Failed to load image' }),
        'error'
      );
    };
    img.src = imageUrl;
  }

  function showImage(src) {
    if (imageEl) {
      imageEl.src = src;
      imageEl.style.transform = '';
      imageEl.style.opacity = '';
      imageEl.style.display = 'block';
      imageEl.classList.add('loaded');
      if (videoEl) {
        videoEl.style.transform = '';
        videoEl.style.opacity = '';
        videoEl.style.display = 'none';
      }
    }
    swipeableViewer?.reset();
  }

  /**
   * Kicks off a server-side transcode for a video the browser cannot play.
   * Returns true when a transcode flow was started (polling completes via
   * pollTranscodeStatus), a hard failure was reported, or the photo went
   * stale — the caller must not set the video source in any of these cases.
   */
  async function tryStartTranscode(videoUrl, photo) {
    try {
      const response = await fetch(videoUrl);
      // The viewer may have been closed, or a newer photo requested, while the
      // transcode was starting — a hidden viewer must not show a transcode toast.
      if (!isOpen || currentPhoto?.hash_sha256 !== photo.hash_sha256) return true;
      if (response.status === 202) {
        const data = await response.json();
        // The viewer may have been closed, or a newer photo requested, while
        // the transcode JSON was loading — a stale viewer must not toast.
        if (!isOpen || currentPhoto?.hash_sha256 !== photo.hash_sha256) return true;
        const pollUrl = data.poll_url;
        showTranscodeToast(
          get(t)('video.transcoding.started', {
            default: 'Video is being converted for playback...',
          })
        );
        await pollTranscodeStatus(pollUrl, photo);
        return true;
      }
      const warningHeader = response.headers.get('X-Transcode-Warning');
      if (warningHeader && warningHeader.trim() !== '') {
        showTranscodeToast(
          get(t)('video.transcoding.failed', { default: 'Video conversion failed' }),
          true
        );
        showToast(
          get(t)('notifications.error', { default: 'Error' }),
          get(t)('video.transcoding.failed', { default: 'Video conversion failed' }),
          'error'
        );
        return true;
      }
    } catch {
      /* ignore */
    }
    return false;
  }

  async function displayVideo(photo, forceTranscode = false) {
    if (!videoEl) return;
    // A different video may be on screen: its stream (and any pending
    // SourceBuffer, or an armed saturation retry) belongs to the old photo —
    // and so does the notice it put up. This photo owns the toast now.
    destroyStreamPlayer();
    hideTranscodeToast();
    // A retry of the SAME photo (a playback failure) bypasses displayPhoto, so
    // the claim from the attempt that just failed is dropped here too: every
    // branch below (or the run it starts) states this attempt's own answer.
    activeEncoder = null;

    if (forceTranscode) {
      // Explicit retry (e.g. HEVC playback failure): jump straight to the
      // transcode flow, no decision round-trip.
      const url = getVideoUrl(photo.hash_sha256, {
        transcode: true,
        clientCodecs: videoCodecSupport.getClientCodecsString(),
      });
      if (await tryStartTranscode(url, photo)) return;
      setVideoSource(photo, url, false);
      return;
    }

    // A fresh playback attempt re-derives the decision; clear any prior
    // "play original" choice so a normal playback failure can retry transcode.
    hasUserChosenOriginal = false;

    // Ask the server for the recommended path (direct play / streamed remux /
    // streamed audio+video conversion / empty). The server owns the
    // codec+container decision using our declared capability set, so we do not
    // re-guess HEVC support client-side.
    const decision = await api.getVideoDecision(
      photo.hash_sha256,
      videoCodecSupport.getClientCodecsString()
    );

    // The viewer may have been closed, or a newer photo requested, while the
    // decision was loading — a hidden viewer must not start playback.
    if (!isOpen || currentPhoto?.hash_sha256 !== photo.hash_sha256) return;

    if (decision.action === 'direct') {
      activeEncoder = decision.encoder ?? null;
      setVideoSource(photo, decision.url, true);
      return;
    }
    if (decision.action === 'stream') {
      // The upcoming run's own header is authoritative for this playback.
      activeEncoder = null;
      if (!decision.mime || !mseSupported(decision.mime)) {
        // Legacy fallback: whole-file conversion for browsers without MSE for
        // this codec; keeps the escape hatch intact.
        const legacy = getVideoUrl(photo.hash_sha256, {
          transcode: true,
          clientCodecs: videoCodecSupport.getClientCodecsString(),
        });
        if (await tryStartTranscode(legacy, photo)) return;
        setVideoSource(photo, legacy, false);
        return;
      }
      playStream(photo, decision);
      return;
    }
    if (decision.action === 'empty') {
      showTranscodeToast(
        get(t)('video.file_empty', { default: 'This video file is empty or still being synced.' }),
        true
      );
      return;
    }
    showTranscodeToast(
      get(t)('video.conversion_reason', {
        values: { reason: decision.reason || '' },
        default: 'Could not convert this video: {reason}',
      }),
      true
    );
  }

  /**
   * How long a refused (503) stream run waits before retrying when the server
   * sent no `Retry-After`. A saturated pool does not answer immediately: the
   * endpoint can hold the request for its whole `TURBO_PIX_STREAM_QUEUE_WAIT_SECS`
   * (20 s by default) before it refuses, and that hold is covered by the
   * player's own slot-wait notice. This delay paces only the retry that follows
   * the refusal; the server's own hint wins inside
   * [STREAM_RETRY_DELAY_MS, STREAM_RETRY_DELAY_MAX_MS].
   */
  const STREAM_RETRY_DELAY_MS = 1500;
  const STREAM_RETRY_DELAY_MAX_MS = 10_000;

  /**
   * The client's escalation ladder for a delivered stream that turned out not
   * to play (FR-009): the same video, converted one step harder each time —
   * remux (container only) → audio (video copied, audio re-encoded) → transcode
   * (everything re-encoded). The server decides what a mode actually means and
   * never downgrades, so a rung is a request for more work, not a promise the
   * bytes will differ; the ladder is therefore bounded by its own end, with
   * "play original anyway" as the way out.
   */
  const STREAM_LADDER = ['remux', 'audio', 'transcode'];

  /**
   * The rung after `currentMode`, or `null` when the ladder is exhausted. The
   * last rung has no successor, which is what terminates the ladder: at most
   * three attempts per photo, then the error state.
   */
  function nextStreamMode(currentMode) {
    const index = STREAM_LADDER.indexOf(currentMode);
    return index >= 0 && index < STREAM_LADDER.length - 1 ? STREAM_LADDER[index + 1] : null;
  }

  /**
   * Play a server-streamed conversion through MSE: the server pipes fragmented
   * MP4 and the player appends it to a SourceBuffer, so playback starts long
   * before the conversion finishes.
   *
   * `keepWaitingNotice` is set by the saturation retry: the "waiting for a
   * free conversion slot" notice must stay up instead of flashing back to the
   * generic "preparing" one on every attempt. `startAt` is the offset this run
   * plays from — a retry after a refused seek resumes the seek, never 0:00.
   */
  function playStream(
    photo,
    decision,
    modeOverride = null,
    { keepWaitingNotice = false, startAt = 0 } = {}
  ) {
    if (!videoEl) return;
    destroyStreamPlayer();
    // The stream path owns this element's failures now: `setVideoSource`
    // installs an `onerror` *property* handler (the whole-file path's retry or
    // its global toast), while msePlayer registers its own `error` listener, so
    // a leftover property handler would deliver every media error twice —
    // raising a spurious global "conversion failed" toast next to the ladder's
    // own notice, or tearing the running ladder down to start a whole-file
    // conversion behind its back. The stream run states its own failures; the
    // escape hatch re-arms the property handler when the user asks for it.
    videoEl.onerror = null;
    hasUserChosenOriginal = false;
    // Every run states its own answer, so a starting run claims nothing: the
    // failed rung's encoder must not survive into the escalated one (a remux
    // retry would otherwise wear the encoder of the transcode that just died)
    // and a saturation retry must not resurrect it either. `onEncoder` reports
    // this run's own `x-turbopix-encoder` header — the value, or `null` for a
    // run that copies the video track — and re-establishes the hint.
    activeEncoder = null;
    const mode = modeOverride || decision.mode;
    const separator = decision.url.includes('?') ? '&' : '?';
    // The decision URL carries neither `mode` nor `start`: the player always
    // appends `start=<seconds>` itself (msePlayer.urlFor), and `mode` is the
    // server-authorized mode the player may only escalate.
    const streamUrl = `${decision.url}${separator}mode=${mode}`;
    if (!keepWaitingNotice) {
      // The notice leaves the waiting state here, so the flag that renders the
      // escape hatch leaves with it: the waiting text *is* the escape hatch's
      // trigger, and a flag left behind under the "preparing" notice would
      // offer "play original anyway" for a run that no longer claims
      // saturation (an escalated rung that failed before its first chunk
      // replaces the waiting notice without going through `streamWaiting`).
      streamWaiting = false;
      showTranscodeToast(
        get(t)('video.stream.buffering', { default: 'Video is being prepared for playback…' })
      );
    }
    streamPlayer = createStreamPlayer(videoEl, {
      streamUrl,
      mime: decision.mime,
      duration: decision.duration,
      onState: (state) => {
        if (!isOpen || currentPhoto?.hash_sha256 !== photo.hash_sha256) return;
        if (state === 'waiting') {
          // The server is holding the request open for a slot: the same wait,
          // so the same escape hatch applies.
          streamWaiting = true;
          showTranscodeToast(
            get(t)('video.stream.waiting', {
              default: 'Waiting for a free conversion slot…',
            })
          );
        } else if (state === 'buffering') {
          // Bytes are flowing: the slot wait is over — and so is its notice.
          // Every notice that was up before this point (the saturation retry's
          // "waiting for a free slot", the runner's own slow-start timer) has
          // stopped being true, and the waiting text also *is* the escape
          // hatch's trigger: leaving it up would strand a run that buffers
          // slowly, or stalls after its first chunk, on a false "pool is
          // saturated" claim with no way out.
          streamWaiting = false;
          showTranscodeToast(
            get(t)('video.stream.buffering', { default: 'Video is being prepared for playback…' })
          );
        } else if (state === 'playing' || state === 'ended') {
          hideTranscodeToast();
        }
      },
      onError: (error) => handleStreamFailure(photo, decision, mode, error),
      // Every run this player begins — the one asked for here and the one it
      // restarts for a seek inside itself — makes a retry armed for an earlier
      // refusal obsolete, and states the offset this run plays from, which is
      // the newest start intent as long as nothing newer lands; see
      // rememberStreamIntent.
      onRunStart: rememberStreamIntent,
      // A position the user picked on the element itself is the other move that
      // leaves a refused offset behind: either this seek starts a run for the
      // new target (which supersedes the refused one and, if that run is
      // refused too, arms its own retry at the newest offset), or the element
      // already holds the target and no run is needed at all, so the refused
      // offset is moot. Either way the armed retry must not outlive the move —
      // it would tear down the media the user moved to and drag them back (see
      // disarmStreamRetry). The element's own position cannot stand in for
      // this signal: the player parks it on a run's offset, so the element
      // standing at 10 s says nothing about a run refused at 25 s.
      //
      // The target it carries is also the newest start intent: it is newer than
      // the offset of a run still in flight (the server can hold that run for
      // its whole queue wait), so a refusal that arrives afterwards must resume
      // here and not at the offset the user has left.
      onUserSeek: rememberStreamIntent,
      onEncoder: (encoder) => {
        if (!isOpen || currentPhoto?.hash_sha256 !== photo.hash_sha256) return;
        activeEncoder = encoder;
      },
    });
    // The element must not stand in for the photo with the previous video's
    // last frame: its own source only arrives with the stream response — and a
    // refused run never assigns one at all, so a saturation retry leaves the
    // element untouched for its whole 1.5-10 s delay. `setVideoSource` is the
    // only other path that clears this, so the stale source would stay up for
    // the entire setup, and the Space handler's `photoHash` guard (which exists
    // to stop playback of the wrong video) would pass while that stale source
    // is the one `play()` would resume.
    //
    // The attribute goes rather than being set to '': `videoEl.src = ''` leaves
    // the element resolving the empty string as a URL, and Chromium answers
    // that with a `MEDIA_ERR_SRC_NOT_SUPPORTED` `error` event (measured: the
    // event fires ~1-6 ms later and is not cancelled by the next `src`
    // assignment). msePlayer listens for `error` on this element to detect
    // undecodable delivered bytes, so that event can arrive after the stream
    // run attached its listeners and escalate the ladder on a healthy run.
    // With no `src` attribute the element simply goes to NETWORK_EMPTY: no
    // error, no frame, nothing left to play.
    videoEl.removeAttribute('src');
    videoEl.load();
    videoEl.dataset.photoHash = photo.hash_sha256;
    videoEl.style.display = 'block';
    videoEl.classList.add('loaded');
    if (imageEl) imageEl.style.display = 'none';
    swipeableViewer?.reset();
    // Fire and forget: `start()` only settles when the stream ends, and the
    // viewer must not stay "loading" until then. The handle stays set so the
    // next playStream/destroyStreamPlayer tears this run down. Setup failures
    // arrive on `onError`; the rejection path covers everything past setup.
    streamPlayer.start(startAt).catch((error) => handleStreamFailure(photo, decision, mode, error));
  }

  /**
   * A refused conversion slot is not a playback failure: the pool is simply
   * full, so keep the waiting notice (with "play original anyway" still
   * reachable, in case the wait is a permanent pool of 0) and retry the same
   * run until the user moves on — a newer run replaces it, and a user seek that
   * starts no run drops it outright (see `disarmStreamRetry`). Retries cannot
   * stack (one timer, and the newest refusal replaces it). Every other error is
   * a real one.
   *
   * `attemptedMode` is the mode the failed run actually asked for, and the
   * retry stays on it: a refusal says nothing about the mode.
   */
  function handleStreamFailure(photo, decision, attemptedMode, error) {
    if (error?.status !== 503) {
      onStreamError(photo, decision, attemptedMode, error);
      return;
    }
    if (!isOpen || currentPhoto?.hash_sha256 !== photo.hash_sha256) return;
    streamWaiting = true;
    showTranscodeToast(
      get(t)('video.stream.waiting', { default: 'Waiting for a free conversion slot…' })
    );
    const photoHash = photo.hash_sha256;
    const startAt = Number.isFinite(streamIntentOffset) ? streamIntentOffset : 0;
    scheduleStreamRetry(() => {
      if (!isOpen || currentPhoto?.hash_sha256 !== photoHash) return;
      // Resume the newest position the user asked for, not the offset this run
      // was refused at: a seek can land while the run is still in flight (the
      // server may hold it for the whole `TURBO_PIX_STREAM_QUEUE_WAIT_SECS`,
      // 20 s by default) and that seek is newer than the refused offset. Both
      // signals that carry it — the run's own `onRunStart` and every user seek
      // through `onUserSeek` — also disarm this retry, so the offset recorded
      // here is still the user's newest move when it fires: any later move
      // either disarmed it or armed its own retry at the newer offset. Reading
      // the element's position instead would follow the player's own setup, not
      // the user: a run parks the element on ITS offset, so a follow-up run
      // refused at the newest target (25 s) finds the element standing on the
      // previous run's parked offset (10 s) and would resume there — discarding
      // the seek this retry exists to resume.
      playStream(photo, decision, attemptedMode, { keepWaitingNotice: true, startAt });
    }, streamRetryDelayMs(error));
  }

  /**
   * When the refused run may be tried again: the server's `Retry-After` hint
   * when it sent one, the local default otherwise. Clamped so neither a missing
   * hint nor a hostile one can hammer the pool or stall playback.
   */
  function streamRetryDelayMs(error) {
    const hinted = Number.isFinite(error?.retryAfterMs)
      ? error.retryAfterMs
      : STREAM_RETRY_DELAY_MS;
    return Math.min(Math.max(hinted, STREAM_RETRY_DELAY_MS), STREAM_RETRY_DELAY_MAX_MS);
  }

  /**
   * Drop the pending saturation retry. A retry is armed for a run the server
   * refused, and it runs `playStream(..., { startAt })`: firing it once a newer
   * run exists would abort that run and drag playback back to the offset the
   * user has already left, so it must never outlive the position it was armed
   * for.
   * Every newer run disarms it the moment it begins — including a run msePlayer
   * restarts for a seek inside its own player, which is why the player reports
   * every run through `onRunStart` instead of only the states it reaches: a
   * refused run can be held by the server for the whole
   * `TURBO_PIX_STREAM_QUEUE_WAIT_SECS` (20 s by default) before it answers, and
   * until that signal nothing in the viewer can tell that a newer run exists.
   *
   * A user seek is the second signal, and it is not derivable from the first:
   * a target the element can serve from its buffer restarts no run, so the
   * viewer would never learn that the offset it armed for is one the user has
   * left — which is why every user seek reaches the viewer through `onUserSeek`
   * and disarms the retry here too. Between the two, no retry can outlive the
   * position it was armed for, which is what lets its firing path resume at
   * that position instead of reading the element (see `handleStreamFailure`).
   */
  function disarmStreamRetry() {
    if (streamRetryTimer === null) return;
    clearTimeout(streamRetryTimer);
    streamRetryTimer = null;
  }

  /**
   * Record the newest position the user asked for and drop any retry armed for
   * an older one. Both signals the player reports state it: `onRunStart`, with
   * the offset every run plays from, and `onUserSeek`, with every position
   * picked on the element itself. The recorded offset is what
   * `handleStreamFailure` re-arms the retry for, because a run's own offset is
   * not necessarily the user's newest move: the server can hold that run for
   * its whole `TURBO_PIX_STREAM_QUEUE_WAIT_SECS` (20 s) before refusing it,
   * while a seek that lands in that window is reported here and nowhere else
   * the viewer can see.
   *
   * The newest signal wins, in both directions: a deliberate restart at 0
   * reports its own `onRunStart(0)` after the older seek, and a seek that lands
   * after a refusal disarms that refusal's retry outright instead of leaving it
   * to fire at the position the user has left.
   */
  function rememberStreamIntent(seconds) {
    if (Number.isFinite(seconds)) streamIntentOffset = seconds;
    disarmStreamRetry();
  }

  /**
   * Arm the one pending saturation retry. `destroyStreamPlayer` disarms it, so
   * a closed viewer or a new photo never leaves a retry running, and both a
   * newer run (`onRunStart`) and a user seek (`onUserSeek`) do.
   *
   * The newest request wins: a retry is armed for the run that was just
   * refused, and an older one still pending belongs to an offset the user has
   * already left (the newest refused seek is the position they picked last).
   * The offset it is armed for is the position it resumes, not a command that
   * overrides a later move: disarming on that move is what keeps the two
   * consistent. Replacing it cannot let retries stack — there is one timer, and
   * arming never leaves two — and it cannot starve playback either: a refusal
   * that keeps arriving is a request that keeps being made, each of which fires
   * its own retry within [STREAM_RETRY_DELAY_MS, STREAM_RETRY_DELAY_MAX_MS].
   */
  function scheduleStreamRetry(callback, delayMs) {
    disarmStreamRetry();
    streamRetryTimer = setTimeout(() => {
      streamRetryTimer = null;
      callback();
    }, delayMs);
  }

  function destroyStreamPlayer() {
    disarmStreamRetry();
    if (!streamPlayer) return;
    streamPlayer.destroy();
    streamPlayer = null;
  }

  /**
   * A streamed conversion failed for real (spawn error, refused response,
   * undecodable bytes). Self-heal first: climb one rung of the ladder and try
   * the same video again, converted harder. Only when the ladder is exhausted
   * (`transcode` has no successor) does the failure surface, with "play
   * original anyway" as the escape hatch.
   *
   * The escalation starts from `attemptedMode` — the mode that just FAILED —
   * never from `decision.mode`: escalating from the decision would send an
   * `audio` failure back to `remux`, which is the loop the ladder exists to
   * avoid. One step per failure, and the ladder's end bounds the retries.
   */
  function onStreamError(photo, decision, attemptedMode, error) {
    logger?.warn('stream playback failed', error, {
      component: 'PhotoViewer',
      decision,
      attemptedMode,
    });
    if (!isOpen || currentPhoto?.hash_sha256 !== photo.hash_sha256) return;
    destroyStreamPlayer();
    const nextMode = nextStreamMode(attemptedMode);
    if (nextMode) {
      playStream(photo, decision, nextMode);
      return;
    }
    // The error toast owns the notice (and its own escape hatch) from here on.
    // Nothing is being video-encoded any more, so no encoder may be claimed —
    // the escalation branch above gets the same reset from `playStream`, which
    // clears it for every run it starts.
    streamWaiting = false;
    activeEncoder = null;
    showTranscodeToast(
      get(t)('video.transcoding.failed', { default: 'Video conversion failed' }),
      true
    );
  }

  function showTranscodeToast(message, isError = false) {
    transcodeMessage = message;
    transcodeError = isError;
    // Auto-hide on success after polling completes
    if (!isError) {
      // The polling handler will clear this
    }
  }

  function hideTranscodeToast() {
    transcodeMessage = '';
    transcodeError = false;
    streamWaiting = false;
  }

  async function pollTranscodeStatus(pollUrl, photo) {
    const POLL_INTERVAL = 2000;
    // Grace beyond the server's own deadline: we stop polling only once the
    // server would actually have given up (its `deadline_ms`), plus a small
    // buffer for network/poll skew — never at a client-invented 5-minute cap.
    const DEADLINE_GRACE_MS = 30 * 1000;
    // Absolute wall-clock time at which we stop (null until the server first
    // reports a deadline). Refreshed on every response that carries one so the
    // stop tracks the server's live countdown.
    let serverStopAt = null;

    return new Promise((resolve) => {
      // Stop polling once the user has moved on to another photo; the
      // server-side transcode continues regardless. transcodePollTimer is
      // shared across polls: clear only the interval this poll owns, and
      // only null shared state / hide the toast while it still points at
      // us — a newer photo's poll may have replaced it. Checked at the
      // interval top AND after every await: a late fetch/json response can
      // otherwise act for the previous photo.
      const bailIfStale = () => {
        if (!isOpen || currentPhoto?.hash_sha256 !== photo.hash_sha256) {
          clearInterval(intervalId);
          if (transcodePollTimer === intervalId) {
            transcodePollTimer = null;
            hideTranscodeToast();
          }
          resolve('Stale');
          return true;
        }
        return false;
      };

      const intervalId = setInterval(async () => {
        if (bailIfStale()) return;

        try {
          const res = await fetch(pollUrl);
          if (!res.ok) return;
          if (bailIfStale()) return;
          const status = await res.json();
          if (bailIfStale()) return;

          // Track the server's own deadline: stop when it would give up (plus
          // the grace buffer), not at an arbitrary client cap.
          if (typeof status.deadline_ms === 'number' && Number.isFinite(status.deadline_ms)) {
            serverStopAt = Date.now() + status.deadline_ms + DEADLINE_GRACE_MS;
          }
          if (serverStopAt !== null && Date.now() >= serverStopAt) {
            clearInterval(intervalId);
            if (transcodePollTimer === intervalId) {
              transcodePollTimer = null;
              hideTranscodeToast();
            }
            showTranscodeToast(
              get(t)('video.transcoding.timeout', { default: 'Video conversion timed out' }),
              true
            );
            resolve('Timeout');
            return;
          }

          if (status.state === 'Completed') {
            clearInterval(intervalId);
            if (transcodePollTimer === intervalId) transcodePollTimer = null;
            hideTranscodeToast();
            const newUrl = getVideoUrl(photo.hash_sha256, {
              transcode: true,
              clientCodecs: videoCodecSupport.getClientCodecsString(),
            });
            // The completed artifact's own encoder: the file delivery carries no
            // response header the client can read (staleness is already ruled
            // out by bailIfStale above).
            activeEncoder = status.encoder ?? null;
            setVideoSource(photo, newUrl, false);
            resolve('Completed');
          } else if (status.state === 'Failed') {
            clearInterval(intervalId);
            if (transcodePollTimer === intervalId) transcodePollTimer = null;
            hideTranscodeToast();
            showTranscodeToast(
              get(t)('video.transcoding.failed', { default: 'Video conversion failed' }),
              true
            );
            resolve(status.state);
          } else if (status.state === 'Timeout') {
            clearInterval(intervalId);
            if (transcodePollTimer === intervalId) transcodePollTimer = null;
            hideTranscodeToast();
            showTranscodeToast(
              get(t)('video.transcoding.timeout', { default: 'Video conversion timed out' }),
              true
            );
            resolve(status.state);
          } else if (status.state === 'InProgress') {
            // Live progress, when the server reports a percent.
            if (typeof status.percent === 'number') {
              showTranscodeToast(
                get(t)('video.transcoding.progress', {
                  values: { percent: status.percent },
                  default: 'Converting… {percent}%',
                })
              );
            }
          }
        } catch {
          /* ignore */
        }
      }, POLL_INTERVAL);
      transcodePollTimer = intervalId;
    });
  }

  /**
   * "Play original anyway": after a transcode failure, try the source bytes
   * directly. Sets hasUserChosenOriginal so a subsequent playback error shows
   * a plain error instead of looping back into the transcode decision.
   */
  function playOriginalAnyway(photo) {
    if (!videoEl) return;
    if (currentPhoto?.hash_sha256 !== photo.hash_sha256) return;
    hasUserChosenOriginal = true;
    // The user has decided: stop the stream run and any armed saturation
    // retry, otherwise the next attempt would put the waiting notice back up
    // and fight the choice they just made.
    destroyStreamPlayer();
    hideTranscodeToast();
    // The original is played as-is: nothing encodes it, so nothing is claimed.
    activeEncoder = null;
    setVideoSource(
      photo,
      getVideoUrl(photo.hash_sha256, {
        clientCodecs: videoCodecSupport.getClientCodecsString(),
      }),
      false
    );
  }

  function setVideoSource(photo, videoUrl, retryOnFailure) {
    videoEl.src = '';
    videoEl.load();
    // Records which photo this video element currently holds; the Space
    // shortcut only plays when it matches the displayed photo (the element
    // retains the previous photo's src while an image or a pending-transcode
    // video is shown).
    videoEl.dataset.photoHash = photo.hash_sha256;
    videoEl.onerror = async () => {
      // A stale photo's playback failure must neither retry nor toast.
      if (currentPhoto?.hash_sha256 !== photo.hash_sha256) return;
      if (retryOnFailure && !hasUserChosenOriginal) {
        await displayVideo(photo, true);
        return;
      }
      showToast(
        get(t)('notifications.error', { default: 'Error' }),
        get(t)('video.transcoding.failed', { default: 'Video conversion failed' }),
        'error'
      );
    };

    videoEl.src = videoUrl;
    videoEl.style.transform = '';
    videoEl.style.opacity = '';
    videoEl.style.display = 'block';
    videoEl.classList.add('loaded');
    if (imageEl) {
      imageEl.style.transform = '';
      imageEl.style.opacity = '';
      imageEl.style.display = 'none';
    }
    swipeableViewer?.reset();

    const settings = api.getViewSettings();
    if (settings.autoPlay) {
      videoEl.play().catch(() => {});
    }
  }

  function updateRotationState() {
    if (!currentPhoto) return;
    const isRaw = isRawFile(currentPhoto.filename);
    const isVid = isVideoFile(currentPhoto.filename);
    rotationDisabled = isRaw || isVid;
    rotationDisabledTitle = isRaw
      ? get(t)('ui.raw_rotation_disabled', { default: 'RAW files cannot be rotated' })
      : isVid
        ? get(t)('ui.video_rotation_disabled', { default: 'Video rotation is not supported' })
        : '';
  }

  function preloadAdjacentPhotos() {
    [currentIndex - 1, currentIndex + 1].forEach((index) => {
      if (index >= 0 && index < photos.length) {
        const photo = photos[index];
        if (!preloadedImages.has(photo.hash_sha256) && !isVideoFile(photo.filename)) {
          const img = new Image();
          const imageUrl = getMediaUrl(photo);
          if (!imageUrl) return;
          img.onload = () => {
            preloadedImages.set(photo.hash_sha256, img);
          };
          img.src = imageUrl;
        }
      }
    });
  }

  // ── Favorite ───────────────────────────────────────────────────────────────
  async function toggleFavorite() {
    if (!currentPhoto || isCollagePhoto(currentPhoto)) return;
    const photoHash = currentPhoto.hash_sha256;
    const isFav = api.isFavorite(currentPhoto);

    try {
      if (isFav) {
        await api.removeFromFavorites(photoHash);
        // Local write FIRST, then the (unconditional) grid event: the event
        // synchronously splices the SHARED photos array (the viewer holds
        // the same reference), so writing photos[currentIndex] after the
        // dispatch would land on the shifted slot and drop the next card in
        // the Favorites view. The dispatch still fires when the user
        // navigated away mid-request — the backend row changed, so the grid
        // card must sync either way.
        if (currentPhoto?.hash_sha256 === photoHash) {
          currentPhoto = { ...currentPhoto, is_favorite: false };
          if (currentIndex !== -1) photos[currentIndex] = currentPhoto;
          addToast(
            get(t)('ui.removed_from_favs', { default: 'Photo removed from favorites' }),
            '',
            'info',
            2000
          );
        }
        window.dispatchEvent(
          new CustomEvent('favoriteToggled', {
            detail: { photoHash, isFavorite: false },
          })
        );
      } else {
        await api.addToFavorites(photoHash);
        if (currentPhoto?.hash_sha256 === photoHash) {
          currentPhoto = { ...currentPhoto, is_favorite: true };
          if (currentIndex !== -1) photos[currentIndex] = currentPhoto;
          addToast(
            get(t)('ui.added_to_favs', { default: 'Photo added to favorites' }),
            '',
            'success',
            2000
          );
        }
        window.dispatchEvent(
          new CustomEvent('favoriteToggled', {
            detail: { photoHash, isFavorite: true },
          })
        );
      }
    } catch {
      addToast(
        get(t)('ui.fav_error', { default: 'Failed to update favorite status' }),
        '',
        'error',
        2000
      );
    }
  }

  // ── Download ───────────────────────────────────────────────────────────────
  function downloadPhoto() {
    if (!currentPhoto) return;
    const mediaUrl = getMediaUrl(currentPhoto);
    if (!mediaUrl) return;

    const link = document.createElement('a');
    link.href = mediaUrl;
    link.download = currentPhoto.filename || `photo-${currentPhoto.hash_sha256?.substring(0, 8)}`;
    link.click();
    addToast(
      get(t)('ui.download_started', { default: 'Photo download started' }),
      '',
      'info',
      2000
    );
  }

  // ── Sidebar ────────────────────────────────────────────────────────────────
  function toggleSidebar() {
    showSidebar = !showSidebar;
  }

  // ── Rotate ─────────────────────────────────────────────────────────────────
  async function rotatePhoto(angle) {
    if (!currentPhoto || isCollagePhoto(currentPhoto)) return;

    if (isRawFile(currentPhoto.filename)) {
      addToast(
        get(t)('ui.cannot_rotate', { default: 'Cannot Rotate' }),
        get(t)('ui.raw_rotation_disabled', { default: 'RAW files cannot be rotated' }),
        'error',
        4000
      );
      return;
    }
    if (isVideoFile(currentPhoto.filename)) {
      addToast(
        get(t)('ui.cannot_rotate', { default: 'Cannot Rotate' }),
        get(t)('ui.video_rotation_disabled', { default: 'Video rotation is not supported' }),
        'error',
        4000
      );
      return;
    }

    const photoHash = currentPhoto.hash_sha256;

    try {
      isLoading = true;
      const updatedPhoto = await api.rotatePhoto(photoHash, angle);
      // Grid sync fires regardless of viewer state: the backend row changed
      // and the grid's card is keyed by the OLD hash (now rewritten), so
      // suppressing the event would leave a card whose media 404s. Only the
      // viewer-local state + URL rewrite below are guarded — a response
      // landing after close() must not reopen the dismissed viewer via the
      // route effect. oldHash lets the grid replace the card that carries
      // the pre-rotation hash.
      window.dispatchEvent(
        new CustomEvent('photoUpdated', {
          detail: { photo: updatedPhoto, oldHash: photoHash },
        })
      );
      if (!isOpen || currentPhoto?.hash_sha256 !== photoHash) return;
      currentPhoto = updatedPhoto;
      if (currentIndex !== -1) photos[currentIndex] = updatedPhoto;
      // The backend rewrites hash_sha256 on rotation; sync the URL so the
      // route effect doesn't treat the old hash as missing (spurious 404)
      // and Back/Forward doesn't land on a dead hash.
      replaceState({ photo: updatedPhoto.hash_sha256 });

      const timestamp = Date.now();
      const newUrl = `${getPhotoUrl(updatedPhoto.hash_sha256)}?t=${timestamp}`;
      if (imageEl) {
        imageEl.src = newUrl;
        imageEl.onload = () => {
          // Only the current photo's reload may clear the spinner.
          if (currentPhoto?.hash_sha256 === updatedPhoto.hash_sha256) {
            isLoading = false;
          }
        };
        imageEl.onerror = () => {
          // A failed reload (e.g. backend 500 on housekeeping candidates)
          // must not leave the loading spinner up forever.
          if (currentPhoto?.hash_sha256 === updatedPhoto.hash_sha256) {
            isLoading = false;
          }
        };
      }
    } catch (error) {
      addToast(
        get(t)('notifications.error', { default: 'Error' }),
        error.message ||
          get(t)('notifications.rotationFailed', { default: 'Failed to rotate photo' }),
        'error',
        5000
      );
      isLoading = false;
    }
  }

  // ── Delete ─────────────────────────────────────────────────────────────────
  async function deletePhoto() {
    if (!currentPhoto || isCollagePhoto(currentPhoto)) return;

    const confirmed = window.confirm(
      get(t)('notifications.confirmDeleteMessage', {
        default:
          'Are you sure you want to permanently delete this photo? This action cannot be undone.',
      })
    );
    if (!confirmed) return;

    const photoHash = currentPhoto.hash_sha256;

    try {
      isLoading = true;
      await api.deletePhoto(photoHash);
      // The grid-sync event, toast and list update must fire even if the
      // viewer was closed mid-request (the deleted card would otherwise stay
      // in the grid and open a dead viewer). Only the re-navigation below is
      // guarded by isOpen — a response landing after close() must not
      // rewrite the URL and reopen the dismissed viewer via the route effect.
      window.dispatchEvent(new CustomEvent('photoRemoved', { detail: { hash: photoHash } }));
      addToast(
        get(t)('notifications.deleted', { default: 'Deleted' }),
        get(t)('notifications.photoDeleted', { default: 'Photo deleted successfully' }),
        'success',
        2000
      );

      photos = photos.filter((p) => p.hash_sha256 !== photoHash);

      if (!isOpen) return;

      // The user may have navigated to another photo while the delete was in
      // flight; keep the list update, but only re-navigate when the viewer
      // still shows the deleted photo. If the viewer moved on, the delete
      // still shifted every index after the removed slot, so re-sync
      // currentIndex to keep hasPrev/hasNext and photos[currentIndex] writes
      // pointing at the photo on screen.
      if (currentPhoto?.hash_sha256 === photoHash) {
        if (photos.length > 0) {
          await showPhotoAtIndex(Math.min(currentIndex, photos.length - 1));
        } else {
          close();
        }
      } else if (currentPhoto) {
        const idx = photos.findIndex((p) => p.hash_sha256 === currentPhoto.hash_sha256);
        currentIndex = idx === -1 ? Math.min(currentIndex, photos.length - 1) : idx;
      }
      isLoading = false;
    } catch (error) {
      let msg = get(t)('notifications.deletionFailed', { default: 'Failed to delete photo' });
      const match = error.message?.match(/HTTP \d+: (.+)/);
      if (match?.[1]) msg = match[1];
      addToast(get(t)('notifications.error', { default: 'Error' }), msg, 'error', 5000);
      isLoading = false;
    }
  }

  // ── Collage accept ─────────────────────────────────────────────────────────
  function getNormalizedCollageId(photo) {
    const collageId = photo?.collageId;
    if (typeof collageId === 'string' && collageId.length > 0) return collageId;
    if (typeof collageId === 'number' && Number.isFinite(collageId)) return `${collageId}`;
    return null;
  }

  function isPendingCollagePhoto(photo) {
    if (!photo || typeof photo !== 'object') return false;
    if (!isCollagePhoto(photo)) return false;
    const collageId = getNormalizedCollageId(photo);
    if (!collageId) return false;
    const hasPendingPath = [photo.path, photo.thumbnail_path].some(
      (value) => typeof value === 'string' && value.includes(`/api/collages/${collageId}/image`)
    );
    const photoHash = photo.hash_sha256 != null ? `${photo.hash_sha256}` : null;
    return photoHash === collageId && hasPendingPath;
  }

  async function acceptCollageFromViewer() {
    if (isAcceptingCollage || !isPendingCollage) return;
    const collageId = getNormalizedCollageId(currentPhoto);
    if (!collageId) return;

    isAcceptingCollage = true;
    // Capture before the await: navigating while the request is in flight
    // must not change which collage the event reports (or which one closes).
    const emittedCollageId = currentPhoto?.collageId ?? collageId;

    try {
      await api.acceptCollage(collageId);
    } catch (error) {
      const errMsg = `${error?.message || ''}`.toLowerCase();
      if (!errMsg.includes('already accepted') && !errMsg.includes('http 409')) {
        addToast(
          get(t)('ui.accept_collage', { default: 'Accept Collage' }),
          get(t)('notifications.collageAcceptFailed', { default: 'Failed to accept collage' }),
          'error',
          3000
        );
        isAcceptingCollage = false;
        isPendingCollage = isPendingCollagePhoto(currentPhoto);
        return;
      }
    }

    addToast(
      get(t)('ui.accept_collage', { default: 'Accept Collage' }),
      get(t)('notifications.collageAccepted', { default: 'Collage accepted' }),
      'success',
      2000
    );
    window.dispatchEvent(
      new CustomEvent('collageAccepted', {
        detail: { collageId: emittedCollageId },
      })
    );
    // Only close the viewer if it still shows this collage: the user may
    // have swiped to another photo while the request was in flight, and an
    // unconditional close() would yank them out of the viewer over the
    // wrong photo. Compare NORMALIZED ids — collageId may arrive as a number.
    if (isOpen && getNormalizedCollageId(currentPhoto) === collageId) {
      close();
    } else {
      isAcceptingCollage = false;
      isPendingCollage = isPendingCollagePhoto(currentPhoto);
    }
  }

  // ── Metadata edit handlers ─────────────────────────────────────────────────
  function openMetadataEdit() {
    metadataEditRef?.open();
  }

  function onMetadataSaved(updatedPhoto) {
    currentPhoto = updatedPhoto;
    const idx = photos.findIndex((p) => p.hash_sha256 === updatedPhoto.hash_sha256);
    if (idx !== -1) photos[idx] = updatedPhoto;
    window.dispatchEvent(new CustomEvent('photoUpdated', { detail: { photo: updatedPhoto } }));
  }

  // ── Keyboard ───────────────────────────────────────────────────────────────
  const viewerKeyHandlers = {
    Escape: (e) => {
      e.preventDefault();
      close();
    },
    ArrowLeft: (e) => {
      e.preventDefault();
      showPrevious();
    },
    ArrowRight: (e) => {
      e.preventDefault();
      showNext();
    },
    ' ': (e) => {
      // Space on a focused interactive element must keep its native
      // activation (close/prev/next/favorite/rotate/delete buttons).
      if (
        e.target instanceof HTMLElement &&
        e.target.closest('button, a, input, select, textarea, [role="button"]')
      )
        return;
      e.preventDefault();
      // Only toggle playback when videoEl actually holds the CURRENT photo's
      // source: for an image (or a video whose transcode is still pending)
      // the hidden videoEl retains the previous photo's src, and play()
      // would resume audible playback of the wrong video.
      if (videoEl?.dataset.photoHash !== currentPhoto?.hash_sha256) return;
      if (videoEl?.paused) videoEl.play().catch(() => {});
      else videoEl?.pause();
    },
    f: (e) => {
      e.preventDefault();
      toggleFavorite();
    },
    d: (e) => {
      e.preventDefault();
      downloadPhoto();
    },
  };

  function onKeydown(e) {
    if (!isOpen) return;
    // Events originating inside the metadata edit modal are handled by the
    // modal itself; the global search input must still close the viewer.
    if (e.target instanceof HTMLElement && e.target.closest('#metadata-edit-modal')) return;
    // Dismiss an open album picker first so it cannot linger over a closed
    // viewer; Escape still closes the viewer itself (closeViewer contract).
    if (pickerOpen) pickerOpen = false;
    // Typing in inputs must not trigger viewer shortcuts (Escape still closes
    // the viewer from the search input).
    if (e.key !== 'Escape') {
      const tag = e.target instanceof HTMLElement ? e.target.tagName : '';
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
    }
    viewerKeyHandlers[e.key]?.(e);
  }

  // ── External event listeners ───────────────────────────────────────────────
  function onOpenViewer(e) {
    const { photo, photos: allPhotos } = e.detail || {};
    if (photo) open(photo, allPhotos || []);
  }

  function onMainClick(e) {
    if (e.target === mainEl) close();
  }

  function stopPropagation(e) {
    e.stopPropagation();
  }

  // ── Lifecycle ──────────────────────────────────────────────────────────────

  $effect(() => {
    window.addEventListener('openViewer', onOpenViewer);
    window.addEventListener('keydown', onKeydown);
    return () => {
      window.removeEventListener('openViewer', onOpenViewer);
      window.removeEventListener('keydown', onKeydown);
    };
  });

  // Initialize SwipeableViewer after mount
  $effect(() => {
    if (mainEl && imageEl && videoEl) {
      const elements = {
        main: mainEl,
        image: imageEl,
        video: videoEl,
      };
      swipeableViewer = new SwipeableViewer(thisForSwipe);
      swipeableViewer.mount(elements);
      return () => {
        swipeableViewer?.destroy();
        swipeableViewer = null;
      };
    }
  });

  // Document-level mouse events for drag (matches original ViewerControls)
  $effect(() => {
    function _onDrag(e) {
      onDrag(e);
    }
    function _endDrag() {
      endDrag();
    }
    document.addEventListener('mousemove', _onDrag);
    document.addEventListener('mouseup', _endDrag);
    return () => {
      document.removeEventListener('mousemove', _onDrag);
      document.removeEventListener('mouseup', _endDrag);
    };
  });

  // Watch route.photo for deep-link
  $effect(() => {
    if (route.photo && !isOpen) {
      openByHash(route.photo);
    } else if (
      route.photo &&
      isOpen &&
      updateUrlEnabled &&
      currentPhoto?.hash_sha256 !== route.photo
    ) {
      // Back/forward navigation between photos with an open viewer: the URL
      // already reflects the target photo, so display it without replaceState.
      const idx = photos.findIndex((p) => p.hash_sha256 === route.photo);
      if (idx !== -1) {
        showPhotoAtIndex(idx, false);
      } else {
        openByHash(route.photo);
      }
    } else if (!route.photo && isOpen && updateUrlEnabled) {
      // Browser Back: only auto-close when the open photo was reflected in the
      // URL. Viewers opened without a URL param (collages) must not be closed.
      close(false);
    }
  });

  // The viewer shares the grid's photos array; when the grid splices it in
  // place (unfavorite in the Favorites view, photo removal), currentIndex
  // must follow or hasPrev/hasNext and photos[currentIndex] writes point at
  // the wrong photo. Only re-sync when the index derived from the current
  // photo's hash differs from the stored one — never fight user navigation.
  $effect(() => {
    if (!isOpen || !currentPhoto) return;
    if (photos.length === 0) return;
    const idx = photos.findIndex((p) => p.hash_sha256 === currentPhoto.hash_sha256);
    if (idx !== -1) {
      if (idx !== currentIndex) currentIndex = idx;
    } else {
      // Photo gone from the array: clamp to the last valid index (mirrors
      // deletePhoto's re-sync) so the viewer keeps working.
      const clamped = Math.min(currentIndex, photos.length - 1);
      if (clamped !== currentIndex) currentIndex = clamped;
    }
  });

  async function openByHash(photoHash) {
    if (!photoHash) return;
    try {
      const photo = await api.getPhoto(photoHash);
      if (route.photo !== photoHash) return; // user pressed Back while the photo was loading
      if (photo) {
        const allPhotos = photoGridState.photos.length > 0 ? photoGridState.photos : [];
        await open(photo, allPhotos, false);
      }
    } catch (error) {
      logger?.error('Failed to open photo from URL', error, {
        component: 'PhotoViewer',
        photoHash,
      });
    }
  }

  // Self-ref for SwipeableViewer to reference this component's methods
  const thisForSwipe = {
    get isOpen() {
      return isOpen;
    },
    get currentPhoto() {
      return currentPhoto;
    },
    get currentIndex() {
      return currentIndex;
    },
    get photos() {
      return photos;
    },
    get preloadedImages() {
      return preloadedImages;
    },
    get gestureManager() {
      return mainEl?.__gestureManager || null;
    },
    get elements() {
      return { main: mainEl, image: imageEl, video: videoEl };
    },
    controls: {
      reset: resetZoom,
      isZoomed,
    },
    showNext,
    showPrevious,
    close,
    triggerHapticFeedback,
    isVideoFile,
    isCollagePhoto,
    getMediaUrl,
    getCurrentPhoto() {
      return currentPhoto;
    },
  };
  // Gesture handlers for use:gestures action
  const gestureHandlers = {
    pinch: onPinch,
    pinchEnd: onPinchEnd,
    doubleTap: onDoubleTap,
    pan: onPan,
    panEnd: onPanEnd,
  };
</script>

<div
  id="photo-viewer"
  class="photo-viewer"
  class:active={isOpen}
  class:collage-mode={isCollage}
  bind:this={viewerEl}
  role="dialog"
  aria-modal="true"
  aria-label={$t('ui.viewer_label', { default: 'Photo viewer' })}
  tabindex="-1"
>
  <div class="viewer-overlay" role="presentation"></div>
  <div class="viewer-content" role="presentation" onclick={stopPropagation}>
    <button
      type="button"
      class="viewer-close close-viewer"
      title={$t('ui.close', { default: 'Close' })}
      aria-label={$t('ui.close', { default: 'Close' })}
      onclick={() => close()}
    >
      <Icon name="x" width={24} height={24} />
    </button>

    <button
      type="button"
      class="viewer-prev"
      class:hidden={!hasPrev}
      title={$t('ui.previous', { default: 'Previous' })}
      aria-label={$t('ui.previous', { default: 'Previous' })}
      onclick={showPrevious}
    >
      <Icon name="chevron-left" width={28} height={28} />
    </button>

    <button
      type="button"
      class="viewer-next"
      class:hidden={!hasNext}
      title={$t('ui.next', { default: 'Next' })}
      aria-label={$t('ui.next', { default: 'Next' })}
      onclick={showNext}
    >
      <Icon name="chevron-right" width={28} height={28} />
    </button>

    <ViewerControls
      {isVideo}
      {isFavorite}
      {rotationDisabled}
      {rotationDisabledTitle}
      sidebarOpen={showSidebar}
      showAcceptCollage={isPendingCollage}
      {isAcceptingCollage}
      onZoomIn={zoomIn}
      onZoomOut={zoomOut}
      onFitToScreen={fitToScreen}
      onFullscreen={toggleFullscreen}
      onFavorite={toggleFavorite}
      onDownload={downloadPhoto}
      onAddToAlbum={openAlbumPicker}
      onMetadata={toggleSidebar}
      onRotateLeft={() => rotatePhoto(270)}
      onRotateRight={() => rotatePhoto(90)}
      onDelete={deletePhoto}
      onAcceptCollage={acceptCollageFromViewer}
    />

    <div
      class="viewer-main"
      class:video-mode={isVideo}
      role="presentation"
      bind:this={mainEl}
      onclick={onMainClick}
      use:gestures={gestureHandlers}
    >
      <img
        id="viewer-image"
        class="viewer-image"
        role="presentation"
        alt={$t('ui.selected_media', { default: 'Selected media' })}
        bind:this={imageEl}
        onmousedown={startDrag}
        draggable="false"
      />
      <video
        id="viewer-video"
        class="viewer-video"
        style="display: none"
        controls
        bind:this={videoEl}
      >
        <track kind="captions" srclang="en" label="Captions" />
      </video>
      <div class="viewer-loading-indicator" class:show={isLoading}>
        <div class="spinner"></div>
      </div>
      {#if activeEncoder}
        <div
          class="viewer-encoder-hint"
          class:is-hardware={encoderIsHardware}
          data-testid="viewer-encoder-hint"
          role="img"
          aria-label={encoderHintLabel}
        >
          <Icon name={encoderIsHardware ? 'zap' : 'cpu'} width={14} height={14} />
        </div>
      {/if}
    </div>

    <div class="viewer-sidebar" id="viewer-sidebar" class:show={showSidebar}>
      <ViewerMetadata
        photo={currentPhoto}
        onEditMetadata={openMetadataEdit}
        onCloseSidebar={() => {
          showSidebar = false;
        }}
      />
    </div>
  </div>
</div>

<ViewerMetadataEdit
  bind:this={metadataEditRef}
  photo={currentPhoto}
  onClose={() => {}}
  onSaved={onMetadataSaved}
/>

{#if transcodeMessage}
  <div
    class="transcode-toast transcode-toast-visible"
    class:transcode-toast-error={transcodeError}
    role={transcodeError ? 'alert' : 'status'}
  >
    <Icon name={transcodeError ? 'alert-triangle' : 'loader'} width={18} height={18} />
    <span class="transcode-toast-message">{transcodeMessage}</span>
    {#if transcodeError || streamWaiting}
      <button
        type="button"
        class="transcode-toast-action"
        data-action="play-original"
        onclick={() => playOriginalAnyway(currentPhoto)}
      >
        {get(t)('video.play_original', { default: 'Play original anyway' })}
      </button>
    {/if}
  </div>
{/if}

{#if pickerOpen}
  <AlbumPicker openHashes={pickerHashes} onDone={() => (pickerOpen = false)} />
{/if}

<style>
  .photo-viewer {
    position: fixed;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    z-index: var(--z-modal-backdrop);
    overscroll-behavior: contain;
    opacity: 0;
    visibility: hidden;
    transition:
      opacity var(--transition-medium),
      visibility var(--transition-medium);
  }

  :global(.photo-viewer.fade-in) {
    animation: viewer-fade-in var(--transition-slow) ease-out;
  }

  .photo-viewer.active {
    opacity: 1;
    visibility: visible;
  }
  .photo-viewer.collage-mode :global(.favorite-btn),
  .photo-viewer.collage-mode :global(.metadata-btn),
  .photo-viewer.collage-mode :global(.rotate-left-btn),
  .photo-viewer.collage-mode :global(.rotate-right-btn),
  .photo-viewer.collage-mode :global(.add-album-btn),
  .photo-viewer.collage-mode :global(.delete-photo-btn) {
    display: none;
  }

  @keyframes viewer-fade-in {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  .viewer-overlay {
    position: absolute;
    inset: 0;
    background: oklch(0% 0 0deg / 80%);
    backdrop-filter: blur(12px) saturate(1.5);
    -webkit-backdrop-filter: blur(12px) saturate(1.5);
    touch-action: none;
  }

  .viewer-content {
    position: relative;
    width: 100%;
    height: 100dvh;
    max-height: 100dvh;
    display: grid;
    grid-template: 'main sidebar' 1fr / 1fr auto;
    transition: grid-template-columns var(--transition-medium);
  }

  .viewer-close {
    position: absolute;
    top: var(--space-6);
    right: var(--space-6);
    width: var(--button-size-lg);
    height: var(--button-size-lg);
    border-radius: var(--radius-full);
    border: 1px solid var(--glass-border, var(--divider-color));
    background: var(--viewer-btn-bg);
    backdrop-filter: blur(8px) saturate(1.5);
    -webkit-backdrop-filter: blur(8px) saturate(1.5);
    color: var(--viewer-btn-color);
    font-size: var(--font-3xl);
    cursor: pointer;
    transition: var(--transition-fast);
    z-index: 10;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .viewer-close:hover {
    background: var(--viewer-btn-hover-bg);
  }

  .viewer-prev {
    left: var(--space-6);
  }

  .viewer-next {
    right: var(--space-6);
  }

  .viewer-prev,
  .viewer-next {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    width: var(--button-size-lg);
    height: var(--button-size-lg);
    border-radius: var(--radius-full);
    border: 1px solid var(--glass-border, var(--divider-color));
    background: var(--viewer-btn-bg);
    backdrop-filter: blur(8px) saturate(1.5);
    -webkit-backdrop-filter: blur(8px) saturate(1.5);
    color: var(--viewer-btn-color);
    font-size: var(--font-3xl);
    cursor: pointer;
    transition: var(--transition-fast);
    z-index: 10;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .viewer-prev.hidden,
  .viewer-next.hidden {
    display: none;
  }

  .viewer-prev:hover,
  .viewer-next:hover {
    background: var(--viewer-btn-hover-bg);
  }

  .viewer-main {
    grid-area: main;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-6);
    min-height: 0;
    overflow: hidden;
    position: relative;
    touch-action: none;
  }

  /* Chromium renders a native <video>'s control strip inside the element's own
     bottom edge, so a height-bound video (any portrait clip) would drop the
     scrubber straight onto the action bar. Reserve the bar's band here: the
     video then stops above the bar instead of behind it. The reserve applies to
     every video — a 16:9 clip is already height-bound in a 16:9 viewport — so
     videos end up slightly shorter than they otherwise would. */
  .viewer-main.video-mode {
    padding-bottom: calc(
      var(--viewer-controls-offset) + var(--viewer-controls-height) + var(--space-6) +
        env(safe-area-inset-bottom, 0px)
    );
  }

  :global(.viewer-adjacent) {
    position: absolute;
    inset: var(--space-6);
    width: calc(100% - (var(--space-6) * 2));
    height: calc(100% - (var(--space-6) * 2));
    object-fit: contain;
    border-radius: var(--radius-md);
    opacity: 0;
    pointer-events: none;
    user-select: none;
    -webkit-touch-callout: none;
    z-index: 0;
  }

  :global(.viewer-adjacent.visible) {
    opacity: 1;
  }

  :global(.viewer-image),
  :global(.viewer-video) {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    border-radius: var(--radius-md);
    user-select: none;
    -webkit-touch-callout: none;
    transition:
      transform var(--transition-medium),
      opacity 50ms ease-out;
    cursor: grab;
    opacity: 0;
    position: relative;
    z-index: 1;
  }

  :global(.viewer-image.loaded),
  :global(.viewer-video.loaded) {
    opacity: 1;
  }

  :global(.viewer-image.gesture-active),
  :global(.viewer-video.gesture-active),
  :global(.viewer-image.swiping),
  :global(.viewer-video.swiping) {
    transition: none;
  }

  :global(.viewer-image.zoomed) {
    cursor: grab;
  }

  :global(.viewer-image.zoomed:active) {
    cursor: grabbing;
  }

  .viewer-loading-indicator {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    display: none;
    align-items: center;
    justify-content: center;
    z-index: 100;
    background: var(--glass-bg, oklch(100% 0 0deg / 10%));
    backdrop-filter: blur(8px) saturate(1.5);
    -webkit-backdrop-filter: blur(8px) saturate(1.5);
    border-radius: var(--radius-lg);
    padding: var(--space-6);
    border: 1px solid var(--glass-border, var(--divider-color));
    pointer-events: none;
  }

  .viewer-loading-indicator.show {
    display: flex;
    animation: fade-in-fast 100ms ease-out;
  }

  @keyframes fade-in-fast {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  .viewer-loading-indicator :global(.spinner) {
    width: var(--button-size-lg);
    height: var(--button-size-lg);
    border: 4px solid var(--divider-color);
    border-top: 4px solid var(--viewer-btn-color);
    border-radius: var(--radius-full);
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* Encoder hint: informational, never interactive. Low opacity so it reads as
     viewer chrome rather than as a badge competing with the notices. */
  .viewer-encoder-hint {
    position: absolute;
    top: var(--space-3);
    left: var(--space-3);
    z-index: var(--z-base);
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border-radius: var(--radius-full);
    background: var(--viewer-btn-bg);
    color: var(--viewer-btn-color);
    opacity: 0.5;
    pointer-events: none;
  }

  .viewer-encoder-hint.is-hardware {
    color: var(--accent-color);
  }

  .viewer-sidebar {
    grid-area: sidebar;
    background: var(--surface-color);
    padding: 0;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    opacity: 0;
    box-shadow: -4px 0 24px rgb(0 0 0 / 10%);
    transition: opacity var(--transition-medium);
  }

  .viewer-sidebar.show {
    opacity: 1;
  }

  @media (max-width: 768px) {
    .viewer-content {
      grid-template-areas: 'main';
      grid-template-columns: 1fr;
    }

    .viewer-close {
      top: var(--space-3);
      right: var(--space-3);
    }

    .viewer-prev {
      left: var(--space-3);
    }

    .viewer-next {
      right: var(--space-3);
    }

    .viewer-sidebar {
      transition: transform 0.4s cubic-bezier(0.4, 0, 0.2, 1);
      padding: var(--space-6) var(--space-5) var(--space-8) var(--space-5);
      box-shadow: 0 -4px 24px oklch(0% 0 0deg / 20%);
    }
  }

  @media (min-width: 769px) {
    .viewer-sidebar {
      width: 0;
      height: 100dvh;
      min-height: 100dvh;
      overflow: hidden;
      transition:
        width var(--transition-medium),
        padding var(--transition-medium),
        opacity var(--transition-medium);
    }

    .viewer-sidebar.show {
      width: 400px;
      padding: var(--space-8) var(--space-6) var(--space-8) var(--space-6);
    }
  }

  @starting-style {
    .photo-viewer {
      opacity: 0;
    }
  }

  /* Transcode toast */
  :global(.transcode-toast) {
    position: fixed;
    bottom: 24px;
    left: 50%;
    transform: translateX(-50%) translateY(100px);
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 20px;
    background: var(--surface-color);
    color: var(--text-primary);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-medium);
    z-index: 10000;
    opacity: 0;
    transition:
      opacity 0.3s ease,
      transform 0.3s ease;
    pointer-events: none;
    font-size: 14px;
    max-width: 90vw;
  }

  :global(.transcode-toast-visible) {
    opacity: 1;
    transform: translateX(-50%) translateY(0);
    pointer-events: auto;
  }

  :global(.transcode-toast-error) {
    border-color: oklch(55% 0.22 25deg);
  }

  :global(.transcode-toast .feather) {
    flex-shrink: 0;
  }

  :global(.transcode-toast .feather-loader) {
    animation: transcode-spin 1.5s linear infinite;
  }

  :global(.transcode-toast-action) {
    margin-left: 8px;
    padding: 6px 14px;
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-sm);
    background: var(--surface-color);
    color: var(--text-primary);
    font-size: 13px;
    cursor: pointer;
    white-space: nowrap;
    transition:
      background 0.15s ease,
      border-color 0.15s ease;
  }

  :global(.transcode-toast-action:hover) {
    border-color: var(--accent-color);
  }

  :global {
    @keyframes transcode-spin {
      from {
        transform: rotate(0deg);
      }
      to {
        transform: rotate(360deg);
      }
    }
  }

  /* Solid-surface fallbacks: scoped so they outrank the base rules when
     backdrop-filter is unsupported or reduced transparency is requested. */
  @supports not (backdrop-filter: blur(1px)) {
    .viewer-overlay {
      background: oklch(0% 0 0deg / 92%);
    }

    .viewer-close,
    .viewer-prev,
    .viewer-next {
      background: oklch(20% 0.01 260deg / 90%);
    }

    .viewer-loading-indicator,
    .viewer-sidebar {
      background: var(--surface-color);
    }
  }

  @media (prefers-reduced-transparency: reduce) {
    .viewer-overlay {
      backdrop-filter: none;
      -webkit-backdrop-filter: none;
      background: oklch(0% 0 0deg / 92%);
    }

    .viewer-close,
    .viewer-prev,
    .viewer-next {
      backdrop-filter: none;
      -webkit-backdrop-filter: none;
      background: oklch(20% 0.01 260deg / 90%);
    }

    .viewer-loading-indicator,
    .viewer-sidebar {
      backdrop-filter: none;
      -webkit-backdrop-filter: none;
      background: var(--surface-color);
    }
  }
</style>
