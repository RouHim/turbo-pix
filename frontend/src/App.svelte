<script>
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import { isLoading } from 'svelte-i18n';
  import { t, initI18n } from './lib/i18n.js';
  import { appState, addToast } from './lib/state.svelte.js';
  import { route, init as initRouter } from './lib/router.svelte.js';
  import { api } from './lib/api.js';
  import { throttle, performance } from './lib/utils.js';
  import { APP_CONSTANTS } from './lib/constants.js';
  import { logger } from './lib/logger.js';
  import Header from './components/Header.svelte';
  import Sidebar from './components/Sidebar.svelte';
  import SortControls from './components/SortControls.svelte';
  import ToastContainer from './lib/ToastContainer.svelte';
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
      const isMobile = window.innerWidth < APP_CONSTANTS.MOBILE_BREAKPOINT;
      if (!isMobile) appState.sidebarOpen = false;
    }, 150);

    updateMobile();
    window.addEventListener('resize', updateMobile);

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
      } catch {
        // config endpoint may not be ready; fall through
      }
      initI18n(defaultLocale);
      initRouter();
      ready = true;
    })();

    return () => {
      window.removeEventListener('resize', updateMobile);
      clearInterval(healthTimer);
      clearInterval(perfTimer);
    };
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
        {#if route.view !== 'collages' && route.view !== 'housekeeping'}
          <SortControls />
        {/if}
      </div>
    </div>

    {#if route.view !== 'collages' && route.view !== 'housekeeping'}
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
    container-type: inline-size;
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
      padding: var(--space-4);
    }
  }
</style>
