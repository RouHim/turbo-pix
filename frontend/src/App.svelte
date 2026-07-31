<script>
  import { onMount } from 'svelte';
  import { isLoading } from 'svelte-i18n';
  import { t, initI18n } from './lib/i18n.js';
  import { appState } from './lib/state.svelte.js';
  import { route, init as initRouter } from './lib/router.svelte.js';
  import { api } from './lib/api.js';
  import { throttle } from './lib/utils.js';
  import { APP_CONSTANTS } from './lib/constants.js';
  import Header from './components/Header.svelte';
  import Sidebar from './components/Sidebar.svelte';
  import SortControls from './components/SortControls.svelte';
  import ToastContainer from './lib/ToastContainer.svelte';

  // Feature components — stubbed until later phases fill them in
  import PhotoGrid from './components/PhotoGrid.svelte';
  import CollagesView from './components/CollagesView.svelte';
  import HousekeepingView from './components/HousekeepingView.svelte';
  import PhotoViewer from './components/PhotoViewer.svelte';
  import IndexingOrbit from './components/IndexingOrbit.svelte';
  import TimelineSlider from './components/TimelineSlider.svelte';

  const titleKeys = {
    all: 'ui.all_photos',
    favorites: 'ui.favorites',
    videos: 'ui.videos',
    collages: 'ui.collages',
    housekeeping: 'ui.housekeeping',
  };

  const titleFallbacks = {
    all: 'All Photos',
    favorites: 'Favorites',
    videos: 'Videos',
    collages: 'Collages',
    housekeeping: 'Housekeeping',
  };

  let ready = $state(false);

  const viewTitle = $derived(
    route.query
      ? null // search results title handled separately
      : $t(titleKeys[route.view] || 'ui.all_photos', {
          default: titleFallbacks[route.view] || 'All Photos',
        })
  );

  onMount(() => {
    const updateMobile = throttle(() => {
      appState.isMobile = window.innerWidth < APP_CONSTANTS.MOBILE_BREAKPOINT;
      if (!appState.isMobile) appState.sidebarOpen = false;
    }, 150);

    updateMobile();
    window.addEventListener('resize', updateMobile);

    (async () => {
      let defaultLocale = 'en';
      try {
        const config = await api.getConfig();
        defaultLocale = config?.default_locale || 'en';
      } catch {
        // config endpoint may not be ready; fall through
      }
      initI18n(defaultLocale);
      initRouter();
      appState.currentView = route.view;
      appState.sortOrder = route.sort;
      appState.searchQuery = route.query || '';
      ready = true;
    })();

    return () => window.removeEventListener('resize', updateMobile);
  });

  // Keep appState.currentView in sync with route
  $effect(() => {
    appState.currentView = route.view;
  });
</script>

{#if ready && !$isLoading}
  <Header />
  <Sidebar />
  <main class="main-content">
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
        <SortControls />
      </div>
    </div>

    {#if route.view !== 'collages' && route.view !== 'housekeeping'}
      <TimelineSlider />
    {/if}

    {#if route.view === 'collages'}
      <CollagesView />
    {:else if route.view === 'housekeeping'}
      <HousekeepingView />
    {:else}
      <PhotoGrid />
    {/if}
  </main>

  <IndexingOrbit />
  <PhotoViewer />
  <ToastContainer />
{:else}
  <div class="app-loading">TurboPix</div>
{/if}

<style>
  .main-content {
    margin-left: var(--sidebar-width);
    margin-top: var(--header-height);
    height: calc(100vh - var(--header-height));
    overflow-y: auto;
    padding: var(--space-6);
    background: var(--background-color);
  }

  .content-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: var(--space-6);
  }

  .content-header h2 {
    margin: 0;
    font-family: var(--font-heading);
    font-size: var(--font-3xl);
    font-weight: var(--font-bold);
    color: var(--text-primary);
  }

  .content-actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
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
      padding: var(--space-4);
    }
  }
</style>
