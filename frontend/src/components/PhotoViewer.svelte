<script>
  import { get } from 'svelte/store';
  import { t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import { route, replaceState } from '../lib/router.svelte.js';
  import { photoGridState } from '../lib/state.svelte.js';
  import { addToast } from '../lib/state.svelte.js';
  import { getPhotoUrl, getVideoUrl, showToast, videoCodecSupport } from '../lib/utils.js';
  import { APP_CONSTANTS } from '../lib/constants.js';
  import { logger } from '../lib/logger.js';
  import { gestures } from '../lib/gestures/action.js';
  import { SwipeableViewer } from '../lib/viewer/SwipeableViewer.js';
  import Icon from '../lib/Icon.svelte';
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

  // Loading / video
  let isLoading = $state(false);
  let transcodeMessage = $state('');
  let transcodeError = $state(false);

  // Collage
  let isPendingCollage = $state(false);
  let isAcceptingCollage = $state(false);

  // Rotation state (raw/video disable)
  let rotationDisabled = $state(false);
  let rotationDisabledTitle = $state('');

  // ── DOM refs ───────────────────────────────────────────────────────────────
  let viewerEl = $state(null);
  let imageEl = $state(null);
  let videoEl = $state(null);
  let mainEl = $state(null);

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
  function isVideoFile(filename) {
    if (!filename) return false;
    const ext = filename.toLowerCase().substring(filename.lastIndexOf('.'));
    return APP_CONSTANTS.VIDEO_EXTENSIONS.includes(ext);
  }

  function isRawFile(filename) {
    if (!filename) return false;
    const ext = filename.toLowerCase().substring(filename.lastIndexOf('.'));
    return APP_CONSTANTS.RAW_EXTENSIONS.includes(ext);
  }

  function isCollagePhoto(photo) {
    return Boolean(photo?.isCollage || photo?.collageId != null);
  }

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

  function onPan(data) {
    const { deltaX, deltaY } = data;

    if (isZoomed()) {
      const panBoundary = isAtPanBoundary();
      const isHorizontalSwipe = mainEl?.__gestureManager?.gestureAxis === 'horizontal';
      const isSwipeToPrevious = isHorizontalSwipe && deltaX > 0 && panBoundary.left;
      const isSwipeToNext = isHorizontalSwipe && deltaX < 0 && panBoundary.right;

      if (isSwipeToPrevious || isSwipeToNext) {
        if (swipeableViewer?.startZoomEdgeSwipe(data)) {
          if (imageEl) imageEl.classList.remove('gesture-active');
          return;
        }
      }

      updateTouchPan(deltaX, deltaY);
      if (imageEl) imageEl.classList.add('gesture-active');
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
  async function open(photo, allPhotos = []) {
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

    showSidebar = false;

    if (updateUrlEnabled) {
      replaceState({ photo: photo.hash_sha256 });
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
    if (transcodePollTimer) {
      clearInterval(transcodePollTimer);
      transcodePollTimer = null;
    }
    // Clearing the poll timer without hiding the toast would leave the
    // transcode message visible forever (the polling handler is the only
    // path that would have hidden it).
    hideTranscodeToast();
    metadataEditRef?.close?.();
    isOpen = false;
    isPendingCollage = false;
    isAcceptingCollage = false;

    if (viewerEl) {
      if (imageEl) imageEl.style.viewTransitionName = '';
      viewerEl.classList.remove('active', 'fade-in');
      document.body.style.overflow = '';
    }

    showSidebar = false;
    swipeableViewer?.reset();

    if (videoEl) videoEl.pause();

    preloadedImages.clear();

    if (updateUrl) {
      replaceState({ photo: null });
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
      isLoading = false;
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

  async function displayVideo(photo, forceTranscode = false) {
    if (!videoEl) return;

    const videoCodec = photo.metadata?.video?.codec || '';
    const isHEVC = videoCodec.toLowerCase() === 'hevc' || videoCodec.toLowerCase() === 'h265';
    let needsTranscode = forceTranscode;

    if (isHEVC && !forceTranscode) {
      const width = photo.width || 1920;
      const height = photo.height || 1080;
      const supportsHEVC = await videoCodecSupport.supportsHEVC(width, height);
      needsTranscode = !supportsHEVC;
    }

    // A newer photo may have been requested while HEVC support was being probed.
    if (currentPhoto?.hash_sha256 !== photo.hash_sha256) return;

    const videoUrl = getVideoUrl(photo.hash_sha256, { transcode: needsTranscode });

    if (needsTranscode) {
      try {
        const response = await fetch(videoUrl);
        // A newer photo may have been requested while the transcode was starting.
        if (currentPhoto?.hash_sha256 !== photo.hash_sha256) return;
        if (response.status === 202) {
          const data = await response.json();
          const pollUrl = data.poll_url;
          showTranscodeToast(
            get(t)('video.transcoding.started', {
              default: 'Video is being converted for playback...',
            })
          );
          await pollTranscodeStatus(pollUrl, photo);
          return;
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
          return;
        }
      } catch {
        /* ignore */
      }
    }

    setVideoSource(photo, videoUrl, needsTranscode, forceTranscode, isHEVC);
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
  }

  async function pollTranscodeStatus(pollUrl, photo) {
    const POLL_INTERVAL = 2000;
    const MAX_POLL_DURATION = 5 * 60 * 1000;
    const startTime = Date.now();

    return new Promise((resolve) => {
      transcodePollTimer = setInterval(async () => {
        // Stop polling once the user has moved on to another photo; the
        // server-side transcode continues regardless.
        if (currentPhoto?.hash_sha256 !== photo.hash_sha256) {
          clearInterval(transcodePollTimer);
          transcodePollTimer = null;
          hideTranscodeToast();
          resolve('Stale');
          return;
        }

        const elapsed = Date.now() - startTime;
        if (elapsed >= MAX_POLL_DURATION) {
          clearInterval(transcodePollTimer);
          transcodePollTimer = null;
          hideTranscodeToast();
          showTranscodeToast(
            get(t)('video.transcoding.timeout', { default: 'Video conversion timed out' }),
            true
          );
          resolve('Timeout');
          return;
        }

        try {
          const res = await fetch(pollUrl);
          if (!res.ok) return;
          const status = await res.json();

          if (status.state === 'Completed') {
            clearInterval(transcodePollTimer);
            transcodePollTimer = null;
            hideTranscodeToast();
            const newUrl = getVideoUrl(photo.hash_sha256, { transcode: true });
            setVideoSource(photo, newUrl, true, true, true);
            resolve('Completed');
          } else if (status.state === 'Failed' || status.state === 'Timeout') {
            clearInterval(transcodePollTimer);
            transcodePollTimer = null;
            hideTranscodeToast();
            showTranscodeToast(
              get(t)('video.transcoding.failed', { default: 'Video conversion failed' }),
              true
            );
            resolve(status.state);
          }
        } catch {
          /* ignore */
        }
      }, POLL_INTERVAL);
    });
  }

  function setVideoSource(photo, videoUrl, needsTranscode, forceTranscode, isHEVC) {
    videoEl.src = '';
    videoEl.load();
    videoEl.onerror = async () => {
      // A stale photo's playback failure must neither retry nor toast.
      if (currentPhoto?.hash_sha256 !== photo.hash_sha256) return;
      if (isHEVC && !needsTranscode && !forceTranscode) {
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
        currentPhoto = { ...currentPhoto, is_favorite: false };
        photos[currentIndex] = currentPhoto;
        addToast(
          get(t)('ui.removed_from_favs', { default: 'Photo removed from favorites' }),
          '',
          'info',
          2000
        );
      } else {
        await api.addToFavorites(photoHash);
        currentPhoto = { ...currentPhoto, is_favorite: true };
        photos[currentIndex] = currentPhoto;
        addToast(
          get(t)('ui.added_to_favs', { default: 'Photo added to favorites' }),
          '',
          'success',
          2000
        );
      }

      window.dispatchEvent(
        new CustomEvent('favoriteToggled', {
          detail: { photoHash, isFavorite: !isFav },
        })
      );
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

    try {
      isLoading = true;
      const updatedPhoto = await api.rotatePhoto(currentPhoto.hash_sha256, angle);
      currentPhoto = updatedPhoto;
      if (currentIndex !== -1) photos[currentIndex] = updatedPhoto;
      // The backend rewrites hash_sha256 on rotation; sync the URL so the
      // route effect doesn't treat the old hash as missing (spurious 404)
      // and Back/Forward doesn't land on a dead hash.
      replaceState({ photo: updatedPhoto.hash_sha256 });
      window.dispatchEvent(new CustomEvent('photoUpdated', { detail: { photo: updatedPhoto } }));

      const timestamp = Date.now();
      const newUrl = `${getPhotoUrl(updatedPhoto.hash_sha256)}?t=${timestamp}`;
      if (imageEl) {
        imageEl.src = newUrl;
        imageEl.onload = () => {
          isLoading = false;
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
      window.dispatchEvent(new CustomEvent('photoRemoved', { detail: { hash: photoHash } }));
      addToast(
        get(t)('notifications.deleted', { default: 'Deleted' }),
        get(t)('notifications.photoDeleted', { default: 'Photo deleted successfully' }),
        'success',
        2000
      );

      photos = photos.filter((p) => p.hash_sha256 !== photoHash);

      if (photos.length > 0) {
        if (currentIndex >= photos.length) currentIndex = photos.length - 1;
        await showPhotoAtIndex(currentIndex);
      } else {
        close();
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
        detail: { collageId: currentPhoto?.collageId ?? collageId },
      })
    );
    close();
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
  function onKeydown(e) {
    if (!isOpen) return;
    // Events originating inside the metadata edit modal are handled by the
    // modal itself; the global search input must still close the viewer.
    if (e.target instanceof HTMLElement) {
      if (e.target.closest('#metadata-edit-modal')) return;
    }
    // Typing in inputs must not trigger viewer shortcuts (Escape still closes
    // the viewer from the search input).
    if (e.key !== 'Escape') {
      const tag = e.target instanceof HTMLElement ? e.target.tagName : '';
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
    }
    switch (e.key) {
      case 'Escape':
        e.preventDefault();
        close();
        break;
      case 'ArrowLeft':
        e.preventDefault();
        showPrevious();
        break;
      case 'ArrowRight':
        e.preventDefault();
        showNext();
        break;
      case ' ':
        e.preventDefault();
        if (videoEl && !videoEl.paused) videoEl.pause();
        else if (videoEl && videoEl.paused) videoEl.play();
        break;
      case 'f':
        e.preventDefault();
        toggleFavorite();
        break;
      case 'd':
        e.preventDefault();
        downloadPhoto();
        break;
    }
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

  async function openByHash(photoHash) {
    if (!photoHash) return;
    try {
      const photo = await api.getPhoto(photoHash);
      if (route.photo !== photoHash) return; // user pressed Back while the photo was loading
      if (photo) {
        const allPhotos = photoGridState.photos.length > 0 ? photoGridState.photos : [];
        await open(photo, allPhotos);
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
>
  <div class="viewer-overlay" role="presentation" onclick={() => close()}></div>
  <div class="viewer-content" role="presentation" onclick={stopPropagation}>
    <button
      type="button"
      class="viewer-close close-viewer"
      title={$t('ui.close', { default: 'Close' })}
      onclick={() => close()}
    >
      <Icon name="x" width={24} height={24} />
    </button>

    <button
      type="button"
      class="viewer-prev"
      class:hidden={!hasPrev}
      title={$t('ui.previous', { default: 'Previous' })}
      onclick={showPrevious}
    >
      <Icon name="chevron-left" width={28} height={28} />
    </button>

    <button
      type="button"
      class="viewer-next"
      class:hidden={!hasNext}
      title={$t('ui.next', { default: 'Next' })}
      onclick={showNext}
    >
      <Icon name="chevron-right" width={28} height={28} />
    </button>

    <ViewerControls
      {isVideo}
      {isFavorite}
      {rotationDisabled}
      {rotationDisabledTitle}
      showAcceptCollage={isPendingCollage}
      {isAcceptingCollage}
      onZoomIn={zoomIn}
      onZoomOut={zoomOut}
      onFitToScreen={fitToScreen}
      onFullscreen={toggleFullscreen}
      onFavorite={toggleFavorite}
      onDownload={downloadPhoto}
      onMetadata={toggleSidebar}
      onRotateLeft={() => rotatePhoto(270)}
      onRotateRight={() => rotatePhoto(90)}
      onDelete={deletePhoto}
      onAcceptCollage={acceptCollageFromViewer}
    />

    <div
      class="viewer-main"
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
    </div>

    <div class="viewer-sidebar" class:show={showSidebar}>
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
  <div class="transcode-toast transcode-toast-visible" class:transcode-toast-error={transcodeError}>
    <Icon name={transcodeError ? 'alert-triangle' : 'loader'} width={18} height={18} />
    <span class="transcode-toast-message">{transcodeMessage}</span>
  </div>
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

    .viewer-sidebar {
      transition: transform 0.4s cubic-bezier(0.4, 0, 0.2, 1);
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
