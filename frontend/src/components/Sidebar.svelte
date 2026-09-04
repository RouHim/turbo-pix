<script>
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import Icon from './Icon.svelte';
  import AlbumDialog from './AlbumDialog.svelte';
  import { t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import {
    addToast,
    albums,
    appState,
    loadAlbums,
    loadSavedSearches,
    savedSearches,
    selectionState,
  } from '../lib/state.svelte.js';
  import { route, pushState } from '../lib/router.svelte.js';
  import { handleError } from '../lib/utils.js';

  const views = [
    { id: 'all', key: 'ui.all_photos', fallback: 'All Photos' },
    { id: 'favorites', key: 'ui.favorites', fallback: 'Favorites' },
    { id: 'videos', key: 'ui.videos', fallback: 'Videos' },
    { id: 'collages', key: 'ui.collages', fallback: 'Collages' },
    { id: 'housekeeping', key: 'ui.housekeeping', fallback: 'Housekeeping' },
  ];

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
    if (route.view === view && route.album == null) {
      // Close the drawer even when tapping the already-active view (mobile).
      appState.sidebarOpen = false;
      return;
    }
    pushState({ view, album: null });
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
    loadAlbums();
  });
  function isActiveSearch(item) {
    return (
      route.view === item.view &&
      route.query === item.query &&
      route.sort === item.sort &&
      route.year === item.year &&
      route.month === item.month
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

  let albumDialogOpen = $state(false);
  let editingAlbum = $state(null);
  let createCount = $state(0);
  let createHashes = $state([]);

  function openCreateAlbum() {
    editingAlbum = null;
    // FR-002: an active grid selection is offered for immediate inclusion;
    // selection keys are photo hash_sha256 strings.
    if (selectionState.active) {
      createHashes = Object.keys(selectionState.selected);
      createCount = createHashes.length;
    } else {
      createHashes = [];
      createCount = 0;
    }
    albumDialogOpen = true;
  }

  function openEditAlbum(item) {
    editingAlbum = item;
    albumDialogOpen = true;
  }

  function openAlbum(item) {
    appState.mobileSearchOpen = false;
    if (route.album === item.id) {
      appState.sidebarOpen = false;
      return;
    }
    pushState({ album: item.id, view: 'all', query: null, year: null, month: null });
    appState.sidebarOpen = false;
  }

  async function deleteAlbum(item) {
    try {
      await api.deleteAlbum(item.id);
      const idx = albums.findIndex((a) => a.id === item.id);
      if (idx !== -1) albums.splice(idx, 1);
      addToast(
        get(t)('albums.deleted', { default: 'Album deleted' }),
        item.name,
        'success',
        3000
      );
      if (route.album === item.id) {
        pushState({ album: null, view: 'all' });
      }
    } catch (error) {
      handleError(error, 'delete album');
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
  class:show={appState.sidebarOpen}
  role="presentation"
  onclick={closeSidebar}
></div>

<nav id="sidebar-nav" class="sidebar" class:open={appState.sidebarOpen}>
  <div class="sidebar-content">
    {#each views as view (view.id)}
      <button
        type="button"
        class="nav-item"
        class:active={route.view === view.id}
        data-view={view.id}
        aria-current={route.view === view.id ? 'page' : undefined}
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
    <div class="album-section-header">
      <span>{$t('albums.sectionTitle', { default: 'Albums' })}</span>
      <button
        type="button"
        class="album-new-btn"
        title={$t('albums.newAlbum', { default: 'New album' })}
        aria-label={$t('albums.newAlbum', { default: 'New album' })}
        onclick={openCreateAlbum}
        data-testid="new-album-btn"
      >
        <Icon name="plus" width={14} height={14} />
      </button>
    </div>
    {#each albums as item (item.id)}
      <div
        class="saved-search-row"
        class:active={route.album === item.id}
        data-testid="album-row"
      >
        <button
          type="button"
          class="saved-search-open"
          title={item.name}
          aria-current={route.album === item.id ? 'true' : undefined}
          onclick={() => openAlbum(item)}
          data-testid="album-open"
        >
          <Icon name="image" width={14} height={14} />
          <span class="saved-search-name">{item.name}</span>
        </button>
        <button
          type="button"
          class="saved-search-action"
          title={$t('albums.rename', { default: 'Rename' })}
          aria-label={$t('albums.rename', { default: 'Rename' })}
          onclick={() => openEditAlbum(item)}
          data-testid="album-rename"><Icon name="edit-2" width={14} height={14} /></button
        >
        <button
          type="button"
          class="saved-search-action"
          title={$t('albums.delete', { default: 'Delete' })}
          aria-label={$t('albums.delete', { default: 'Delete' })}
          onclick={() => deleteAlbum(item)}
          data-testid="album-delete"><Icon name="trash-2" width={14} height={14} /></button
        >
      </div>
    {/each}
  </div>
</nav>
<AlbumDialog
  bind:open={albumDialogOpen}
  album={editingAlbum}
  initialCount={createCount}
  initialHashes={createHashes}
 />

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

  @media (max-width: 768px) {
    .sidebar {
      transform: translateX(-100%);
      visibility: hidden;
      transition:
        transform var(--transition-medium),
        visibility var(--transition-medium);
      z-index: 95;
      width: 280px;
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

  .album-section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-top: var(--space-2);
    padding: var(--space-2) var(--space-4);
    font-size: var(--font-sm);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-secondary);
  }

  .album-new-btn {
    display: flex;
    padding: var(--space-1);
    border: none;
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
    border-radius: var(--radius-md);
  }

  .album-new-btn:hover {
    color: var(--text-primary);
    background: var(--background-secondary);
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
</style>
