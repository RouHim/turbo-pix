<script>
  import { onMount } from 'svelte';
  import { t } from '../lib/i18n.js';
  import Icon from '../lib/Icon.svelte';
  import { api } from '../lib/api.js';
  import { addToast } from '../lib/state.svelte.js';
  import { getThumbnailUrl, formatDate, handleError } from '../lib/utils.js';

  let candidates = $state([]);
  let loading = $state(true);
  let error = $state(false);
  let loaded = $state(false);
  let scanning = $state(false);

  onMount(() => {
    loadAndRender();
    window.addEventListener('indexingStatusChanged', handleIndexingStatusChanged);
    return () => window.removeEventListener('indexingStatusChanged', handleIndexingStatusChanged);
  });

  function handleIndexingStatusChanged(e) {
    const phases = e.detail?.phases || [];
    const hk = phases.find((p) => p.id === 'housekeeping');
    if (hk?.state === 'active') {
      scanning = true;
    } else if (hk?.state === 'done') {
      scanning = false;
      loadAndRender();
    }
  }

  async function loadAndRender() {
    loading = true;
    error = false;
    try {
      const response = await api.getHousekeepingCandidates();
      if (response && response.candidates) {
        candidates = response.candidates;
      } else {
        candidates = [];
      }
      loaded = true;
    } catch (e) {
      console.error('Failed to load housekeeping candidates:', e);
      error = true;
    } finally {
      loading = false;
    }
  }

  function enrichPhoto(candidate) {
    const p = { ...candidate.photo };
    p.housekeepingReason = candidate.reason;
    p.housekeepingScore = candidate.score;
    return p;
  }

  async function keepPhoto(photo) {
    try {
      await api.removeHousekeepingCandidate(photo.hash_sha256);
      window.dispatchEvent(
        new CustomEvent('housekeepingCandidateRemoved', {
          detail: { hash: photo.hash_sha256 },
        })
      );
      // Remove from local list
      candidates = candidates.filter((c) => c.photo.hash_sha256 !== photo.hash_sha256);
    } catch (e) {
      handleError(e, 'Keep photo');
    }
  }

  async function deletePhoto(photo) {
    const msg = $t('notifications.confirmDeleteMessage', {
      default:
        'Are you sure you want to permanently delete this photo? This action cannot be undone.',
    });
    if (!confirm(msg)) return;

    try {
      await api.deletePhoto(photo.hash_sha256);
      addToast($t('notifications.photoDeleted', { default: 'Photo deleted' }), '', 'success');
      window.dispatchEvent(
        new CustomEvent('housekeepingCandidateRemoved', {
          detail: { hash: photo.hash_sha256 },
        })
      );
      candidates = candidates.filter((c) => c.photo.hash_sha256 !== photo.hash_sha256);
    } catch (e) {
      handleError(e, 'Delete photo');
    }
  }

  function handleCardClick(e, photo) {
    if (e.target.closest('.card-action-btn')) return;
    window.dispatchEvent(
      new CustomEvent('openViewer', {
        detail: { photo, photos: candidates.map(enrichPhoto) },
      })
    );
  }
</script>

<div class="housekeeping-view">
  {#if loading && !loaded}
    <div class="loading-skeleton">
      <div class="skeleton-item"></div>
      <div class="skeleton-item"></div>
      <div class="skeleton-item"></div>
      <div class="skeleton-item"></div>
    </div>
  {:else if error}
    <div class="error-message">
      {$t('ui.housekeeping_load_failed', { default: 'Could not check for issues' })}
      <button type="button" class="btn" onclick={loadAndRender}>
        {$t('ui.try_again', { default: 'Try Again' })}
      </button>
    </div>
  {:else if scanning}
    <div class="no-photos">
      {$t('ui.housekeeping_scanning', { default: 'Looking for duplicates and issues...' })}
    </div>
  {:else if candidates.length === 0}
    <div class="no-photos">
      {$t('ui.no_housekeeping_candidates', { default: 'No issues found. Your library is clean!' })}
    </div>
  {:else}
    <div class="photo-grid" id="photo-grid">
      {#each candidates as candidate (candidate.photo.hash_sha256)}
        {@const photo = enrichPhoto(candidate)}
        <div
          class="photo-card"
          data-photo-id={photo.hash_sha256}
          role="button"
          tabindex="0"
          onclick={(e) => handleCardClick(e, photo)}
          onkeydown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              handleCardClick(e, photo);
            }
          }}
        >
          <div class="photo-card-image-container image-loaded">
            <img
              class="photo-card-image"
              src={getThumbnailUrl(photo, 'medium')}
              alt={photo.filename || 'Photo'}
              loading="lazy"
              style="opacity: 1"
            />
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
                  Score: {photo.housekeepingScore.toFixed(0)}
                </span>
              {/if}
            </span>
          </div>
          <div class="photo-card-actions">
            <button
              type="button"
              class="card-action-btn keep-btn"
              data-action="keep"
              title={$t('ui.yes', { default: 'Keep' })}
              aria-label={$t('ui.yes', { default: 'Keep' })}
              onclick={() => keepPhoto(photo)}
            >
              <Icon name="check" width={18} height={18} />
            </button>
            <button
              type="button"
              class="card-action-btn delete-housekeeping-btn"
              data-action="delete-housekeeping"
              title={$t('ui.delete_photo', { default: 'Delete' })}
              aria-label={$t('ui.delete_photo', { default: 'Delete' })}
              onclick={() => deletePhoto(photo)}
            >
              <Icon name="trash-2" width={18} height={18} />
            </button>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .housekeeping-view {
    padding: var(--space-4);
  }

  .error-message {
    text-align: center;
    padding: var(--space-8);
    color: var(--color-danger);
    font-size: var(--font-lg);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-4);
  }

  .no-photos {
    text-align: center;
    padding: var(--space-16) var(--space-4);
    color: var(--text-secondary);
    font-size: var(--font-lg);
  }

  .housekeeping-reason {
    color: var(--color-danger, oklch(55% 0.2 25deg));
    font-weight: var(--font-medium);
  }

  .housekeeping-score {
    font-size: var(--font-xs);
    opacity: 0.8;
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
