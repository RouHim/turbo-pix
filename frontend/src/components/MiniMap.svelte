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
    const places = get(t)('map.placeAttributionShort', { default: '© GeoNames' });
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

    // No `Leaflet |` prefix: it carries Leaflet's promo link and a flag emoji,
    // neither of which belongs in this chrome, and it alone forced the credit
    // onto a second line inside a 160px preview.
    map.attributionControl.setPrefix(false);
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
    /* Leaflet puts its own font stack on this same element; the scoped class wins. */
    font-family: inherit;
    /* Vertical gestures stay with the panel's scroll container. */
    touch-action: pan-y;
    cursor: pointer;
  }

  /* One line at 11px: the default control wraps to two lines and eats 21% of a
     160px preview. Token ink keeps both links at AA on the translucent plate. */
  .mini-map :global(.leaflet-control-attribution) {
    padding: 2px 6px;
    background: color-mix(in oklch, var(--surface-color) 88%, transparent);
    color: var(--text-secondary);
    font-family: inherit;
    font-size: 11px;
    line-height: 1.4;
  }

  .mini-map :global(.leaflet-control-attribution a) {
    color: var(--primary-ink);
  }

  /* Dark mode turns the room's lights off (DESIGN.md); a daylight raster at
     full brightness reads as a light box inside the paper-dark panel. The full
     map keeps its brightness — it is the content, this preview is chrome. */
  :global(html.dark-theme) .mini-map :global(.leaflet-tile-pane) {
    filter: brightness(0.82) saturate(0.9);
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
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-secondary);
    font: inherit;
    font-size: var(--font-sm);
    cursor: pointer;
  }

  /* The viewer is a touch surface too: 44px is the contract's target size. */
  @media (pointer: coarse) {
    .mini-map-open {
      min-height: 44px;
    }
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
