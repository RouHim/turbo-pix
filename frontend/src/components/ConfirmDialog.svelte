<script>
  import { t } from '../lib/i18n.js';
  import { confirmState, resolveConfirm } from '../lib/confirmDialog.svelte.js';

  let dialogEl = $state(null);

  $effect(() => {
    if (confirmState.open) {
      dialogEl?.showModal();
    } else {
      dialogEl?.close();
    }
  });
</script>

<dialog
  class="confirm-dialog"
  bind:this={dialogEl}
  aria-labelledby="confirm-dialog-message"
  data-testid="confirm-dialog"
  onclose={() => resolveConfirm(false)}
>
  <p id="confirm-dialog-message" class="confirm-message" data-testid="confirm-dialog-message">
    {confirmState.message}
  </p>
  <div class="confirm-actions">
    <!-- Cancel first: native <dialog> focuses the first focusable element, and
         the safe choice is the right default for a destructive question. -->
    <button
      type="button"
      class="confirm-cancel"
      data-testid="confirm-dialog-cancel"
      onclick={() => resolveConfirm(false)}
    >
      {$t('ui.cancel', { default: 'Cancel' })}
    </button>
    <button
      type="button"
      class="confirm-accept"
      class:danger={confirmState.danger}
      data-testid="confirm-dialog-accept"
      onclick={() => resolveConfirm(true)}
    >
      {confirmState.confirmLabel}
    </button>
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
