<script>
  import { get } from 'svelte/store';
  import { api } from '../lib/api.js';
  import { addToast } from '../lib/state.svelte.js';
  import { formatDate, formatFileSize, getThumbnailUrl, getPhotoUrl } from '../lib/utils.js';
  import { toDataURL } from '../lib/blurhash.js';
  import { t } from '../lib/i18n.js';
  import Icon from '../lib/Icon.svelte';

  const { photo, onOpen } = $props();

  // --- Derived state ---
  const isVideo = $derived(photo?.metadata?.video?.codec != null);
  const title = $derived.by(
    () =>
      photo?.filename ||
      `${get(t)('ui.photo', { default: 'Photo' })} ${photo?.hash_sha256?.substring(0, 8)}`
  );
  const meta = $derived.by(() => {
    if (!photo) return '';
    const parts = [];
    if (photo.taken_at) parts.push(formatDate(photo.taken_at));
    const camera = photo.metadata?.camera;
    if (camera?.make && camera?.model) parts.push(`${camera.make} ${camera.model}`);
    if (photo.file_size) parts.push(formatFileSize(photo.file_size));
    return parts.join(' • ');
  });
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

  // --- Image loaded state ---
  let imageLoaded = $state(false);

  // --- Favorite state (optimistic) ---
  const favoriteActive = $derived(!!photo?.is_favorite);

  // Guards against double-clicks while a favorite toggle request is in flight.
  let favoritePending = false;

  function handleCardClick(e) {
    if (!e.target.closest('.card-action-btn')) {
      if (onOpen) {
        onOpen(photo);
      } else {
        window.dispatchEvent(new CustomEvent('openViewer', { detail: { photo } }));
      }
    }
  }

  async function toggleFavorite(e) {
    e.stopPropagation();
    if (favoritePending) return;
    favoritePending = true;
    const wasFavorite = photo.is_favorite;
    const newState = !wasFavorite;

    // Optimistic UI update
    photo.is_favorite = newState;

    try {
      if (newState) {
        await api.addToFavorites(photo.hash_sha256);
      } else {
        await api.removeFromFavorites(photo.hash_sha256);
      }

      const message = newState
        ? $t('messages.photo_added_to_favorites', { default: 'Photo added to favorites' })
        : $t('messages.photo_removed_from_favorites', { default: 'Photo removed from favorites' });
      const title = newState
        ? $t('ui.added', { default: 'Added' })
        : $t('ui.removed', { default: 'Removed' });
      addToast(title, message, 'success', 2000);

      window.dispatchEvent(
        new CustomEvent('favoriteToggled', {
          detail: { photoHash: photo.hash_sha256, isFavorite: newState },
        })
      );
    } catch (error) {
      // Revert on failure
      photo.is_favorite = wasFavorite;
      console.error('Error toggling favorite:', error);
      addToast(
        $t('notifications.error', { default: 'Error' }),
        $t('messages.error_updating_favorite', { default: 'Error updating favorite status' }),
        'error',
        3000
      );
    } finally {
      favoritePending = false;
    }
  }

  function downloadPhoto(e) {
    e.stopPropagation();
    const link = document.createElement('a');
    link.href = getPhotoUrl(photo.hash_sha256);
    link.download = photo.filename || `photo-${photo.hash_sha256.substring(0, 8)}`;
    link.click();
    addToast(
      $t('ui.download', { default: 'Download' }),
      $t('messages.photo_download_started', { default: 'Photo download started' }),
      'info',
      2000
    );
  }

  function onImageLoad() {
    imageLoaded = true;
  }

  let imageError = $state(false);
  function onImageError() {
    imageError = true;
  }
</script>

<div class="photo-card" data-photo-id={photo?.hash_sha256}>
  <div
    class="photo-card-open-layer"
    role="button"
    tabindex="0"
    aria-label={title}
    onclick={handleCardClick}
    onkeydown={(e) => {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        handleCardClick(e);
      }
    }}
  ></div>
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
          alt={title}
          loading="lazy"
          decoding="async"
          onload={onImageLoad}
          onerror={onImageError}
        />
      </picture>
    {:else}
      <div class="photo-card-placeholder error-placeholder"></div>
    {/if}

    {#if isVideo}
      <div class="video-play-icon" aria-hidden="true">
        <Icon name="play" width={20} height={20} class="video-play-svg" />
      </div>
    {/if}
  </div>

  <div class="photo-card-overlay">
    <div class="photo-card-title">{title}</div>
    <div class="photo-card-meta">{meta}</div>
  </div>

  <div class="photo-card-actions">
    <!-- Default context: favorite + download -->
    <button
      class="card-action-btn favorite-btn"
      class:active={favoriteActive}
      title={favoriteActive
        ? $t('ui.remove_from_favorites', { default: 'Remove from Favorites' })
        : $t('ui.add_to_favorites', { default: 'Add to Favorites' })}
      aria-label={favoriteActive
        ? $t('ui.remove_from_favorites', { default: 'Remove from Favorites' })
        : $t('ui.add_to_favorites', { default: 'Add to Favorites' })}
      data-action="favorite"
      onclick={toggleFavorite}
    >
      <Icon name="heart" width={18} height={18} />
    </button>
    <button
      class="card-action-btn download-btn"
      title={$t('ui.download', { default: 'Download' })}
      aria-label={$t('ui.download', { default: 'Download' })}
      data-action="download"
      onclick={downloadPhoto}
    >
      <Icon name="download" width={18} height={18} />
    </button>
  </div>
</div>

<style>
  @keyframes fade-in {
    from {
      opacity: 0;
      transform: translateY(10px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .photo-card {
    position: relative;
    border-radius: var(--radius-md);
    overflow: hidden;
    box-shadow: var(--shadow-light);
    transition: var(--transition-medium);
    cursor: pointer;
    background: var(--surface-color);
    border: 1px solid var(--divider-color);
    animation: fade-in 0.6s ease-out forwards;
    opacity: 0;
    content-visibility: auto;
    contain-intrinsic-size: auto 300px;
  }

  .photo-card:hover {
    box-shadow: var(--shadow-heavy);
    border-color: var(--primary-light);
  }

  .photo-card-open-layer {
    position: absolute;
    inset: 0;
    z-index: 3;
    border-radius: inherit;
  }

  .photo-card-open-layer:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: -2px;
  }

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

  .favorite-btn.active :global(svg) {
    fill: currentcolor;
  }

  .card-action-btn {
    width: 56px;
    height: 56px;
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-full);
    background: var(--surface-color);
    color: var(--text-primary);
    cursor: pointer;
    transition: all var(--transition-fast);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: var(--font-xl);
    box-shadow: var(--shadow-medium);
  }

  .card-action-btn:hover {
    background: var(--primary-color);
    color: white;
    transform: scale(1.15);
    box-shadow: var(--shadow-heavy);
  }

  .card-action-btn:active {
    transform: scale(1.05);
  }

  .card-action-btn.favorite-btn.active {
    color: var(--accent-color);
    transform: scale(1.1);
  }

  .card-action-btn.download-btn {
    font-size: var(--font-2xl);
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

  @media (hover: none) and (pointer: coarse) {
    .photo-card:hover {
      transform: none;
      box-shadow: var(--shadow-light);
    }

    .photo-card:hover .photo-card-image {
      transform: none;
    }

    .photo-card:active {
      transform: scale(0.98);
    }

    .photo-card-overlay {
      opacity: 1;
      background: linear-gradient(to bottom, transparent 0%, oklch(0% 0 0deg / 50%) 100%);
    }

    .photo-card-actions {
      opacity: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .photo-card {
      animation-duration: 0.01ms !important;
      animation-iteration-count: 1 !important;
      transition-duration: 0.01ms !important;
    }
  }

  /* Mobile compact grid: must live in scoped styles — global @container rules
     lose the cascade to scoped rules (see AGENTS.md). Triggered by the
     .main-content content container (container-type: inline-size), matching
     the pre-migration @container (width <= 768px) behavior. */
  @container (width <= 768px) {
    .photo-card {
      border-radius: 0;
      border: none;
    }

    .photo-card-overlay,
    .photo-card-actions {
      display: none;
    }
  }
</style>
