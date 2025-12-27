// API client for TurboPix

class TurboPixAPI {
  constructor(baseUrl = '') {
    this.baseUrl = baseUrl;
    this.defaultHeaders = {
      'Content-Type': 'application/json',
      Accept: 'application/json',
    };
  }

  /**
   * Makes an HTTP request to the API
   * @param {string} endpoint - The API endpoint
   * @param {Object} options - Fetch options (method, headers, body, signal, etc.)
   * @returns {Promise<any>} The response data
   * @throws {Error} If the request fails
   */
  async request(endpoint, options = {}) {
    const url = `${this.baseUrl}${endpoint}`;
    const config = {
      headers: { ...this.defaultHeaders, ...options.headers },
      ...options,
    };

    try {
      utils.performance.mark(`api-start-${endpoint}`);

      const response = await fetch(url, config);

      utils.performance.mark(`api-end-${endpoint}`);
      utils.performance.measure(`api-${endpoint}`, `api-start-${endpoint}`, `api-end-${endpoint}`);

      if (!response.ok) {
        const errorText = await response.text();
        if (window.logger) {
          window.logger.warn(`API request failed for ${endpoint}`, {
            component: 'API',
            endpoint,
            status: response.status,
            statusText: response.statusText,
          });
        }
        throw new Error(`HTTP ${response.status}: ${errorText || response.statusText}`);
      }

      if (window.logger) {
        window.logger.debug(`API request successful for ${endpoint}`, {
          component: 'API',
          endpoint,
          status: response.status,
        });
      }

      const contentType = response.headers.get('content-type');
      if (contentType && contentType.includes('application/json')) {
        return await response.json();
      }

      return await response.text();
    } catch (error) {
      if (window.logger) {
        window.logger.error(`API Error for ${endpoint}`, error, {
          component: 'API',
          endpoint,
          method: config.method || 'GET',
        });
      } else {
        console.error(`API Error for ${endpoint}:`, error);
      }
      throw error;
    }
  }

  /**
   * Retrieves photos with optional filtering and pagination
   * @param {Object} params - Query parameters (page, limit, query, sort, order, etc.)
   * @param {Object} options - Fetch options (signal for AbortController, etc.)
   * @returns {Promise<Object>} Response containing photos array and metadata
   */
  async getPhotos(params = {}, options = {}) {
    const searchParams = new URLSearchParams();

    // Add parameters
    if (params.page !== undefined) searchParams.set('page', params.page);
    if (params.limit !== undefined) searchParams.set('limit', params.limit);
    if (params.query) searchParams.set('q', params.query);
    if (params.sort) searchParams.set('sort', params.sort);
    if (params.order) searchParams.set('order', params.order);
    if (params.dateFrom) searchParams.set('date_from', params.dateFrom);
    if (params.dateTo) searchParams.set('date_to', params.dateTo);
    if (params.cameraMake) searchParams.set('camera_make', params.cameraMake);
    if (params.cameraModel) searchParams.set('camera_model', params.cameraModel);
    if (params.hasGps !== undefined) searchParams.set('has_gps', params.hasGps);
    if (params.year !== undefined) searchParams.set('year', params.year);
    if (params.month !== undefined) searchParams.set('month', params.month);

    const queryString = searchParams.toString();
    const endpoint = `/api/photos${queryString ? `?${queryString}` : ''}`;

    return this.request(endpoint, options);
  }

  async getPhoto(hash) {
    return this.request(`/api/photos/${hash}`);
  }

  async getPhotoThumbnail(hash, size = 'medium') {
    const response = await fetch(`/api/photos/${hash}/thumbnail?size=${size}`);
    if (!response.ok) {
      throw new Error(`Failed to load thumbnail: ${response.statusText}`);
    }
    return response.blob();
  }

  async getConfig() {
    return this.request('/api/config');
  }

  async searchPhotos(query, params = {}) {
    return this.getPhotos({ ...params, query });
  }

  /**
   * Performs semantic search using AI/ML embeddings
   * @param {string} query - Natural language search query
   * @param {number} limit - Maximum number of results
   * @param {number} offset - Number of results to skip (for pagination)
   * @returns {Promise<Object>} Search results with photo hashes and scores
   */
  async semanticSearch(query, limit = 50, offset = 0) {
    const searchParams = new URLSearchParams();
    searchParams.set('q', query);
    searchParams.set('limit', limit);
    searchParams.set('offset', offset);
    const endpoint = `/api/search/semantic?${searchParams.toString()}`;
    return this.request(endpoint);
  }

  // Health check
  async healthCheck() {
    return this.request('/health');
  }

  // Indexing status
  async getIndexingStatus() {
    return this.request('/api/indexing/status');
  }

  // Collections and cameras features removed

  // Batch operations
  async batchRequest(requests) {
    const promises = requests.map(({ endpoint, options }) =>
      this.request(endpoint, options).catch((error) => ({ error: error.message }))
    );

    return Promise.all(promises);
  }

  // Helper methods for common operations
  async getRecentPhotos(limit = 50) {
    return this.getPhotos({
      limit,
      sort: 'date_indexed',
      order: 'desc',
    });
  }

  async getPhotosWithGPS(params = {}) {
    return this.getPhotos({
      ...params,
      hasGps: true,
    });
  }

  async getPhotosInDateRange(startDate, endDate, params = {}) {
    return this.getPhotos({
      ...params,
      dateFrom: startDate,
      dateTo: endDate,
    });
  }

  // Favorites (using backend API)
  async toggleFavorite(photoHash, isFavorite) {
    return this.request(`/api/photos/${photoHash}/favorite`, {
      method: 'PUT',
      body: JSON.stringify({ is_favorite: isFavorite }),
    });
  }

  async addToFavorites(photoHash) {
    try {
      const result = await this.toggleFavorite(photoHash, true);
      return result;
    } catch (error) {
      console.error('Error adding to favorites:', error);
      throw error;
    }
  }

  async removeFromFavorites(photoHash) {
    try {
      const result = await this.toggleFavorite(photoHash, false);
      return result;
    } catch (error) {
      console.error('Error removing from favorites:', error);
      throw error;
    }
  }

  // Metadata editing
  async updatePhotoMetadata(photoHash, updates) {
    return this.request(`/api/photos/${photoHash}/metadata`, {
      method: 'PATCH',
      body: JSON.stringify(updates),
    });
  }

  // Image editing
  async rotatePhoto(photoHash, angle) {
    return this.request(`/api/photos/${photoHash}/rotate`, {
      method: 'POST',
      body: JSON.stringify({ angle }),
    });
  }

  async deletePhoto(photoHash) {
    return this.request(`/api/photos/${photoHash}`, {
      method: 'DELETE',
    });
  }

  isFavorite(photo) {
    // Check if photo has is_favorite field from backend
    if (typeof photo === 'object' && photo.is_favorite !== undefined) {
      return photo.is_favorite;
    }
    // If passed a hash string, we can't determine favorite status locally
    if (typeof photo === 'string') {
      console.warn('isFavorite called with photo hash instead of photo object');
      return false;
    }
    return false;
  }

  async getFavoritePhotos(params = {}) {
    return this.getPhotos({
      ...params,
      query: 'is_favorite:true',
    });
  }

  // View settings (stored locally)
  getViewSettings() {
    return utils.storage.get('viewSettings', {
      gridSize: 'medium',
      sortBy: 'date_desc',
      showMetadata: true,
      autoPlay: false,
    });
  }

  setViewSettings(settings) {
    const current = this.getViewSettings();
    const updated = { ...current, ...settings };
    utils.storage.set('viewSettings', updated);
    return updated;
  }

  // Search history
  getSearchHistory() {
    return utils.storage.get('searchHistory', []);
  }

  addToSearchHistory(query) {
    if (!query || query.trim().length < 2) return;

    const history = this.getSearchHistory();
    const normalizedQuery = query.trim().toLowerCase();

    // Remove if already exists
    const filtered = history.filter((item) => item.query.toLowerCase() !== normalizedQuery);

    // Add to beginning
    filtered.unshift({
      query: query.trim(),
      timestamp: new Date().toISOString(),
    });

    // Keep only last 20 searches
    const trimmed = filtered.slice(0, 20);
    utils.storage.set('searchHistory', trimmed);
  }

  clearSearchHistory() {
    utils.storage.remove('searchHistory');
  }

  // Collages
  async getPendingCollages() {
    return this.request('/api/collages/pending');
  }

  async generateCollages() {
    return this.request('/api/collages/generate', {
      method: 'POST',
    });
  }

  async acceptCollage(collageId) {
    return this.request(`/api/collages/${collageId}/accept`, {
      method: 'POST',
    });
  }

  async rejectCollage(collageId) {
    return this.request(`/api/collages/${collageId}/reject`, {
      method: 'DELETE',
    });
  }

  // Housekeeping
  async getHousekeepingCandidates() {
    return this.request('/api/housekeeping/candidates');
  }

  async removeHousekeepingCandidate(hash) {
    return this.request(`/api/housekeeping/candidates/${hash}`, {
      method: 'DELETE',
    });
  }
}

// Create global API instance
window.api = new TurboPixAPI();
