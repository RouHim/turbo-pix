<script>
  import { onMount, untrack } from 'svelte';
  import {
    photoGridState,
    indexingState,
    addToast,
    selectionState,
    enterSelectionMode,
    toggleSelected,
    selectRange,
    pruneSelection,
  } from '../lib/state.svelte.js';
  import { route } from '../lib/router.svelte.js';
  import { api } from '../lib/api.js';
  import { logger } from '../lib/logger.js';
  import { t } from '../lib/i18n.js';
  import Icon from './Icon.svelte';
  import { APP_CONSTANTS } from '../lib/constants.js';
  import { isPrefixQuery } from '../lib/utils.js';
  import PhotoCard from './PhotoCard.svelte';

  const DEFAULT_BATCH_SIZE = APP_CONSTANTS.DEFAULT_BATCH_SIZE;

  // --- Internal state ---
  let abortController = null;
  let pendingLoadTimer = null;
  let reloadToken = 0;
  let lastLoadSignature = '';
  let loadingStartTime = 0;
  let loadError = $state(null);
  let lastLoadErrorAt = $state(null);

  // --- Derived state ---
  const loading = $derived(photoGridState.loading);
  const hasMore = $derived(photoGridState.hasMore);
  const photos = $derived(photoGridState.photos);
  const currentQuery = $derived(photoGridState.currentQuery);

  // --- Scroll container ref ---
  let scrollContainer = null;
  let throttleTimer = null;
  const THROTTLE_DELAY = 250;
  const LOAD_RETRY_COOLDOWN_MS = 5000;
  const SCROLL_THRESHOLD = 800;

  // ===========================================================================
  // Photo loading
  // ===========================================================================

  /**
   * Builds API filter params from the current route.
   */
  function buildFilters() {
    const filters = {};
    if (route.view === 'favorites') filters.query = 'is_favorite:true';
    if (route.view === 'videos') filters.query = 'type:video';
    if (route.sort) {
      const [field, order] = route.sort.split('_');
      filters.sort = field;
      filters.order = order || 'desc';
    }
    if (route.year) filters.year = route.year;
    if (route.month) filters.month = route.month;
    return filters;
  }

  /**
   * Loads photos with pagination and filtering.
   * @param {boolean} reset - Whether to reset pagination and clear the grid
   */
  /**
   * Resets pagination and reconciles semantic-search mode with the route.
   * Filtered views (favorites/videos) always use the regular search path:
   * semantic results are unfilterable and must not leak into them.
   */
  function applyResetState() {
    if (route.album != null) {
      photoGridState.semanticSearchMode = false;
      photoGridState.currentQuery = null;
    }
    photoGridState.photos = [];
    photoGridState.currentPage = 1;
    photoGridState.hasMore = true;
    if (route.view !== 'all') {
      photoGridState.semanticSearchMode = false;
    } else if (route.query && !isPrefixQuery(route.query)) {
      // Returning to 'all' with a non-prefix query: SearchBar routes every
      // non-prefix query semantically, so a Back from a filtered view must
      // restore semantic mode — otherwise the same URL degrades to a
      // regular text search (route-sync effect no-ops: query unchanged).
      photoGridState.semanticSearchMode = true;
    }
    if (!photoGridState.semanticSearchMode) {
      photoGridState.currentQuery = route.query || null;
    }
    if (!route.query && photoGridState.semanticSearchMode) {
      // URL-driven navigation to a query-less URL must end semantic search
      // (Back from a search). Safety net: works even if PhotoGrid's route
      // effect runs before SearchBar's (SearchBar mounts first — Header
      // precedes <main> in App.svelte — so the pipeline normally wins).
      photoGridState.semanticSearchMode = false;
      photoGridState.currentQuery = null;
    }
  }

  /**
   * Loads one page of semantic results (embeddings are slow, ~3s). Returns
   * null when the response is stale (a newer query/load superseded it) — the
   * caller must no-op instead of polluting the fresh grid.
   * @param {AbortSignal} signal
   * @returns {Promise<Array|null>}
   */
  async function loadSemanticPage(signal) {
    const queryAtStart = photoGridState.currentQuery;
    const offset = (photoGridState.currentPage - 1) * DEFAULT_BATCH_SIZE;
    const result = await api.semanticSearch(
      photoGridState.currentQuery,
      DEFAULT_BATCH_SIZE,
      offset,
      {
        signal,
      }
    );

    if (!result.results || result.results.length === 0) return [];

    const photoHashes = result.results.map((r) => r.hash);
    const photosData = await Promise.all(
      photoHashes.map(async (hash) => {
        try {
          return await api.getPhoto(hash, { signal });
        } catch (e) {
          if (e?.name === 'AbortError') throw e;
          logger.warn(`Failed to load photo ${hash}`, { component: 'PhotoGrid' }, e);
          return null;
        }
      })
    );
    const photosList = photosData.filter((p) => p !== null);
    // Stale ~3s semantic response: a newer query/load superseded this one.
    if (queryAtStart !== photoGridState.currentQuery || signal.aborted) {
      return null;
    }
    if (logger) {
      logger.info('Semantic search results loaded', {
        component: 'PhotoGrid',
        photosCount: photosList.length,
        offset,
        query: queryAtStart,
      });
    }
    return photosList;
  }

  /**
   * Loads one page through the regular search API, merging the user's search
   * term with the view filter (e.g. Favorites + "cat" → "cat is_favorite:true");
   * the backend ANDs the tokens.
   * @param {AbortSignal} signal
   * @returns {Promise<Array>}
   */
  async function loadRegularPage(signal) {
    if (route.album != null) {
      const { sort, order } = buildFilters();
      const response = await api.getEventAlbumPhotos(
        route.album,
        { page: photoGridState.currentPage, limit: DEFAULT_BATCH_SIZE, sort, order },
        { signal }
      );
      return response.photos || [];
    }
    const { query: viewQuery, ...filters } = buildFilters();
    const params = {
      page: photoGridState.currentPage,
      limit: DEFAULT_BATCH_SIZE,
      query: [photoGridState.currentQuery, viewQuery].filter(Boolean).join(' ') || null,
      ...filters,
    };
    const response = await api.getPhotos(params, { signal });
    return response.photos || [];
  }

  /**
   * Appends a page to the grid and advances pagination state.
   * @param {Array} photosList
   */
  function appendPhotos(photosList) {
    if (photosList.length > 0) {
      photoGridState.photos.push(...photosList);
      photoGridState.currentPage++;
      photoGridState.hasMore = photosList.length === DEFAULT_BATCH_SIZE;

      if (logger) {
        logger.info('Photos loaded successfully', {
          component: 'PhotoGrid',
          photosCount: photosList.length,
          totalPhotos: photosList.length,
          page: photoGridState.currentPage - 1,
          hasMore: photoGridState.hasMore,
        });
      }
    } else {
      photoGridState.hasMore = false;
    }
  }

  async function loadPhotos(reset = true) {
    // Dedupe identical concurrent loads (effect + onMount can both fire);
    // reloadToken is bumped by handleIndexingCompleted so a completion
    // reload is never swallowed by the dedupe.
    const sig = `${reset}|${route.view}|${route.query}|${route.sort}|${route.year}|${route.month}|${route.album}|${photoGridState.currentPage}|${reloadToken}`;
    if (sig === lastLoadSignature) return;
    lastLoadSignature = sig;

    // A newer load superseded a pending min-display timer; drop it so the
    // stale load's finally block can't clear the new load's loading state.
    if (pendingLoadTimer) {
      clearTimeout(pendingLoadTimer);
      pendingLoadTimer = null;
    }

    // Cancel any in-flight request
    if (abortController) {
      abortController.abort();
    }

    abortController = new AbortController();
    const signal = abortController.signal;

    photoGridState.loading = true;
    loadingStartTime = Date.now();
    loadError = null;

    try {
      if (reset) applyResetState();

      let photosList;
      if (photoGridState.semanticSearchMode && photoGridState.currentQuery) {
        photosList = await loadSemanticPage(signal);
        // Stale semantic response: no-op instead of polluting the fresh grid
        // and corrupting page state.
        if (photosList === null) return;
      } else {
        photosList = await loadRegularPage(signal);
      }

      appendPhotos(photosList);
      // Backend recovered: allow the next scroll-triggered retry immediately.
      lastLoadErrorAt = null;
    } catch (error) {
      if (error.name === 'AbortError') {
        if (logger)
          logger.debug('Photo load request was cancelled', {
            component: 'PhotoGrid',
            query: photoGridState.currentQuery,
          });
        return;
      }
      // A failed load must not poison the dedupe: without this, every retry
      // (scroll-triggered loadMore, Load More button) rebuilds the same
      // signature and is silently swallowed until a route change or reload.
      lastLoadSignature = null;
      // Cooldown for scroll-triggered retries: a dead backend would otherwise
      // spawn a toast + request per scroll event. Manual retry paths bypass
      // checkScrollPosition and stay immediate.
      lastLoadErrorAt = Date.now();
      if (logger) {
        logger.error('Error loading photos', error, {
          component: 'PhotoGrid',
          method: 'loadPhotos',
          query: photoGridState.currentQuery,
          page: photoGridState.currentPage,
        });
      } else {
        console.error('Error loading photos:', error);
      }
      addToast(
        $t('errors.error_loading_photos', { default: 'Error Loading Photos' }),
        error.message,
        'error',
        5000
      );
      loadError =
        error.message || $t('errors.unexpectedError', { default: 'An unexpected error occurred' });
      // The finally guard below only clears loading while the load is still
      // current, but the error path nulled lastLoadSignature — clear it here
      // or the skeleton/spinner stays forever and retry stays blocked.
      photoGridState.loading = false;
    } finally {
      // Ensure loading indicator shows for at least 300ms. Only the load
      // that is still current may clear the loading state: a superseded
      // (aborted) load must not flip loading = false mid-request (empty-state
      // flash) nor trigger a spurious checkScrollPosition()/loadMore().
      if (sig === lastLoadSignature) {
        const loadingDuration = Date.now() - loadingStartTime;
        const minDisplayTime = 300;
        const remainingTime = Math.max(0, minDisplayTime - loadingDuration);

        pendingLoadTimer = setTimeout(() => {
          photoGridState.loading = false;
          // Recheck scroll after load in case more content fits
          requestAnimationFrame(() => {
            setTimeout(() => checkScrollPosition(), 50);
          });
        }, remainingTime);
      }
    }
  }

  async function loadMore() {
    if (!photoGridState.hasMore || photoGridState.loading) return;
    await loadPhotos(false);
  }

  // ===========================================================================
  // Infinite scroll
  // ===========================================================================

  function onScroll() {
    if (throttleTimer) return;
    throttleTimer = setTimeout(() => {
      throttleTimer = null;
      checkScrollPosition();
    }, THROTTLE_DELAY);
  }

  function checkScrollPosition() {
    if (photoGridState.loading || !photoGridState.hasMore) return;
    // Rate-limit scroll-triggered retries after a failed load (toast/request
    // spam on scroll with a dead backend); manual retries bypass this path.
    if (lastLoadErrorAt && Date.now() - lastLoadErrorAt < LOAD_RETRY_COOLDOWN_MS) return;
    if (!scrollContainer) return;

    const scrollTop = scrollContainer.scrollTop;
    const clientHeight = scrollContainer.clientHeight;
    const scrollHeight = scrollContainer.scrollHeight;
    const distanceFromBottom = scrollHeight - (scrollTop + clientHeight);

    if (distanceFromBottom <= SCROLL_THRESHOLD) {
      loadMore();
    }
  }

  // ===========================================================================
  // Viewer
  // ===========================================================================

  function openViewer(photo) {
    window.dispatchEvent(
      new CustomEvent('openViewer', {
        detail: { photo, photos: photoGridState.photos },
      })
    );
  }

  // ===========================================================================
  // Selection mode
  // ===========================================================================

  function handleSelect(photo, event) {
    if (event.shiftKey && selectionState.anchorKey != null) {
      selectRange(
        selectionState.anchorKey,
        photo.hash_sha256,
        photoGridState.photos.map((p) => p.hash_sha256)
      );
    } else {
      toggleSelected(photo.hash_sha256);
    }
  }

  function handleLongPress(photo) {
    if (selectionState.active) return;
    enterSelectionMode();
    toggleSelected(photo.hash_sha256);
  }

  // Keep the surface's visible keys in display order for range selection and
  // select-all-visible. Reading photoGridState.photos makes this reactive;
  // the orderedKeys assignment is not a read, so there is no loop.
  $effect(() => {
    selectionState.orderedKeys = photoGridState.photos.map((p) => p.hash_sha256);
  });

  // ===========================================================================
  // Lifecycle — route effects
  // ===========================================================================

  $effect(() => {
    // Track route dependencies to trigger reload
    route.view;
    route.query;
    route.sort;
    route.year;
    route.month;
    route.album;
    untrack(() => loadPhotos(true));
  });
  // ===========================================================================
  // Event listeners
  // ===========================================================================

  function handleFavoriteToggled(event) {
    const { photoHash, isFavorite } = event.detail;
    const idx = photoGridState.photos.findIndex((p) => p.hash_sha256 === photoHash);
    if (idx === -1) return;
    if (route.view === 'favorites' && !isFavorite) {
      photoGridState.photos.splice(idx, 1);
      pruneSelection(photoGridState.photos.map((p) => p.hash_sha256));
      refillIfEmpty();
    } else {
      photoGridState.photos[idx].is_favorite = isFavorite;
    }
  }

  function handlePhotoUpdated(event) {
    const updatedPhoto = event.detail?.photo;
    if (!updatedPhoto?.hash_sha256) return;
    // Rotation rewrites hash_sha256: the grid card still carries the OLD
    // hash (hash-embedded thumbnail URLs), so match by the event's oldHash
    // first, then by the new hash (e.g. when the card was added after the
    // rotation), then by file_path.
    const oldHash = event.detail?.oldHash;
    const idx = photoGridState.photos.findIndex(
      (p) =>
        (oldHash && p.hash_sha256 === oldHash) ||
        p.hash_sha256 === updatedPhoto.hash_sha256 ||
        (p.file_path && updatedPhoto.file_path && p.file_path === updatedPhoto.file_path)
    );
    if (idx !== -1) photoGridState.photos[idx] = updatedPhoto;
  }

  function handlePhotoRemoved(event) {
    const { hash } = event.detail || {};
    if (!hash) return;
    const idx = photoGridState.photos.findIndex((p) => p.hash_sha256 === hash);
    if (idx !== -1) {
      photoGridState.photos.splice(idx, 1);
      pruneSelection(photoGridState.photos.map((p) => p.hash_sha256));
      refillIfEmpty();
    }
  }

  // If splicing removed the last visible photo while more pages exist, load the
  // next page so the grid never shows a false empty state ("No Photos Found")
  // with unreachable remaining results.
  function refillIfEmpty() {
    if (photoGridState.photos.length === 0 && photoGridState.hasMore && !photoGridState.loading) {
      loadMore();
    }
  }

  function handleIndexingCompleted() {
    reloadToken++;
    loadPhotos(true);
  }

  function handleReloadRequested() {
    loadPhotos(true);
  }

  onMount(() => {
    window.addEventListener('favoriteToggled', handleFavoriteToggled);
    window.addEventListener('photoUpdated', handlePhotoUpdated);
    window.addEventListener('photoRemoved', handlePhotoRemoved);
    window.addEventListener('indexingCompleted', handleIndexingCompleted);
    window.addEventListener('photosReloadRequested', handleReloadRequested);

    // Find the scroll container
    scrollContainer = document.querySelector('.main-content');
    if (scrollContainer) {
      scrollContainer.addEventListener('scroll', onScroll, { passive: true });
    }

    return () => {
      window.removeEventListener('favoriteToggled', handleFavoriteToggled);
      window.removeEventListener('photoUpdated', handlePhotoUpdated);
      window.removeEventListener('photoRemoved', handlePhotoRemoved);
      window.removeEventListener('indexingCompleted', handleIndexingCompleted);
      window.removeEventListener('photosReloadRequested', handleReloadRequested);
      if (scrollContainer) {
        scrollContainer.removeEventListener('scroll', onScroll);
      }
      if (pendingLoadTimer) {
        clearTimeout(pendingLoadTimer);
        pendingLoadTimer = null;
      }
      scrollContainer = null;
      if (abortController) {
        abortController.abort();
      }
      // A load aborted by unmount must not schedule the min-display timer in
      // its finally block: without this, the global loading flag can be
      // flipped by a destroyed instance and a duplicate page-1 request can
      // slip through on remount. Also clear the flag itself — nothing else
      // would (the aborted load's finally skips its reset), which would leave
      // SearchBar's spinner stuck after a view switch.
      lastLoadSignature = null;
      photoGridState.loading = false;
    };
  });
</script>

<!-- ========================================================================= -->
<!-- Template                                                                  -->
<!-- ========================================================================= -->

<div id="photo-grid" class="photo-grid">
  {#if loading && photos.length === 0}
    <!-- Skeleton loading: items are DIRECT grid children of .photo-grid so the
         skeleton shares the exact tracks of the real cards (a nested grid's fr
         tracks + aspect-ratio items mis-resolve under intrinsic sizing in some
         engines, overflowing the grid). Zero layout shift on arrival. -->
    <!-- eslint-disable-next-line no-unused-vars -- placeholder _ index is part of the skeleton keying idiom -->
    {#each Array(6) as _, i (i)}
      <div class="skeleton-item"></div>
    {/each}
  {:else if photos.length === 0 && !loading}
    <!-- Empty state -->
    {#if loadError}
      <div class="error-state">
        <div class="error-state-icon">
          <Icon name="alert-triangle" width={64} height={64} />
        </div>
        <div class="error-state-title">
          {$t('errors.error_loading_photos', { default: 'Error Loading Photos' })}
        </div>
        <div class="error-state-message">{loadError}</div>
        <button
          class="error-state-button"
          onclick={() => {
            reloadToken++;
            loadPhotos(true);
          }}
        >
          {$t('ui.try_again', { default: 'Try Again' })}
        </button>
      </div>
    {:else if indexingState.isIndexing && !currentQuery}
      <div class="error-state indexing-in-progress">
        <div class="error-state-icon">
          <Icon name="camera" width={64} height={64} />
        </div>
        <div class="error-state-title">
          {$t('messages.indexing_in_progress_title', { default: 'Indexing Your Photos' })}
        </div>
        <div class="error-state-message">
          {$t('messages.indexing_in_progress_message', {
            default:
              'Photos will appear as they are indexed. This may take a while for large collections.',
          })}
        </div>
      </div>
    {:else}
      <div class="error-state">
        <div class="error-state-icon">
          <Icon name={route.album != null ? 'calendar' : 'camera'} width={64} height={64} />
        </div>
        <div class="error-state-title">
          {#if route.album != null}
            {$t('eventAlbums.emptyTitle', { default: 'No Photos' })}
          {:else}
            {$t('ui.no_photos_found', { default: 'No Photos Found' })}
          {/if}
        </div>
        <div class="error-state-message">
          {#if route.album != null}
            {$t('eventAlbums.emptyState', {
              default: "No photos match this album's criteria.",
            })}
          {:else if currentQuery}
            {$t('messages.no_photos_match_search', {
              default: `No photos match your search for "${currentQuery}"`,
              values: { query: currentQuery },
            })}
          {:else}
            {$t('messages.no_photos_indexed', { default: 'No photos have been indexed yet' })}
          {/if}
        </div>
        {#if !currentQuery && route.album == null}
          <button class="error-state-button" onclick={() => window.location.reload()}>
            {$t('ui.refresh', { default: 'Refresh' })}
          </button>
        {/if}
      </div>
    {/if}
  {:else}
    <!-- Photo cards -->
    {#each photos as photo (photo.hash_sha256)}
      <PhotoCard
        {photo}
        onOpen={openViewer}
        selectionMode={selectionState.active}
        selected={!!selectionState.selected[photo.hash_sha256]}
        onSelect={handleSelect}
        onLongPress={handleLongPress}
      />
    {/each}
  {/if}
</div>

<div id="load-more-container" class="load-more-container" class:empty={photos.length === 0}>
  {#if loading && photos.length > 0}
    <!-- Loading more: dot wave animation -->
    <div class="infinite-scroll-loading">
      <div class="dot-wave">
        <div class="dot-wave-dot"></div>
        <div class="dot-wave-dot"></div>
        <div class="dot-wave-dot"></div>
      </div>
    </div>
  {:else if !hasMore && photos.length > 0}
    <!-- End of results -->
    <div class="infinite-scroll-end">
      <div class="end-dots">
        <div class="end-dot"></div>
        <div class="end-dot"></div>
        <div class="end-dot"></div>
      </div>
    </div>
  {:else if photos.length > 0 && !(loading && photos.length > 0) && !(!hasMore && photos.length > 0)}
    <!-- Load More button (shown when idle and hasMore) -->
    <button
      type="button"
      id="load-more-btn"
      class="load-more-btn"
      onclick={loadMore}
      disabled={loading || !hasMore}
    >
      {$t('ui.load_more', { default: 'Load More' })}
    </button>
  {/if}
</div>

<style>
  @keyframes skeleton-loading {
    0% {
      background-position: 100% 50%;
    }
    100% {
      background-position: -100% 50%;
    }
  }

  @keyframes dot-wave {
    0%,
    60%,
    100% {
      transform: translateY(0);
      opacity: 0.7;
    }
    30% {
      transform: translateY(-15px);
      opacity: 1;
    }
  }

  .photo-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: var(--space-6);
    margin-bottom: var(--space-12);
    min-height: 400px;
  }

  .skeleton-item {
    aspect-ratio: 1;
    /* Empty boxes with aspect-ratio get a ~200px auto min-content contribution
       in Chromium, which inflates 1fr grid tracks (3x199 in a 358px grid).
       min-width: 0 opts the tile out of that sizing. */
    min-width: 0;
    background: linear-gradient(
      90deg,
      var(--divider-color) 25%,
      transparent 37%,
      var(--divider-color) 63%
    );
    background-size: 400% 100%;
    animation: skeleton-loading 1.5s ease-in-out infinite;
    border-radius: var(--radius-md);
  }

  .load-more-container {
    display: flex;
    justify-content: center;
    margin: var(--space-8) 0 var(--space-16);
    padding-bottom: var(--space-8);
  }

  .load-more-container.empty {
    display: none;
  }

  .load-more-btn {
    padding: var(--space-3) var(--space-8);
    background: var(--primary-color);
    color: white;
    border: none;
    border-radius: var(--radius-md);
    font-size: var(--font-lg);
    cursor: pointer;
    transition: var(--transition-fast);
  }

  .load-more-btn:hover {
    background: var(--primary-dark);
  }

  .load-more-btn:disabled {
    background: var(--text-secondary);
    cursor: not-allowed;
  }

  .infinite-scroll-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-8) 0;
    min-height: 60px;
  }

  .dot-wave {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .dot-wave-dot {
    width: var(--space-3);
    height: var(--space-3);
    border-radius: var(--radius-full);
    background-color: var(--primary-color);
    animation: dot-wave 1.4s ease-in-out infinite;
    box-shadow: 0 2px 4px rgb(0 0 0 / 10%);
  }

  .dot-wave-dot:nth-child(1) {
    animation-delay: 0s;
  }
  .dot-wave-dot:nth-child(2) {
    animation-delay: 0.2s;
  }
  .dot-wave-dot:nth-child(3) {
    animation-delay: 0.4s;
  }

  .infinite-scroll-end {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-5) 0;
  }

  .end-dots {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .end-dot {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full);
    background-color: var(--text-muted);
    opacity: 0.5;
  }

  /* Empty / error states */
  .error-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: var(--space-12) var(--space-6);
    text-align: center;
    color: var(--text-secondary);
    grid-column: 1 / -1;
  }

  .error-state-icon {
    font-size: var(--font-5xl);
    margin-bottom: var(--space-4);
    opacity: 0.5;
  }

  .error-state-title {
    font-size: var(--font-xl);
    font-weight: var(--font-semibold);
    margin-bottom: var(--space-2);
    color: var(--text-primary);
  }

  .error-state-message {
    font-size: var(--font-base);
    margin-bottom: var(--space-6);
  }

  .error-state-button {
    padding: var(--space-3) var(--space-6);
    background: var(--primary-color);
    color: white;
    border: none;
    border-radius: var(--radius-md);
    cursor: pointer;
    transition: var(--transition-fast);
  }

  .error-state-button:hover {
    background: var(--primary-dark);
  }

  /* Responsive */
  @media (width <= 1200px) {
    .photo-grid {
      grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
      gap: var(--space-5);
    }
  }

  @media (width <= 1024px) {
    .photo-grid {
      grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
      gap: var(--space-4);
    }
  }

  @media (width >= 1400px) {
    .photo-grid {
      grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
      gap: var(--space-8);
    }
  }

  /* Mobile compact grid: must live in scoped styles — global @container rules
     lose the cascade to scoped rules (see AGENTS.md). Triggered by the
     .main-content content container (container-type: inline-size), matching
     the pre-migration @container (width <= 768px) behavior. */
  @container (width <= 768px) {
    .photo-grid {
      grid-template-columns: repeat(3, 1fr);
      gap: var(--space-1);
    }

    .skeleton-item {
      border-radius: 0;
    }
  }

  @container (width <= 480px) {
    .photo-grid {
      grid-template-columns: repeat(3, 1fr);
      gap: 2px;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .skeleton-item {
      animation: none;
      background: var(--divider-color);
    }
  }
</style>
