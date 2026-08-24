<script>
  import { get } from 'svelte/store';
  import { t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import { addToast, eventAlbums } from '../lib/state.svelte.js';
  import { handleError } from '../lib/utils.js';

  // `open` must be `let` ($bindable); `album` is a non-bindable prop.
  // eslint-disable-next-line prefer-const
  let { open = $bindable(false), album = null } = $props();

  let name = $state('');
  let startDate = $state('');
  let endDate = $state('');
  let location = $state('');
  let error = $state(null);
  let saving = $state(false);
  let dialogEl = $state(null);

  $effect(() => {
    if (open) {
      name = album?.name ?? '';
      startDate = album?.start_date ?? '';
      endDate = album?.end_date ?? '';
      location = album?.location ?? '';
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
      return get(t)('eventAlbums.errorNameRequired', { default: 'Name cannot be empty' });
    }
    if (!startDate || !endDate) {
      return get(t)('eventAlbums.errorDateRequired', {
        default: 'Start and end dates are required',
      });
    }
    if (startDate > endDate) {
      return get(t)('eventAlbums.errorInvalidRange', {
        default: 'End date must be on or after the start date',
      });
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
    const payload = {
      name: name.trim(),
      start_date: startDate,
      end_date: endDate,
      location: location.trim() || null,
    };
    saving = true;
    error = null;
    try {
      if (album) {
        const updated = await api.updateEventAlbum(album.id, payload);
        const idx = eventAlbums.findIndex((a) => a.id === updated.id);
        if (idx !== -1) eventAlbums[idx] = updated;
      } else {
        const created = await api.createEventAlbum(payload);
        eventAlbums.unshift(created); // newest-first: fresh row has the max id
      }
      addToast(
        get(t)(album ? 'eventAlbums.saved' : 'eventAlbums.created', {
          default: album ? 'Album updated' : 'Album created',
        }),
        payload.name,
        'success',
        3000
      );
      close();
    } catch (err) {
      handleError(err, album ? 'update event album' : 'create event album');
    } finally {
      saving = false;
    }
  }
</script>

<dialog bind:this={dialogEl} class="album-dialog" onclose={close}>
  <form onsubmit={onSubmit}>
    <h3>
      {$t(album ? 'eventAlbums.editTitle' : 'eventAlbums.createTitle', {
        default: album ? 'Edit event album' : 'New event album',
      })}
    </h3>

    <label for="album-name">{$t('eventAlbums.name', { default: 'Name' })}</label>
    <input
      id="album-name"
      type="text"
      bind:value={name}
      maxlength="200"
      data-testid="album-name-input"
    />

    <label for="album-start">{$t('eventAlbums.startDate', { default: 'Start date' })}</label>
    <input id="album-start" type="date" bind:value={startDate} data-testid="album-start-input" />

    <label for="album-end">{$t('eventAlbums.endDate', { default: 'End date' })}</label>
    <input id="album-end" type="date" bind:value={endDate} data-testid="album-end-input" />

    <label for="album-location">{$t('eventAlbums.location', { default: 'Location' })}</label>
    <input
      id="album-location"
      type="text"
      bind:value={location}
      placeholder={$t('eventAlbums.locationPlaceholder', { default: 'e.g. Berlin' })}
      data-testid="album-location-input"
    />

    {#if error}
      <p class="album-error" role="alert" data-testid="album-error">{error}</p>
    {/if}

    <div class="album-dialog-actions">
      <button type="button" class="album-cancel" onclick={close}>
        {$t('eventAlbums.cancel', { default: 'Cancel' })}
      </button>
      <button type="submit" class="album-submit" disabled={saving} data-testid="album-submit">
        {$t(album ? 'eventAlbums.save' : 'eventAlbums.create', {
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
  .album-dialog input {
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-md);
    background: var(--surface-elevated);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--font-md);
  }
  .album-dialog input:focus {
    outline: none;
    border-color: var(--primary-color);
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
