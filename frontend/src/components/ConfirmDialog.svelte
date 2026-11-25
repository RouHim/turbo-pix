<script>
  import { t } from '../lib/i18n.js';
  import { confirmState, resolveConfirm } from '../lib/confirmDialog.svelte.js';
  import SlideToConfirm from './SlideToConfirm.svelte';

  let dialogEl = $state(null);
  let cancelEl = $state(null);
  let sliderEl = $state(null);
  let pending = $state(false);
  let errorMessage = $state('');

  $effect(() => {
    if (confirmState.open) {
      if (!dialogEl?.open) dialogEl?.showModal();
    } else {
      dialogEl?.close();
    }
  });

  let shownRequest = $state(0);

  // Every request starts at rest with the safe choice focused: the slider is
  // the destructive accept, so Enter on open must never arm or delete. The
  // request id also covers a request superseding another while the dialog is
  // open.
  $effect(() => {
    if (!confirmState.open) {
      pending = false;
      errorMessage = '';
      shownRequest = 0;
      return;
    }
    if (shownRequest !== confirmState.requestId) {
      shownRequest = confirmState.requestId;
      pending = false;
      errorMessage = '';
      sliderEl?.reset();
    }
    cancelEl?.focus();
  });

  async function handleSlideConfirm() {
    // A closed dialog must never start the destructive call: pressing Escape
    // with the thumb at the far end resolves the request false, yet the hidden
    // track still owns the pointer capture, so the following mouseup reaches
    // `complete()` and would otherwise delete a photo the user cancelled.
    if (pending || !confirmState.open) return;
    const requestId = confirmState.requestId;
    pending = true;
    errorMessage = '';
    try {
      await confirmState.onConfirm?.();
      if (confirmState.requestId !== requestId || !confirmState.open) return;
      resolveConfirm(true);
    } catch (error) {
      if (confirmState.requestId !== requestId || !confirmState.open) return;
      pending = false;
      // The dialog stays open, so the failure belongs here: a toast would be
      // painted behind the modal's top layer.
      errorMessage =
        error instanceof Error && error.message
          ? error.message
          : $t('ui.action_failed', { default: 'Action failed' });
      sliderEl?.reset();
      sliderEl?.focus();
    }
  }

  function handleCancel() {
    if (pending) return;
    resolveConfirm(false);
  }

  function onDialogKeydown(event) {
    if (event.key !== 'Tab' || !confirmState.slide) return;

    // Cancel is the last tabbable control and Chromium does not wrap Tab back
    // to the slider; close the loop explicitly. While the request runs Cancel
    // is disabled, so the slider is the only focusable node.
    if (pending) {
      event.preventDefault();
      sliderEl?.focus();
      return;
    }

    const target = event.target;
    if (!(target instanceof HTMLElement)) return;

    if (event.shiftKey) {
      if (target.closest('[data-testid="confirm-dialog-slider"]')) {
        event.preventDefault();
        cancelEl?.focus();
      }
      return;
    }

    if (cancelEl?.contains(target)) {
      event.preventDefault();
      sliderEl?.focus();
    }
  }
</script>

<dialog
  class="confirm-dialog"
  bind:this={dialogEl}
  aria-labelledby={confirmState.title ? 'confirm-dialog-title' : 'confirm-dialog-message'}
  aria-describedby={confirmState.title ? 'confirm-dialog-message' : undefined}
  aria-busy={pending}
  data-testid="confirm-dialog"
  onkeydown={onDialogKeydown}
  oncancel={(event) => {
    // The request cannot be aborted once sent: closing mid-flight would fake
    // a cancellation the server does not honour.
    if (pending) {
      event.preventDefault();
      return;
    }
    // Escape is a user cancel and must take effect before the close event
    // (which only runs after the element is hidden): otherwise a pointerup
    // still captured by the slider reaches `complete()` while the request is
    // still open and deletes what the user just cancelled. Resolve here
    // synchronously; the state change above closes the dialog and `onclose`
    // stays as the fallback path.
    event.preventDefault();
    sliderEl?.reset();
    resolveConfirm(false);
  }}
  onclose={() => {
    pending = false;
    errorMessage = '';
    // The dialog can close mid-drag (Escape). Drop the captured travel so a
    // later pointerup cannot arm or confirm anything.
    sliderEl?.reset();
    resolveConfirm(false);
  }}
>
  {#if confirmState.title}
    <h2 id="confirm-dialog-title" class="confirm-title">{confirmState.title}</h2>
  {/if}
  <p id="confirm-dialog-message" class="confirm-message" data-testid="confirm-dialog-message">
    {confirmState.message}
  </p>
  {#if errorMessage}
    <p class="confirm-error" role="alert" data-testid="confirm-dialog-error">{errorMessage}</p>
  {/if}
  {#if confirmState.slide}
    <SlideToConfirm
      bind:this={sliderEl}
      label={$t('ui.slide_to_delete', { default: 'Slide to delete' })}
      armedLabel={$t('ui.release_to_delete', { default: 'Release to delete' })}
      keyboardArmedLabel={$t('ui.press_enter_to_delete', { default: 'Press Enter to delete' })}
      busyLabel={$t('ui.deleting', { default: 'Deleting…' })}
      ariaLabel={confirmState.confirmLabel}
      {pending}
      onconfirm={handleSlideConfirm}
    />
  {/if}
  <div class="confirm-actions">
    <!-- Cancel stays focused first: native <dialog> focuses the first focusable
         element, and the safe choice is the right default for a destructive
         question. In slide mode Tab wraps from here to the slider. -->
    <button
      type="button"
      class="confirm-cancel"
      data-testid="confirm-dialog-cancel"
      bind:this={cancelEl}
      disabled={pending}
      onclick={handleCancel}
    >
      {$t('ui.cancel', { default: 'Cancel' })}
    </button>
    {#if !confirmState.slide}
      <button
        type="button"
        class="confirm-accept"
        class:danger={confirmState.danger}
        data-testid="confirm-dialog-accept"
        onclick={() => resolveConfirm(true)}
      >
        {confirmState.confirmLabel}
      </button>
    {/if}
  </div>
</dialog>

<style>
  .confirm-dialog {
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-lg);
    background: var(--surface-color);
    color: var(--text-primary);
    padding: var(--space-6);
    width: min(420px, 90vw);
    box-shadow: var(--shadow-heavy);
    font-family: var(--font-body);
  }

  .confirm-dialog::backdrop {
    background: oklch(0% 0 0deg / 40%);
  }

  .confirm-title {
    margin: 0 0 var(--space-2);
    font-size: var(--font-2xl);
    font-weight: var(--font-semibold);
    line-height: var(--leading-tight);
  }

  .confirm-message {
    margin: 0 0 var(--space-4);
    font-size: var(--font-md);
    line-height: var(--leading-normal);
  }

  .confirm-error {
    margin: 0 0 var(--space-3);
    padding: var(--space-2) var(--space-3);
    border: 1px solid color-mix(in oklch, var(--color-danger) 30%, transparent);
    border-radius: var(--radius-sm);
    background: color-mix(in oklch, var(--color-danger) 10%, transparent);
    color: var(--color-danger-ink);
    font-size: var(--font-sm);
    line-height: var(--leading-normal);
  }

  .confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-3);
    margin-top: var(--space-4);
    padding-top: var(--space-3);
    border-top: 1px solid var(--divider-color);
  }

  .confirm-cancel,
  .confirm-accept {
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius-md);
    border: 1px solid var(--divider-color);
    background: var(--surface-elevated);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--font-base);
    min-height: var(--button-size);
    cursor: pointer;
  }

  .confirm-cancel:hover {
    border-color: var(--primary-color);
  }

  .confirm-cancel:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  .confirm-cancel:disabled:hover {
    border-color: var(--divider-color);
  }

  .confirm-accept.danger {
    background: var(--color-danger);
    border-color: var(--color-danger);
    color: white;
  }

  .confirm-accept.danger:hover {
    background: var(--color-danger-hover);
    border-color: var(--color-danger-hover);
  }

  .confirm-cancel:focus-visible,
  .confirm-accept:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: 2px;
  }
</style>
