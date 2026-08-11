<script>
  import { get } from 'svelte/store';
  import { onDestroy } from 'svelte';
  import { t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import { route, pushState, replaceState } from '../lib/router.svelte.js';
  import { appState, addToast, photoGridState, savedSearches } from '../lib/state.svelte.js';
  import { isPrefixQuery } from '../lib/utils.js';
  import Icon from './Icon.svelte';

  let query = $state('');
  let searchHistory = $state(api.getSearchHistory() || []);
  let showSuggestions = $state(false);
  let suggestions = $state([]);
  let searching = $state(false);
  let inputEl = $state(null);
  let focused = $state(false);
  let currentQuery = $state('');
  let searchTimer = null;

  // Save control is offered for searchable views (all/favorites/videos)
  // whenever the state is not the fully-default one (FR-001).
  const canSave = $derived(
    ['all', 'favorites', 'videos'].includes(route.view) &&
      !(
        route.view === 'all' &&
        !route.query &&
        route.sort === 'date_desc' &&
        !route.year &&
        !route.month
      )
  );

  function buildDefaultName() {
    const yearPart = route.year
      ? ` ${route.year}${route.month ? '-' + String(route.month).padStart(2, '0') : ''}`
      : '';
    return (
      ((route.query ?? '') + yearPart).trim() ||
      get(t)('savedSearches.defaultName', { default: 'Saved search' })
    );
  }

  async function saveCurrentSearch() {
    if (!canSave) return;
    try {
      const created = await api.createSavedSearch({
        name: buildDefaultName(),
        query: route.query,
        view: route.view,
        sort: route.sort,
        year: route.year,
        month: route.month,
      });
      savedSearches.unshift(created); // newest-first: fresh row has the max id
      addToast(
        get(t)('savedSearches.saved', { default: 'Search saved' }),
        created.name,
        'success',
        3000
      );
    } catch (error) {
      if (error?.status === 409 && error?.data?.saved_search) {
        addToast(
          get(t)('savedSearches.alreadySaved', { default: 'Search already saved' }),
          error.data.saved_search.name,
          'info',
          4000
        );
        return;
      }
      addToast(
        get(t)('savedSearches.saveFailed', { default: 'Could not save search' }),
        '',
        'error',
        4000
      );
    }
  }

  // Sync query input + grid state from route on popstate / initial load.
  // Re-running the pipeline routes prefix queries (type:/location:/is_favorite:)
  // and semantic queries through their correct paths.
  $effect(() => {
    if (!route) return;
    if (route.query && route.query !== currentQuery) {
      performSearch(route.query, false);
    } else if (!route.query && currentQuery) {
      clearSearch(false);
    }
  });

  // Listen for external search requests
  $effect(() => {
    function onRequestSearch(e) {
      const q = e.detail?.query;
      if (q) {
        query = q;
        if (searchTimer) clearTimeout(searchTimer);
        performSearch(q, true);
      }
    }
    window.addEventListener('requestSearch', onRequestSearch);
    return () => window.removeEventListener('requestSearch', onRequestSearch);
  });

  function performSearch(q, addToHistory = false) {
    // Update URL immediately for explicit searches
    if (addToHistory) {
      pushState({ query: q || null });
    }

    if (!q) return;

    // Record history BEFORE the dedupe early return: type-then-Enter and
    // selectSuggestion reach this with q === currentQuery (the live-search
    // debounce already set it) and would otherwise silently drop the entry.
    // addToSearchHistory dedupes internally, so a repeat call for the same
    // query only bumps its recency — no duplicate entries.
    if (addToHistory) {
      api.addToSearchHistory(q);
      searchHistory = api.getSearchHistory() || [];
    }

    if (q === currentQuery) return;

    currentQuery = q;
    query = q;

    searching = true;

    try {
      // Prefix queries (type:, location:, is_favorite:) use regular search path
      if (isPrefixQuery(q)) {
        photoGridState.semanticSearchMode = false;
        photoGridState.currentQuery = q;
        photoGridState.currentPage = 1;
        searching = false;
        return;
      }

      // Semantic search (default)
      performSemanticSearch(q);
    } catch (error) {
      console.error('Search error:', error);
      searching = false;
    }
  }

  function performSemanticSearch(q) {
    // Remove @ prefix if present
    const cleanQuery = q.startsWith('@') ? q.substring(1).trim() : q;

    photoGridState.semanticSearchMode = true;
    photoGridState.currentQuery = cleanQuery;
    photoGridState.currentPage = 1;
  }

  // The flag clears once the grid finishes loading the search results — and
  // immediately on non-grid views (housekeeping/collages), where PhotoGrid
  // never mounts so photoGridState.loading never flips back.
  $effect(() => {
    if (!photoGridState.loading || !['all', 'favorites', 'videos'].includes(route.view)) {
      searching = false;
    }
  });

  function clearSearch(updateUrl = true) {
    if (searchTimer) {
      clearTimeout(searchTimer);
      searchTimer = null;
    }
    query = '';
    currentQuery = '';
    showSuggestions = false;
    searching = false;
    photoGridState.semanticSearchMode = false;
    photoGridState.currentQuery = '';
    photoGridState.currentPage = 1;

    if (updateUrl && route.query) {
      pushState({ query: null });
    }
  }

  function submitSearch() {
    const q = query.trim();
    showSuggestions = false;
    appState.mobileSearchOpen = false;
    if (!q) {
      clearSearch();
      return;
    }
    // Debounce like typing so the grid request lands after the click handler
    // returns (deterministic for E2E waitForResponse) and dedupes with onInput.
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      performSearch(q, true);
    }, 300);
  }

  function onKeydown(e) {
    if (e.key === 'Enter') {
      e.preventDefault();
      submitSearch();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      clearSearch();
      showSuggestions = false;
    }
  }

  function onFocus() {
    focused = true;
    const history = api.getSearchHistory() || [];
    searchHistory = history;
    generateSuggestions(query.trim());
    showSuggestions = suggestions.length > 0;
  }

  function onInput() {
    const q = query.trim();
    // Generate suggestions as user types
    generateSuggestions(q);
    showSuggestions = suggestions.length > 0;

    // Debounced live search (matches old search.js behavior)
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      if (q.length >= 2) {
        performSearch(q, false);
        replaceState({ query: q });
      } else if (q.length === 0) {
        clearSearch(false);
        replaceState({ query: null });
      }
    }, 300);
  }

  onDestroy(() => {
    if (searchTimer) clearTimeout(searchTimer);
  });

  function selectSuggestion(s) {
    if (searchTimer) clearTimeout(searchTimer);
    appState.mobileSearchOpen = false;
    query = s.query;
    performSearch(s.query, true);
    showSuggestions = false;
  }

  function generateSuggestions(currentValue) {
    const items = [];
    const history = searchHistory || [];

    // Recent searches
    if (history.length > 0) {
      const recentMatches = history
        .filter(
          (item) => !currentValue || item.query.toLowerCase().includes(currentValue.toLowerCase())
        )
        .slice(0, 3)
        .map((item) => ({
          query: item.query,
          text: item.query,
          icon: 'clock',
          subtitle: get(t)('ui.recent_search', { default: 'Recent search' }),
        }));
      items.push(...recentMatches);
    }

    // Dynamic search suggestions based on current value
    if (currentValue.length >= 2) {
      const dynamicSuggestions = getSearchSuggestions(currentValue);
      items.push(...dynamicSuggestions);
    }

    // Quick filters (when empty) — only prefixes the backend tokenizer
    // supports (type:/location:/is_favorite:). camera:*/date:*/has:* queries
    // fall through to semantic search and can never filter, so they are not
    // offered.
    if (!currentValue) {
      items.push({
        query: 'type:video',
        text: get(t)('ui.videos_only', { default: 'Videos only' }),
        icon: 'video',
        subtitle: get(t)('ui.filter_by_type', { default: 'Filter by type' }),
      });
    }

    suggestions = items.slice(0, 8);
  }

  function getSearchSuggestions(value) {
    const items = [];
    const lowerValue = value.toLowerCase();

    // Type suggestions
    if (lowerValue.includes('video')) {
      items.push({
        query: 'type:video',
        text: get(t)('ui.videos_only', { default: 'Videos only' }),
        icon: 'video',
      });
    }
    if (lowerValue.includes('raw')) {
      items.push({
        query: 'type:raw',
        text: get(t)('ui.raw_files_only', { default: 'RAW files only' }),
        icon: 'image',
      });
    }

    return items;
  }

  $effect(() => {
    if (appState.mobileSearchOpen) {
      inputEl?.focus();
    }
  });
</script>

<div id="search-bar" class="search-container" class:mobile-show={appState.mobileSearchOpen}>
  <input
    type="text"
    id="search-input"
    class="search-input"
    placeholder={$t('ui.search_ai_placeholder', { default: 'AI-powered photo search...' })}
    aria-label={$t('ui.search', { default: 'Search' })}
    bind:value={query}
    bind:this={inputEl}
    onkeydown={onKeydown}
    onfocus={onFocus}
    oninput={onInput}
    onblur={() => {
      focused = false;
      setTimeout(() => (showSuggestions = false), 150);
    }}
  />
  <button type="button" id="search-btn" class="search-btn" class:searching onclick={submitSearch}>
    {searching ? '' : $t('ui.search', { default: 'Search' })}
  </button>

  {#if canSave}
    <button
      type="button"
      id="save-search-btn"
      class="save-search-btn"
      title={$t('savedSearches.save', { default: 'Save search' })}
      aria-label={$t('savedSearches.save', { default: 'Save search' })}
      onclick={saveCurrentSearch}
      data-testid="save-search-btn"
    >
      <Icon name="bookmark" width={16} height={16} />
    </button>
  {/if}

  <!-- Search hint -->
  <div class="search-hint" class:visible={focused && !query} data-search-hint="true">
    <Icon name="info" width={14} height={14} class="search-hint-icon" />
    <span
      >{$t('ui.search_hint', {
        default: 'Try: type:video \u00b7 location:city \u00b7 is_favorite:true',
      })}</span
    >
  </div>

  {#if showSuggestions}
    <div id="search-suggestions" class="search-suggestions show">
      {#each suggestions as s (s.query + s.icon)}
        <button type="button" class="suggestion-item" onclick={() => selectSuggestion(s)}>
          <span class="suggestion-icon">
            <Icon name={s.icon} width={16} height={16} />
          </span>
          <span class="suggestion-text">{s.text}</span>
          {#if s.subtitle}
            <span class="suggestion-subtitle">{s.subtitle}</span>
          {/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .search-container {
    position: relative;
    display: flex;
    align-items: center;
    flex: 1;
    max-width: 600px;
    margin: 0 var(--space-8);
  }

  .search-input {
    flex: 1;
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-md);
    font-size: var(--font-lg);
    transition: var(--transition-fast);
    background: var(--background-color);
    color: var(--text-primary);
    font-family: var(--font-body);
  }

  .search-input:focus {
    outline: none;
    border-color: var(--primary-color);
    background: var(--surface-color);
  }

  .search-btn {
    position: relative;
    overflow: visible;
    padding: var(--space-3) var(--space-5);
    margin-left: var(--space-2);
    background: var(--primary-color);
    color: white;
    border: none;
    border-radius: var(--radius-md);
    font-size: var(--font-base);
    font-family: var(--font-body);
    cursor: pointer;
    transition: background var(--transition-fast);
    white-space: nowrap;
  }

  .search-btn:hover {
    background: var(--primary-dark);
  }

  .save-search-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: var(--button-size);
    height: var(--button-size);
    margin-left: var(--space-2);
    border: none;
    background: transparent;
    color: var(--text-primary);
    cursor: pointer;
    border-radius: var(--radius-md);
  }

  .save-search-btn:hover {
    background: var(--background-secondary);
  }

  .search-btn.searching::before,
  .search-btn.searching::after {
    content: '';
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: 100%;
    height: 100%;
    border-radius: inherit;
    border: 2px solid var(--primary-color);
    opacity: 0;
    pointer-events: none;
    animation: radar-pulse 1.5s cubic-bezier(0.4, 0, 0.6, 1) infinite;
  }

  .search-btn.searching::after {
    animation-delay: 0.5s;
  }

  @keyframes radar-pulse {
    0% {
      transform: translate(-50%, -50%) scale(1);
      opacity: 0.8;
    }
    50% {
      opacity: 0.4;
    }
    100% {
      transform: translate(-50%, -50%) scale(2.5);
      opacity: 0;
    }
  }

  /* Search hint */
  .search-hint {
    position: absolute;
    top: 100%;
    left: 0;
    right: 0;
    padding: var(--space-3) var(--space-4);
    margin-top: var(--space-2);
    font-size: var(--font-sm, 13px);
    color: var(--text-muted);
    background: var(--glass-bg, oklch(100% 0 0deg / 70%));
    backdrop-filter: blur(12px) saturate(1.5);
    -webkit-backdrop-filter: blur(12px) saturate(1.5);
    border: 1px solid var(--glass-border, var(--divider-color));
    border-radius: var(--radius-md, 8px);
    box-shadow: var(--shadow-light);
    display: flex;
    align-items: center;
    z-index: var(--z-modal, 100);
    pointer-events: none;
    opacity: 0;
    transition: opacity var(--transition-fast, 0.2s) ease;
  }

  .search-hint.visible {
    opacity: 1;
  }

  :global(.search-hint-icon) {
    margin-right: var(--space-2);
    flex-shrink: 0;
    font-size: 14px;
  }

  /* Suggestions dropdown */
  .search-suggestions {
    position: absolute;
    top: 100%;
    left: 0;
    right: 0;
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-light);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    background: var(--glass-bg, oklch(100% 0 0deg / 95%));
    margin-top: var(--space-2);
    overflow: hidden;
    max-height: 0;
    opacity: 0;
    transition:
      max-height var(--transition-medium),
      opacity var(--transition-medium);
    z-index: var(--z-modal);
  }

  .search-suggestions.show {
    max-height: 400px;
    opacity: 1;
  }

  .suggestion-item {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    padding: var(--space-3) var(--space-4);
    cursor: pointer;
    transition: background var(--transition-fast);
    border-bottom: 1px solid var(--divider-color);
    border: none;
    background: transparent;
    text-align: left;
    color: var(--text-primary);
    font-size: var(--font-base);
    font-family: var(--font-body);
  }

  .suggestion-item:last-child {
    border-bottom: none;
  }

  .suggestion-item:hover {
    background: var(--background-secondary);
  }

  .suggestion-icon {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
  }

  .suggestion-text {
    flex: 1;
    color: var(--text-primary);
    font-weight: var(--font-medium);
  }

  .suggestion-subtitle {
    font-size: var(--font-xs);
    color: var(--text-muted);
    margin-left: auto;
    flex-shrink: 0;
  }

  @media (width <= 1024px) {
    .search-container {
      margin: 0 var(--space-4);
      max-width: 400px;
    }
  }

  @media (max-width: 768px) {
    .search-container {
      display: none;
      position: fixed;
      top: var(--header-height);
      left: 0;
      right: 0;
      margin: 0;
      max-width: none;
      background: var(--surface-color);
      padding: var(--space-4);
      border-bottom: 1px solid var(--divider-color);
      z-index: 200;
      box-shadow: var(--shadow-medium);
      flex-direction: row;
      gap: var(--space-2);
    }

    .search-container.mobile-show {
      display: flex;
    }
  }
</style>
