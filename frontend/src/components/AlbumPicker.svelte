<script>
  import { get } from 'svelte/store';
  import { t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import { addToast, albums, loadAlbums } from '../lib/state.svelte.js';
  import { handleError } from '../lib/utils.js';

  // `openHashes` are photo hash_sha256 strings to add; `onDone` closes the
  // picker (called on success, cancel, and dialog dismiss).
  // eslint-disable-next-line prefer-const
  let { openHashes = [], onDone = () => {} } = $props();

  let dialogEl = $state(null);
  let newName = $state('');
  let showNew = $state(false);
  let saving = $state(false);
  let error = $state(null);

  $effect(() => {
    dialogEl?.showModal();
    loadAlbums();
  });

  function close() {
    dialogEl?.close();
  }

  function handleClose() {
    onDone();
  }

  async function addTo(item) {
    if (saving) return;
    saving = true;
    error = null;
    try {
      const res = await api.addAlbumMembers(item.id, openHashes);
      // Member adds are idempotent: added=0 means every hash was already a
      // member (or unknown), so report that instead of a false success.
      if (res && typeof res.added === 'number' && res.added === 0) {
        addToast(
          get(t)('albums.alreadyAdded', { default: 'Photos already in album' }),
          item.name,
          'info',
          3000
        );
      } else {
        addToast(
          get(t)('albums.added', { default: 'Photos added to album' }),
          item.name,
          'success',
          3000
        );
      }
      close();
    } catch (err) {
      handleError(err, 'add photos to album');
    } finally {
      saving = false;
    }
  }

  async function createAndAdd(e) {
    e.preventDefault();
    const name = newName.trim();
    if (!name) {
      error = get(t)('albums.errorNameRequired', { default: 'Name cannot be empty' });
      return;
    }
    saving = true;
    error = null;
    try {
      const created = await api.createAlbum({ name, initial_hashes: openHashes });
      albums.unshift(created); // newest-first: fresh row has the max id
      addToast(
        get(t)('albums.created', { default: 'Album created' }),
        created.name,
        'success',
        3000
      );
      close();
    } catch (err) {
      handleError(err, 'create album');
    } finally {
      saving = false;
    }
  }
</script>

<dialog bind:this={dialogEl} class="album-picker" onclose={handleClose}>
  <h3>{$t('albums.pickerTitle', { default: 'Choose album' })}</h3>

  {#each albums as item (item.id)}
    <button
      type="button"
      class="picker-row"
      disabled={saving}
      onclick={() => addTo(item)}
      data-testid="album-pick-row"
      data-album-id={item.id}
    >
      <span class="picker-name">{item.name}</span>
    </button>
  {/each}

  {#if showNew}
    <form class="picker-new-form" onsubmit={createAndAdd}>
      <input
        type="text"
        bind:value={newName}
        maxlength="200"
        placeholder={$t('albums.name', { default: 'Name' })}
        aria-label={$t('albums.name', { default: 'Name' })}
        data-testid="album-pick-new-input"
      />
      <button type="submit" disabled={saving} data-testid="album-pick-new-submit">
        {$t('albums.create', { default: 'Create album' })}
      </button>
    </form>
  {:else}
    <button
      type="button"
      class="picker-new-btn"
      onclick={() => (showNew = true)}
      data-testid="album-pick-new"
    >
      {$t('albums.newAlbumOption', { default: 'New album…' })}
    </button>
  {/if}

  {#if error}
    <p class="picker-error" role="alert" data-testid="album-pick-error">{error}</p>
  {/if}

  <div class="picker-actions">
    <button type="button" class="picker-cancel" onclick={close}>
      {$t('albums.cancel', { default: 'Cancel' })}
    </button>
  </div>
</dialog>

<style>
  .album-picker {
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-lg);
    background: var(--surface-color);
    color: var(--text-primary);
    padding: var(--space-6);
    width: min(360px, 90vw);
    box-shadow: var(--shadow-lg);
  }
  .album-picker::backdrop {
    background: oklch(0% 0 0deg / 40%);
  }
  .album-picker h3 {
    margin: 0 0 var(--space-2);
    font-family: var(--font-display);
  }
  .picker-row {
    display: flex;
    width: 100%;
    align-items: center;
    padding: var(--space-2) var(--space-3);
    border: 1px solid transparent;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--font-md);
    cursor: pointer;
    text-align: left;
  }
  .picker-row:hover {
    background: var(--surface-elevated);
    border-color: var(--divider-color);
  }
  .picker-row:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .picker-new-btn {
    width: 100%;
    margin-top: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: 1px dashed var(--divider-color);
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-secondary);
    font-family: var(--font-body);
    cursor: pointer;
  }
  .picker-new-form {
    display: flex;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
  .picker-new-form input {
    flex: 1;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-md);
    background: var(--surface-elevated);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--font-md);
  }
  .picker-new-form button {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    border: 1px solid var(--primary-color);
    background: var(--primary-color);
    color: white;
    font-family: var(--font-body);
    cursor: pointer;
    white-space: nowrap;
  }
  .picker-error {
    color: var(--danger-color, var(--error-color));
    font-size: var(--font-sm);
    margin: var(--space-1) 0 0;
  }
  .picker-actions {
    display: flex;
    justify-content: flex-end;
    margin-top: var(--space-4);
  }
  .picker-cancel {
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius-md);
    border: 1px solid var(--divider-color);
    background: var(--surface-elevated);
    color: var(--text-primary);
    font-family: var(--font-body);
    cursor: pointer;
  }
  @media (prefers-reduced-motion: reduce) {
    .album-picker {
      transition: none;
    }
  }
</style>
