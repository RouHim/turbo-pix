<script>
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import { getLocale, t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import { addToast, albums, appState, loadAlbums } from '../lib/state.svelte.js';
  import { route, pushState } from '../lib/router.svelte.js';
  import { getThumbnailUrl, handleError } from '../lib/utils.js';
  import Icon from './Icon.svelte';
  import AlbumDialog from './AlbumDialog.svelte';

  let albumDialogOpen = $state(false);
  let editingAlbum = $state(null);
  // Album id -> first member's hash, shown as the row cover. Covers resolve
  // lazily after the list so one slow album never blocks the overview.
  let covers = $state({});

  async function loadCovers(ids) {
    const entries = await Promise.all(
      ids.map(async (id) => {
        try {
          const data = await api.getAlbumPhotos(id, { page: 1, limit: 1 });
          return [id, data?.photos?.[0]?.hash_sha256 ?? null];
        } catch {
          return [id, null];
        }
      })
    );
    const next = { ...covers };
    for (const [id, hash] of entries) {
      if (hash) next[id] = hash;
    }
    covers = next;
  }
  onMount(() => {
    loadAlbums().then(() => loadCovers(albums.map((a) => a.id)));
    window.addEventListener('openCreateAlbum', openCreateAlbum);
    return () => window.removeEventListener('openCreateAlbum', openCreateAlbum);
  });

  function openCreateAlbum() {
    editingAlbum = null;
    albumDialogOpen = true;
  }

  function openEditAlbum(item) {
    editingAlbum = item;
    albumDialogOpen = true;
  }

  function openAlbum(item) {
    appState.mobileSearchOpen = false;
    if (route.album === item.id) return;
    pushState({ album: item.id, view: 'all', query: null, year: null, month: null });
  }

  async function deleteAlbum(item) {
    try {
      await api.deleteAlbum(item.id);
      const idx = albums.findIndex((a) => a.id === item.id);
      if (idx !== -1) albums.splice(idx, 1);
      addToast(get(t)('albums.deleted', { default: 'Album deleted' }), item.name, 'success', 3000);
      if (route.album === item.id) {
        pushState({ album: null, view: 'albums' });
      }
    } catch (error) {
      handleError(error, 'delete album');
    }
  }

  function formatUpdated(value) {
    const time = new Date(value).getTime();
    if (Number.isNaN(time)) return '';
    return new Date(time).toLocaleDateString(getLocale(), { dateStyle: 'medium' });
  }
</script>

<div class="albums-view">
  {#if albums.length === 0}
    <div class="albums-empty">
      <span class="albums-empty-tile"><Icon name="image" width={28} height={28} /></span>
      <p>{$t('albums.emptyList', { default: 'No albums yet' })}</p>
      <button type="button" class="btn-primary albums-empty-btn" onclick={openCreateAlbum}>
        <Icon name="plus" width={16} height={16} />
        <span>{$t('albums.newAlbum', { default: 'New album' })}</span>
      </button>
    </div>
  {:else}
    <div class="album-list">
      {#each albums as item, index (item.id)}
        <div class="album-row" style="--index: {index}" data-testid="album-row">
          <button
            type="button"
            class="album-open"
            title={item.name}
            onclick={() => openAlbum(item)}
            data-testid="album-open"
          >
            <span class="album-tile">
              {#if covers[item.id]}
                <img
                  class="album-cover"
                  src={getThumbnailUrl({ hash_sha256: covers[item.id] }, 'small')}
                  alt=""
                  loading="lazy"
                />
              {:else}
                <Icon name="image" width={18} height={18} />
              {/if}
            </span>
            <span class="album-text">
              <span class="album-name">{item.name}</span>
              {#if formatUpdated(item.updated_at)}
                <span class="album-meta">{formatUpdated(item.updated_at)}</span>
              {/if}
            </span>
          </button>
          <button
            type="button"
            class="album-action"
            title={$t('albums.rename', { default: 'Rename' })}
            aria-label={$t('albums.rename', { default: 'Rename' })}
            onclick={() => openEditAlbum(item)}
            data-testid="album-rename"><Icon name="edit-2" width={14} height={14} /></button
          >
          <button
            type="button"
            class="album-action album-delete"
            title={$t('albums.delete', { default: 'Delete' })}
            aria-label={$t('albums.delete', { default: 'Delete' })}
            onclick={() => deleteAlbum(item)}
            data-testid="album-delete"><Icon name="trash-2" width={14} height={14} /></button
          >
        </div>
      {/each}
    </div>
  {/if}
</div>

<AlbumDialog bind:open={albumDialogOpen} album={editingAlbum} initialCount={0} initialHashes={[]} />

<style>
  .albums-view {
    max-width: 720px;
    margin: 0 auto;
    padding: 0 var(--space-4) var(--space-8);
  }

  .albums-empty-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    white-space: nowrap;
  }

  .album-tile {
    overflow: hidden;
  }

  .album-cover {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .albums-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-16) var(--space-4);
    text-align: center;
  }

  .albums-empty-tile {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 72px;
    height: 72px;
    border-radius: var(--radius-lg);
    background: var(--background-secondary);
    color: var(--text-muted);
  }

  .albums-empty p {
    margin: 0;
    color: var(--text-secondary);
  }

  .album-list {
    display: flex;
    flex-direction: column;
  }

  .album-row {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
    border-bottom: 1px solid var(--divider-color);
    animation: album-enter var(--transition-medium) backwards;
    animation-delay: calc(var(--index, 0) * 40ms);
  }

  .album-row:first-child {
    border-top: 1px solid var(--divider-color);
  }

  @keyframes album-enter {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .album-open {
    flex: 1;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
    padding: var(--space-2);
    border: none;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-primary);
    cursor: pointer;
    text-align: left;
    transition: background var(--transition-fast);
  }

  .album-open:hover {
    background: var(--background-secondary);
  }

  .album-open:active {
    transform: scale(0.99);
  }

  .album-tile {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 44px;
    height: 44px;
    border-radius: var(--radius-md);
    background: var(--background-secondary);
    color: var(--primary-color);
  }

  .album-text {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .album-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: var(--font-medium);
  }

  .album-meta {
    font-size: var(--font-xs);
    color: var(--text-muted);
  }

  .album-action {
    flex: none;
    display: flex;
    padding: var(--space-2);
    border: none;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    transition:
      color var(--transition-fast),
      background var(--transition-fast);
  }

  .album-action:hover {
    color: var(--text-primary);
    background: var(--background-secondary);
  }

  .album-delete:hover {
    color: var(--color-danger);
  }

  @media (prefers-reduced-motion: reduce) {
    .album-row {
      animation: none;
    }
  }
</style>
