// Search Component

class Search {
  constructor() {
    this.searchInput = utils.$('#search-input');
    this.searchBtn = utils.$('#search-btn');
    this.currentQuery = '';
    this.searchHistory = [];
    this.suggestions = [];

    this.init();
  }

  init() {
    this.loadSearchHistory();
    this.bindEvents();
    this.setupSearchSuggestions();
  }

  bindEvents() {
    if (this.searchInput) {
      // Debounced search as user types
      utils.on(
        this.searchInput,
        'input',
        utils.debounce((e) => {
          const query = e.target.value.trim();
          if (query.length >= 2) {
            this.performSearch(query);
          } else if (query.length === 0 && this.currentQuery) {
            this.clearSearch();
          }
        }, 300)
      );

      // Enter key search
      utils.on(this.searchInput, 'keydown', (e) => {
        if (e.key === 'Enter') {
          e.preventDefault();
          const query = e.target.value.trim();
          if (query) {
            this.performSearch(query, true);
          }
        }
      });

      utils.on(this.searchInput, 'blur', () => {
        // Delay hiding to allow clicking on suggestions
        setTimeout(() => this.hideSearchSuggestions(), 150);
      });
    }

    if (this.searchBtn) {
      utils.on(this.searchBtn, 'click', () => {
        const query = this.searchInput?.value.trim();
        if (query) {
          this.performSearch(query, true);
        }
      });
    }

    // Listen for external search requests
    utils.on(window, 'requestSearch', (e) => {
      const query = e.detail.query;
      this.setSearchQuery(query);
      this.performSearch(query, true);
    });
  }

  setupSearchSuggestions() {
    // Create suggestions container
    if (this.searchInput && !utils.$('#search-suggestions')) {
      const suggestions = utils.createElement('div', 'search-suggestions');
      suggestions.id = 'search-suggestions';
      this.searchInput.parentNode.appendChild(suggestions);
    }
  }

  /**
   * Performs a search with the given query
   * Uses semantic AI search by default
   * @param {string} query - The search query
   * @param {boolean} addToHistory - Whether to add the query to search history
   * @returns {Promise<void>}
   */
  async performSearch(query, addToHistory = false) {
    if (!query || query === this.currentQuery) return;

    this.currentQuery = query;
    this.setSearchQuery(query);

    if (addToHistory) {
      this.addToSearchHistory(query);
    }

    // Update page title and view
    this.updateSearchState(query);

    try {
      if (window.logger) {
        window.logger.info('Performing AI semantic search', {
          component: 'Search',
          query,
          addToHistory,
        });
      }

      // Always use semantic search (AI search is default)
      await this.performSemanticSearch(query);

      // Update URL without page reload
      const url = new URL(window.location);
      if (query) {
        url.searchParams.set('q', query);
      } else {
        url.searchParams.delete('q');
      }
      window.history.replaceState({}, '', url);
    } catch (error) {
      if (window.logger) {
        window.logger.error('Search error', error, {
          component: 'Search',
          query,
          addToHistory,
        });
      } else {
        console.error('Search error:', error);
      }
      utils.handleError(error, 'Search.performSearch');
    }
  }

  async performSemanticSearch(query) {
    // Remove @ prefix if present
    const cleanQuery = query.startsWith('@') ? query.substring(1).trim() : query;

    if (window.logger) {
      window.logger.info('Performing semantic search', {
        component: 'Search',
        query: cleanQuery,
      });
    }

    try {
      const result = await api.semanticSearch(cleanQuery, 50);

      if (window.photoGrid) {
        // Convert semantic search results to photo hashes
        const photoHashes = result.results.map((r) => r.hash);

        // Load full photo data for these hashes
        const photosData = await Promise.all(
          photoHashes.map(async (hash) => {
            try {
              return await api.getPhoto(hash);
            } catch (e) {
              console.warn(`Failed to load photo ${hash}:`, e);
              return null;
            }
          })
        );

        const photos = photosData.filter((p) => p !== null);

        // Display results
        window.photoGrid.displayPhotos(photos);
        window.photoGrid.totalPhotos = photos.length;
        window.photoGrid.hasMore = false;

        if (window.logger) {
          window.logger.info('Semantic search completed', {
            component: 'Search',
            query: cleanQuery,
            resultsCount: photos.length,
          });
        }
      }
    } catch (error) {
      if (window.logger) {
        window.logger.error('Semantic search error', error, {
          component: 'Search',
          query: cleanQuery,
        });
      }
      throw error;
    }
  }

  clearSearch() {
    this.currentQuery = '';
    this.setSearchQuery('');
    this.updateSearchState('');

    if (window.photoGrid) {
      window.photoGrid.search('');
    }

    // Update URL
    const url = new URL(window.location);
    url.searchParams.delete('q');
    window.history.replaceState({}, '', url);
  }

  setSearchQuery(query) {
    if (this.searchInput) {
      this.searchInput.value = query;
    }
  }

  updateSearchState(query) {
    const title = utils.$('#current-view-title');
    if (title) {
      if (query) {
        title.textContent = `Search: "${query}"`;
      } else {
        title.textContent = 'All Photos';
      }
    }

    // Update active nav item
    const navItems = utils.$$('.nav-item');
    navItems.forEach((item) => item.classList.remove('active'));

    if (!query) {
      const allPhotosItem = utils.$('.nav-item[data-view="all"]');
      if (allPhotosItem) allPhotosItem.classList.add('active');
    }
  }

  loadSearchHistory() {
    this.searchHistory = api.getSearchHistory();
  }

  addToSearchHistory(query) {
    api.addToSearchHistory(query);
    this.loadSearchHistory();
  }

  showSearchSuggestions() {
    const suggestionsEl = utils.$('#search-suggestions');
    if (!suggestionsEl) return;

    const currentValue = this.searchInput?.value.trim() || '';
    const suggestions = this.generateSuggestions(currentValue);

    if (suggestions.length === 0) {
      this.hideSearchSuggestions();
      return;
    }

    // Clear existing suggestions (safe - no interpolation)
    suggestionsEl.innerHTML = '';

    // Build suggestions with DOM API to prevent XSS
    suggestions.forEach((suggestion) => {
      const item = utils.createElement('div', 'suggestion-item');
      item.dataset.query = suggestion.query; // Safe - dataset API escapes

      const icon = utils.createElement('span', 'suggestion-icon', suggestion.icon);
      const text = utils.createElement('span', 'suggestion-text', suggestion.text);

      item.appendChild(icon);
      item.appendChild(text);

      if (suggestion.subtitle) {
        const subtitle = utils.createElement('span', 'suggestion-subtitle', suggestion.subtitle);
        item.appendChild(subtitle);
      }

      // Bind click event
      utils.on(item, 'click', () => {
        this.performSearch(suggestion.query, true);
        this.hideSearchSuggestions();
      });

      suggestionsEl.appendChild(item);
    });

    suggestionsEl.classList.add('show');
  }

  hideSearchSuggestions() {
    const suggestionsEl = utils.$('#search-suggestions');
    if (suggestionsEl) {
      suggestionsEl.classList.remove('show');
    }
  }

  generateSuggestions(currentValue) {
    const suggestions = [];

    // Recent searches
    if (this.searchHistory.length > 0) {
      const recentMatches = this.searchHistory
        .filter(
          (item) => !currentValue || item.query.toLowerCase().includes(currentValue.toLowerCase())
        )
        .slice(0, 3)
        .map((item) => ({
          query: item.query,
          text: item.query,
          icon: '🕒',
          subtitle: 'Recent search',
        }));
      suggestions.push(...recentMatches);
    }

    // Search suggestions based on current value
    if (currentValue.length >= 2) {
      const searchSuggestions = this.getSearchSuggestions(currentValue);
      suggestions.push(...searchSuggestions);
    }

    // Quick filters
    if (!currentValue) {
      suggestions.push(
        { query: 'camera:canon', text: 'Canon photos', icon: '📷', subtitle: 'Filter by camera' },
        { query: 'camera:nikon', text: 'Nikon photos', icon: '📷', subtitle: 'Filter by camera' },
        { query: 'has:gps', text: 'Photos with location', icon: '📍', subtitle: 'Has GPS data' },
        { query: 'type:video', text: 'Videos only', icon: '🎥', subtitle: 'Filter by type' }
      );
    }

    return suggestions.slice(0, 8); // Limit to 8 suggestions
  }

  getSearchSuggestions(value) {
    const suggestions = [];
    const lowerValue = value.toLowerCase();

    // Camera suggestions
    if (lowerValue.includes('canon') || lowerValue.includes('camera')) {
      suggestions.push({ query: 'camera:canon', text: 'Canon cameras', icon: '📷' });
    }
    if (lowerValue.includes('nikon') || lowerValue.includes('camera')) {
      suggestions.push({ query: 'camera:nikon', text: 'Nikon cameras', icon: '📷' });
    }
    if (lowerValue.includes('sony') || lowerValue.includes('camera')) {
      suggestions.push({ query: 'camera:sony', text: 'Sony cameras', icon: '📷' });
    }

    // Date suggestions
    if (lowerValue.includes('2024') || lowerValue.includes('today')) {
      suggestions.push({ query: 'date:2024', text: 'Photos from 2024', icon: '📅' });
    }
    if (lowerValue.includes('2023')) {
      suggestions.push({ query: 'date:2023', text: 'Photos from 2023', icon: '📅' });
    }

    // Type suggestions
    if (lowerValue.includes('video')) {
      suggestions.push({ query: 'type:video', text: 'Videos only', icon: '🎥' });
    }
    if (lowerValue.includes('raw')) {
      suggestions.push({ query: 'type:raw', text: 'RAW files only', icon: '📸' });
    }

    // Location suggestions
    if (lowerValue.includes('gps') || lowerValue.includes('location')) {
      suggestions.push({ query: 'has:gps', text: 'Photos with GPS', icon: '📍' });
    }

    return suggestions;
  }

  // Advanced search methods
  parseSearchQuery(query) {
    const filters = {
      text: '',
      camera: null,
      date: null,
      type: null,
      hasGps: null,
    };

    // Split query into terms
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

  buildSearchFilters(query) {
    const parsed = this.parseSearchQuery(query);
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
      // Simple date filtering - can be enhanced
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

  // Public API
  getSearchQuery() {
    return this.currentQuery;
  }

  focus() {
    if (this.searchInput) {
      this.searchInput.focus();
    }
  }

  clear() {
    this.clearSearch();
  }
}

// Initialize global search when DOM is ready
document.addEventListener('DOMContentLoaded', () => {
  window.search = new Search();

  // Handle initial search from URL
  const urlParams = new URLSearchParams(window.location.search);
  const initialQuery = urlParams.get('q');
  if (initialQuery) {
    window.search.setSearchQuery(initialQuery);
    window.search.performSearch(initialQuery, false);
  }
});
