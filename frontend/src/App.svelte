<script>
  import { onMount, untrack } from 'svelte';
  import { MediaQuery } from 'svelte/reactivity';
  import { get } from 'svelte/store';
  import { isLoading } from 'svelte-i18n';
  import { t, initI18n } from './lib/i18n.js';
  import {
    appState,
    addToast,
    selectionState,
    enterSelectionMode,
    exitSelectionMode,
    albums,
    loadAlbums,
    people,
    loadPeople,
  } from './lib/state.svelte.js';
  import { route, init as initRouter, replaceState } from './lib/router.svelte.js';
  import { api } from './lib/api.js';
  import { performance } from './lib/utils.js';
  import { APP_CONSTANTS } from './lib/constants.js';
  import { logger } from './lib/logger.js';
  import Icon from './components/Icon.svelte';
  import Header from './components/Header.svelte';
  import Sidebar from './components/Sidebar.svelte';
  import SortControls from './components/SortControls.svelte';
  import ToastContainer from './components/ToastContainer.svelte';
  import SelectionBar from './components/SelectionBar.svelte';
  import PhotoGrid from './components/PhotoGrid.svelte';
  import CollagesView from './components/CollagesView.svelte';
  import AlbumsView from './components/AlbumsView.svelte';
  import HousekeepingView from './components/HousekeepingView.svelte';
  import PeopleView from './components/PeopleView.svelte';
  import MapView from './components/MapView.svelte';
  import PhotoViewer from './components/PhotoViewer.svelte';
  import IndexingOrbit from './components/IndexingOrbit.svelte';
  import TimelineSlider from './components/TimelineSlider.svelte';
  import ConfirmDialog from './components/ConfirmDialog.svelte';

  const titleKeys = {
    all: 'ui.all_photos',
    favorites: 'ui.favorites',
    videos: 'ui.videos',
    albums: 'albums.sectionTitle',
    collages: 'ui.collages',
    housekeeping: 'ui.housekeeping',
    map: 'ui.map',
    people: 'ui.people',
  };

  const titleFallbacks = {
    all: 'All Photos',
    favorites: 'Favorites',
    videos: 'Videos',
    albums: 'Albums',
    collages: 'Collages',
    housekeeping: 'Housekeeping',
    map: 'Map',
    people: 'People',
  };

  let ready = $state(false);
  const viewTitle = $derived(
    route.album != null
      ? (albums.find((a) => a.id === route.album)?.name ??
          $t('albums.sectionTitle', { default: 'Albums' }))
      : route.query
        ? null // search results title handled separately
        : route.person != null
          ? (people.find((person) => person.id === route.person)?.name ??
            $t('ui.people_person_number', {
              values: { number: route.person },
              default: 'Person {number}',
            }))
          : $t(titleKeys[route.view] || 'ui.all_photos', {
              default: titleFallbacks[route.view] || 'All Photos',
            })
  );

  // Same breakpoint the scoped CSS uses (max-width: 768px), so the drawer
  // state and the layout agree exactly at 768px.
  const isMobileViewport = new MediaQuery(`(max-width: ${APP_CONSTANTS.MOBILE_BREAKPOINT}px)`);
  const SIDEBAR_NAV_ID = 'sidebar-nav';
  const SIDEBAR_COLLAPSE_TOGGLE_ID = 'sidebar-collapse-toggle';

  // The drawer is a mobile surface: crossing back to desktop retires it. If the
  // desktop sidebar lands hidden (`collapsed`), the browser blurs the focused
  // nav item and drops focus on <body>; hand it to the edge tab instead of
  // leaving keyboard users at the top of the document.
  $effect(() => {
    if (isMobileViewport.current) return;
    if (!untrack(() => appState.sidebarOpen)) return;
    // The nav is still focusable at this instant: its `visibility: hidden`
    // transition only ends later, so capturing focus synchronously (the
    // toggle pattern) is the only reliable moment to notice it.
    const focusInDrawer = Boolean(
      document.getElementById(SIDEBAR_NAV_ID)?.contains(document.activeElement)
    );
    appState.sidebarOpen = false;
    if (!appState.sidebarCollapsed) return;
    if (!focusInDrawer) return;
    // The tab is `display: none` until the media query's style recalc lands, so
    // a synchronous focus() can no-op (observed as a flake in the full suite).
    // Focus on the next frame and verify once more if the first attempt lost.
    const focusTab = () => document.getElementById(SIDEBAR_COLLAPSE_TOGGLE_ID)?.focus();
    requestAnimationFrame(() => {
      focusTab();
      if (document.activeElement?.id !== SIDEBAR_COLLAPSE_TOGGLE_ID) {
        requestAnimationFrame(focusTab);
      }
    });
  });

  // A disabled feature must also retire a People route reached through
  // history (the initial deep link is handled after initRouter).
  $effect(() => {
    if (!ready) return;
    if (appState.faceRecognitionEnabled) return;
    if (route.view !== 'people') return;
    // A popstate-driven flush runs inside the router's guard window, where
    // replaceState only rewrites the route and skips the history write; one
    // microtask later that guard has cleared, so re-check and retry then.
    queueMicrotask(() => {
      if (!ready || appState.faceRecognitionEnabled || route.view !== 'people') return;
      replaceState({ view: 'all', person: null });
    });
  });

  // FR-013: selection never leaks across surfaces. A different view OR a
  // different query is a different surface (semantic search results are one);
  // sort/date-range changes keep the same result set, so they do not clear.
  $effect(() => {
    route.view;
    route.query;
    route.album;
    route.person;
    untrack(() => {
      if (selectionState.active) exitSelectionMode();
    });
  });

  onMount(() => {
    let connectionWasUp = true;
    const healthTimer = setInterval(async () => {
      try {
        await api.healthCheck();
        connectionWasUp = true;
      } catch {
        if (connectionWasUp) {
          addToast(
            get(t)('errors.connectionLost', { default: 'Server connection lost' }),
            '',
            'error',
            4000
          );
        }
        connectionWasUp = false;
      }
    }, 30000);

    const perfTimer = setInterval(() => {
      const apiCalls = performance
        .getEntries()
        .filter((e) => e.entryType === 'measure' && e.name.startsWith('api-'));
      if (apiCalls.length > 0) {
        const avg = apiCalls.reduce((sum, e) => sum + e.duration, 0) / apiCalls.length;
        logger.info('Performance metrics', {
          component: 'App',
          metric: 'average_api_response_time',
          averageDurationMs: avg,
          apiCalls: apiCalls.length,
        });
      }
    }, 60000);
    (async () => {
      let defaultLocale = 'en';
      try {
        const config = await api.getConfig();
        defaultLocale = config?.default_locale || 'en';
        appState.tileUrl = config?.tile_url ?? null;
        appState.faceRecognitionEnabled = config?.face_recognition_enabled !== false;
      } catch {
        // config endpoint may not be ready; fall through
      }
      initI18n(defaultLocale);
      initRouter();
      // Populate the albums/people stores on fresh loads so deep-linked
      // titles resolve without their view mounting.
      loadAlbums();
      if (appState.faceRecognitionEnabled) {
        loadPeople();
      } else if (route.view === 'people') {
        replaceState({ view: 'all', person: null });
      }
      ready = true;
    })();

    return () => {
      clearInterval(healthTimer);
      clearInterval(perfTimer);
    };
  });
</script>

{#if ready && !$isLoading}
  <Header />
  <Sidebar />
  <main
    class="main-content"
    data-testid="main-content"
    class:map-mode={route.view === 'map'}
    class:selection-mode={selectionState.active || selectionState.busy}
    class:sidebar-collapsed={appState.sidebarCollapsed}
  >
    <div class="content-header">
      <h2 id="current-view-title">
        {#if route.query}
          {$t('ui.search_results', {
            values: { query: route.query },
            default: `Search: ${route.query}`,
          })}
        {:else}
          {viewTitle}
        {/if}
      </h2>
      <div class="content-actions">
        {#if route.view !== 'collages' && route.view !== 'housekeeping' && !(route.view === 'people' && route.person == null && route.query == null) && (route.view !== 'albums' || route.query != null)}
          <SortControls />
        {/if}
        {#if route.view !== 'map' && !(route.view === 'people' && route.person == null && route.query == null)}
          <button
            type="button"
            class="select-mode-btn"
            data-action="select-mode"
            onclick={() => (selectionState.active ? exitSelectionMode() : enterSelectionMode())}
            aria-pressed={selectionState.active}
          >
            <Icon name="check-square" width={16} height={16} />
            {$t('ui.select', { default: 'Select' })}
          </button>
        {/if}
        {#if route.view === 'albums' && route.album == null && route.query == null}
          <button
            type="button"
            class="btn-primary new-album-btn"
            data-testid="new-album-btn"
            title={$t('albums.newAlbum', { default: 'New album' })}
            onclick={() => window.dispatchEvent(new CustomEvent('openCreateAlbum'))}
          >
            <Icon name="plus" width={16} height={16} />
            {$t('albums.newAlbum', { default: 'New album' })}
          </button>
        {/if}
      </div>
    </div>

    {#if route.view !== 'collages' && route.view !== 'housekeeping' && route.view !== 'people' && (route.view !== 'albums' || route.query != null) && route.album == null}
      <TimelineSlider />
    {/if}

    <svelte:boundary
      onerror={(error) => logger.error('View render error', error, { component: 'App' })}
    >
      {#snippet failed(_error, reset)}
        <div class="view-error">
          <p>{$t('errors.view_crashed', { default: 'Something went wrong' })}</p>
          <button type="button" class="view-error-retry" onclick={reset}>
            {$t('ui.retry', { default: 'Retry' })}
          </button>
        </div>
      {/snippet}
      {#if route.view === 'collages'}
        <CollagesView />
      {:else if route.view === 'housekeeping'}
        <HousekeepingView />
      {:else if route.view === 'map'}
        <MapView />
      {:else if route.view === 'albums' && route.album == null && route.query == null}
        <AlbumsView />
      {:else if route.view === 'people' && route.person == null && route.query == null}
        <PeopleView />
      {:else}
        <PhotoGrid />
      {/if}
    </svelte:boundary>
  </main>

  <IndexingOrbit />
  <svelte:boundary
    onerror={(error) => logger.error('View render error', error, { component: 'App' })}
  >
    {#snippet failed(_error, reset)}
      <div class="view-error">
        <p>{$t('errors.view_crashed', { default: 'Something went wrong' })}</p>
        <button type="button" class="view-error-retry" onclick={reset}>
          {$t('ui.retry', { default: 'Retry' })}
        </button>
      </div>
    {/snippet}
    <PhotoViewer />
  </svelte:boundary>
  {#if selectionState.active || selectionState.busy}
    <SelectionBar />
  {/if}
  <ToastContainer />
  <ConfirmDialog />
{:else}
  <div class="app-loading">TurboPix</div>
{/if}

<style>
  /* The canvas starts at the viewport top and scrolls UNDER the fixed glass
     header; the header's blur only reads as frosted glass when the grid
     passes beneath it. The top inset is padding (not a margin), so it scrolls
     away and the photos reach the header's bottom edge. */
  .main-content {
    margin-left: var(--sidebar-width);
    height: 100vh;
    overflow-y: auto;
    padding: calc(var(--header-height) + var(--space-6)) var(--space-6) var(--space-6);
    background: var(--background-color);
    container-type: inline-size;
    /* The drawer owns horizontal swipes; without this the browser treats the
       first horizontal move as a scroll attempt and cancels the touch stream
       before preventDefault can run. Vertical scrolling and page pinch-zoom
       stay available; the map/timeline/date-rail children set their own. */
    touch-action: pan-y pinch-zoom;
  }

  /* The mobile drawer overlays the content, so only desktop reclaims width.
     The margin snaps (no transition): a layout-property animation here
     re-measures the timeline while it runs and breaks resize geometry. */
  @media (width > 768px) {
    .main-content.sidebar-collapsed {
      margin-left: 0;
    }
  }

  .content-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-6);
    /* Only the header reserves the rail's edge; the grid floats beneath the
       glass. Set by DateScrubber while the rail shows. */
    padding-right: var(--date-rail-inset, 0px);
  }

  @media (max-width: 640px) {
    /* The wrapped selection panel is ~216 px tall at 390 px; reserve its band
       so the last row can still scroll clear of it. */
    .main-content.selection-mode {
      padding-bottom: calc(232px + var(--space-4));
    }
  }

  /* The Map view fills the shell instead of scrolling: the map owns the
     remaining height and pans internally. */
  .main-content.map-mode {
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .content-header h2 {
    margin: 0;
    font-family: var(--font-heading);
    font-size: var(--font-4xl);
    font-weight: var(--font-semibold);
    color: var(--text-primary);
    text-wrap: balance;
  }

  .content-actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .select-mode-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    white-space: nowrap;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-md);
    background: var(--surface-color);
    color: var(--text-primary);
    font-size: var(--font-base);
    font-family: var(--font-body);
    cursor: pointer;
  }
  .select-mode-btn:hover {
    border-color: var(--primary-color);
  }
  .select-mode-btn[aria-pressed='true'] {
    border-color: var(--primary-color);
    color: var(--primary-color);
  }
  .select-mode-btn:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: 2px;
  }
  .select-mode-btn :global(svg) {
    width: 16px;
    height: 16px;
  }
  .new-album-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    white-space: nowrap;
  }

  .view-error {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-8);
    color: var(--text-secondary);
  }

  .view-error-retry {
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius-md);
    background: var(--accent-color);
    color: var(--text-on-accent);
    border: none;
    cursor: pointer;
  }

  .view-error-retry:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: 2px;
  }

  /* Responsive content header: scoped so these win over the base rules
     (global @media overrides of equal specificity are outranked by scoped
     rules). */
  @media (max-width: 480px) {
    .content-header {
      flex-direction: column;
      align-items: flex-start;
      gap: var(--space-3);
      margin-bottom: var(--space-4);
    }
    .content-header h2 {
      font-size: var(--font-2xl);
      margin: 0;
    }
  }

  .app-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100vh;
    font-family: var(--font-heading);
    font-size: var(--font-4xl);
    color: var(--text-primary);
  }

  @media (max-width: 768px) {
    .main-content {
      margin-left: 0;
      padding: calc(var(--header-height) + var(--space-4)) var(--space-4) var(--space-4);
    }

    .select-mode-btn {
      min-height: var(--button-size);
    }
  }
</style>
