<script>
  import { onMount, onDestroy } from 'svelte';
  import { api } from '../lib/api.js';
  import { addToast, loadPeople, people } from '../lib/state.svelte.js';
  import { pushState, route } from '../lib/router.svelte.js';
  import { t } from '../lib/i18n.js';
  import Icon from './Icon.svelte';

  const NOTICE_KEY = 'turbopix_face_notice_dismissed';
  const NOTICE_LINK =
    'https://github.com/deepinsight/insightface/tree/master/python-package#license';
  const SKELETON_CARDS = [0, 1, 2, 3, 4, 5];

  let includeHidden = $state(false);
  let loaded = $state(false);
  let loading = $state(false);
  let loadError = $state(false);
  // Guards the shared view state against a superseded refresh finishing last;
  // not rendered, so a plain local is enough.
  let refreshToken = 0;
  let noticeDismissed = $state(true);
  let brokenCovers = $state({});
  let busyHiddenIds = $state({});
  let editingPerson = $state(null);
  let editName = $state('');
  let editInput = $state(null);
  let dialogEl = $state(null);
  let hiddenToggleEl = $state(null);
  let renameError = $state('');
  let saving = $state(false);
  let renameTrigger = null;

  function readNotice() {
    try {
      noticeDismissed = localStorage.getItem(NOTICE_KEY) === 'true';
    } catch {
      noticeDismissed = false;
    }
  }

  function dismissNotice() {
    noticeDismissed = true;
    try {
      localStorage.setItem(NOTICE_KEY, 'true');
    } catch {
      // Storage may be unavailable; the banner stays dismissed this session.
    }
    requestAnimationFrame(() => hiddenToggleEl?.focus());
  }

  async function refresh() {
    const token = ++refreshToken;
    loading = true;
    loadError = false;
    try {
      const ok = await loadPeople(includeHidden);
      // loadPeople resolves true for a superseded request without applying a
      // list; only the newest refresh may write the shared view state, or an
      // older one landing last would reveal the empty state or clear a
      // failure that belongs to the list actually shown.
      if (token !== refreshToken) return;
      loadError = !ok;
      // A fresh list carries fresh cover ids, so markers recorded for the
      // previous one must not keep a crop from being requested again.
      if (ok) brokenCovers = {};
    } finally {
      if (token === refreshToken) {
        loading = false;
        loaded = true;
      }
    }
  }

  onMount(() => {
    readNotice();
    refresh();
    window.addEventListener('indexingCompleted', onIndexingCompleted);
  });

  onDestroy(() => {
    window.removeEventListener('indexingCompleted', onIndexingCompleted);
  });

  function onIndexingCompleted() {
    refresh();
  }

  $effect(() => {
    if (!editingPerson || !dialogEl) return;
    if (!dialogEl.open) dialogEl.showModal();
    requestAnimationFrame(() => {
      editInput?.focus();
      editInput?.select();
    });
  });

  function openPerson(person) {
    if (route.person === person.id) return;
    pushState({
      view: 'people',
      person: person.id,
      query: null,
      album: null,
      year: null,
      month: null,
      to_year: null,
      to_month: null,
    });
  }

  function startRename(person, event) {
    renameTrigger = event.currentTarget;
    editingPerson = person;
    editName = person.name || '';
    renameError = '';
    saving = false;
  }

  async function saveRename() {
    if (!editingPerson || saving) return;
    const person = editingPerson;
    const name = editName.trim();
    if (!name) {
      renameError = $t('ui.people_name_required', { default: 'Enter a name.' });
      editInput?.focus();
      return;
    }

    saving = true;
    renameError = '';
    try {
      await api.updatePerson(person.id, { name });
      await refresh();
      dialogEl?.close();
      addToast($t('ui.people_renamed', { default: 'Person renamed' }), name, 'success', 2000);
    } catch {
      renameError = $t('ui.people_rename_failed', {
        default: 'Could not rename this person',
      });
      editInput?.focus();
    } finally {
      saving = false;
    }
  }

  function setHiddenBusy(id, busy) {
    if (busy) {
      busyHiddenIds = { ...busyHiddenIds, [id]: true };
      return;
    }
    const next = { ...busyHiddenIds };
    delete next[id];
    busyHiddenIds = next;
  }

  async function toggleHidden(person) {
    // A second click would send the opposite PATCH while the first is in
    // flight, and the responses merge by id without ordering.
    if (busyHiddenIds[person.id]) return;
    const index = people.findIndex((item) => item.id === person.id);
    if (index === -1) return;
    const previousHidden = Boolean(people[index].hidden);
    const nextHidden = !previousHidden;
    people[index] = { ...people[index], hidden: nextHidden };
    setHiddenBusy(person.id, true);

    try {
      const response = await api.updatePerson(person.id, { hidden: nextHidden });
      // A refresh can splice the list while the PATCH is in flight: resolve
      // the row by id again and leave the list alone when it is gone.
      const currentIndex = people.findIndex((item) => item.id === person.id);
      if (response?.person && currentIndex !== -1) {
        people[currentIndex] = { ...people[currentIndex], ...response.person };
      }
      addToast(
        $t(nextHidden ? 'ui.people_hidden_success' : 'ui.people_unhidden_success', {
          default: nextHidden ? 'Hidden from People' : 'Visible again',
        }),
        $t('ui.people_hidden_message', { default: 'Photos stay indexed.' }),
        'success',
        5000,
        nextHidden
          ? {
              label: $t('ui.people_show_hidden_action', { default: 'Show hidden' }),
              onClick: () => {
                includeHidden = true;
                refresh();
              },
            }
          : null
      );
    } catch {
      // Same race as the success path: re-resolve by id so the rollback
      // cannot land on a different person and, if the row is gone, skip it.
      const currentIndex = people.findIndex((item) => item.id === person.id);
      if (currentIndex !== -1) {
        people[currentIndex] = { ...people[currentIndex], hidden: previousHidden };
      }
      addToast(
        $t('notifications.error', { default: 'Error' }),
        $t('ui.people_hide_failed', { default: 'Could not update this person' }),
        'error',
        3000
      );
    } finally {
      setHiddenBusy(person.id, false);
    }
  }

  async function toggleHiddenFilter() {
    includeHidden = !includeHidden;
    await refresh();
  }

  function closeRename() {
    if (saving) return;
    dialogEl?.close();
  }

  function handleDialogClose() {
    const trigger = renameTrigger;
    editingPerson = null;
    editName = '';
    renameError = '';
    saving = false;
    requestAnimationFrame(() => trigger?.focus());
  }

  function onDialogCancel(event) {
    if (saving) event.preventDefault();
  }

  function coverUrl(person) {
    if (!person.cover_face_id || brokenCovers[person.id]) return null;
    return api.faceCropUrl(person.cover_face_id);
  }

  function breakCover(person) {
    brokenCovers = { ...brokenCovers, [person.id]: true };
  }

  function personLabel(person) {
    return (
      person.name ||
      $t('ui.people_person_number', {
        values: { number: person.id },
        default: 'Person {number}',
      })
    );
  }

  function faceCountLabel(person) {
    const count = Number(person.face_count) || 0;
    if (count === 1) return $t('ui.people_face_count_one', { default: '1 face' });
    return $t('ui.people_face_count', { values: { count }, default: '{count} faces' });
  }

  function personAriaLabel(person) {
    const label = `${personLabel(person)} — ${faceCountLabel(person)}`;
    return person.hidden
      ? `${label} — ${$t('ui.people_hidden_badge', { default: 'Hidden' })}`
      : label;
  }

  function personInitial(person) {
    return person.name?.trim()?.charAt(0)?.toUpperCase() || null;
  }
</script>

<div class="people-view">
  <div class="people-toolbar">
    <button
      type="button"
      class="people-hidden-toggle"
      class:active={includeHidden}
      aria-pressed={includeHidden}
      data-testid="people-hidden-toggle"
      bind:this={hiddenToggleEl}
      onclick={toggleHiddenFilter}
    >
      <Icon name="eye" width={14} height={14} />
      <span>{$t('ui.people_show_hidden', { default: 'Show hidden people' })}</span>
    </button>
  </div>

  {#if loadError && people.length > 0}
    <div class="people-inline-error" role="alert">
      <span>{$t('ui.people_load_error', { default: 'Could not load people' })}</span>
      <button type="button" class="btn-secondary" onclick={refresh}>
        {$t('ui.retry', { default: 'Retry' })}
      </button>
    </div>
  {/if}

  {#if !loaded}
    <span class="people-sr-only" role="status">{$t('ui.loading', { default: 'Loading...' })}</span>
    <div class="people-skeleton" aria-hidden="true" data-testid="people-skeleton">
      {#each SKELETON_CARDS as card (card)}
        <div class="people-skeleton-card"></div>
      {/each}
    </div>
  {:else if loadError && people.length === 0}
    <div class="people-empty" role="alert">
      <span class="people-empty-tile people-empty-tile-error">
        <Icon name="alert-circle" width={28} height={28} />
      </span>
      <p>{$t('ui.people_load_error', { default: 'Could not load people' })}</p>
      <button type="button" class="btn-primary" onclick={refresh}>
        {$t('ui.retry', { default: 'Retry' })}
      </button>
    </div>
  {:else if people.length === 0}
    <div class="people-empty" role="status">
      <span class="people-empty-tile"><Icon name="users" width={28} height={28} /></span>
      <p>
        {$t('ui.people_empty', { default: 'No people yet. Face indexing runs in the background.' })}
      </p>
    </div>
  {:else}
    <div class="people-grid" class:loading aria-busy={loading} data-testid="people-grid">
      {#each people as person (person.id)}
        <div class="person-card" data-testid="person-card" data-person-id={person.id}>
          <button
            type="button"
            class="person-open"
            aria-label={personAriaLabel(person)}
            data-testid="person-open"
            onclick={() => openPerson(person)}
          >
            <span class="person-cover">
              {#if coverUrl(person)}
                <img
                  src={coverUrl(person)}
                  alt=""
                  loading="lazy"
                  onerror={() => breakCover(person)}
                />
              {:else if personInitial(person)}
                <span class="person-initial" aria-hidden="true">{personInitial(person)}</span>
              {:else}
                <Icon name="users" width={28} height={28} />
              {/if}
              {#if person.hidden}
                <span class="person-hidden-badge">
                  {$t('ui.people_hidden_badge', { default: 'Hidden' })}
                </span>
              {/if}
            </span>
            <span class="person-caption">
              <span class="person-name" title={personLabel(person)}>{personLabel(person)}</span>
              <span class="person-meta">{faceCountLabel(person)}</span>
            </span>
          </button>
          <span class="person-actions">
            <button
              type="button"
              class="person-action"
              title={$t('ui.people_rename', { default: 'Rename' })}
              aria-label={$t('ui.people_rename', { default: 'Rename' })}
              onclick={(event) => startRename(person, event)}
            >
              <Icon name="edit-2" width={16} height={16} />
            </button>
            <button
              type="button"
              class="person-action"
              title={person.hidden
                ? $t('ui.people_unhide', { default: 'Unhide' })
                : $t('ui.people_hide', { default: 'Hide' })}
              aria-label={person.hidden
                ? $t('ui.people_unhide', { default: 'Unhide' })
                : $t('ui.people_hide', { default: 'Hide' })}
              onclick={() => toggleHidden(person)}
            >
              <Icon name={person.hidden ? 'eye' : 'eye-off'} width={16} height={16} />
            </button>
          </span>
        </div>
      {/each}
    </div>
  {/if}

  {#if !noticeDismissed}
    <div class="people-notice" role="note">
      <p>
        {$t('ui.people_notice', {
          default:
            'Faces are detected locally. The optional InsightFace models are licensed for non-commercial use only.',
        })}
        <a href={NOTICE_LINK} target="_blank" rel="noopener noreferrer"
          >{$t('ui.people_notice_link', { default: 'Model license' })}</a
        >
      </p>
      <button
        type="button"
        class="people-notice-dismiss"
        aria-label={$t('ui.people_notice_dismiss', { default: 'Dismiss' })}
        onclick={dismissNotice}
      >
        <Icon name="x" width={16} height={16} />
      </button>
    </div>
  {/if}
</div>

<dialog
  bind:this={dialogEl}
  class="person-dialog"
  aria-labelledby="person-dialog-title"
  onclose={handleDialogClose}
  oncancel={onDialogCancel}
  onclick={(event) => {
    // Clicks inside the dialog's own padding also target the <dialog> element,
    // so only treat coordinates outside its box as a backdrop dismissal.
    if (event.target !== dialogEl) return;

    const bounds = dialogEl.getBoundingClientRect();
    const outside =
      event.clientX < bounds.left ||
      event.clientX > bounds.right ||
      event.clientY < bounds.top ||
      event.clientY > bounds.bottom;

    if (outside) closeRename();
  }}
>
  {#if editingPerson}
    <form
      onsubmit={(event) => {
        event.preventDefault();
        saveRename();
      }}
    >
      <header class="person-dialog-header">
        <span class="person-dialog-avatar">
          {#if coverUrl(editingPerson)}
            <img src={coverUrl(editingPerson)} alt="" onerror={() => breakCover(editingPerson)} />
          {:else if personInitial(editingPerson)}
            <span aria-hidden="true">{personInitial(editingPerson)}</span>
          {:else}
            <Icon name="users" width={20} height={20} />
          {/if}
        </span>
        <div class="person-dialog-heading">
          <h2 id="person-dialog-title">
            {$t(editingPerson.name ? 'ui.people_rename_title' : 'ui.people_name_title', {
              default: editingPerson.name ? 'Rename person' : 'Name this person',
            })}
          </h2>
          <p>{personLabel(editingPerson)}</p>
        </div>
      </header>

      <label class="person-dialog-label" for="person-name-input">
        {$t('ui.people_name_label', { default: 'Name' })}
      </label>
      <input
        id="person-name-input"
        bind:this={editInput}
        bind:value={editName}
        maxlength="100"
        autocomplete="off"
        aria-invalid={renameError ? 'true' : undefined}
        aria-describedby={renameError ? 'person-name-error' : undefined}
      />
      {#if renameError}
        <p id="person-name-error" class="person-dialog-error" role="alert">{renameError}</p>
      {/if}
      <div class="person-dialog-actions">
        <button type="button" class="btn-secondary" disabled={saving} onclick={closeRename}>
          {$t('ui.people_cancel', { default: 'Cancel' })}
        </button>
        <button type="submit" class="btn-primary" disabled={saving} aria-busy={saving}>
          {$t(saving ? 'ui.people_saving' : 'ui.people_save', {
            default: saving ? 'Saving…' : 'Save',
          })}
        </button>
      </div>
    </form>
  {/if}
</dialog>

<style>
  .people-view {
    padding-bottom: var(--space-8);
  }

  .people-toolbar {
    display: flex;
    justify-content: flex-end;
    margin-bottom: var(--space-4);
  }

  .people-hidden-toggle {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 32px;
    padding: 0 var(--space-3);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-full);
    background: transparent;
    color: var(--text-secondary);
    font-family: var(--font-body);
    font-size: var(--font-sm);
    cursor: pointer;
    transition:
      color var(--transition-fast),
      border-color var(--transition-fast),
      background var(--transition-fast);
  }

  .people-hidden-toggle:hover {
    border-color: var(--primary-color);
    color: var(--text-primary);
  }

  .people-hidden-toggle.active {
    border-color: var(--primary-color);
    background: color-mix(in oklch, var(--primary-color) 12%, transparent);
    color: var(--primary-ink);
  }

  .people-hidden-toggle:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: 2px;
  }

  .people-inline-error {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
    padding: var(--space-2) var(--space-3);
    border: 1px solid color-mix(in oklch, var(--color-danger) 30%, transparent);
    border-radius: var(--radius-md);
    background: color-mix(in oklch, var(--color-danger) 10%, transparent);
    color: var(--color-danger-ink);
    font-size: var(--font-sm);
  }

  .people-sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }

  .people-grid,
  .people-skeleton {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: var(--space-6);
  }

  .people-grid.loading {
    opacity: 0.6;
  }

  .people-skeleton-card {
    aspect-ratio: 4 / 5;
    min-width: 0;
    border-radius: var(--radius-md);
    background: linear-gradient(
      90deg,
      color-mix(in oklch, var(--text-muted) 40%, var(--background-secondary)) 25%,
      var(--background-secondary) 37%,
      color-mix(in oklch, var(--text-muted) 40%, var(--background-secondary)) 63%
    );
    background-size: 400% 100%;
    animation: people-skeleton-loading 1.5s ease-in-out infinite;
  }

  @keyframes people-skeleton-loading {
    0% {
      background-position: 100% 50%;
    }
    100% {
      background-position: -100% 50%;
    }
  }

  .person-card {
    position: relative;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-md);
    background: var(--surface-color);
    color: var(--text-primary);
    box-shadow: var(--shadow-light);
    text-align: left;
    transition:
      transform var(--transition-fast),
      box-shadow var(--transition-fast);
  }

  .person-card:hover {
    transform: translateY(-2px);
    box-shadow: var(--shadow-heavy);
  }

  .person-card:focus-within {
    border-color: var(--primary-color);
  }

  .person-open {
    display: flex;
    flex-direction: column;
    width: 100%;
    min-width: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .person-open:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: -2px;
  }

  .person-cover {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    aspect-ratio: 1;
    background: var(--background-secondary);
    color: var(--text-muted);
    overflow: hidden;
  }

  .person-cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .person-initial {
    color: var(--text-muted);
    font-family: var(--font-heading);
    font-size: var(--font-5xl);
    font-weight: var(--font-semibold);
    line-height: 1;
  }

  .person-hidden-badge {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-full);
    background: var(--surface-color);
    color: var(--text-secondary);
    font-size: var(--font-xs);
    box-shadow: var(--shadow-light);
  }

  .person-caption {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-3);
    min-width: 0;
  }

  .person-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: var(--font-medium);
  }

  .person-meta {
    color: var(--text-muted);
    font-size: var(--font-sm);
  }

  .person-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-1);
    padding: 0 var(--space-2) var(--space-2);
  }

  .person-action {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 44px;
    height: 44px;
    padding: 0;
    border: none;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    transition:
      color var(--transition-fast),
      background var(--transition-fast);
  }

  .person-action:hover {
    color: var(--text-primary);
    background: var(--background-secondary);
  }

  .person-action:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: 1px;
  }

  .people-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-16) var(--space-4);
    text-align: center;
    color: var(--text-secondary);
  }

  .people-empty-tile {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 72px;
    height: 72px;
    border-radius: var(--radius-lg);
    background: var(--background-secondary);
    color: var(--text-muted);
  }

  .people-empty-tile-error {
    color: var(--color-danger-ink);
  }

  .people-empty p {
    margin: 0;
  }

  .people-notice {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-top: var(--space-6);
    padding-top: var(--space-3);
    border-top: 1px solid var(--divider-color);
    color: var(--text-muted);
    font-size: var(--font-xs);
  }

  .people-notice p {
    flex: 1;
    margin: 0;
    line-height: 1.5;
  }

  .people-notice a {
    color: var(--primary-ink);
    margin-left: var(--space-1);
    text-underline-offset: 2px;
  }

  .people-notice a:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: 2px;
  }

  .people-notice-dismiss {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
  }

  .people-notice-dismiss:hover {
    color: var(--text-primary);
  }

  .people-notice-dismiss:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: 2px;
  }

  .person-dialog {
    width: min(420px, calc(100vw - var(--space-8)));
    padding: var(--space-6);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-lg);
    background: var(--surface-color);
    color: var(--text-primary);
    box-shadow: var(--shadow-heavy);
  }

  .person-dialog::backdrop {
    background: oklch(0% 0 0deg / 40%);
  }

  .person-dialog form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .person-dialog-header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
  }

  .person-dialog-avatar {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 48px;
    height: 48px;
    overflow: hidden;
    border-radius: var(--radius-full);
    background: var(--background-secondary);
    color: var(--text-muted);
    font-family: var(--font-heading);
    font-size: var(--font-xl);
    font-weight: var(--font-semibold);
  }

  .person-dialog-avatar img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .person-dialog-heading {
    min-width: 0;
  }

  .person-dialog-heading h2 {
    margin: 0;
    font-family: var(--font-body);
    font-size: var(--font-xl);
    font-weight: var(--font-semibold);
  }

  .person-dialog-heading p {
    margin: 2px 0 0;
    overflow: hidden;
    color: var(--text-muted);
    font-size: var(--font-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .person-dialog-label {
    color: var(--text-secondary);
    font-size: var(--font-sm);
  }

  .person-dialog input {
    width: 100%;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-md);
    background: var(--background-color);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--font-base);
  }

  .person-dialog input:focus-visible {
    border-color: var(--primary-color);
    outline: 2px solid var(--primary-color);
    outline-offset: 2px;
  }

  .person-dialog-error {
    margin: 0;
    color: var(--color-danger-ink);
    font-size: var(--font-sm);
  }

  .person-dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-1);
  }

  .person-dialog-actions button:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: 2px;
  }

  @media (width <= 1200px) {
    .people-grid,
    .people-skeleton {
      grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
      gap: var(--space-5);
    }
  }

  @media (width <= 1024px) {
    .people-grid,
    .people-skeleton {
      grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
      gap: var(--space-4);
    }
  }

  @media (width >= 1400px) {
    .people-grid,
    .people-skeleton {
      grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
      gap: var(--space-8);
    }
  }

  @container (width <= 768px) {
    .people-grid,
    .people-skeleton {
      grid-template-columns: repeat(3, 1fr);
      gap: var(--space-1);
    }

    .people-skeleton-card {
      border-radius: 0;
    }

    .person-card {
      border: 0;
      border-radius: 0;
      background: transparent;
      box-shadow: none;
    }

    .person-card:hover {
      transform: none;
      box-shadow: none;
    }

    .person-card:focus-within {
      border-color: transparent;
    }

    .person-caption {
      padding: var(--space-2) var(--space-1) 0;
    }

    .person-name {
      font-size: var(--font-sm);
    }

    .person-meta {
      font-size: var(--font-xs);
    }

    .person-actions {
      gap: 0;
      padding: 0 var(--space-1) var(--space-2);
    }

    .people-hidden-toggle {
      min-height: 44px;
    }

    .people-hidden-toggle span {
      font-size: var(--font-xs);
    }

    .people-notice {
      align-items: flex-start;
      margin-top: var(--space-4);
      padding-top: var(--space-2);
    }

    .people-notice-dismiss {
      width: 44px;
      height: 44px;
    }

    .person-dialog {
      padding: var(--space-5);
    }

    .person-dialog-actions button {
      min-height: 44px;
    }
  }

  @container (width <= 480px) {
    .people-grid,
    .people-skeleton {
      gap: 2px;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .person-card,
    .person-card:hover {
      transform: none;
      transition: none;
    }

    .people-skeleton-card {
      animation: none;
      background: var(--divider-color);
    }
  }
</style>
