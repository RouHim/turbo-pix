<script>
  import { onMount } from 'svelte';
  import { t } from '../lib/i18n.js';
  import { themeState } from '../lib/state.svelte.js';
  import { storage } from '../lib/utils.js';
  import Icon from '../lib/Icon.svelte';

  function applyTheme(theme) {
    document.documentElement.classList.remove('light-theme', 'dark-theme');
    document.documentElement.classList.add(`${theme}-theme`);
    themeState.theme = theme;
    storage.set('theme', theme);
  }

  function toggle() {
    applyTheme(themeState.theme === 'dark' ? 'light' : 'dark');
  }

  onMount(() => {
    let theme = storage.get('theme');
    if (theme !== 'light' && theme !== 'dark') {
      theme = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
    }
    applyTheme(theme);
  });
</script>

<button
  type="button"
  id="theme-toggle"
  class="theme-toggle"
  title={$t('ui.toggle_theme', { default: 'Toggle theme' })}
  onclick={toggle}
  aria-label={$t('ui.toggle_theme', { default: 'Toggle theme' })}
>
  <span class="theme-icon">
    <Icon name={themeState.theme === 'dark' ? 'sun' : 'moon'} width={20} height={20} />
  </span>
</button>

<style>
  .theme-toggle {
    width: var(--button-size);
    height: var(--button-size);
    border: none;
    background: transparent;
    color: var(--text-primary);
    cursor: pointer;
    border-radius: var(--radius-md);
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background var(--transition-fast);
  }

  .theme-toggle:hover {
    background: var(--background-secondary);
  }

  .theme-icon {
    display: flex;
    align-items: center;
    justify-content: center;
  }
</style>
