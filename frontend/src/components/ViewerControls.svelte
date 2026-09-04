<script>
  import Icon from './Icon.svelte';
  import { t } from '../lib/i18n.js';

  const {
    isVideo = false,
    onZoomIn = () => {},
    onZoomOut = () => {},
    onFitToScreen = () => {},
    onFavorite = () => {},
    onDownload = () => {},
    onAddToAlbum = () => {},
    onMetadata = () => {},
    onFullscreen = () => {},
    onRotateLeft = () => {},
    onRotateRight = () => {},
    onDelete = () => {},
    onAcceptCollage = () => {},
    isFavorite = false,
    showAcceptCollage = false,
    isAcceptingCollage = false,
    rotationDisabled = false,
    rotationDisabledTitle = '',
    sidebarOpen = false,
  } = $props();
</script>

<div class="viewer-controls">
  <div class="viewer-controls-inner">
    <button
      type="button"
      class="zoom-btn zoom-out"
      title={isVideo
        ? $t('ui.zoom_video_unsupported', { default: 'Zoom not supported for videos' })
        : $t('ui.zoom_out', { default: 'Zoom Out' })}
      disabled={isVideo}
      onclick={onZoomOut}
    >
      <Icon name="minus" width={18} height={18} />
    </button>
    <button
      type="button"
      class="zoom-btn zoom-in"
      title={isVideo
        ? $t('ui.zoom_video_unsupported', { default: 'Zoom not supported for videos' })
        : $t('ui.zoom_in', { default: 'Zoom In' })}
      disabled={isVideo}
      onclick={onZoomIn}
    >
      <Icon name="plus" width={18} height={18} />
    </button>
    <button
      type="button"
      class="zoom-btn zoom-fit"
      title={$t('ui.fit_to_screen', { default: 'Fit to Screen' })}
      onclick={onFitToScreen}
    >
      <Icon name="minimize" width={18} height={18} />
    </button>
    <button
      type="button"
      class="zoom-btn fullscreen-btn"
      title={$t('ui.fullscreen', { default: 'Fullscreen' })}
      onclick={onFullscreen}
    >
      <Icon name="maximize" width={18} height={18} />
    </button>
    <button
      type="button"
      class="zoom-btn favorite-btn"
      class:active={isFavorite}
      title={isFavorite
        ? $t('ui.remove_from_favorites', { default: 'Remove from Favorites' })
        : $t('ui.add_to_favorites', { default: 'Add to Favorites' })}
      onclick={onFavorite}
    >
      <Icon name="heart" width={18} height={18} />
    </button>
    <button
      type="button"
      class="zoom-btn download-btn"
      title={$t('ui.download', { default: 'Download' })}
      onclick={onDownload}
    >
      <Icon name="download" width={18} height={18} />
    </button>
    <button
      type="button"
      class="zoom-btn add-album-btn"
      data-action="add-to-album"
      title={$t('albums.addToAlbum', { default: 'Add to album' })}
      aria-label={$t('albums.addToAlbum', { default: 'Add to album' })}
      onclick={onAddToAlbum}
    >
      <Icon name="plus" width={18} height={18} />
    </button>
    <button
      type="button"
      class="zoom-btn metadata-btn"
      title={$t('ui.view_details', { default: 'View Details' })}
      aria-expanded={sidebarOpen}
      aria-controls="viewer-sidebar"
      onclick={onMetadata}
    >
      <Icon name="info" width={18} height={18} />
    </button>
    <button
      type="button"
      class="zoom-btn rotate-left-btn"
      class:btn-disabled={rotationDisabled}
      title={rotationDisabled
        ? rotationDisabledTitle
        : $t('ui.rotate_left', { default: 'Rotate Left 90°' })}
      disabled={rotationDisabled}
      onclick={onRotateLeft}
    >
      <Icon name="rotate-ccw" width={18} height={18} />
    </button>
    <button
      type="button"
      class="zoom-btn rotate-right-btn"
      class:btn-disabled={rotationDisabled}
      title={rotationDisabled
        ? rotationDisabledTitle
        : $t('ui.rotate_right', { default: 'Rotate Right 90°' })}
      disabled={rotationDisabled}
      onclick={onRotateRight}
    >
      <Icon name="rotate-cw" width={18} height={18} />
    </button>
    {#if showAcceptCollage}
      <button
        type="button"
        class="zoom-btn accept-collage-btn"
        data-action="accept-collage"
        title={$t('ui.accept_collage', { default: 'Accept Collage' })}
        disabled={isAcceptingCollage}
        onclick={onAcceptCollage}
      >
        <Icon name="check" width={18} height={18} />
      </button>
    {/if}
    <button
      type="button"
      class="zoom-btn delete-photo-btn"
      title={$t('ui.delete_photo', { default: 'Delete Photo' })}
      onclick={onDelete}
    >
      <Icon name="trash-2" width={18} height={18} />
    </button>
  </div>
</div>

<style>
  .viewer-controls {
    position: absolute;
    bottom: calc(var(--space-6) + env(safe-area-inset-bottom, 0px));
    left: 50%;
    transform: translateX(-50%);
    background: var(--glass-bg);
    backdrop-filter: blur(16px) saturate(1.5);
    -webkit-backdrop-filter: blur(16px) saturate(1.5);
    border: 1px solid var(--glass-border, var(--divider-color));
    box-shadow: var(--shadow-medium);
    border-radius: var(--radius-md);
    padding: var(--space-2);
    z-index: 10;
    max-width: 88%;
  }

  .viewer-controls :global(.accept-collage-btn) {
    background: var(--color-success, oklch(65% 0.18 155deg));
  }
  .viewer-controls :global(.accept-collage-btn:hover) {
    background: var(--color-success-hover, oklch(58% 0.18 155deg));
  }

  @media (width <= 768px) {
    .viewer-controls {
      bottom: calc(var(--space-8) + env(safe-area-inset-bottom, 0px));
    }

    .viewer-controls-inner {
      gap: var(--space-3);
    }

    .zoom-btn {
      width: var(--button-size);
      height: var(--button-size);
    }

    .fullscreen-btn {
      display: none;
    }
  }

  .viewer-controls-inner {
    display: flex;
    gap: var(--space-2);
    overflow: auto hidden;
    -webkit-overflow-scrolling: touch;
    scrollbar-width: none;
    -ms-overflow-style: none;
  }

  .viewer-controls-inner::-webkit-scrollbar {
    display: none;
  }

  .viewer-controls::before,
  .viewer-controls::after {
    content: '';
    position: absolute;
    top: 0;
    bottom: 0;
    width: 40px;
    pointer-events: none;
    z-index: 1;
  }

  .viewer-controls::before {
    left: 0;
    background: linear-gradient(to right, var(--glass-bg), transparent);
    border-radius: var(--radius-md) 0 0 var(--radius-md);
  }

  .viewer-controls::after {
    right: 0;
    background: linear-gradient(to left, var(--glass-bg), transparent);
    border-radius: 0 var(--radius-md) var(--radius-md) 0;
  }

  .zoom-btn {
    width: var(--button-size-sm);
    height: var(--button-size-sm);
    border: none;
    border-radius: var(--radius-full);
    background: var(--viewer-btn-bg);
    backdrop-filter: blur(8px) saturate(1.5);
    -webkit-backdrop-filter: blur(8px) saturate(1.5);
    color: var(--viewer-btn-color);
    font-size: var(--font-xl);
    font-weight: var(--font-bold);
    cursor: pointer;
    transition: var(--transition-fast);
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    position: relative;
    z-index: 2;
  }

  .zoom-btn:hover {
    background: var(--viewer-btn-hover-bg);
    transform: scale(1.1);
  }

  .zoom-btn.active {
    background: var(--primary-color);
    color: white;
  }

  .zoom-btn:disabled,
  .zoom-btn.btn-disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .zoom-btn:disabled:hover,
  .zoom-btn.btn-disabled:hover {
    transform: none;
    background: var(--viewer-btn-bg);
  }

  /* Solid-surface fallbacks: scoped so they outrank the base rules when
     backdrop-filter is unsupported or reduced transparency is requested. */
  @supports not (backdrop-filter: blur(1px)) {
    .viewer-controls {
      background: var(--surface-color);
    }

    .zoom-btn {
      background: oklch(20% 0.01 260deg / 90%);
    }
  }

  @media (prefers-reduced-transparency: reduce) {
    .viewer-controls,
    .zoom-btn {
      backdrop-filter: none;
      -webkit-backdrop-filter: none;
    }

    .viewer-controls {
      background: var(--surface-color);
    }

    .zoom-btn {
      background: oklch(20% 0.01 260deg / 90%);
    }
  }
</style>
