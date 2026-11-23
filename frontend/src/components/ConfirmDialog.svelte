<script>
  import { t } from '../lib/i18n.js';
  import { confirmState, resolveConfirm } from '../lib/confirmDialog.svelte.js';
  import SlideToConfirm from './SlideToConfirm.svelte';

  let dialogEl = $state(null);
  let cancelEl = $state(null);
  let sliderEl = $state(null);
  let pending = $state(false);

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
      shownRequest = 0;
      return;
    }
    if (shownRequest !== confirmState.requestId) {
      shownRequest = confirmState.requestId;
      pending = false;
      sliderEl?.reset();
    }
    cancelEl?.focus();
  });

  async function handleSlideConfirm() {
    if (pending) return;
    const requestId = confirmState.requestId;
    pending = true;
    try {
      await confirmState.onConfirm?.();
      if (confirmState.requestId !== requestId || !confirmState.open) return;
      resolveConfirm(true);
    } catch {
      if (confirmState.requestId !== requestId || !confirmState.open) return;
      pending = false;
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
  aria-labelledby="confirm-dialog-message"
  aria-busy={pending}
  data-testid="confirm-dialog"
  onkeydown={onDialogKeydown}
  oncancel={(event) => {
    // The request cannot be aborted once sent: closing mid-flight would fake
    // a cancellation the server does not honour.
    if (pending) event.preventDefault();
  }}
  onclose={() => {
    pending = false;
    resolveConfirm(false);
  }}
>
  <p id="confirm-dialog-message" class="confirm-message" data-testid="confirm-dialog-message">
    {confirmState.message}
  </p>
  {#if confirmState.slide}
    <SlideToConfirm
      bind:this={sliderEl}
      label={$t('ui.slide_to_delete', { default: 'Slide to delete' })}
      armedLabel={$t('ui.release_to_delete', { default: 'Release to delete' })}
      busyLabel={$t('ui.deleting', { default: 'Deleting…' })}
      ariaLabel={confirmState.confirmLabel}
      {pending}
      onconfirm={handleSlideConfirm}
    />
  {/if}
  <div class="confirm-actions" class:slide-actions={confirmState.slide}>
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

  .confirm-message {
    margin: 0 0 var(--space-4);
    font-size: var(--font-md);
    line-height: var(--leading-normal);
  }

  .confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }

  .confirm-actions.slide-actions {
    margin-top: var(--space-2);
  }

  .confirm-cancel,
  .confirm-accept {
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius-md);
    border: 1px solid var(--divider-color);
    background: var(--surface-elevated);
    color: var(--text-primary);
    font-family: var(--font-body);
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

  @media (max-width: 640px) {
    .confirm-cancel,
    .confirm-accept {
      min-height: var(--button-size);
    }
  }
</style>
