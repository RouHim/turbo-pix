<script>
  import { get } from 'svelte/store';
  import { t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import { addToast, albums } from '../lib/state.svelte.js';
  import { handleError } from '../lib/utils.js';

  // `open` must be `let` ($bindable); `album` is a non-bindable prop.
  // In create mode with an active grid selection, `initialHashes` carries the
  // selected photo hashes and the dialog offers to include them (FR-002).
  // eslint-disable-next-line prefer-const
  let { open = $bindable(false), album = null, initialCount = 0, initialHashes = [] } = $props();

  let name = $state('');
  let includeSelection = $state(true);
  let error = $state(null);
  let saving = $state(false);
  let dialogEl = $state(null);

  $effect(() => {
    if (open) {
      name = album?.name ?? '';
      includeSelection = true;
      error = null;
      saving = false;
      dialogEl?.showModal();
    } else {
      dialogEl?.close();
    }
  });

  function close() {
    open = false;
  }

  function validate() {
    if (!name.trim()) {
      return get(t)('albums.errorNameRequired', { default: 'Name cannot be empty' });
    }
    return null;
  }

  async function onSubmit(e) {
    e.preventDefault();
    const validationError = validate();
    if (validationError) {
      error = validationError;
      return;
    }
    saving = true;
    error = null;
    try {
      if (album) {
        const renamed = await api.renameAlbum(album.id, name.trim());
        const idx = albums.findIndex((a) => a.id === renamed.id);
        if (idx !== -1) albums[idx] = renamed;
        addToast(
          get(t)('albums.renamed', { default: 'Album renamed' }),
          renamed.name,
          'success',
          3000
        );
      } else {
        const hashes = includeSelection && initialCount > 0 ? initialHashes : [];
        const created = await api.createAlbum({ name: name.trim(), initial_hashes: hashes });
        albums.unshift(created); // newest-first: fresh row has the max id
        addToast(
          get(t)('albums.created', { default: 'Album created' }),
          created.name,
          'success',
          3000
        );
      }
      close();
    } catch (err) {
      handleError(err, album ? 'rename album' : 'create album');
    } finally {
      saving = false;
    }
  }
</script>

<dialog bind:this={dialogEl} class="album-dialog" onclose={close}>
  <form onsubmit={onSubmit}>
    <h3>
      {$t(album ? 'albums.renameTitle' : 'albums.createTitle', {
        default: album ? 'Rename album' : 'New album',
      })}
    </h3>

    <label for="album-name">{$t('albums.name', { default: 'Name' })}</label>
    <input
      id="album-name"
      type="text"
      bind:value={name}
      maxlength="200"
      data-testid="album-name-input"
    />

    {#if !album && initialCount > 0}
      <label class="album-include-row">
        <input type="checkbox" bind:checked={includeSelection} data-testid="album-include-selection" />
        {$t('albums.includeSelection', {
          default: 'Include {count} selected photos',
          values: { count: initialCount },
        })}
      </label>
    {/if}

    {#if error}
      <p class="album-error" role="alert" data-testid="album-error">{error}</p>
    {/if}

    <div class="album-dialog-actions">
      <button type="button" class="album-cancel" onclick={close}>
        {$t('albums.cancel', { default: 'Cancel' })}
      </button>
      <button type="submit" class="album-submit" disabled={saving} data-testid="album-submit">
        {$t(album ? 'albums.save' : 'albums.create', {
          default: album ? 'Save changes' : 'Create album',
        })}
      </button>
    </div>
  </form>
</dialog>

<style>
  .album-dialog {
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-lg);
    background: var(--surface-color);
    color: var(--text-primary);
    padding: var(--space-6);
    width: min(420px, 90vw);
    box-shadow: var(--shadow-lg);
  }
  .album-dialog::backdrop {
    background: oklch(0% 0 0deg / 40%);
  }
  .album-dialog form {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .album-dialog h3 {
    margin: 0 0 var(--space-2);
    font-family: var(--font-display);
  }
  .album-dialog label {
    font-size: var(--font-sm);
    color: var(--text-secondary);
  }
  .album-dialog input[type='text'] {
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-md);
    background: var(--surface-elevated);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--font-md);
  }
  .album-dialog input[type='text']:focus {
    outline: none;
    border-color: var(--primary-color);
  }
  .album-include-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .album-error {
    color: var(--danger-color, var(--error-color));
    font-size: var(--font-sm);
    margin: var(--space-1) 0 0;
  }
  .album-dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-4);
  }
  .album-cancel,
  .album-submit {
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius-md);
    border: 1px solid var(--divider-color);
    background: var(--surface-elevated);
    color: var(--text-primary);
    font-family: var(--font-body);
    cursor: pointer;
  }
  .album-submit {
    background: var(--primary-color);
    color: white;
    border-color: var(--primary-color);
  }
  .album-submit:disabled {
    opacity: 0.6;
    cursor: default;
  }
  @media (prefers-reduced-motion: reduce) {
    .album-dialog {
      transition: none;
    }
  }
</style>
