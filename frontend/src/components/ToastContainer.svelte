<script>
  import { toasts, removeToast } from '../lib/state.svelte.js';
  import { t } from '../lib/i18n.js';
  import Icon from './Icon.svelte';

  const typeIcons = {
    success: 'check-circle',
    error: 'alert-circle',
    warning: 'alert-triangle',
    info: 'info',
  };
</script>

<div class="toast-region">
  {#snippet toastItem(toast)}
    <div class="toast toast-{toast.type}" class:leaving={toast.leaving}>
      <Icon name={typeIcons[toast.type] || 'info'} width={18} height={18} />
      <div class="toast-content">
        {#if toast.title}<div class="toast-title">{toast.title}</div>{/if}
        {#if toast.message}<span class="toast-message">{toast.message}</span>{/if}
      </div>
      <button
        type="button"
        class="toast-close"
        aria-label={$t('ui.close', { default: 'Close' })}
        onclick={() => removeToast(toast.id)}
      >
        <Icon name="x" width={14} height={14} />
      </button>
    </div>
  {/snippet}

  <div class="toast-container" role="alert" aria-live="assertive">
    {#each toasts.filter((toast) => toast.type === 'error') as toast (toast.id)}
      {@render toastItem(toast)}
    {/each}
  </div>
  <div class="toast-container" role="status" aria-live="polite">
    {#each toasts.filter((toast) => toast.type !== 'error') as toast (toast.id)}
      {@render toastItem(toast)}
    {/each}
  </div>
</div>

<style>
  .toast-region {
    position: fixed;
    bottom: 24px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 10000;
    display: flex;
    flex-direction: column;
    gap: 8px;
    pointer-events: none;
  }
  .toast-container {
    display: flex;
    flex-direction: column;
    gap: 8px;
    pointer-events: none;
  }
  .toast {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 20px;
    border-radius: var(--radius-md);
    background: var(--glass-bg);
    backdrop-filter: blur(16px) saturate(1.5);
    -webkit-backdrop-filter: blur(16px) saturate(1.5);
    color: var(--text-primary);
    font-size: 14px;
    box-shadow: var(--shadow-heavy);
    pointer-events: auto;
    animation: toast-in 0.3s ease-out;
  }
  .toast.leaving {
    animation: toast-out 0.3s ease-in forwards;
    pointer-events: none;
  }
  .toast-content {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .toast-title {
    font-weight: 600;
  }
  .toast-close {
    margin-left: auto;
    background: none;
    border: none;
    color: var(--text-primary);
    opacity: 0.6;
    cursor: pointer;
    padding: 2px;
    display: flex;
  }
  .toast-close:hover {
    opacity: 1;
  }
  /* The type is carried by the leading icon, not a side tab: a colored
     border-left is the category's default tell (see craft floor). */
  .toast-error > :global(svg) {
    color: var(--color-danger);
  }
  .toast-success > :global(svg) {
    color: var(--color-success);
  }
  .toast-warning > :global(svg) {
    color: var(--accent-color);
  }
  .toast-info > :global(svg) {
    color: var(--primary-color);
  }

  @supports not (backdrop-filter: blur(1px)) {
    .toast {
      background: var(--surface-color);
    }
  }

  @media (prefers-reduced-transparency: reduce) {
    .toast {
      background: var(--surface-color);
      backdrop-filter: none;
      -webkit-backdrop-filter: none;
    }
  }

  @keyframes toast-in {
    from {
      opacity: 0;
      transform: translateY(12px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  @keyframes toast-out {
    from {
      opacity: 1;
      transform: translateY(0);
    }
    to {
      opacity: 0;
      transform: translateY(12px);
    }
  }
</style>
