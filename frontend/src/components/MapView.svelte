<script>
  import { get } from 'svelte/store';
  import { onMount, untrack } from 'svelte';
  import L from 'leaflet';
  import 'leaflet/dist/leaflet.css';

  import { t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import { addToast, appState } from '../lib/state.svelte.js';
  import { route } from '../lib/router.svelte.js';
  import { logger } from '../lib/logger.js';
  import {
    buildMapFilters,
    fetchSemanticPhotoSet,
    groupPhotosByLocation,
    isSemanticQuery,
  } from '../lib/map.js';

  const MAX_ZOOM = 19;
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
  let resizeObserver = null;
  let abortController = null;
  let loadToken = 0;
  let anyTileLoaded = false;
  let hasFittedOnce = false;

  // Locations are the shell's aggregation unit: the unlocated notice counts the
  // photos left over, and Task 6 renders one marker per location.
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
      map?.remove();
      map = null;
      tileLayer = null;
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
</script>

<div class="map-view" data-testid="map-view">
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
