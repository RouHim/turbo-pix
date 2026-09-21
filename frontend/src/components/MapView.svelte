<script>
  import { get } from 'svelte/store';
  import { flushSync, mount, onMount, unmount, untrack } from 'svelte';
  import { SvelteMap } from 'svelte/reactivity';
  import L from 'leaflet';
  import Supercluster from 'supercluster';
  import 'leaflet/dist/leaflet.css';

  import { t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import { addToast, appState } from '../lib/state.svelte.js';
  import { route } from '../lib/router.svelte.js';
  import { logger } from '../lib/logger.js';
  import {
    buildMapFilters,
    fetchSemanticPhotoSet,
    formatCoordinates,
    getLocationLabel,
    groupPhotosByLocation,
    isSemanticQuery,
    wrapLongitudeForView,
  } from '../lib/map.js';
  import MapPopup from './MapPopup.svelte';

  const MAX_ZOOM = 19;
  const CLUSTER_RADIUS = 60;
  const CLUSTER_MAX_ZOOM = 18;
  const INITIAL_CENTER = [20, 0];
  const INITIAL_ZOOM = 2;
  const FIT_MAX_ZOOM = 14;

  // FR-018: the OS "reduce motion" preference disables every map transition.
  const prefersReducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  let photos = $state([]);
  let loading = $state(true);
  let loadError = $state(null);
  let tilesFailed = $state(false);

  let mapEl = null;
  let map = null;
  let tileLayer = null;
  let clusterLayer = null;
  let clusterIndex = null;
  let resizeObserver = null;
  let abortController = null;
  let loadToken = 0;
  let anyTileLoaded = false;
  let hasFittedOnce = false;
  // Location key of the open popup, and whether a marker re-render was asked
  // for while it was open (see renderClusters).
  let openPopupKey = null;
  let renderPending = false;
  // Set when the popup is dismissed from inside itself, so focus returns to the
  // marker without guessing from the browser's focus teardown.
  let restoreFocusOnClose = false;
  // Marker → mounted MapPopup component, so a popup's DOM is torn down with it.
  // (SvelteMap: the linter rejects a plain Map in component scope; this one is
  // only ever read imperatively.)
  const popupHandles = new SvelteMap();
  // Location key → its current marker, so focus can be restored to the marker
  // that survives a re-render.
  const locationMarkers = new SvelteMap();

  // Locations are the aggregation unit: the unlocated notice counts the photos
  // left over, and each location renders as exactly one marker (FR-009).
  const locations = $derived(groupPhotosByLocation(photos));
  const unlocatedCount = $derived(
    photos.length - locations.reduce((count, location) => count + location.photos.length, 0)
  );
  const showEmptyState = $derived(!loading && !loadError && locations.length === 0);

  // ── Data loading ──────────────────────────────────────────────────────────

  async function fetchPhotoSet(currentRoute, signal) {
    if (currentRoute.album != null) {
      // Album detail mirrors the grid: album scope + sort/order, no q/year/month.
      const { sort, order } = buildMapFilters(currentRoute);
      const response = await api.getMapPhotos(
        { album: currentRoute.album, sort, order },
        { signal }
      );
      return response.photos ?? [];
    }

    if (isSemanticQuery(currentRoute.query)) {
      return fetchSemanticPhotoSet(api, currentRoute.query, { signal });
    }

    const response = await api.getMapPhotos(buildMapFilters(currentRoute), { signal });
    return response.photos ?? [];
  }

  async function loadPhotos() {
    const token = ++loadToken;
    abortController?.abort();
    abortController = new AbortController();
    const { signal } = abortController;

    loading = true;
    loadError = null;

    try {
      const loaded = await fetchPhotoSet(route, signal);
      if (token !== loadToken) return;
      photos = loaded;
      fitToLocationsOnce();
    } catch (error) {
      if (error?.name === 'AbortError') return;
      if (token !== loadToken) return;
      logger.error('Error loading map photos', error, { component: 'MapView' });
      loadError = error.message || error;
      addToast(
        get(t)('errors.error_loading_photos', { default: 'Error Loading Photos' }),
        error.message,
        'error',
        5000
      );
    } finally {
      if (token === loadToken) loading = false;
    }
  }

  // ── Map lifecycle ─────────────────────────────────────────────────────────

  function attributionHtml() {
    const label = get(t)('map.attributionLabel', { default: 'OpenStreetMap contributors' });
    return `&copy; <a href="https://www.openstreetmap.org/copyright" target="_blank" rel="noopener">${label}</a>`;
  }

  /** Frames the current result set exactly once per mount (FR-016). */
  function fitToLocationsOnce() {
    if (hasFittedOnce || !map || locations.length === 0) return;
    hasFittedOnce = true;
    const bounds = L.latLngBounds(
      locations.map((location) => [location.latitude, location.longitude])
    );
    map.fitBounds(bounds, {
      padding: [32, 32],
      maxZoom: FIT_MAX_ZOOM,
      animate: !prefersReducedMotion,
    });
  }

  // ── Marker rendering ──────────────────────────────────────────────────────

  /** Count bubble; grows with the number it carries so 3+ digits stay readable. */
  function clusterIcon(count) {
    const size = count < 10 ? 34 : count < 100 ? 42 : 48;
    return L.divIcon({
      className: 'map-cluster-marker',
      html: `<span>${count}</span>`,
      iconSize: [size, size],
      iconAnchor: [size / 2, size / 2],
    });
  }

  /** A location is one dot, whatever the number of photos behind it (FR-009). */
  function locationIcon() {
    return L.divIcon({ className: 'map-location-marker', iconSize: [18, 18], iconAnchor: [9, 9] });
  }

  function labelFor(location) {
    return getLocationLabel(location) ?? formatCoordinates(location);
  }

  /**
   * Leaflet only opens a popup on Enter (its own `keypress` path), so both
   * marker kinds handle Enter and Space themselves (FR-018).
   */
  function activateOnKeyboard(element, handler) {
    element.addEventListener('keydown', (event) => {
      if (event.key !== 'Enter' && event.key !== ' ') return;
      // The default would let Leaflet's keypress handler toggle the popup shut
      // again right after this opens it.
      event.preventDefault();
      handler();
    });
  }

  /**
   * Moves focus onto the popup's first thumbnail (FR-018).
   *
   * Leaflet's `DivOverlay.update()` writes and clears an inline
   * `visibility: hidden` on the popup container while it lays the popup out.
   * Under `prefers-reduced-motion` the global `transition-duration: 0.01ms` rule
   * pairs with the initial `transition-property: all`, so that write becomes a
   * transition: the container — and then the thumbnails inside it, one nesting
   * level per frame — still compute `hidden` when `popupopen` fires, and
   * Chromium refuses focus on a hidden element. Retry on the frames the
   * transition needs; the cap keeps a popup that never gets a thumbnail from
   * spinning, and a closed popup ends the retries.
   */
  function focusFirstPopupItem(popupNode, popupEl, framesLeft = 8) {
    popupNode.querySelector('button')?.focus();
    // Done once focus is inside the popup at all: the thumbnail focus landed, or
    // the user has already moved on within it.
    if (popupEl.contains(document.activeElement) || !popupNode.isConnected || framesLeft === 0) {
      return;
    }
    requestAnimationFrame(() => focusFirstPopupItem(popupNode, popupEl, framesLeft - 1));
  }

  function bindLocationMarker(marker, location) {
    const popupNode = document.createElement('div');
    popupNode.className = 'map-popup-host';
    marker.bindPopup(popupNode, {
      maxWidth: 320,
      minWidth: 260,
      // FR-018: `autoPan` animates a 250 ms `panBy`, so the preference turns
      // the pan off — the popup still opens, it just arrives without movement.
      autoPan: !prefersReducedMotion,
      closeButton: true,
    });

    // Where focus was inside the popup when it closed. With the fade animation
    // Leaflet's removal is deferred, so the popup is still connected — and still
    // holds focus — when `popupclose` fires; the reduced-motion path has already
    // detached it and dropped focus to <body>. Both signatures are read below.
    let lastFocused = null;
    popupNode.addEventListener('focusin', (event) => {
      lastFocused = event.target;
    });

    marker.on('popupopen', () => {
      lastFocused = null;
      openPopupKey = location.key;
      popupHandles.set(
        marker,
        mount(MapPopup, {
          target: popupNode,
          props: { location, onOpenPhoto: (photo) => openViewer(photo) },
        })
      );
      // Popup panes come after the marker pane in DOM order, but with many
      // markers Tab would walk through every marker first — move focus into the
      // popup so keyboard users land on the thumbnails (FR-018). flushSync makes
      // the freshly mounted markup available right here, so every (re)open lands
      // on a thumbnail.
      flushSync();
      focusFirstPopupItem(popupNode, marker.getPopup().getElement());
    });

    marker.on('popupclose', () => {
      // Keyboard dismissal sets the flag. Otherwise ask where focus was rather
      // than where the browser's teardown left it: on the default configuration
      // the popup is faded out, so its DOM keeps focus for another 200 ms and
      // `activeElement` still sits inside it here, while the reduced-motion path
      // has detached the popup and dropped focus to <body>.
      const popupEl = marker.getPopup()?.getElement();
      const focusDropped =
        restoreFocusOnClose ||
        (popupEl != null && popupEl.contains(document.activeElement)) ||
        (!lastFocused?.isConnected && document.activeElement === document.body);
      restoreFocusOnClose = false;
      lastFocused = null;
      const handle = popupHandles.get(marker);
      if (handle) {
        popupHandles.delete(marker);
        void unmount(handle);
      }
      if (openPopupKey === location.key) openPopupKey = null;
      // A render requested while the popup was open replaces every marker, so it
      // is replayed on the next frame instead of here: Leaflet closes the popup
      // from the synthetic `preclick` it dispatches BEFORE it resolves the
      // `click` target, so clearing the layers inside that dispatch would detach
      // the clicked marker and swallow its popup / cluster expansion until a
      // second click. `fromViewChange` keeps the wait-for-popup rule for the
      // popup that click may have opened by then; the replay moves focus to the
      // replacement marker itself, so the handback below stays on the element
      // still in the DOM.
      if (renderPending) {
        renderPending = false;
        requestAnimationFrame(() => renderClusters({ fromViewChange: true }));
      }
      // Only take focus back if the popup dropped it: a mouse user closing the
      // popup left focus on the map container and must not have it yanked away.
      const icon = (locationMarkers.get(location.key) ?? marker).getElement();
      if (focusDropped && icon?.isConnected) icon.focus();
    });
  }

  /** Zooms to the level at which the cluster's members become individual dots. */
  function expandCluster(clusterId, latlng) {
    if (!map || !clusterIndex) return;
    const zoom = clusterIndex.getClusterExpansionZoom(clusterId);
    map.setView(latlng, Math.min(zoom, MAX_ZOOM), { animate: !prefersReducedMotion });
  }

  /** Pan/zoom redraws are the ones that must wait for an open popup to close. */
  function handleViewChange() {
    renderClusters({ fromViewChange: true });
  }

  /**
   * Escape dismisses the open popup while it is the focused surface. Leaflet's
   * own Escape handling only runs while the map container itself has focus; with
   * focus inside the popup it is unhooked, so the popup would be undismissable.
   */
  function handleMapKeydown(event) {
    if (event.key !== 'Escape' || !openPopupKey) return;
    if (!map?.getPane('popupPane')?.contains(event.target)) return;
    event.preventDefault();
    event.stopPropagation();
    restoreFocusOnClose = true;
    map.closePopup();
  }

  /**
   * Redraws the markers. `fromViewChange` marks the pan/zoom path: removing a
   * marker closes the popup bound to it (Leaflet's `bindPopup` registers
   * `remove: closePopup`), and opening a popup pans the map — so rendering on
   * that pan's `moveend` would close the popup the pan was for. Only that path
   * waits for the popup to close; a data change drops the popup and redraws
   * immediately, so the markers never lag the active filters (FR-004/SC-003).
   */
  function renderClusters({ fromViewChange = false } = {}) {
    if (!map || !clusterLayer || !clusterIndex) return;

    if (openPopupKey) {
      if (fromViewChange) {
        renderPending = true;
        return;
      }
      // This path rebuilds every marker itself, so the popup's close handler
      // must not schedule the replay frame as well: consume the flag first.
      renderPending = false;
      map.closePopup();
    }

    const bounds = map.getBounds();
    const features = clusterIndex.getClusters(
      [bounds.getWest(), bounds.getSouth(), bounds.getEast(), bounds.getNorth()],
      Math.round(map.getZoom())
    );
    const byKey = new Map(locations.map((location) => [location.key, location]));
    // A re-render replaces every marker; a keyboard user parked on one keeps
    // their place by having focus moved to its replacement. A cluster is not
    // replaced — activating it zooms until its members become individual dots —
    // so its element disappears with nothing to inherit focus and the map
    // container takes it back instead of letting it fall to <body>.
    const activeElement = document.activeElement;
    const focusedLocation = activeElement?.getAttribute?.('data-map-location') ?? null;
    const focusedCluster =
      focusedLocation === null && activeElement?.hasAttribute?.('data-map-cluster');

    clusterLayer.clearLayers();
    locationMarkers.clear();

    for (const feature of features) {
      const [rawLongitude, latitude] = feature.geometry.coordinates;
      const longitude = wrapLongitudeForView(rawLongitude, bounds.getWest(), bounds.getEast());

      if (feature.properties.cluster) {
        // FR-006: the bubble, `data-map-cluster`, and the aria-label all carry
        // the photos the cluster aggregates — `point_count` counts the
        // locations behind it, which would understate a multi-photo location.
        const photoCount = feature.properties.photoCount;
        const marker = L.marker([latitude, longitude], {
          icon: clusterIcon(photoCount),
          keyboard: true,
        });
        const expand = () => expandCluster(feature.properties.cluster_id, [latitude, longitude]);
        marker.on('click', expand);
        marker.addTo(clusterLayer);
        const element = marker.getElement();
        if (element) {
          element.setAttribute('data-map-cluster', String(photoCount));
          element.setAttribute(
            'aria-label',
            get(t)('map.clusterLabel', {
              values: { count: photoCount },
              default: '{count} photos, activate to zoom in',
            })
          );
          activateOnKeyboard(element, expand);
        }
        continue;
      }

      const location = byKey.get(feature.properties.key);
      if (!location) continue;

      const marker = L.marker([latitude, longitude], {
        icon: locationIcon(),
        keyboard: true,
      });
      marker.addTo(clusterLayer);
      locationMarkers.set(location.key, marker);
      const element = marker.getElement();
      if (element) {
        element.setAttribute('data-map-location', location.key);
        element.setAttribute('data-map-location-count', String(location.photos.length));
        element.setAttribute(
          'aria-label',
          get(t)('map.markerLabel', {
            values: { count: location.photos.length, place: labelFor(location) },
            default: '{count} photos at {place}',
          })
        );
        activateOnKeyboard(element, () => marker.openPopup());
      }
      bindLocationMarker(marker, location);
    }

    if (focusedLocation) {
      const focusedIcon = locationMarkers.get(focusedLocation)?.getElement();
      if (focusedIcon?.isConnected) focusedIcon.focus();
    } else if (focusedCluster) {
      map.getContainer().focus();
    }
  }

  function openViewer(photo) {
    // The viewer is a modal overlay: leaving the popup open behind it would keep
    // its thumbnails tabbable and let the map claim Escape from the viewer.
    map?.closePopup();
    // FR-011: the viewer gets the map's complete filtered, sorted set — the
    // same array the grid would page through — so next/previous stay in scope.
    window.dispatchEvent(new CustomEvent('openViewer', { detail: { photo, photos } }));
  }

  onMount(() => {
    map = L.map(mapEl, {
      center: INITIAL_CENTER,
      zoom: INITIAL_ZOOM,
      zoomControl: true,
      attributionControl: true,
      maxZoom: MAX_ZOOM,
      // FR-018: reduced motion preference suppresses animated pan/zoom/fade.
      zoomAnimation: !prefersReducedMotion,
      fadeAnimation: !prefersReducedMotion,
      markerZoomAnimation: !prefersReducedMotion,
    });

    // Attribution is registered on the control itself, independently of the
    // tile layer, so it stays visible in the degraded no-tiles state (SC-005).
    map.attributionControl.addAttribution(attributionHtml());

    if (appState.tileUrl) {
      tileLayer = L.tileLayer(appState.tileUrl, { maxZoom: MAX_ZOOM });
      tileLayer.on('tileerror', () => {
        if (!anyTileLoaded) tilesFailed = true;
      });
      tileLayer.on('tileload', () => {
        anyTileLoaded = true;
        tilesFailed = false;
      });
      tileLayer.addTo(map);
    } else {
      // No configured endpoint — the map stays usable without a background.
      tilesFailed = true;
    }

    // Markers live in their own group so a re-render can replace them without
    // touching tiles or the map itself.
    clusterLayer = L.layerGroup().addTo(map);
    map.on('moveend zoomend', handleViewChange);
    const container = map.getContainer();
    container.addEventListener('keydown', handleMapKeydown);

    // The shell resizes (sidebar toggle, window resize) without remounting the
    // view, so Leaflet must be told to re-measure its container.
    resizeObserver = new ResizeObserver(() => map?.invalidateSize());
    resizeObserver.observe(mapEl);

    // If the result set resolved before the map existed, fit it now.
    fitToLocationsOnce();

    return () => {
      resizeObserver?.disconnect();
      resizeObserver = null;
      abortController?.abort();
      map?.off('moveend zoomend', handleViewChange);
      container.removeEventListener('keydown', handleMapKeydown);
      // Leaflet does not remove layers on map.remove(), so a popup left open by
      // a view change would keep its component instance alive.
      for (const handle of popupHandles.values()) {
        void unmount(handle);
      }
      popupHandles.clear();
      locationMarkers.clear();
      openPopupKey = null;
      renderPending = false;
      map?.remove();
      map = null;
      tileLayer = null;
      clusterLayer = null;
      clusterIndex = null;
    };
  });

  $effect(() => {
    // FR-004/SC-003: the map shows exactly the photos the active filters select.
    route.view;
    route.query;
    route.sort;
    route.year;
    route.month;
    route.album;
    untrack(() => loadPhotos());
  });

  $effect(() => {
    // FR-006: cluster the unique coordinate locations; each point carries its
    // location's photo count so the index itself can aggregate photos, and the
    // index is rebuilt whenever the result set changes (markers follow
    // immediately).
    clusterIndex = new Supercluster({
      radius: CLUSTER_RADIUS,
      maxZoom: CLUSTER_MAX_ZOOM,
      map: (properties) => ({ photoCount: properties.photoCount }),
      reduce: (accumulated, properties) => {
        accumulated.photoCount += properties.photoCount;
      },
    }).load(
      locations.map((location) => ({
        type: 'Feature',
        geometry: { type: 'Point', coordinates: [location.longitude, location.latitude] },
        properties: { key: location.key, photoCount: location.photos.length },
      }))
    );
    untrack(() => renderClusters());
  });

  // ── Viewer feedback ───────────────────────────────────────────────────────

  // The viewer shares the map's `photos` array, so viewer actions mutate it in
  // place and the map (markers, unlocated count) stays coherent (FR-011).
  function handlePhotoRemoved(event) {
    const { hash } = event.detail || {};
    if (!hash) return;
    const index = photos.findIndex((photo) => photo.hash_sha256 === hash);
    if (index !== -1) photos.splice(index, 1);
  }

  function handlePhotoUpdated(event) {
    const updatedPhoto = event.detail?.photo;
    if (!updatedPhoto?.hash_sha256) return;
    // Rotation rewrites hash_sha256, so match on the old hash too.
    const oldHash = event.detail?.oldHash;
    const index = photos.findIndex(
      (photo) =>
        (oldHash && photo.hash_sha256 === oldHash) ||
        photo.hash_sha256 === updatedPhoto.hash_sha256 ||
        (photo.file_path && updatedPhoto.file_path && photo.file_path === updatedPhoto.file_path)
    );
    if (index !== -1) photos[index] = updatedPhoto;
  }

  function handleFavoriteToggled(event) {
    const { photoHash, isFavorite } = event.detail || {};
    const index = photos.findIndex((photo) => photo.hash_sha256 === photoHash);
    if (index === -1) return;
    photos[index].is_favorite = isFavorite;
    // An `is_favorite:true` query stops matching the photo once unfavorited.
    if (!isFavorite && route.query?.split(/\s+/).includes('is_favorite:true')) {
      photos.splice(index, 1);
    }
  }

  $effect(() => {
    const reload = () => loadPhotos();
    const listeners = {
      photoRemoved: handlePhotoRemoved,
      photoUpdated: handlePhotoUpdated,
      favoriteToggled: handleFavoriteToggled,
      indexingCompleted: reload,
      photosReloadRequested: reload,
    };
    for (const [name, handler] of Object.entries(listeners)) {
      window.addEventListener(name, handler);
    }
    return () => {
      for (const [name, handler] of Object.entries(listeners)) {
        window.removeEventListener(name, handler);
      }
    };
  });
</script>

<div class="map-view" data-testid="map-view" data-reduced-motion={String(prefersReducedMotion)}>
  <div class="map-status">
    {#if !loading && !loadError && unlocatedCount > 0}
      <p class="map-notice" data-testid="map-unlocated-notice">
        {$t('map.withoutLocation', {
          values: { count: unlocatedCount },
          default: '{count} photos without location data',
        })}
      </p>
    {/if}
    {#if tilesFailed}
      <p class="map-notice map-notice-warning" role="status" data-testid="map-tiles-notice">
        {$t('map.tilesUnavailable', {
          default: 'Map tiles unavailable — markers are shown without a map background.',
        })}
      </p>
    {/if}
  </div>

  <div class="map-stage">
    <div class="map-canvas" bind:this={mapEl} data-testid="map-canvas"></div>

    {#if loading}
      <div class="map-overlay" data-testid="map-loading">
        {$t('ui.loading', { default: 'Loading...' })}
      </div>
    {:else if loadError}
      <div class="map-overlay" data-testid="map-error">
        <p>{$t('errors.error_loading_photos', { default: 'Error Loading Photos' })}</p>
        <button type="button" class="btn-primary" onclick={() => loadPhotos()}>
          {$t('ui.retry', { default: 'Retry' })}
        </button>
      </div>
    {:else if showEmptyState}
      <div class="map-overlay" data-testid="map-empty-state">
        <p>
          {photos.length === 0
            ? $t('ui.no_photos_found', { default: 'No Photos Found' })
            : $t('map.noGeoPhotos', {
                default: 'No photos with location data in the current filters',
              })}
        </p>
      </div>
    {/if}
  </div>
</div>

<style>
  .map-view {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    min-height: 0;
  }

  .map-status {
    display: flex;
    flex-shrink: 0;
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  .map-notice {
    margin: 0 0 var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--surface-color);
    color: var(--text-secondary);
    font-size: var(--font-sm);
  }

  .map-notice-warning {
    color: var(--text-primary);
  }

  /* The canvas is absolutely positioned so Leaflet always measures a definite
     box, whatever the flex height resolution of the surrounding shell is. */
  .map-stage {
    position: relative;
    flex: 1 1 auto;
    min-height: 260px;
  }

  .map-canvas {
    position: absolute;
    inset: 0;
    background: var(--surface-color);
  }

  /* Leaflet owns the marker DOM, so these are global on purpose. */
  :global(.map-location-marker) {
    width: 18px;
    height: 18px;
    border: 2px solid var(--background-color);
    border-radius: 50%;
    background: var(--primary-color);
    box-shadow: 0 1px 4px rgb(0 0 0 / 40%);
    cursor: pointer;
  }

  :global(.map-cluster-marker) {
    display: flex;
    align-items: center;
    justify-content: center;
    border: 3px solid var(--background-color);
    border-radius: 50%;
    background: var(--primary-color);
    /* White, not `--background-color`: on `--primary-color` the token reaches
       only 4.47:1, under the 4.5:1 the count needs at 13px (SC-008). White is
       what the app already puts on `--primary-color` (see `.btn-primary`) and
       measures 4.83:1. */
    color: white;
    font-size: var(--font-sm);
    font-weight: var(--font-semibold);
  }

  :global(.map-location-marker:focus-visible),
  :global(.map-cluster-marker:focus-visible) {
    outline: 2px solid var(--primary-color);
    outline-offset: 3px;
  }

  /* Above every Leaflet pane (markers 600, popups 700) but below the control
     corners (1000), so the zoom control and the attribution stay reachable. */
  .map-overlay {
    position: absolute;
    inset: 0;
    z-index: 800;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    background: var(--background-color);
    color: var(--text-secondary);
    text-align: center;
  }

  .map-canvas:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: 2px;
  }
</style>
