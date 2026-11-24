<script>
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import L from 'leaflet';
  import 'leaflet/dist/leaflet.css';

  import { t } from '../lib/i18n.js';
  import { MAP_MAX_ZOOM, mapAttributionHtml } from '../lib/mapAttribution.js';
  import { appState } from '../lib/state.svelte.js';
  import Icon from './Icon.svelte';

  const MINI_MAP_ZOOM = 14;

  const { latitude, longitude, onOpenMap = () => {} } = $props();

  const prefersReducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  let el = null;
  let map = null;
  let marker = null;

  /**
   * The credit for both data sources the picture stands on: the tiles come from
   * the configured OSM endpoint, the place name from the bundled GeoNames data.
   */
  function attributionHtml() {
    const tiles = get(t)('map.attributionLabel', { default: 'OpenStreetMap contributors' });
    const places = get(t)('map.placeAttributionLabel', {
      default: 'Place data © GeoNames',
    });
    return mapAttributionHtml(tiles, places);
  }

  function markerIcon() {
    return L.divIcon({
      className: 'mini-map-marker',
      iconSize: [18, 18],
      iconAnchor: [9, 9],
    });
  }

  function handleMapClick(event) {
    // The attribution links are the one interactive thing on this map: they
    // must open their page, not navigate away from the photo.
    if (event.target.closest?.('a')) return;
    onOpenMap();
  }

  onMount(() => {
    map = L.map(el, {
      center: [latitude, longitude],
      zoom: MINI_MAP_ZOOM,
      zoomControl: false,
      attributionControl: true,
      // Non-interactive on purpose: the map is a preview, and every drag or
      // wheel it swallowed would steal the panel's own scrolling.
      dragging: false,
      touchZoom: false,
      scrollWheelZoom: false,
      doubleClickZoom: false,
      boxZoom: false,
      keyboard: false,
      zoomAnimation: !prefersReducedMotion,
      fadeAnimation: !prefersReducedMotion,
      markerZoomAnimation: !prefersReducedMotion,
    });

    // Registered on the control itself, independently of the tile layer, so it
    // stays visible when the endpoint is unreachable.
    map.attributionControl.addAttribution(attributionHtml());

    if (appState.tileUrl) {
      L.tileLayer(appState.tileUrl, { maxZoom: MAP_MAX_ZOOM }).addTo(map);
    }

    marker = L.marker([latitude, longitude], { interactive: false, icon: markerIcon() }).addTo(map);

    el.addEventListener('click', handleMapClick);

    // The panel is resized by the sidebar/sheet without remounting the map.
    const resizeObserver = new ResizeObserver(() => map?.invalidateSize());
    resizeObserver.observe(el);

    return () => {
      resizeObserver.disconnect();
      el.removeEventListener('click', handleMapClick);
      map?.remove();
      map = null;
      marker = null;
    };
  });

  $effect(() => {
    // The viewer keeps one panel instance and swaps the photo underneath it, so
    // both the marker and the view have to follow the current coordinates.
    if (!map || !marker) return;
    const lat = latitude;
    const lon = longitude;
    marker.setLatLng([lat, lon]);
    const current = map.getCenter();
    if (current.lat === lat && current.lng === lon && map.getZoom() === MINI_MAP_ZOOM) return;
    map.setView([lat, lon], MINI_MAP_ZOOM, { animate: false });
  });
</script>

<div class="mini-map-block">
  <div class="mini-map" bind:this={el} data-testid="mini-map"></div>
  <button type="button" class="mini-map-open" data-testid="mini-map-open" onclick={onOpenMap}>
    <Icon name="map-pin" width={14} height={14} />
    <span>{$t('ui.metadata.show_on_map', { default: 'Show on map' })}</span>
  </button>
</div>

<style>
  .mini-map-block {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .mini-map {
    height: 160px;
    border-radius: var(--radius-md);
    overflow: hidden;
    background: var(--surface-color);
    /* Vertical gestures stay with the panel's scroll container. */
    touch-action: pan-y;
    cursor: pointer;
  }

  .mini-map :global(.leaflet-container) {
    background: var(--surface-color);
    font-family: inherit;
  }

  :global(.mini-map-marker) {
    width: 18px;
    height: 18px;
    border: 2px solid var(--background-color);
    border-radius: 50%;
    background: var(--primary-color);
    box-shadow: 0 1px 4px rgb(0 0 0 / 40%);
  }

  .mini-map-open {
    display: inline-flex;
    align-items: center;
    align-self: flex-start;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-secondary);
    font: inherit;
    font-size: var(--font-sm);
    cursor: pointer;
  }

  .mini-map-open:hover {
    color: var(--text-primary);
    border-color: var(--text-secondary);
  }

  .mini-map-open:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: 2px;
  }
</style>
