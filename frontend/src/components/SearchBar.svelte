<script>
  import { get } from 'svelte/store';
  import { t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import { route, pushState } from '../lib/router.svelte.js';
  import { appState, photoGridState } from '../lib/state.svelte.js';

  const MAX_HISTORY = 20;

  let query = $state('');
  let searchHistory = $state([]);
  let showSuggestions = $state(false);
  let suggestions = $state([]);
  let searching = $state(false);
  let inputEl = $state(null);
  let focused = $state(false);
  let currentQuery = $state('');

  $effect(() => {
    searchHistory = api.getSearchHistory() || [];
  });

  // Sync query input from route on popstate / initial load
  $effect(() => {
    if (route && route.query && route.query !== currentQuery) {
      query = route.query;
      currentQuery = route.query;
    } else if (route && !route.query && currentQuery) {
      query = '';
      currentQuery = '';
    }
  });

  // Listen for external search requests
  $effect(() => {
    function onRequestSearch(e) {
      const q = e.detail?.query;
      if (q) {
        query = q;
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

    if (!q || q === currentQuery) return;

    currentQuery = q;
    query = q;

    if (addToHistory) {
      api.addToSearchHistory(q);
      searchHistory = api.getSearchHistory() || [];
    }

    searching = true;

    try {
      // Prefix queries (type:, location:, is_favorite:) use regular search path
      if (
        q.startsWith('type:') ||
        q.startsWith('location:') ||
        q.startsWith('is_favorite:')
      ) {
        photoGridState.semanticSearchMode = false;
        photoGridState.currentQuery = q;
        photoGridState.currentPage = 1;
        appState.searchQuery = q;
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
    appState.searchQuery = cleanQuery;

    searching = false;
  }

  function clearSearch(updateUrl = true) {
    const q = query.trim();
    query = '';
    currentQuery = '';
    showSuggestions = false;
    searching = false;
    photoGridState.semanticSearchMode = false;
    photoGridState.currentQuery = '';
    photoGridState.currentPage = 1;
    appState.searchQuery = '';

    if (updateUrl) {
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
    performSearch(q, true);
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
    // Generate suggestions as user types (not performing search on every input)
    generateSuggestions(q);
    showSuggestions = suggestions.length > 0;
  }

  function selectSuggestion(s) {
    query = s.query;
    performSearch(s.query, true);
    showSuggestions = false;
  }

  function generateSuggestions(currentValue) {
    const items = [];
    const _t = (key, fallback) => get(t)(key, fallback) || fallback;
    const history = searchHistory || [];

    // Recent searches
    if (history.length > 0) {
      const recentMatches = history
        .filter(
          (item) =>
            !currentValue ||
            item.query.toLowerCase().includes(currentValue.toLowerCase())
        )
        .slice(0, 3)
        .map((item) => ({
          query: item.query,
          text: item.query,
          icon: '\u{1F552}', // 🕒
          subtitle: get(t)('ui.recent_search', 'Recent search'),
        }));
      items.push(...recentMatches);
    }

    // Dynamic search suggestions based on current value
    if (currentValue.length >= 2) {
      const dynamicSuggestions = getSearchSuggestions(currentValue);
      items.push(...dynamicSuggestions);
    }

    // Quick filters (when empty)
    if (!currentValue) {
      items.push(
        {
          query: 'camera:canon',
          text: get(t)('ui.canon_photos', 'Canon photos'),
          icon: '\u{1F4F7}', // 📷
          subtitle: get(t)('ui.filter_by_camera', 'Filter by camera'),
        },
        {
          query: 'camera:nikon',
          text: get(t)('ui.nikon_photos', 'Nikon photos'),
          icon: '\u{1F4F7}',
          subtitle: get(t)('ui.filter_by_camera', 'Filter by camera'),
        },
        {
          query: 'has:gps',
          text: get(t)('ui.photos_with_location', 'Photos with location'),
          icon: '\u{1F4CD}', // 📍
          subtitle: get(t)('ui.has_gps_data', 'Has GPS data'),
        },
        {
          query: 'type:video',
          text: get(t)('ui.videos_only', 'Videos only'),
          icon: '\u{1F3A5}', // 🎥
          subtitle: get(t)('ui.filter_by_type', 'Filter by type'),
        }
      );
    }

    suggestions = items.slice(0, 8);
  }

  function getSearchSuggestions(value) {
    const items = [];
    const lowerValue = value.toLowerCase();
    const _t = (key, fallback) => get(t)(key, fallback) || fallback;

    // Camera suggestions
    if (lowerValue.includes('canon') || lowerValue.includes('camera')) {
      items.push({
        query: 'camera:canon',
        text: get(t)('ui.canon_photos', 'Canon photos'),
        icon: '\u{1F4F7}',
      });
    }
    if (lowerValue.includes('nikon') || lowerValue.includes('camera')) {
      items.push({
        query: 'camera:nikon',
        text: get(t)('ui.nikon_photos', 'Nikon photos'),
        icon: '\u{1F4F7}',
      });
    }
    if (lowerValue.includes('sony') || lowerValue.includes('camera')) {
      items.push({
        query: 'camera:sony',
        text: get(t)('ui.sony_photos', 'Sony photos'),
        icon: '\u{1F4F7}',
      });
    }

    // Date suggestions
    if (lowerValue.includes('2024') || lowerValue.includes('today')) {
      items.push({
        query: 'date:2024',
        text: get(t)('ui.photos_from_year', { year: '2024' }) || '2024 photos',
        icon: '\u{1F4C5}', // 📅
      });
    }
    if (lowerValue.includes('2023')) {
      items.push({
        query: 'date:2023',
        text: get(t)('ui.photos_from_year', { year: '2023' }) || '2023 photos',
        icon: '\u{1F4C5}',
      });
    }

    // Type suggestions
    if (lowerValue.includes('video')) {
      items.push({
        query: 'type:video',
        text: get(t)('ui.videos_only', 'Videos only'),
        icon: '\u{1F3A5}',
      });
    }
    if (lowerValue.includes('raw')) {
      items.push({
        query: 'type:raw',
        text: get(t)('ui.raw_files_only', 'RAW files only'),
        icon: '\u{1F4F8}', // 📸
      });
    }

    // Location suggestions
    if (lowerValue.includes('gps') || lowerValue.includes('location')) {
      items.push({
        query: 'has:gps',
        text: get(t)('ui.photos_with_gps', 'Photos with GPS'),
        icon: '\u{1F4CD}',
      });
    }

    return items;
  }

  /**
   * Parse special query terms into structured filters.
   * Used by the regular search path (non-semantic).
   */
  export function parseSearchQuery(query) {
    const filters = {
      text: '',
      camera: null,
      date: null,
      type: null,
      hasGps: null,
    };

    const terms = query.match(/(?:[^\s"]+|"[^"]*")+/g) || [];

    terms.forEach((term) => {
      if (term.startsWith('camera:')) {
        filters.camera = term.substring(7).replace(/"/g, '');
      } else if (term.startsWith('date:')) {
        filters.date = term.substring(5).replace(/"/g, '');
      } else if (term.startsWith('type:')) {
        filters.type = term.substring(5).replace(/"/g, '');
      } else if (term === 'has:gps') {
        filters.hasGps = true;
      } else {
        filters.text += (filters.text ? ' ' : '') + term.replace(/"/g, '');
      }
    });

    return filters;
  }

  /**
   * Convert parsed filters into API-ready filter object.
   * Used by the regular search path (non-semantic).
   */
  export function buildSearchFilters(query) {
    const parsed = parseSearchQuery(query);
    const filters = {};

    if (parsed.camera) {
      const parts = parsed.camera.split(/[\s-]+/);
      if (parts.length >= 2) {
        filters.cameraMake = parts[0];
        filters.cameraModel = parts.slice(1).join(' ');
      } else {
        filters.cameraMake = parts[0];
      }
    }

    if (parsed.date) {
      if (parsed.date.match(/^\d{4}$/)) {
        filters.dateFrom = `${parsed.date}-01-01`;
        filters.dateTo = `${parsed.date}-12-31`;
      }
    }

    if (parsed.hasGps !== null) {
      filters.hasGps = parsed.hasGps;
    }

    return { query: parsed.text.trim(), filters };
  }
</script>

<div class="search-container" class:mobile-show={appState.mobileSearchOpen}>
  <input
    type="text"
    id="search-input"
    class="search-input"
    placeholder={$t('ui.search_ai_placeholder', { default: 'AI-powered photo search...' })}
    bind:value={query}
    bind:this={inputEl}
    onkeydown={onKeydown}
    onfocus={onFocus}
    oninput={onInput}
    onblur={() => { focused = false; setTimeout(() => (showSuggestions = false), 150); }}
  />
  <button
    type="button"
    id="search-btn"
    class="search-btn"
    class:searching
    onclick={submitSearch}
  >
    {searching ? '' : $t('ui.search', { default: 'Search' })}
  </button>

  <!-- Search hint -->
  <div class="search-hint" class:visible={focused && !query}>
    <span class="search-hint-icon">ⓘ</span>
    <span>{$t('ui.search_hint', { default: 'Try: type:video \u00b7 location:city \u00b7 is_favorite:true' })}</span>
  </div>

  {#if showSuggestions}
    <div id="search-suggestions" class="search-suggestions show">
      {#each suggestions as s (s.query + s.icon)}
        <button type="button" class="suggestion-item" onclick={() => selectSuggestion(s)}>
          <span class="suggestion-icon">{s.icon}</span>
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

  .search-hint-icon {
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
    font-size: var(--font-xl);
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

  @media (max-width: 768px) {
    .search-container {
      display: none;
      position: fixed;
      top: var(--header-height);
      left: 0;
      right: 0;
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
