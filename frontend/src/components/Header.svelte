<script>
  import { t } from '../lib/i18n.js';
  import { appState } from '../lib/state.svelte.js';
  import { route, pushState } from '../lib/router.svelte.js';
  import SearchBar from './SearchBar.svelte';
  import ThemeToggle from './ThemeToggle.svelte';
  import Icon from './Icon.svelte';

  function toggleSidebar() {
    appState.sidebarOpen = !appState.sidebarOpen;
  }

  function toggleMobileSearch() {
    appState.mobileSearchOpen = !appState.mobileSearchOpen;
  }

  function onLogoClick(e) {
    e.preventDefault();
    appState.mobileSearchOpen = false;
    if (!(route.view === 'all' && !route.query && route.album == null)) {
      pushState({ view: 'all', query: null, album: null });
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
      aria-expanded={appState.sidebarOpen}
      aria-controls="sidebar-nav"
    >
      <Icon name="menu" width={20} height={20} />
    </button>
    <button
      type="button"
      class="mobile-search-btn"
      title={$t('ui.search', { default: 'Search' })}
      onclick={toggleMobileSearch}
      aria-label={$t('ui.search', { default: 'Search' })}
      aria-expanded={appState.mobileSearchOpen}
      aria-controls="search-bar"
    >
      <Icon name="search" width={20} height={20} />
    </button>
    <h1 class="logo">
      <a href="/" onclick={onLogoClick}>
        <img src="/favicon.svg" alt={$t('ui.logo_alt', { default: 'TurboPix logo' })} />TurboPix
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
      font-size: var(--font-2xl);
      transition: var(--transition-fast);
    }

    .menu-btn:active,
    .mobile-search-btn:active {
      background: var(--surface-elevated);
      transform: scale(0.95);
    }

    .header-actions {
      gap: var(--space-1);
    }
  }

  /* Solid-surface fallbacks: must sit in scoped styles (same specificity as
     the base rule + later source order) so they win when backdrop-filter is
     unsupported or reduced transparency is requested. */
  @supports not (backdrop-filter: blur(1px)) {
    .header {
      background: var(--surface-color);
    }
  }

  @media (prefers-reduced-transparency: reduce) {
    .header {
      backdrop-filter: none;
      -webkit-backdrop-filter: none;
      background: var(--surface-color);
    }
  }

  /* Responsive padding: scoped so these win over the base rule (global
     @media overrides of equal specificity are outranked by scoped rules). */
  @media (width <= 1024px) {
    .header-content {
      padding: 0 var(--space-4);
    }
  }

  @media (width <= 480px) {
    .header-content {
      padding: 0 var(--space-2);
    }
    .logo {
      font-size: var(--font-xl);
    }
  }
</style>
