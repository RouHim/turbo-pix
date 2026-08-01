<script>
  import { toasts, removeToast } from '../lib/state.svelte.js';
  import { t } from './i18n.js';
  import Icon from './Icon.svelte';

  const typeIcons = {
    success: 'check-circle',
    error: 'alert-circle',
    warning: 'alert-triangle',
    info: 'info',
  };
</script>

<div class="toast-container" aria-live="polite">
  {#each toasts as toast (toast.id)}
    <div class="toast toast-{toast.type}">
      <Icon name={typeIcons[toast.type] || 'info'} width={18} height={18} />
      <span class="toast-content">
        {#if toast.title}<div class="toast-title">{toast.title}</div>{/if}
        {#if toast.message}<span class="toast-message">{toast.message}</span>{/if}
      </span>
      <button
        type="button"
        class="toast-close"
        aria-label={$t('ui.close', { default: 'Close' })}
        onclick={() => removeToast(toast.id)}
      >
        <Icon name="x" width={14} height={14} />
      </button>
    </div>
  {/each}
</div>

<style>
  .toast-container {
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
  .toast {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 20px;
    border-radius: 12px;
    background: var(--glass-bg);
    backdrop-filter: blur(16px) saturate(1.5);
    -webkit-backdrop-filter: blur(16px) saturate(1.5);
    color: var(--text-primary);
    font-size: 14px;
    box-shadow: 0 4px 24px rgba(0, 0, 0, 0.15);
    pointer-events: auto;
    animation: toast-in 0.3s ease-out;
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
  .toast-error {
    border-left: 3px solid var(--error, #e74c3c);
  }
  .toast-success {
    border-left: 3px solid var(--success, #2ecc71);
  }
  .toast-warning {
    border-left: 3px solid var(--warning, #f39c12);
  }
  .toast-info {
    border-left: 3px solid var(--accent, #3498db);
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
</style>
