<script>
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import Icon from './Icon.svelte';
  import { t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import {
    addToast,
    appState,
    loadSavedSearches,
    savedSearches,
    setSidebarCollapsed,
  } from '../lib/state.svelte.js';
  import { route, pushState } from '../lib/router.svelte.js';
  import { handleError } from '../lib/utils.js';

  const views = [
    { id: 'all', key: 'ui.all_photos', fallback: 'All Photos' },
    { id: 'favorites', key: 'ui.favorites', fallback: 'Favorites' },
    { id: 'videos', key: 'ui.videos', fallback: 'Videos' },
    { id: 'albums', key: 'albums.sectionTitle', fallback: 'Albums' },
    { id: 'map', key: 'ui.map', fallback: 'Map' },
    { id: 'collages', key: 'ui.collages', fallback: 'Collages' },
    { id: 'housekeeping', key: 'ui.housekeeping', fallback: 'Housekeeping' },
  ];

  // Desktop edge tab: the sidebar's own collapse control.
  const collapseLabel = $derived(
    appState.sidebarCollapsed
      ? $t('ui.expand_sidebar', { default: 'Expand sidebar' })
      : $t('ui.collapse_sidebar', { default: 'Collapse sidebar' })
  );

  function navigate(view) {
    appState.mobileSearchOpen = false;
    if (view === 'all') {
      // Clear any active search — matches the Header logo behavior and the
      // old app.js nav handler (which called search.clearSearch()).
      if (route.view === 'all' && !route.query && route.album == null) {
        appState.sidebarOpen = false;
        return;
      }
      pushState({ view: 'all', query: null, album: null });
      appState.sidebarOpen = false;
      return;
    }
    if (route.view === view && route.album == null && (view !== 'albums' || route.query == null)) {
      // Close the drawer even when tapping the already-active view (mobile).
      appState.sidebarOpen = false;
      return;
    }
    // Albums list shows no search results: entering it with ?q= set would
    // render the grid under an album-list header, so clear the query.
    pushState(view === 'albums' ? { view, album: null, query: null } : { view, album: null });
    appState.sidebarOpen = false;
  }

  function closeSidebar() {
    appState.sidebarOpen = false;
  }

  let renamingId = $state(null);
  let renameName = $state('');
  let renameInputEl = $state(null);
  onMount(() => {
    loadSavedSearches();
  });
  function isActiveSearch(item) {
    // Saved searches never carry an album; an open album means the search
    // state is not what is shown, even if view/query/sort/year/month match.
    return (
      route.album == null &&
      route.view === item.view &&
      route.query === item.query &&
      route.sort === item.sort &&
      route.year === item.year &&
      route.month === item.month &&
      route.to_year === item.to_year &&
      route.to_month === item.to_month
    );
  }

  function openSavedSearch(item) {
    appState.mobileSearchOpen = false;
    if (isActiveSearch(item)) {
      // Already showing this state — mirrors navigate().
      appState.sidebarOpen = false;
      return;
    }
    pushState({
      view: item.view,
      query: item.query,
      sort: item.sort,
      year: item.year,
      month: item.month,
      to_year: item.to_year,
      to_month: item.to_month,
      album: null,
    });
    appState.sidebarOpen = false;
    // NOTE: photo is deliberately not touched — same as existing view navigation.
  }

  function startRename(item) {
    renamingId = item.id;
    renameName = item.name;
  }

  $effect(() => {
    if (renamingId !== null) {
      renameInputEl?.focus();
      renameInputEl?.select();
    }
  });

  function cancelRename() {
    renamingId = null;
  }

  async function confirmRename() {
    const name = renameName.trim();
    if (!name) {
      addToast(
        get(t)('savedSearches.errorNameRequired', { default: 'Name cannot be empty' }),
        '',
        'error',
        3000
      );
      return; // stay in edit mode; previous name is kept on cancel
    }
    const id = renamingId;
    try {
      const updated = await api.renameSavedSearch(id, name);
      const item = savedSearches.find((s) => s.id === id);
      if (item) item.name = updated.name; // $state mutation is reactive
      renamingId = null;
    } catch (error) {
      handleError(error, 'rename saved search');
    }
  }

  function onRenameKeydown(e) {
    if (e.key === 'Enter') {
      e.preventDefault();
      confirmRename();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      cancelRename();
    }
  }

  async function deleteSearch(item) {
    try {
      await api.deleteSavedSearch(item.id);
      const idx = savedSearches.findIndex((s) => s.id === item.id);
      if (idx !== -1) savedSearches.splice(idx, 1);
    } catch (error) {
      handleError(error, 'delete saved search');
    }
  }

  // Escape closes the sidebar whenever it is open.
  $effect(() => {
    function onKey(e) {
      if (e.key === 'Escape' && appState.sidebarOpen) appState.sidebarOpen = false;
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });
</script>

<div
  class="sidebar-overlay"
  data-testid="sidebar-overlay"
  class:show={appState.sidebarOpen}
  role="presentation"
  onclick={closeSidebar}
></div>

<nav
  id="sidebar-nav"
  class="sidebar"
  data-testid="sidebar-nav"
  class:open={appState.sidebarOpen}
  class:collapsed={appState.sidebarCollapsed}
>
  <div class="sidebar-content">
    {#each views as view (view.id)}
      {@const isAlbumsNav =
        view.id === 'albums' && (route.view === 'albums' || route.album != null)}
      {@const isAllNav =
        view.id === 'all'
          ? route.view === 'all' && route.album == null
          : route.view === view.id && view.id !== 'albums'}
      {@const isActive = view.id === 'albums' ? isAlbumsNav : isAllNav}
      <button
        type="button"
        class="nav-item"
        class:active={isActive}
        data-view={view.id}
        aria-current={isActive ? 'page' : undefined}
        onclick={() => navigate(view.id)}
      >
        {$t(view.key, { default: view.fallback })}
      </button>
    {/each}

    {#if savedSearches.length > 0}
      <div class="sidebar-section-title">
        {$t('savedSearches.sectionTitle', { default: 'Saved searches' })}
      </div>
      {#each savedSearches as item (item.id)}
        <div
          class="saved-search-row"
          class:active={isActiveSearch(item)}
          data-testid="saved-search-row"
        >
          <button
            type="button"
            class="saved-search-open"
            title={item.name}
            aria-current={isActiveSearch(item) ? 'true' : undefined}
            onclick={() => openSavedSearch(item)}
            data-testid="saved-search-open"
          >
            <Icon name="bookmark" width={14} height={14} />
            <span class="saved-search-name">{item.name}</span>
          </button>
          {#if renamingId === item.id}
            <input
              type="text"
              class="saved-search-rename-input"
              bind:value={renameName}
              bind:this={renameInputEl}
              onkeydown={onRenameKeydown}
              onblur={cancelRename}
              aria-label={$t('savedSearches.rename', { default: 'Rename' })}
              data-testid="saved-search-name-input"
            />
          {:else}
            <button
              type="button"
              class="saved-search-action"
              title={$t('savedSearches.rename', { default: 'Rename' })}
              aria-label={$t('savedSearches.rename', { default: 'Rename' })}
              onclick={() => startRename(item)}
              data-testid="saved-search-rename"
              ><Icon name="edit-2" width={14} height={14} /></button
            >
            <button
              type="button"
              class="saved-search-action"
              title={$t('savedSearches.delete', { default: 'Delete' })}
              aria-label={$t('savedSearches.delete', { default: 'Delete' })}
              onclick={() => deleteSearch(item)}
              data-testid="saved-search-delete"
              ><Icon name="trash-2" width={14} height={14} /></button
            >
          {/if}
        </div>
      {/each}
    {/if}
  </div>
</nav>

<button
  type="button"
  id="sidebar-collapse-toggle"
  class="sidebar-toggle"
  class:collapsed={appState.sidebarCollapsed}
  data-testid="sidebar-collapse-toggle"
  title={collapseLabel}
  aria-label={collapseLabel}
  aria-expanded={!appState.sidebarCollapsed}
  aria-controls="sidebar-nav"
  onclick={() => setSidebarCollapsed(!appState.sidebarCollapsed)}
>
  <span class="sidebar-toggle-tab">
    <Icon
      name={appState.sidebarCollapsed ? 'chevron-right' : 'chevron-left'}
      width={16}
      height={16}
    />
  </span>
</button>

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
    transition:
      transform var(--transition-medium),
      visibility var(--transition-medium);
  }

  .sidebar-content {
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  /* The sidebar's own collapse control. The button keeps a 44×48 hit area
     while the visible 24×48 tab straddles the sidebar border. Bottom-anchored
     so the collapsed tab never covers the view title; collapsed it parks flush
     against the viewport edge (left: -10px keeps the tab clear of the
     content's 24px padding) and stays the way back in. The tab carries the
     primary accent so it reads as a control against the paper-white shell. */
  .sidebar-toggle {
    position: fixed;
    bottom: var(--space-6);
    left: calc(var(--sidebar-width) - 22px);
    z-index: 95;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 44px;
    height: 48px;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
    transition: left var(--transition-medium);
  }

  .sidebar-toggle.collapsed {
    left: -10px;
  }

  .sidebar-toggle-tab {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 48px;
    border: 1px solid var(--divider-color);
    border-left: none;
    border-radius: 0 var(--radius-md) var(--radius-md) 0;
    background: var(--surface-color);
    box-shadow: var(--shadow-light);
    opacity: 0.55;
    transition:
      opacity var(--transition-fast),
      background var(--transition-fast),
      border-color var(--transition-fast);
  }

  .sidebar-toggle:hover .sidebar-toggle-tab,
  .sidebar-toggle:focus-visible .sidebar-toggle-tab {
    opacity: 1;
  }

  /* Collapsed, the tab is the only way back in, so it carries the primary
     accent at full contrast; expanded it stays quiet shell chrome. */
  .sidebar-toggle.collapsed .sidebar-toggle-tab {
    border-color: transparent;
    background: var(--primary-color);
    color: white;
    opacity: 1;
  }

  .sidebar-toggle.collapsed:hover .sidebar-toggle-tab {
    background: var(--primary-dark);
  }

  .sidebar-toggle:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: 2px;
    border-radius: var(--radius-md);
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

  .nav-item:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: -2px;
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

  @media (max-width: 768px) {
    .sidebar {
      transform: translateX(-100%);
      visibility: hidden;
      z-index: 95;
      width: 280px;
      /* The drawer floats over the photo mosaic: text on 70% glass over a
         busy image is unreadable, so the mobile drawer is a solid surface. */
      background: var(--surface-color);
      backdrop-filter: none;
      -webkit-backdrop-filter: none;
    }

    .sidebar.open {
      transform: translateX(0);
      visibility: visible;
    }

    .sidebar-overlay {
      display: block;
      opacity: 0;
      visibility: hidden;
      transition:
        opacity var(--transition-medium),
        visibility var(--transition-medium);
    }

    .sidebar-overlay.show {
      opacity: 1;
      visibility: visible;
    }

    /* The edge tab is desktop chrome; the hamburger owns the mobile drawer. */
    .sidebar-toggle {
      display: none;
    }
  }

  /* Desktop collapse; the mobile drawer keeps its own transform lifecycle. */
  @media (width > 768px) {
    .sidebar.collapsed {
      transform: translateX(-100%);
      visibility: hidden;
    }
  }

  @media (width <= 480px) {
    .nav-item {
      padding: 14px var(--space-5);
      font-size: var(--font-md);
    }
  }

  /* Solid-surface fallback: scoped so it outranks the base rule when
     backdrop-filter is unsupported or reduced transparency is requested. */
  @supports not (backdrop-filter: blur(1px)) {
    .sidebar {
      background: var(--surface-color);
    }
  }

  @media (prefers-reduced-transparency: reduce) {
    .sidebar {
      backdrop-filter: none;
      -webkit-backdrop-filter: none;
      background: var(--surface-color);
    }
  }

  .sidebar-section-title {
    margin-top: var(--space-2);
    padding: var(--space-2) var(--space-4);
    font-size: var(--font-sm);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-secondary);
  }

  .saved-search-row {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    border-radius: var(--radius-md);
  }

  .saved-search-row.active {
    background: var(--primary-color);
    color: white;
  }

  .saved-search-open {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex: 1;
    min-width: 0;
    padding: var(--space-3) var(--space-4);
    background: transparent;
    border: none;
    color: inherit;
    font-family: var(--font-body);
    font-size: var(--font-md);
    cursor: pointer;
    text-align: left;
  }

  .saved-search-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .saved-search-action {
    display: flex;
    padding: var(--space-2);
    border: none;
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
    border-radius: var(--radius-md);
  }

  .saved-search-action:hover {
    color: var(--text-primary);
    background: var(--background-secondary);
  }

  .saved-search-rename-input {
    flex: 1;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    border: 1px solid var(--divider-color);
    background: var(--surface-elevated);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--font-md);
  }

  @media (prefers-reduced-motion: reduce) {
    .sidebar,
    .sidebar-toggle {
      transition: none;
    }
  }
</style>
