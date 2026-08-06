<script>
  import { onMount } from 'svelte';
  import { t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import {
    addToast,
    indexingState,
    selectionState,
    enterSelectionMode,
    toggleSelected,
    selectRange,
    pruneSelection,
  } from '../lib/state.svelte.js';
  import { handleError } from '../lib/utils.js';
  import HousekeepingCard from './HousekeepingCard.svelte';

  let candidates = $state([]);
  let loading = $state(true);
  let error = $state(false);
  let loaded = $state(false);
  let scanning = $state(false);
  let busy = $state(false);
  let lastHkState = null;
  let abortController = null;

  onMount(() => {
    const hk = (indexingState.phases || []).find((p) => p.id === 'housekeeping');
    if (hk?.state === 'active') {
      scanning = true;
      lastHkState = 'active';
    }
    loadAndRender();
    window.addEventListener('indexingStatusChanged', handleIndexingStatusChanged);
    window.addEventListener('photoRemoved', handlePhotoRemoved);
    window.addEventListener('housekeepingKept', handleHousekeepingKept);
    return () => {
      window.removeEventListener('indexingStatusChanged', handleIndexingStatusChanged);
      window.removeEventListener('photoRemoved', handlePhotoRemoved);
      window.removeEventListener('housekeepingKept', handleHousekeepingKept);
      if (abortController) abortController.abort();
    };
  });

  function handleHousekeepingKept(event) {
    const hashes = event.detail?.hashes || [];
    if (hashes.length === 0) return;
    candidates = candidates.filter((c) => !hashes.includes(c.photo.hash_sha256));
    pruneSelection(candidates.map((c) => c.photo.hash_sha256));
  }

  function handlePhotoRemoved(event) {
    const { hash } = event.detail || {};
    if (hash) candidates = candidates.filter((c) => c.photo.hash_sha256 !== hash);
  }

  function handleIndexingStatusChanged(e) {
    const phases = e.detail?.phases || [];
    const hk = phases.find((p) => p.id === 'housekeeping');
    const wasActive = lastHkState === 'active';
    if (hk?.state === 'active') {
      scanning = true;
      lastHkState = 'active';
    } else if (wasActive) {
      // Scan ended in any way (done / error / interrupted run) — recover the
      // list like the vanilla view did for every non-indexing state, instead
      // of stranding the view in the scanning message.
      scanning = false;
      lastHkState = hk?.state ?? null;
      loadAndRender();
    } else {
      lastHkState = hk?.state ?? null;
    }
  }

  async function loadAndRender() {
    loading = true;
    error = false;
    const controller = new AbortController();
    abortController?.abort();
    abortController = controller;
    try {
      const response = await api.getHousekeepingCandidates({
        signal: controller.signal,
      });
      if (response && response.candidates) {
        candidates = response.candidates;
      } else {
        candidates = [];
      }
      // No pagination here: the loaded list IS the surface. Drop selections
      // for candidates a background rescan removed mid-session.
      pruneSelection(candidates.map((c) => c.photo.hash_sha256));
      loaded = true;
    } catch (e) {
      if (e?.name === 'AbortError') return;
      console.error('Failed to load housekeeping candidates:', e);
      error = true;
    } finally {
      // A superseded (aborted) request must not clear the newer request's flag.
      if (abortController === controller) loading = false;
    }
  }

  function enrichPhoto(candidate) {
    const p = { ...candidate.photo };
    p.housekeepingReason = candidate.reason;
    p.housekeepingScore = candidate.score;
    return p;
  }

  async function keepPhoto(photo) {
    if (busy) return;
    busy = true;
    try {
      await api.removeHousekeepingCandidate(photo.hash_sha256);
      addToast(
        $t('notifications.kept', { default: 'Kept' }),
        $t('notifications.photoKept', { default: 'Photo removed from housekeeping candidates' }),
        'success',
        2000
      );
      // Remove from local list
      candidates = candidates.filter((c) => c.photo.hash_sha256 !== photo.hash_sha256);
    } catch (e) {
      handleError(e, 'Keep photo');
    } finally {
      busy = false;
    }
  }

  async function deletePhoto(photo) {
    const msg = $t('notifications.confirmDeleteMessage', {
      default:
        'Are you sure you want to permanently delete this photo? This action cannot be undone.',
    });
    if (!confirm(msg)) return;

    if (busy) return;
    busy = true;
    try {
      await api.deletePhoto(photo.hash_sha256);
      // Keep the shared deletion contract: other views listen for
      // photoRemoved to sync their photo lists.
      window.dispatchEvent(
        new CustomEvent('photoRemoved', { detail: { hash: photo.hash_sha256 } })
      );
      addToast($t('notifications.photoDeleted', { default: 'Photo deleted' }), '', 'success');
      candidates = candidates.filter((c) => c.photo.hash_sha256 !== photo.hash_sha256);
    } catch (e) {
      handleError(e, 'Delete photo');
    } finally {
      busy = false;
    }
  }

  // ===========================================================================
  // Selection mode
  // ===========================================================================

  // HousekeepingCard passes the enriched photo (hash on the photo itself),
  // not the raw candidate wrapper.
  function handleSelect(photo, event) {
    const key = photo.hash_sha256;
    if (event.shiftKey && selectionState.anchorKey != null) {
      selectRange(
        selectionState.anchorKey,
        key,
        candidates.map((c) => c.photo.hash_sha256)
      );
    } else {
      toggleSelected(key);
    }
  }

  function handleLongPress(photo) {
    if (selectionState.active) return;
    enterSelectionMode();
    toggleSelected(photo.hash_sha256);
  }

  // Surface keys in display order for range selection and select-all-visible.
  $effect(() => {
    selectionState.orderedKeys = candidates.map((c) => c.photo.hash_sha256);
  });
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
        <HousekeepingCard
          {photo}
          {busy}
          onKeep={() => keepPhoto(photo)}
          onDelete={() => deletePhoto(photo)}
          onOpen={(p) => {
            window.dispatchEvent(
              new CustomEvent('openViewer', {
                detail: { photo: p, photos: candidates.map(enrichPhoto) },
              })
            );
          }}
          selectionMode={selectionState.active}
          selected={!!selectionState.selected[photo.hash_sha256]}
          onSelect={handleSelect}
          onLongPress={handleLongPress}
        />
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
</style>
