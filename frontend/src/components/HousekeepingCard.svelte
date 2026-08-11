<script>
  import { t } from '../lib/i18n.js';
  import Icon from './Icon.svelte';
  import { formatDate, getThumbnailUrl } from '../lib/utils.js';
  import { toDataURL } from '../lib/blurhash.js';
  import { longpress } from '../lib/longpress.js';

  const {
    photo,
    busy = false,
    onKeep = () => {},
    onDelete = () => {},
    onOpen = () => {},
    selected = false,
    selectionMode = false,
    onSelect = null,
    onLongPress = null,
  } = $props();

  let imageLoaded = $state(false);
  let imageError = $state(false);

  const blurhashUrl = $derived.by(() => {
    if (photo?.blurhash) {
      try {
        return toDataURL(photo.blurhash, 32, 32, 1);
      } catch {
        return null;
      }
    }
    return null;
  });

  function handleImageLoad() {
    imageLoaded = true;
  }

  function handleImageError() {
    imageError = true;
  }

  function handleCardClick(e) {
    if (selectionMode) {
      onSelect?.(photo, e);
      return;
    }
    if (e.target.closest('.card-action-btn')) return;
    onOpen(photo);
  }
</script>

<div
  class="photo-card"
  data-photo-id={photo.hash_sha256}
  role="button"
  tabindex="0"
  class:selected
  use:longpress={{ onLongPress: () => onLongPress?.(photo) }}
  onclick={handleCardClick}
  onkeydown={(e) => {
    if (e.key === 'Enter' || e.key === ' ') {
      if (e.target !== e.currentTarget) return; // let action buttons handle their own keys
      e.preventDefault();
      handleCardClick(e);
    }
  }}
>
  {#if selectionMode}
    <div class="photo-card-selection-badge" aria-hidden="true">
      <Icon name={selected ? 'check-square' : 'square'} width={18} height={18} />
    </div>
  {/if}
  <div class="photo-card-image-container" class:image-loaded={imageLoaded}>
    {#if blurhashUrl && !imageLoaded && !imageError}
      <img class="photo-card-blurhash" src={blurhashUrl} alt="" aria-hidden="true" />
    {/if}

    {#if !imageError}
      <picture>
        <source
          type="image/webp"
          srcset="{getThumbnailUrl(photo, 'small')}&format=webp 200w, {getThumbnailUrl(
            photo,
            'medium'
          )}&format=webp 400w, {getThumbnailUrl(photo, 'large')}&format=webp 800w"
          sizes="(max-width: 640px) 200px, (max-width: 1024px) 400px, 800px"
        />
        <source
          type="image/jpeg"
          srcset="{getThumbnailUrl(photo, 'small')}&format=jpeg 200w, {getThumbnailUrl(
            photo,
            'medium'
          )}&format=jpeg 400w, {getThumbnailUrl(photo, 'large')}&format=jpeg 800w"
          sizes="(max-width: 640px) 200px, (max-width: 1024px) 400px, 800px"
        />
        <img
          class="photo-card-image"
          src="{getThumbnailUrl(photo, 'medium')}&format=jpeg"
          alt={photo.filename || $t('ui.photo', { default: 'Photo' })}
          loading="lazy"
          decoding="async"
          onload={handleImageLoad}
          onerror={handleImageError}
        />
      </picture>
    {:else}
      <div class="photo-card-placeholder error-placeholder"></div>
    {/if}

    {#if photo.metadata?.video?.codec}
      <div class="video-play-icon" aria-hidden="true">
        <Icon name="play" width={20} height={20} class="video-play-svg" />
      </div>
    {/if}
  </div>
  <div class="photo-card-overlay">
    <span class="photo-card-title">
      {photo.filename || $t('ui.photo', { default: 'Photo' })}
    </span>
    <span class="photo-card-meta">
      <span>{formatDate(photo.taken_at || photo.date_modified)}</span>
      {#if photo.housekeepingReason}
        <span class="housekeeping-reason">{photo.housekeepingReason}</span>
      {/if}
      {#if photo.housekeepingScore != null}
        <span class="housekeeping-score">
          {$t('ui.housekeeping_score', { default: 'Score' })}: {photo.housekeepingScore.toFixed(0)}
        </span>
      {/if}
    </span>
  </div>
  {#if !selectionMode}
    <div class="photo-card-actions">
      <button
        type="button"
        class="card-action-btn keep-btn"
        data-action="keep"
        title={$t('ui.keep_photo', { default: 'Keep (Remove from housekeeping list)' })}
        aria-label={$t('ui.keep_photo', { default: 'Keep (Remove from housekeeping list)' })}
        disabled={busy}
        onclick={onKeep}
      >
        <Icon name="check" width={18} height={18} />
      </button>
      <button
        type="button"
        class="card-action-btn delete-housekeeping-btn"
        data-action="delete-housekeeping"
        title={$t('ui.delete_photo', { default: 'Delete' })}
        aria-label={$t('ui.delete_photo', { default: 'Delete' })}
        disabled={busy}
        onclick={onDelete}
      >
        <Icon name="trash-2" width={18} height={18} />
      </button>
    </div>
  {/if}
</div>

<style>
  .photo-card-image-container {
    width: 100%;
    aspect-ratio: 1;
    overflow: hidden;
    background: var(--background-secondary);
    display: flex;
    align-items: center;
    justify-content: center;
    position: relative;
  }

  .photo-card-image-container :global(picture) {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    display: block;
  }

  .photo-card-blurhash {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    z-index: 1;
  }

  .photo-card-image {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    transition:
      opacity 0.3s ease-in-out,
      transform var(--transition-slow),
      filter var(--transition-slow);
    filter: brightness(1) contrast(1) saturate(1);
    opacity: 0;
    z-index: 2;
  }

  .photo-card-image-container.image-loaded .photo-card-image {
    opacity: 1;
  }

  .photo-card:hover .photo-card-image {
    transform: scale(1.08);
    filter: brightness(1.1) contrast(1.05) saturate(1.1);
  }

  /* Selection mode: outline + badge (mirrors PhotoCard). */
  .photo-card.selected {
    outline: 2px solid var(--primary-color);
    outline-offset: -2px;
  }

  .photo-card-selection-badge {
    position: absolute;
    top: var(--space-2);
    left: var(--space-2);
    z-index: 4;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    background: var(--glass-bg, oklch(100% 0 0deg / 10%));
    backdrop-filter: blur(8px) saturate(1.5);
    -webkit-backdrop-filter: blur(8px) saturate(1.5);
    color: var(--primary-color);
    pointer-events: none;
  }

  .photo-card.selected .photo-card-selection-badge {
    background: var(--primary-color);
    color: var(--color-bg, white);
  }

  .photo-card-placeholder {
    width: 100%;
    height: 100%;
    background: var(--background-secondary);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-muted);
    font-size: 48px;
  }

  .photo-card-placeholder::before {
    content: '';
    display: inline-block;
    width: var(--space-12);
    height: var(--space-12);
    background: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='48' height='48' viewBox='0 0 24 24' fill='none' stroke='currentColor' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2z'%3E%3C/path%3E%3Ccircle cx='12' cy='13' r='4'%3E%3C/circle%3E%3C/svg%3E")
      center/contain no-repeat;
  }

  .error-placeholder {
    color: var(--color-danger);
    font-size: var(--font-3xl);
  }

  .photo-card-overlay {
    position: absolute;
    inset: 0;
    background: linear-gradient(to top, oklch(0% 0 0deg / 70%) 0%, transparent 100%);
    opacity: 0;
    transition: var(--transition-medium);
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    padding: var(--space-5);
    backdrop-filter: blur(1px);
    -webkit-backdrop-filter: blur(1px);
  }

  .photo-card:hover .photo-card-overlay {
    opacity: 1;
  }

  .photo-card-title {
    color: white;
    font-size: var(--font-md);
    font-weight: var(--font-semibold);
    margin-bottom: 6px;
    text-shadow: 0 1px 2px rgb(0 0 0 / 50%);
    letter-spacing: -0.2px;
  }

  .photo-card-meta {
    color: rgb(255 255 255 / 90%);
    font-size: var(--font-sm);
    font-weight: var(--font-normal);
    text-shadow: 0 1px 2px rgb(0 0 0 / 50%);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .photo-card-actions {
    position: absolute;
    bottom: var(--space-2);
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    opacity: 0;
    transition: var(--transition-medium);
    z-index: 15;
  }

  .photo-card:hover .photo-card-actions {
    opacity: 1;
  }

  .housekeeping-reason {
    color: var(--color-danger, oklch(55% 0.2 25deg));
    font-weight: var(--font-medium);
  }

  .housekeeping-score {
    font-size: var(--font-xs);
    opacity: 0.8;
  }

  .video-play-icon {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: var(--button-size-lg);
    height: var(--button-size-lg);
    background: oklch(0% 0 0deg / 55%);
    border-radius: var(--radius-full);
    display: flex;
    align-items: center;
    justify-content: center;
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    box-shadow: var(--shadow-medium);
    pointer-events: none;
    color: white;
    z-index: 5;
  }

  .video-play-icon :global(svg) {
    fill: white;
    stroke: white;
  }

  .keep-btn {
    background: var(--color-success, oklch(65% 0.18 155deg));
    color: white;
  }

  .keep-btn:hover {
    background: var(--color-success-hover, oklch(58% 0.18 155deg));
  }

  .delete-housekeeping-btn {
    background: var(--color-danger, oklch(55% 0.2 25deg));
    color: white;
  }

  .delete-housekeeping-btn:hover {
    background: var(--color-danger-hover, oklch(48% 0.2 25deg));
  }
</style>
