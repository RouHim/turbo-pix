<script>
  import { onMount } from 'svelte';
  import { t } from '../lib/i18n.js';
  import Icon from './Icon.svelte';
  import { api } from '../lib/api.js';
  import {
    addToast,
    selectionState,
    enterSelectionMode,
    toggleSelected,
    selectRange,
    pruneSelection,
  } from '../lib/state.svelte.js';
  import { formatCollageDate, handleError } from '../lib/utils.js';
  import { longpress } from '../lib/longpress.js';

  let collages = $state([]);
  let loading = $state(true);
  let error = $state(false);
  let actionBusy = $state(false);
  let abortController = null;

  function handleCollageAction(e) {
    const id = e.detail?.collageId;
    if (id == null) return;
    collages = collages.filter((c) => c.id !== id);
  }

  onMount(() => {
    window.addEventListener('collageAccepted', handleCollageAction);
    window.addEventListener('collageRejected', handleCollageAction);
    loadPendingCollages();
    return () => {
      window.removeEventListener('collageAccepted', handleCollageAction);
      window.removeEventListener('collageRejected', handleCollageAction);
      if (abortController) abortController.abort();
    };
  });

  async function loadPendingCollages() {
    loading = true;
    error = false;
    if (abortController) abortController.abort();
    abortController = new AbortController();
    try {
      collages = await api.getPendingCollages({ signal: abortController.signal });
      // Reload-prune: covers collages settled elsewhere (another tab, the
      // viewer) while this view was showing a stale list.
      pruneSelection(collages.map((c) => String(c.id)));
    } catch (e) {
      if (e?.name === 'AbortError') return;
      console.error('Failed to load pending collages:', e);
      error = true;
    } finally {
      loading = false;
    }
  }

  async function acceptCollage(collage) {
    if (actionBusy) return;
    actionBusy = true;
    try {
      await api.acceptCollage(collage.id);
      addToast($t('notifications.collageAccepted', { default: 'Collage accepted' }), '', 'success');
      window.dispatchEvent(
        new CustomEvent('collageAccepted', { detail: { collageId: collage.id } })
      );
      // Remove from local list
      collages = collages.filter((c) => c.id !== collage.id);
    } catch (e) {
      handleError(e, 'Accept collage');
    } finally {
      actionBusy = false;
    }
  }

  async function rejectCollage(collage) {
    const msg = $t('messages.confirm_reject_collage', {
      default: 'Are you sure you want to reject this collage?',
    });
    if (!confirm(msg)) return;
    if (actionBusy) return;
    actionBusy = true;

    try {
      await api.rejectCollage(collage.id);
      addToast($t('notifications.collageRejected', { default: 'Collage rejected' }), '', 'info');
      window.dispatchEvent(
        new CustomEvent('collageRejected', { detail: { collageId: collage.id } })
      );
      collages = collages.filter((c) => c.id !== collage.id);
    } catch (e) {
      handleError(e, 'Reject collage');
    } finally {
      actionBusy = false;
    }
  }

  async function generateCollages() {
    if (abortController) abortController.abort();
    abortController = new AbortController();
    try {
      loading = true;
      const result = await api.generateCollages({ signal: abortController.signal });
      const count = result?.count ?? result?.collages_created ?? 0;
      addToast(
        $t('notifications.collagesGenerated', {
          default: `${count} collage(s) generated`,
          values: { count },
        }),
        '',
        'success'
      );
      await loadPendingCollages();
    } catch (e) {
      if (e?.name === 'AbortError') {
        loading = false;
        return;
      }
      handleError(e, 'Generate collages');
      loading = false;
    }
  }

  function collageToPhoto(collage) {
    return {
      hash_sha256: collage.id,
      thumbnail_path: `/api/collages/${collage.id}/image`,
      path: `/api/collages/${collage.id}/image`,
      filename: formatCollageDate(collage.date),
      isCollage: true,
      collageId: collage.id,
      collageDate: collage.date,
      collagePhotoCount: collage.photo_count,
    };
  }

  function openViewer(collage) {
    window.dispatchEvent(
      new CustomEvent('openViewer', {
        detail: { photo: collageToPhoto(collage), photos: collages.map(collageToPhoto) },
      })
    );
  }

  function handleCardClick(e, collage) {
    if (selectionState.active) {
      if (e.shiftKey && selectionState.anchorKey != null) {
        selectRange(
          selectionState.anchorKey,
          String(collage.id),
          collages.map((c) => String(c.id))
        );
      } else {
        toggleSelected(String(collage.id));
      }
      return;
    }
    if (e.target.closest('.card-action-btn')) return;
    openViewer(collage);
  }

  function handleLongPress(collage) {
    if (selectionState.active) return;
    enterSelectionMode();
    toggleSelected(String(collage.id));
  }

  // Surface keys in display order for range selection and select-all-visible.
  $effect(() => {
    selectionState.orderedKeys = collages.map((c) => String(c.id));
  });
</script>

<div class="collages-view">
  {#if loading}
    <div class="loading-skeleton">
      <div class="skeleton-item"></div>
      <div class="skeleton-item"></div>
      <div class="skeleton-item"></div>
      <div class="skeleton-item"></div>
    </div>
  {:else if error}
    <div class="empty-state">
      <div class="empty-state-icon">
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="64"
          height="64"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <circle cx="12" cy="12" r="10"></circle>
          <line x1="12" y1="8" x2="12" y2="12"></line>
          <line x1="12" y1="16" x2="12.01" y2="16"></line>
        </svg>
      </div>
      <p class="empty-state-message">
        {$t('ui.collages_load_failed', { default: 'Failed to load collages' })}
      </p>
      <button type="button" class="btn-primary" onclick={loadPendingCollages}>
        {$t('ui.try_again', { default: 'Try Again' })}
      </button>
    </div>
  {:else if collages.length === 0}
    <div class="empty-state">
      <div class="empty-state-icon">
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="64"
          height="64"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <rect x="3" y="3" width="18" height="18" rx="2" ry="2"></rect>
          <circle cx="8.5" cy="8.5" r="1.5"></circle>
          <polyline points="21 15 16 10 5 21"></polyline>
        </svg>
      </div>
      <p class="empty-state-message">
        {$t('ui.no_pending_collages', { default: 'No pending collages' })}
      </p>
      <button type="button" class="btn-primary" onclick={generateCollages}>
        {$t('ui.generate', { default: 'Generate' })}
      </button>
    </div>
  {:else}
    <div class="collages-header">
      <h2 class="view-title">{$t('ui.pending_collages', { default: 'Pending Collages' })}</h2>
      <button type="button" class="btn-primary generate-btn" onclick={generateCollages}>
        {$t('ui.generate', { default: 'Generate' })}
      </button>
    </div>

    <div class="photo-grid" id="photo-grid">
      {#each collages as collage (collage.id)}
        {@const selected = !!selectionState.selected[String(collage.id)]}
        <div
          class="photo-card collage-card"
          data-photo-id={collage.id}
          class:selected
          use:longpress={{ onLongPress: () => handleLongPress(collage) }}
        >
          {#if selectionState.active}
            <div class="photo-card-selection-badge" aria-hidden="true">
              <Icon name={selected ? 'check-square' : 'square'} width={18} height={18} />
            </div>
          {/if}
          <div
            class="photo-card-open-layer"
            role="button"
            tabindex="0"
            aria-label={$t('ui.collage_for', {
              default: `Collage for ${formatCollageDate(collage.date)}`,
              values: { date: formatCollageDate(collage.date) },
            })}
            aria-pressed={selectionState.active ? selected : undefined}
            onclick={(e) => handleCardClick(e, collage)}
            onkeydown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                if (selectionState.active) {
                  toggleSelected(String(collage.id));
                } else {
                  openViewer(collage);
                }
              }
            }}
          ></div>
          <div class="photo-card-image-container">
            <img
              class="photo-card-image"
              src={`/api/collages/${collage.id}/image`}
              alt={formatCollageDate(collage.date)}
              loading="lazy"
              style="opacity: 1"
            />
          </div>
          <div class="photo-card-overlay">
            <span class="photo-card-title">
              {$t('ui.collage_for', {
                default: `Collage for ${formatCollageDate(collage.date)}`,
                values: { date: formatCollageDate(collage.date) },
              })}
            </span>
            <span class="photo-card-meta">
              {$t('ui.collage_photos', {
                default: `${collage.photo_count} photos`,
                values: { count: collage.photo_count },
              })}
            </span>
          </div>
          {#if !selectionState.active}
            <div class="photo-card-actions collage-actions">
              <button
                type="button"
                class="card-action-btn accept-collage-btn"
                data-action="accept-collage"
                title={$t('ui.accept_collage', { default: 'Accept' })}
                aria-label={$t('ui.accept_collage', { default: 'Accept' })}
                disabled={actionBusy}
                onclick={() => acceptCollage(collage)}
              >
                <Icon name="check" width={18} height={18} />
              </button>
              <button
                type="button"
                class="card-action-btn reject-collage-btn"
                data-action="reject-collage"
                title={$t('ui.reject_collage', { default: 'Reject' })}
                aria-label={$t('ui.reject_collage', { default: 'Reject' })}
                disabled={actionBusy}
                onclick={() => rejectCollage(collage)}
              >
                <Icon name="x" width={18} height={18} />
              </button>
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .collages-view {
    padding: var(--space-4);
  }

  .collages-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-4);
  }

  .view-title {
    font-size: var(--font-xl);
    font-weight: var(--font-semibold);
    margin: 0;
  }

  .generate-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  .empty-state {
    text-align: center;
    padding: var(--space-16) var(--space-4);
  }

  .empty-state-icon {
    color: var(--text-muted);
    margin-bottom: var(--space-4);
  }

  .empty-state-message {
    color: var(--text-secondary);
    font-size: var(--font-lg);
    margin-bottom: var(--space-4);
  }

  .collage-actions {
    display: flex;
    gap: var(--space-2);
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

  .collage-actions .card-action-btn:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .accept-collage-btn {
    background: var(--color-success, oklch(65% 0.18 155deg));
    color: white;
  }

  .accept-collage-btn:hover {
    background: var(--color-success-hover, oklch(58% 0.18 155deg));
  }

  .reject-collage-btn {
    background: var(--color-danger, oklch(55% 0.2 25deg));
    color: white;
  }

  .reject-collage-btn:hover {
    background: var(--color-danger-hover, oklch(48% 0.2 25deg));
  }

  /* Mobile compact grid: mirror PhotoGrid's scoped @container rules so the
     collage grid keeps the compact 3-column layout on narrow containers.
     Scoped rules are required — global @container rules lose the cascade to
     scoped rules (see AGENTS.md). Triggered by the .main-content content
     container (container-type: inline-size). */
  @container (width <= 768px) {
    .photo-grid {
      grid-template-columns: repeat(3, 1fr);
      gap: var(--space-1);
    }
  }

  @container (width <= 480px) {
    .photo-grid {
      grid-template-columns: repeat(3, 1fr);
      gap: 2px;
    }
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
</style>
