<script>
  import { onMount } from 'svelte';
  import { t } from '../lib/i18n.js';
  import Icon from '../lib/Icon.svelte';
  import { api } from '../lib/api.js';
  import { logger } from '../lib/logger.js';
  import { route } from '../lib/router.svelte.js';
  import {
    selectionState,
    addToast,
    exitSelectionMode,
    selectAllVisible,
  } from '../lib/state.svelte.js';

  const keys = $derived(Object.keys(selectionState.selected));
  const count = $derived(keys.length);
  const canAct = $derived(count > 0 && !selectionState.busy);

  const allVisibleSelected = $derived(
    selectionState.orderedKeys.length > 0 &&
      selectionState.orderedKeys.every((k) => selectionState.selected[k])
  );

  // Per-surface action set: housekeeping candidates are photos, so they get
  // the five photo actions plus keep; collages get accept/reject only.
  const actionConfig = $derived(
    route.view === 'collages'
      ? [
          { id: 'accept', labelKey: 'ui.accept_collage', icon: 'check' },
          { id: 'reject', labelKey: 'ui.reject_collage', icon: 'x' },
        ]
      : route.view === 'housekeeping'
        ? [
            { id: 'keep', labelKey: 'ui.keep_photo', icon: 'check' },
            { id: 'delete', labelKey: 'ui.delete_photo', icon: 'trash-2' },
            { id: 'addFavorite', labelKey: 'ui.add_to_favorites', icon: 'heart' },
            { id: 'removeFavorite', labelKey: 'ui.remove_from_favorites', icon: 'heart' },
            { id: 'dateShift', labelKey: 'ui.shift_dates', icon: 'calendar' },
            { id: 'export', labelKey: 'ui.export', icon: 'archive' },
          ]
        : [
            { id: 'delete', labelKey: 'ui.delete_photo', icon: 'trash-2' },
            { id: 'addFavorite', labelKey: 'ui.add_to_favorites', icon: 'heart' },
            { id: 'removeFavorite', labelKey: 'ui.remove_from_favorites', icon: 'heart' },
            { id: 'dateShift', labelKey: 'ui.shift_dates', icon: 'calendar' },
            { id: 'export', labelKey: 'ui.export', icon: 'archive' },
          ]
  );

  // Action button data-action names (E2E contract).
  const actionDataName = {
    delete: 'batch-delete',
    keep: 'batch-keep',
    addFavorite: 'batch-add-favorite',
    removeFavorite: 'batch-remove-favorite',
    dateShift: 'batch-date-shift',
    export: 'batch-export',
    accept: 'batch-accept',
    reject: 'batch-reject',
  };

  let dateShiftOpen = $state(false);
  let daysInput = $state('');
  const daysValid = $derived(
    daysInput !== '' && Number.isInteger(Number(daysInput)) && Number(daysInput) !== 0
  );

  /** Drop applied keys from the selection map and re-arm the range anchor. */
  function dropSelectedKeys(applied) {
    for (const key of applied) delete selectionState.selected[key];
    if (selectionState.anchorKey != null && !selectionState.selected[selectionState.anchorKey]) {
      selectionState.anchorKey = null;
    }
  }

  function reportResult(res) {
    if (res.failed?.length) {
      addToast(
        $t('errors.batchActionFailed', { default: 'Batch action failed' }),
        $t('notifications.batchFailed', {
          default: '{count} items failed',
          values: { count: res.failed.length },
        }),
        'error',
        5000
      );
      logger.warn('Batch action reported failures', { component: 'SelectionBar', res });
    }
  }

  async function runAction(actionId) {
    if (!canAct) return;

    if (actionId === 'delete') {
      const msg = $t('notifications.batchDeleteConfirm', {
        default: 'Permanently delete {count} photo(s)? This cannot be undone.',
        values: { count },
      });
      if (!confirm(msg)) return;
      selectionState.busy = 'delete';
      try {
        const res = await api.batchDelete(keys);
        // Shared deletion contract: every view splices its list on
        // photoRemoved; the selection prunes itself from the applied set.
        for (const hash of res.applied || []) {
          window.dispatchEvent(new CustomEvent('photoRemoved', { detail: { hash } }));
        }
        dropSelectedKeys(res.applied || []);
        addToast(
          $t('notifications.batchDeleted', {
            default: '{count} photos deleted',
            values: { count: res.applied?.length || 0 },
          }),
          '',
          'success'
        );
        reportResult(res);
      } catch (error) {
        logger.error('Batch delete failed', { component: 'SelectionBar' }, error);
        addToast(
          $t('errors.batchActionFailed', { default: 'Batch action failed' }),
          error?.message,
          'error',
          5000
        );
      } finally {
        selectionState.busy = null;
        if (count === 0) exitSelectionMode();
      }
      return;
    }

    if (actionId === 'keep') {
      selectionState.busy = 'keep';
      try {
        const res = await api.batchRemoveHousekeepingCandidates(keys);
        window.dispatchEvent(
          new CustomEvent('housekeepingKept', { detail: { hashes: res.applied || [] } })
        );
        dropSelectedKeys(res.applied || []);
        addToast(
          $t('notifications.batchKept', {
            default: '{count} candidates kept',
            values: { count: res.applied?.length || 0 },
          }),
          '',
          'success'
        );
        reportResult(res);
      } catch (error) {
        logger.error('Batch keep failed', { component: 'SelectionBar' }, error);
        addToast(
          $t('errors.batchActionFailed', { default: 'Batch action failed' }),
          error?.message,
          'error',
          5000
        );
      } finally {
        selectionState.busy = null;
        if (count === 0) exitSelectionMode();
      }
      return;
    }

    if (actionId === 'addFavorite' || actionId === 'removeFavorite') {
      const isFavorite = actionId === 'addFavorite';
      selectionState.busy = isFavorite ? 'favorite' : 'unfavorite';
      try {
        const res = await api.batchSetFavorite(keys, isFavorite);
        for (const hash of res.applied || []) {
          window.dispatchEvent(
            new CustomEvent('favoriteToggled', { detail: { photoHash: hash, isFavorite } })
          );
        }
        addToast(
          isFavorite
            ? $t('notifications.batchFavorited', {
                default: '{count} photos added to favorites',
                values: { count: res.applied?.length || 0 },
              })
            : $t('notifications.batchUnfavorited', {
                default: '{count} photos removed from favorites',
                values: { count: res.applied?.length || 0 },
              }),
          '',
          'success'
        );
        reportResult(res);
      } catch (error) {
        logger.error('Batch favorite failed', { component: 'SelectionBar' }, error);
        addToast(
          $t('errors.batchActionFailed', { default: 'Batch action failed' }),
          error?.message,
          'error',
          5000
        );
      } finally {
        selectionState.busy = null;
        // In the favorites view an unfavorite splices the cards out of the
        // surface; when the selection empties, selection mode must end.
        if (count === 0) exitSelectionMode();
      }
      return;
    }

    if (actionId === 'dateShift') {
      dateShiftOpen = true;
      return;
    }

    if (actionId === 'export') {
      selectionState.busy = 'export';
      // Short-lived info toast; the button's spinner + "Working…" carry the
      // progress state (a persistent toast would need a removal handle).
      addToast($t('ui.exporting', { default: 'Exporting…' }), '', 'info', 2000);
      try {
        const { blob, filename } = await api.batchExport(keys);
        const url = URL.createObjectURL(blob);
        const link = document.createElement('a');
        link.href = url;
        link.download = filename;
        link.click();
        setTimeout(() => URL.revokeObjectURL(url), 1000);
        addToast(
          $t('notifications.batchExported', {
            default: '{count} photos exported',
            values: { count },
          }),
          '',
          'success'
        );
      } catch (error) {
        logger.error('Batch export failed', { component: 'SelectionBar' }, error);
        addToast(
          $t('errors.batchActionFailed', { default: 'Batch action failed' }),
          error?.message,
          'error',
          5000
        );
      } finally {
        selectionState.busy = null;
      }
      return;
    }

    if (actionId === 'accept' || actionId === 'reject') {
      if (actionId === 'reject') {
        const msg = $t('notifications.batchRejectConfirm', {
          default: 'Reject {count} collage(s)? This cannot be undone.',
          values: { count },
        });
        if (!confirm(msg)) return;
      }
      selectionState.busy = actionId;
      try {
        const ids = keys.map(Number);
        const res =
          actionId === 'accept'
            ? await api.batchAcceptCollages(ids)
            : await api.batchRejectCollages(ids);
        for (const id of res.applied || []) {
          window.dispatchEvent(
            new CustomEvent(actionId === 'accept' ? 'collageAccepted' : 'collageRejected', {
              detail: { collageId: Number(id) },
            })
          );
        }
        dropSelectedKeys(res.applied || []);
        addToast(
          actionId === 'accept'
            ? $t('notifications.batchAccepted', {
                default: '{count} collages accepted',
                values: { count: res.applied?.length || 0 },
              })
            : $t('notifications.batchRejected', {
                default: '{count} collages rejected',
                values: { count: res.applied?.length || 0 },
              }),
          '',
          actionId === 'accept' ? 'success' : 'info'
        );
        reportResult(res);
      } catch (error) {
        logger.error('Batch collage action failed', { component: 'SelectionBar' }, error);
        addToast(
          $t('errors.batchActionFailed', { default: 'Batch action failed' }),
          error?.message,
          'error',
          5000
        );
      } finally {
        selectionState.busy = null;
        if (count === 0) exitSelectionMode();
      }
      return;
    }
  }

  async function applyDateShift() {
    if (!daysValid || selectionState.busy) return;
    const days = Number(daysInput);
    selectionState.busy = 'dateShift';
    try {
      const res = await api.batchDateShift(keys, days);
      // The backend returns only hashes, not updated photo objects; one
      // reload keeps every surface consistent (order may change too).
      window.dispatchEvent(new CustomEvent('photosReloadRequested'));
      addToast(
        $t('notifications.batchDateShifted', {
          default: '{count} photos date-shifted',
          values: { count: res.applied?.length || 0 },
        }),
        '',
        'success'
      );
      if ((res.skipped || []).length > 0) {
        addToast(
          $t('notifications.batchSkippedNoDate', {
            default: '{count} photos skipped (no taken date)',
            values: { count: res.skipped.length },
          }),
          '',
          'info',
          5000
        );
      }
      reportResult(res);
      dateShiftOpen = false;
    } catch (error) {
      logger.error('Batch date shift failed', { component: 'SelectionBar' }, error);
      addToast(
        $t('errors.batchActionFailed', { default: 'Batch action failed' }),
        error?.message,
        'error',
        5000
      );
    } finally {
      selectionState.busy = null;
    }
  }

  function onKeydown(e) {
    // Escape exits selection mode — but never steal it from an open viewer.
    if (e.key === 'Escape' && selectionState.active && !selectionState.busy && !route.photo) {
      exitSelectionMode();
    }
  }

  onMount(() => {
    document.addEventListener('keydown', onKeydown);
    return () => document.removeEventListener('keydown', onKeydown);
  });
</script>

<div
  id="selection-bar"
  role="toolbar"
  aria-label={$t('ui.select', { default: 'Select' })}
  class:busy={!!selectionState.busy}
>
  <span class="selection-count" data-testid="selection-count">
    {$t('ui.selected_count', { default: '{count} selected', values: { count } })}
  </span>

  <button
    type="button"
    class="btn select-all-btn"
    data-action="batch-select-all"
    aria-pressed={allVisibleSelected}
    disabled={!selectionState.active || selectionState.busy}
    onclick={selectAllVisible}
  >
    <Icon name="check-square" width={16} height={16} />
    {$t('ui.select_all_visible', { default: 'Select all visible' })}
  </button>

  {#each actionConfig as action (action.id)}
    {#if action.id === 'dateShift' && dateShiftOpen}
      <span class="date-shift-row">
        <Icon name="calendar" width={16} height={16} />
        <input
          id="batch-days-input"
          type="number"
          step="1"
          aria-label={$t('ui.days', { default: 'Days' })}
          bind:value={daysInput}
          onkeydown={(e) => {
            if (e.key === 'Enter') applyDateShift();
          }}
        />
        <button
          type="button"
          class="btn"
          data-action="batch-date-shift-apply"
          disabled={!daysValid || !!selectionState.busy}
          onclick={applyDateShift}
        >
          {$t('ui.apply', { default: 'Apply' })}
        </button>
        <button
          type="button"
          class="btn"
          disabled={!!selectionState.busy}
          onclick={() => {
            dateShiftOpen = false;
            daysInput = '';
          }}
        >
          <Icon name="x" width={16} height={16} />
        </button>
      </span>
    {:else}
      <button
        type="button"
        class="btn batch-action-btn"
        data-action={actionDataName[action.id]}
        disabled={!canAct}
        onclick={() => runAction(action.id)}
      >
        {#if selectionState.busy === action.id}
          <span class="spin">
            <Icon name="loader" width={16} height={16} />
          </span>
          {$t('ui.working', { default: 'Working…' })}
        {:else}
          <Icon name={action.icon} width={16} height={16} />
          {$t(action.labelKey)}
        {/if}
      </button>
    {/if}
  {/each}

  <button
    type="button"
    class="btn exit-btn"
    data-action="batch-exit"
    title={$t('ui.cancel_selection', { default: 'Cancel selection' })}
    aria-label={$t('ui.cancel_selection', { default: 'Cancel selection' })}
    disabled={!!selectionState.busy}
    onclick={exitSelectionMode}
  >
    <Icon name="x" width={16} height={16} />
  </button>
</div>

<style>
  #selection-bar {
    position: fixed;
    bottom: var(--space-4);
    left: 50%;
    transform: translateX(-50%);
    z-index: 200; /* above header(100)/sidebar(90), below viewer backdrop(2000) */
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg, 12px);
    background: var(--glass-bg, oklch(100% 0 0deg / 10%));
    backdrop-filter: blur(16px) saturate(1.5);
    -webkit-backdrop-filter: blur(16px) saturate(1.5);
    box-shadow: var(--shadow-medium);
    max-width: calc(100vw - 2 * var(--space-4));
    overflow-x: auto;
  }

  .selection-count {
    font-weight: var(--font-semibold);
    white-space: nowrap;
  }

  .date-shift-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  #batch-days-input {
    width: 6rem;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-sm);
    background: var(--surface-color);
    color: var(--text-primary);
  }

  .btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    white-space: nowrap;
  }

  .spin {
    animation: selection-bar-spin 0.8s linear infinite;
  }

  @keyframes selection-bar-spin {
    to {
      transform: rotate(360deg);
    }
  }

  #selection-bar.busy .batch-action-btn {
    cursor: wait;
  }

  /* Narrow screens: keep the bar usable with a horizontal scroll instead of
     wrapping (scoped rule — global media overrides lose to component rules). */
  @media (max-width: 640px) {
    #selection-bar {
      left: var(--space-2);
      right: var(--space-2);
      transform: none;
    }
  }
</style>
