<script>
  import { t } from '../lib/i18n.js';
  import { appState } from '../lib/state.svelte.js';
  import { route, pushState } from '../lib/router.svelte.js';
  import SearchBar from './SearchBar.svelte';
  import ThemeToggle from './ThemeToggle.svelte';
  import Icon from '../lib/Icon.svelte';

  function toggleSidebar() {
    appState.sidebarOpen = !appState.sidebarOpen;
  }

  function toggleMobileSearch() {
    appState.mobileSearchOpen = !appState.mobileSearchOpen;
  }

  function onLogoClick(e) {
    e.preventDefault();
    if (!(route.view === 'all' && !route.query)) {
      pushState({ view: 'all', query: null });
    }
    appState.sidebarOpen = false;
  }
</script>

<header class="header">
  <div class="header-content">
    <button
      type="button"
      class="menu-btn"
      title={$t('ui.menu', { default: 'Menu' })}
      onclick={toggleSidebar}
      aria-label={$t('ui.menu', { default: 'Menu' })}
    >
      <Icon name="menu" width={20} height={20} />
    </button>
    <button
      type="button"
      class="mobile-search-btn"
      title={$t('ui.search', { default: 'Search' })}
      onclick={toggleMobileSearch}
      aria-label={$t('ui.search', { default: 'Search' })}
    >
      <Icon name="search" width={20} height={20} />
    </button>
    <h1 class="logo">
      <a href="/" id="logo-link" onclick={onLogoClick}>
        <img src="/favicon.svg" alt="TurboPix logo" />TurboPix
      </a>
    </h1>
    <div class="header-actions">
      <SearchBar />
      <ThemeToggle />
    </div>
  </div>
</header>

<style>
  .header {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    height: var(--header-height);
    background: var(--glass-bg);
    backdrop-filter: blur(16px) saturate(1.5);
    -webkit-backdrop-filter: blur(16px) saturate(1.5);
    border-bottom: 1px solid var(--glass-border, var(--divider-color));
    box-shadow: var(--shadow-light);
    z-index: 100;
  }

  .header-content {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 var(--space-6);
    height: 100%;
    max-width: none;
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .logo {
    font-size: var(--font-3xl);
    font-weight: var(--font-bold);
    margin: 0;
    color: var(--text-primary);
    font-family: var(--font-heading);
    letter-spacing: -0.5px;
    display: flex;
    align-items: center;
  }

  .logo a {
    text-decoration: none;
    color: inherit;
    display: flex;
    align-items: center;
    cursor: pointer;
    transition: opacity 0.2s ease;
  }

  .logo a:hover {
    opacity: 0.8;
  }

  .logo a :global(img) {
    height: 2em;
    margin-right: 0.5em;
  }

  .menu-btn {
    display: none;
    width: var(--button-size);
    height: var(--button-size);
    border: none;
    background: transparent;
    color: var(--text-primary);
    cursor: pointer;
    border-radius: var(--radius-md);
    align-items: center;
    justify-content: center;
    margin-right: var(--space-2);
  }

  .menu-btn:hover {
    background: var(--background-secondary);
  }

  .mobile-search-btn {
    display: none;
    width: var(--button-size);
    height: var(--button-size);
    border: none;
    background: transparent;
    color: var(--text-primary);
    cursor: pointer;
    border-radius: var(--radius-md);
    align-items: center;
    justify-content: center;
  }

  .mobile-search-btn:hover {
    background: var(--background-secondary);
  }
  @media (max-width: 768px) {
    .menu-btn,
    .mobile-search-btn {
      display: flex;
    }
  }
</style>
