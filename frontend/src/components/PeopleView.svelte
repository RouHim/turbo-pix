<script>
  import { onMount } from 'svelte';
  import { api } from '../lib/api.js';
  import { addToast, loadPeople, people } from '../lib/state.svelte.js';
  import { pushState } from '../lib/router.svelte.js';
  import { t } from '../lib/i18n.js';
  import Icon from './Icon.svelte';

  const NOTICE_KEY = 'turbopix_face_notice_dismissed';
  const NOTICE_LINK =
    'https://github.com/deepinsight/insightface/tree/master/python-package#license';

  let includeHidden = $state(false);
  let loading = $state(false);
  let loadError = $state(false);
  let noticeDismissed = $state(true);
  let brokenCovers = $state({});
  let editingPerson = $state(null);
  let editName = $state('');
  let editInput = $state(null);

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
  }

  async function refresh() {
    loading = true;
    loadError = false;
    try {
      loadError = !(await loadPeople(includeHidden));
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    readNotice();
    refresh();
  });

  $effect(() => {
    if (editingPerson) {
      requestAnimationFrame(() => {
        editInput?.focus();
        editInput?.select();
      });
    }
  });

  function openPerson(person) {
    pushState({ view: 'people', person: person.id, query: null, album: null });
  }

  function startRename(person, event) {
    event.stopPropagation();
    editingPerson = person;
    editName = person.name || '';
  }

  async function saveRename() {
    if (!editingPerson) return;
    const person = editingPerson;
    editingPerson = null;
    try {
      await api.updatePerson(person.id, { name: editName });
      await refresh();
      addToast(
        $t('ui.people_renamed', { default: 'Person renamed' }),
        editName.trim(),
        'success',
        2000
      );
    } catch {
      addToast(
        $t('notifications.error', { default: 'Error' }),
        $t('ui.people_rename_failed', { default: 'Could not rename this person' }),
        'error',
        3000
      );
    }
  }

  async function toggleHidden(person, event) {
    event.stopPropagation();
    try {
      await api.updatePerson(person.id, { hidden: !person.hidden });
      await refresh();
    } catch {
      addToast(
        $t('notifications.error', { default: 'Error' }),
        $t('ui.people_hide_failed', { default: 'Could not update this person' }),
        'error',
        3000
      );
    }
  }

  function closeRename() {
    editingPerson = null;
  }

  function onDialogKeydown(event) {
    if (event.key === 'Escape') closeRename();
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
      $t('ui.person_number', {
        values: { number: person.id },
        default: 'Person {number}',
      })
    );
  }
</script>

<div class="people-view">
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

  <div class="people-toolbar">
    <label class="people-hidden-toggle">
      <input type="checkbox" bind:checked={includeHidden} onchange={refresh} />
      <span>{$t('ui.people_show_hidden', { default: 'Show hidden people' })}</span>
    </label>
  </div>

  {#if loadError}
    <div class="people-empty">
      <p>{$t('ui.people_load_error', { default: 'Could not load people' })}</p>
      <button type="button" class="btn-primary" onclick={refresh}>
        {$t('ui.retry', { default: 'Retry' })}
      </button>
    </div>
  {:else if people.length === 0}
    <div class="people-empty">
      <span class="people-empty-tile"><Icon name="users" width={28} height={28} /></span>
      <p>
        {$t('ui.people_empty', { default: 'No people yet. Face indexing runs in the background.' })}
      </p>
    </div>
  {:else}
    <div class="people-grid" class:loading>
      {#each people as person (person.id)}
        <div
          class="person-card"
          role="button"
          tabindex="0"
          aria-label={personLabel(person)}
          data-testid="person-card"
          data-person-id={person.id}
          onclick={() => openPerson(person)}
          onkeydown={(event) => {
            if (event.target !== event.currentTarget) return;
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault();
              openPerson(person);
            }
          }}
        >
          <span class="person-cover">
            {#if coverUrl(person)}
              <img
                src={coverUrl(person)}
                alt=""
                loading="lazy"
                onerror={() => breakCover(person)}
              />
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
            <span class="person-name">{personLabel(person)}</span>
            <span class="person-meta">
              {$t('ui.people_face_count', {
                values: { count: person.face_count },
                default: '{count} faces',
              })}
            </span>
          </span>
          <span class="person-actions">
            <button
              type="button"
              class="person-action"
              title={$t('ui.people_rename', { default: 'Rename' })}
              aria-label={$t('ui.people_rename', { default: 'Rename' })}
              onclick={(event) => startRename(person, event)}
            >
              <Icon name="edit-2" width={14} height={14} />
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
              onclick={(event) => toggleHidden(person, event)}
            >
              <Icon name={person.hidden ? 'eye' : 'eye-off'} width={14} height={14} />
            </button>
          </span>
        </div>
      {/each}
    </div>
  {/if}
</div>

{#if editingPerson}
  <div class="person-dialog-backdrop" role="presentation" onclick={closeRename}>
    <div
      class="person-dialog"
      role="dialog"
      tabindex="-1"
      aria-modal="true"
      aria-labelledby="person-dialog-title"
      onclick={(event) => event.stopPropagation()}
      onkeydown={onDialogKeydown}
    >
      <h2 id="person-dialog-title">
        {$t('ui.people_rename_title', { default: 'Rename person' })}
      </h2>
      <label class="person-dialog-label" for="person-name-input">
        {$t('ui.people_name_label', { default: 'Name' })}
      </label>
      <input
        id="person-name-input"
        bind:this={editInput}
        bind:value={editName}
        maxlength="100"
        autocomplete="off"
        onkeydown={(event) => {
          if (event.key === 'Enter') {
            event.preventDefault();
            saveRename();
          }
        }}
      />
      <div class="person-dialog-actions">
        <button type="button" class="btn-secondary" onclick={closeRename}>
          {$t('ui.people_cancel', { default: 'Cancel' })}
        </button>
        <button type="button" class="btn-primary" onclick={saveRename}>
          {$t('ui.people_save', { default: 'Save' })}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .people-view {
    max-width: 1100px;
    margin: 0 auto;
    padding: 0 var(--space-4) var(--space-8);
  }

  .people-notice {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-md);
    background: var(--background-secondary);
    color: var(--text-secondary);
  }

  .people-notice p {
    flex: 1;
    margin: 0;
    line-height: 1.5;
  }

  .people-notice a {
    color: var(--primary-color);
    margin-left: var(--space-1);
  }

  .people-notice-dismiss {
    flex: none;
    display: flex;
    padding: var(--space-1);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
  }

  .people-notice-dismiss:hover {
    color: var(--text-primary);
  }

  .people-toolbar {
    display: flex;
    justify-content: flex-end;
    margin-bottom: var(--space-3);
  }

  .people-hidden-toggle {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-secondary);
    font-size: var(--font-sm);
  }

  .people-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: var(--space-4);
  }

  .people-grid.loading {
    opacity: 0.6;
  }

  .person-card {
    position: relative;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-lg);
    background: var(--surface-color);
    color: var(--text-primary);
    cursor: pointer;
    text-align: left;
    transition:
      transform var(--transition-fast),
      box-shadow var(--transition-fast);
  }

  .person-card:hover {
    transform: translateY(-2px);
    box-shadow: var(--shadow-md);
  }

  .person-card:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: 2px;
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

  .person-hidden-badge {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-full);
    background: var(--surface-color);
    color: var(--text-secondary);
    font-size: var(--font-xs);
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
    font-size: var(--font-xs);
  }

  .person-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-1);
    padding: 0 var(--space-2) var(--space-2);
  }

  .person-action {
    display: flex;
    padding: var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
  }

  .person-action:hover {
    color: var(--text-primary);
    background: var(--background-secondary);
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

  .people-empty p {
    margin: 0;
  }

  .person-dialog-backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-4);
    background: color-mix(in oklch, black 45%, transparent);
  }

  .person-dialog {
    width: min(420px, 100%);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-5);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-lg);
    background: var(--surface-color);
    box-shadow: var(--shadow-heavy);
  }

  .person-dialog h2 {
    margin: 0;
    font-size: var(--font-lg);
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
  }

  .person-dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }

  @media (max-width: 768px) {
    .people-grid {
      grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
      gap: var(--space-3);
    }
  }
</style>
