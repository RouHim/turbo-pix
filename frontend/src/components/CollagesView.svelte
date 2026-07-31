<script>
  import { onMount } from 'svelte';
  import { t } from '../lib/i18n.js';
  import Icon from '../lib/Icon.svelte';
  import { api } from '../lib/api.js';
  import { addToast } from '../lib/state.svelte.js';
  import { formatCollageDate, handleError } from '../lib/utils.js';

  let collages = $state([]);
  let loading = $state(true);
  let error = $state(false);

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
    };
  });

  async function loadPendingCollages() {
    loading = true;
    error = false;
    try {
      collages = await api.getPendingCollages();
    } catch (e) {
      console.error('Failed to load pending collages:', e);
      error = true;
    } finally {
      loading = false;
    }
  }

  async function acceptCollage(collage) {
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
    }
  }

  async function rejectCollage(collage) {
    const msg = $t('messages.confirm_reject_collage', {
      default: 'Are you sure you want to reject this collage?',
    });
    if (!confirm(msg)) return;

    try {
      await api.rejectCollage(collage.id);
      addToast($t('notifications.collageRejected', { default: 'Collage rejected' }), '', 'info');
      window.dispatchEvent(
        new CustomEvent('collageRejected', { detail: { collageId: collage.id } })
      );
      collages = collages.filter((c) => c.id !== collage.id);
    } catch (e) {
      handleError(e, 'Reject collage');
    }
  }

  async function generateCollages() {
    try {
      loading = true;
      const result = await api.generateCollages();
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
      <button type="button" class="btn" onclick={loadPendingCollages}>
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
      <button type="button" class="btn" onclick={generateCollages}>
        {$t('ui.refresh', { default: 'Generate' })}
      </button>
    </div>
  {:else}
    <div class="collages-header">
      <h2 class="view-title">{$t('ui.pending_collages', { default: 'Pending Collages' })}</h2>
      <button type="button" class="btn generate-btn" onclick={generateCollages}>
        + {#if loading}<span class="spinner-sm"></span>{/if}
        {$t('ui.refresh', { default: 'Generate' })}
      </button>
    </div>

    <div class="photo-grid" id="photo-grid">
      {#each collages as collage (collage.id)}
        <div class="photo-card collage-card" data-photo-id={collage.id}>
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="photo-card-image-container" onclick={() => openViewer(collage)}>
            <img
              class="photo-card-image"
              src={`/api/collages/${collage.id}/image`}
              alt={formatCollageDate(collage.date)}
              loading="lazy"
              style="opacity: 1"
            />
          </div>
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="photo-card-overlay" onclick={() => openViewer(collage)}>
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
          <div class="photo-card-actions collage-actions">
            <button
              type="button"
              class="card-action-btn accept-collage-btn"
              data-action="accept-collage"
              title={$t('ui.accept_collage', { default: 'Accept' })}
              aria-label={$t('ui.accept_collage', { default: 'Accept' })}
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
              onclick={() => rejectCollage(collage)}
            >
              <Icon name="x" width={18} height={18} />
            </button>
          </div>
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

  .spinner-sm {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid currentColor;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
