<script>
  import { getLocale, t } from '../lib/i18n.js';
  import { getThumbnailUrl } from '../lib/utils.js';
  import { formatDateTime } from '../lib/datetime.js';
  import { formatCoordinates, getLocationLabel } from '../lib/map.js';

  const { location, onOpenPhoto } = $props();

  const place = $derived(getLocationLabel(location) ?? formatCoordinates(location));
</script>

<div class="map-popup">
  <p class="map-popup-place" data-testid="map-popup-place">{place}</p>
  <p class="map-popup-count">
    {location.photos.length === 1
      ? $t('ui.photos_count_one', { default: '1 photo' })
      : $t('ui.photos_count', {
          values: { count: location.photos.length },
          default: '{count} photos',
        })}
  </p>
  <ul class="map-popup-list">
    {#each location.photos as photo (photo.hash_sha256)}
      <li>
        <button
          type="button"
          class="map-popup-item"
          data-map-popup-photo={photo.hash_sha256}
          aria-label={$t('map.openPhoto', {
            values: {
              date: photo.taken_at
                ? formatDateTime(photo.taken_at, getLocale())
                : $t('ui.unknown', { default: 'Unknown' }),
            },
            default: 'Open photo from {date}',
          })}
          onclick={() => onOpenPhoto(photo)}
        >
          <img src={getThumbnailUrl(photo, 'small')} alt="" loading="lazy" decoding="async" />
          <span>
            {photo.taken_at
              ? formatDateTime(photo.taken_at, getLocale())
              : $t('ui.unknown', { default: 'Unknown' })}
          </span>
        </button>
      </li>
    {/each}
  </ul>
</div>

<style>
  .map-popup-place {
    margin: 0;
    color: var(--text-primary);
    font-weight: var(--font-semibold);
  }

  .map-popup-count {
    margin: 0 0 var(--space-2);
    color: var(--text-secondary);
    font-size: var(--font-sm);
  }

  .map-popup-list {
    max-height: 232px;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }

  .map-popup-item {
    display: flex;
    width: 100%;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-primary);
    cursor: pointer;
    text-align: left;
  }

  .map-popup-item:hover,
  .map-popup-item:focus-visible {
    background: var(--surface-color);
  }

  /* Leaflet paints a white wrapper, so a one-line slate ring is the only focus
     indicator that meets contrast in both themes (the UA's black ring reaches
     1.05:1 on a dark tile). Inset so the scroll list cannot clip it. */
  .map-popup-item:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: -2px;
  }

  .map-popup-item img {
    width: var(--space-12);
    height: var(--space-12);
    border-radius: var(--radius-sm);
    object-fit: cover;
  }
</style>
