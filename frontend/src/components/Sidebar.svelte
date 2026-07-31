<script>
  import { t } from '../lib/i18n.js';
  import { appState } from '../lib/state.svelte.js';
  import { route, pushState } from '../lib/router.svelte.js';

  const views = [
    { id: 'all', key: 'ui.all_photos', fallback: 'All Photos' },
    { id: 'favorites', key: 'ui.favorites', fallback: 'Favorites' },
    { id: 'videos', key: 'ui.videos', fallback: 'Videos' },
    { id: 'collages', key: 'ui.collages', fallback: 'Collages' },
    { id: 'housekeeping', key: 'ui.housekeeping', fallback: 'Housekeeping' },
  ];

  function navigate(view) {
    pushState({ view, query: null, year: null, month: null });
    appState.currentView = view;
    appState.sidebarOpen = false;
  }

  function closeSidebar() {
    appState.sidebarOpen = false;
  }
</script>

<div
  class="sidebar-overlay"
  class:show={appState.sidebarOpen}
  role="presentation"
  onclick={closeSidebar}
></div>

<nav class="sidebar" class:open={appState.sidebarOpen}>
  <div class="sidebar-content">
    {#each views as view (view.id)}
      <button
        type="button"
        class="nav-item"
        class:active={route.view === view.id}
        data-view={view.id}
        onclick={() => navigate(view.id)}
      >
        {$t(view.key, { default: view.fallback })}
      </button>
    {/each}
  </div>
</nav>

<style>
  .sidebar {
    position: fixed;
    top: var(--header-height);
    left: 0;
    width: var(--sidebar-width);
    height: calc(100vh - var(--header-height));
    background: var(--glass-bg);
    backdrop-filter: blur(16px) saturate(1.5);
    -webkit-backdrop-filter: blur(16px) saturate(1.5);
    border-right: 1px solid var(--glass-border, var(--divider-color));
    z-index: 90;
    overflow-y: auto;
  }

  .sidebar-content {
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .nav-item {
    display: flex;
    align-items: center;
    width: 100%;
    padding: var(--space-3) var(--space-4);
    border: none;
    background: transparent;
    color: var(--text-secondary);
    font-size: var(--font-md);
    font-family: var(--font-body);
    text-align: left;
    cursor: pointer;
    border-radius: var(--radius-md);
    transition:
      background var(--transition-fast),
      color var(--transition-fast);
  }

  .nav-item:hover {
    background: var(--background-secondary);
    color: var(--text-primary);
  }

  .nav-item.active {
    background: var(--primary-color);
    color: white;
    font-weight: var(--font-medium);
  }

  .sidebar-overlay {
    display: none;
    position: fixed;
    inset: 0;
    background: oklch(0% 0 0deg / 40%);
    z-index: 85;
  }

  .sidebar-overlay.show {
    display: block;
  }

  @media (max-width: 768px) {
    .sidebar {
      transform: translateX(-100%);
      transition: transform var(--transition-medium);
      z-index: 95;
    }

    .sidebar.open {
      transform: translateX(0);
    }
  }
</style>
