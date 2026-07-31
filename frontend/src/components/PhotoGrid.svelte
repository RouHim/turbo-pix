<script>
  import { onMount, untrack } from 'svelte';
  import { photoGridState, indexingState, addToast } from '../lib/state.svelte.js';
  import { route } from '../lib/router.svelte.js';
  import { api } from '../lib/api.js';
  import { logger } from '../lib/logger.js';
  import { t } from '../lib/i18n.js';
  import Icon from '../lib/Icon.svelte';
  import { APP_CONSTANTS } from '../lib/constants.js';
  import PhotoCard from './PhotoCard.svelte';

  const DEFAULT_BATCH_SIZE = APP_CONSTANTS.DEFAULT_BATCH_SIZE;

  // --- Internal state ---
  let abortController = null;
  let lastLoadSignature = '';
  let loadingStartTime = 0;

  // --- Derived state ---
  const loading = $derived(photoGridState.loading);
  const hasMore = $derived(photoGridState.hasMore);
  const photos = $derived(photoGridState.photos);
  const currentQuery = $derived(photoGridState.currentQuery);
  const semanticSearchMode = $derived(photoGridState.semanticSearchMode);

  // --- Scroll container ref ---
  let scrollContainer = null;
  let throttleTimer = null;
  const THROTTLE_DELAY = 250;
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
    if (route.view === 'videos') {
      filters.query =
        (filters.query ? filters.query + ' ' : '') +
        APP_CONSTANTS.VIDEO_EXTENSIONS.map((e) => `ext:${e}`).join(' OR ');
    }
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
  async function loadPhotos(reset = true) {
    // Dedupe identical concurrent loads (effect + onMount can both fire)
    const sig = `${reset}|${route.view}|${route.query}|${route.sort}|${route.year}|${route.month}|${photoGridState.currentPage}`;
    if (sig === lastLoadSignature) return;
    lastLoadSignature = sig;

    // Cancel any in-flight request
    if (abortController) {
      abortController.abort();
    }

    abortController = new AbortController();
    const signal = abortController.signal;

    photoGridState.loading = true;
    loadingStartTime = Date.now();

    try {
      if (reset) {
        photoGridState.photos = [];
        photoGridState.currentPage = 1;
        photoGridState.hasMore = true;
        photoGridState.currentQuery = route.query || null;
        photoGridState.semanticSearchMode = false;
      }
      let photosList = [];

      // Semantic search path
      if (photoGridState.semanticSearchMode && photoGridState.currentQuery) {
        const offset = (photoGridState.currentPage - 1) * DEFAULT_BATCH_SIZE;
        const result = await api.semanticSearch(
          photoGridState.currentQuery,
          DEFAULT_BATCH_SIZE,
          offset
        );

        if (result.results && result.results.length > 0) {
          const photoHashes = result.results.map((r) => r.hash);
          const photosData = await Promise.all(
            photoHashes.map(async (hash) => {
              try {
                return await api.getPhoto(hash);
              } catch (e) {
                logger.warn(`Failed to load photo ${hash}`, { component: 'PhotoGrid' }, e);
                return null;
              }
            })
          );
          photosList = photosData.filter((p) => p !== null);
          if (logger) {
            logger.info('Semantic search results loaded', {
              component: 'PhotoGrid',
              photosCount: photosList.length,
              offset,
              query: photoGridState.currentQuery,
            });
          }
        }
      } else {
        // Regular photo loading
        const filters = buildFilters();
        const params = {
          page: photoGridState.currentPage,
          limit: DEFAULT_BATCH_SIZE,
          query: photoGridState.currentQuery,
          ...filters,
        };

        const response = await api.getPhotos(params, { signal });
        photosList = response.photos || [];
      }

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
    } catch (error) {
      if (error.name === 'AbortError') {
        if (logger)
          logger.debug('Photo load request was cancelled', {
            component: 'PhotoGrid',
            query: photoGridState.currentQuery,
          });
        return;
      }
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
    } finally {
      // Ensure loading indicator shows for at least 300ms
      const loadingDuration = Date.now() - loadingStartTime;
      const minDisplayTime = 300;
      const remainingTime = Math.max(0, minDisplayTime - loadingDuration);

      setTimeout(() => {
        photoGridState.loading = false;
        // Recheck scroll after load in case more content fits
        requestAnimationFrame(() => {
          setTimeout(() => checkScrollPosition(), 50);
        });
      }, remainingTime);
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
  // Lifecycle — route effects
  // ===========================================================================

  $effect(() => {
    // Track route dependencies to trigger reload
    route.view;
    route.query;
    route.sort;
    route.year;
    route.month;
    untrack(() => loadPhotos(true));
  });
  // ===========================================================================
  // Event listeners
  // ===========================================================================

  function handleFavoriteToggled(event) {
    const { photoHash, isFavorite } = event.detail;
    const card = photoGridState.photos.find((p) => p.hash_sha256 === photoHash);
    if (card) card.is_favorite = isFavorite;
  }

  function handleIndexingStatusChanged() {
    // Force re-eval of the empty state by triggering a reactive update
    // The template checks indexingState.isIndexing directly
    // This just marks reactivity for the empty state recalculation
  }

  onMount(() => {
    window.addEventListener('favoriteToggled', handleFavoriteToggled);
    window.addEventListener('indexingStatusChanged', handleIndexingStatusChanged);

    // Find the scroll container
    scrollContainer = document.querySelector('.main-content');
    if (scrollContainer) {
      scrollContainer.addEventListener('scroll', onScroll, { passive: true });
    }

    return () => {
      window.removeEventListener('favoriteToggled', handleFavoriteToggled);
      window.removeEventListener('indexingStatusChanged', handleIndexingStatusChanged);
      if (scrollContainer) {
        scrollContainer.removeEventListener('scroll', onScroll);
      }
      if (abortController) {
        abortController.abort();
      }
    };
  });

  // ===========================================================================
  // Helpers
  // ===========================================================================

  function showEmptyState() {
    // Handled in template
  }

  function refresh() {
    loadPhotos(true);
  }
</script>

<!-- ========================================================================= -->
<!-- Template                                                                  -->
<!-- ========================================================================= -->

<div id="photo-grid" class="photo-grid">
  {#if loading && photos.length === 0}
    <!-- Skeleton loading -->
    <div class="loading-skeleton">
      {#each Array(6) as _, i (i)}
        <div class="skeleton-item"></div>
      {/each}
    </div>
  {:else if photos.length === 0 && !loading}
    <!-- Empty state -->
    {#if indexingState.isIndexing && !currentQuery}
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
          <Icon name="camera" width={64} height={64} />
        </div>
        <div class="error-state-title">
          {$t('ui.no_photos_found', { default: 'No Photos Found' })}
        </div>
        <div class="error-state-message">
          {#if currentQuery}
            {$t(
              'messages.no_photos_match_search',
              { default: `No photos match your search for "${currentQuery}"` },
              { values: { query: currentQuery } }
            )}
          {:else}
            {$t('messages.no_photos_indexed', { default: 'No photos have been indexed yet' })}
          {/if}
        </div>
        {#if !currentQuery}
          <button class="error-state-button" onclick={() => window.location.reload()}>
            {$t('ui.refresh', { default: 'Refresh' })}
          </button>
        {/if}
      </div>
    {/if}
  {:else}
    <!-- Photo cards -->
    {#each photos as photo (photo.hash_sha256)}
      <PhotoCard {photo} onOpen={openViewer} />
    {/each}
  {/if}
</div>

<div id="load-more-container" class="load-more-container">
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
  {:else if !(loading && photos.length > 0) && !(!hasMore && photos.length > 0)}
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

  .loading-skeleton {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: var(--space-6);
  }

  .loading-skeleton .skeleton-item {
    height: 200px;
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
    .photo-grid,
    .loading-skeleton {
      grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
      gap: var(--space-5);
    }
  }

  @media (width <= 1024px) {
    .photo-grid,
    .loading-skeleton {
      grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
      gap: var(--space-4);
    }
  }

  @media (width >= 1400px) {
    .photo-grid,
    .loading-skeleton {
      grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
      gap: var(--space-8);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .skeleton-item {
      animation: none;
      background: var(--divider-color);
    }
  }
</style>
