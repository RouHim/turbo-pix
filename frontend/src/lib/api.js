// API client for TurboPix
import { performance, storage } from './utils.js';
import { logger } from './logger.js';

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
      ...options,
      headers: { ...this.defaultHeaders, ...options.headers },
    };

    try {
      // Stable name: endpoints carry varying query strings (pagination, search,
      // thumbnail sizes); using them verbatim would create unbounded
      // mark/measure names.
      const name = endpoint.split('?')[0];

      performance.mark(`api-start-${name}`);

      const response = await fetch(url, config);

      performance.mark(`api-end-${name}`);
      performance.measure(`api-${name}`, `api-start-${name}`, `api-end-${name}`);

      if (!response.ok) {
        const errorText = await response.text();
        if (logger) {
          logger.warn(`API request failed for ${endpoint}`, {
            component: 'API',
            endpoint,
            status: response.status,
            statusText: response.statusText,
          });
        }
        throw new Error(`HTTP ${response.status}: ${errorText || response.statusText}`);
      }

      if (logger) {
        logger.debug(`API request successful for ${endpoint}`, {
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
      if (error?.name === 'AbortError') throw error;
      if (logger) {
        logger.error(`API Error for ${endpoint}`, error, {
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

  async getPhoto(hash, options = {}) {
    return this.request(`/api/photos/${hash}`, options);
  }

  /**
   * Ask the server for the recommended playback action for a video.
   * Uses a raw fetch (not `request`) because a 202 is a valid "transcoding
   * started" response here, not an error. The server echoes the capability
   * string back as the header value it made its decision with.
   * @param {string} hash - photo hash
   * @param {string} clientCodecs - e.g. 'h264-8,h264-10' (from getClientCodecsString)
   * @returns {Promise<{action: string, url?: string, reason?: string, pollUrl?: string}>}
   */
  async getVideoDecision(hash, clientCodecs) {
    try {
      const res = await fetch(
        `/api/photos/${hash}/video?decision&client=${encodeURIComponent(clientCodecs)}`
      );
      if (res.status === 202) {
        const data = await res.json();
        return { action: 'transcode', pollUrl: data.poll_url };
      }
      if (!res.ok) return { action: 'error', reason: `HTTP ${res.status}` };
      const data = await res.json();
      return data;
    } catch (e) {
      if (logger) {
        logger.warn('getVideoDecision failed', e, { component: 'API', hash });
      }
      return { action: 'error', reason: String(e) };
    }
  }

  async getConfig() {
    return this.request('/api/config');
  }

  /**
   * Performs semantic search using AI/ML embeddings
   * @param {string} query - Natural language search query
   * @param {number} limit - Maximum number of results
   * @param {number} offset - Number of results to skip (for pagination)
   * @returns {Promise<Object>} Search results with photo hashes and scores
   */
  async semanticSearch(query, limit = 50, offset = 0, options = {}) {
    const searchParams = new URLSearchParams();
    searchParams.set('q', query);
    searchParams.set('limit', limit);
    searchParams.set('offset', offset);
    const endpoint = `/api/search/semantic?${searchParams.toString()}`;
    return this.request(endpoint, options);
  }

  // Health check
  async healthCheck() {
    return this.request('/health');
  }

  // Indexing status
  async getIndexingStatus() {
    return this.request('/api/indexing/status', {
      cache: 'no-store',
      headers: {
        'Cache-Control': 'no-cache',
        Pragma: 'no-cache',
      },
    });
  }

  // Collections and cameras features removed

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

  // View settings (stored locally)
  getViewSettings() {
    return storage.get('viewSettings', {
      gridSize: 'medium',
      sortBy: 'date_desc',
      showMetadata: true,
      autoPlay: false,
    });
  }

  // Search history
  getSearchHistory() {
    return storage.get('searchHistory', []);
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
    storage.set('searchHistory', trimmed);
  }

  // Collages
  async getPendingCollages(options = {}) {
    return this.request('/api/collages/pending', options);
  }

  async generateCollages(options = {}) {
    return this.request('/api/collages/generate', { method: 'POST', ...options });
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
  async getHousekeepingCandidates(options = {}) {
    return this.request('/api/housekeeping/candidates', options);
  }

  async removeHousekeepingCandidate(hash) {
    return this.request(`/api/housekeeping/candidates/${hash}`, {
      method: 'DELETE',
    });
  }

  // Saved searches
  async getSavedSearches() {
    return this.request('/api/saved-searches');
  }

  async createSavedSearch(payload) {
    // Uses _json: the duplicate case (409) must surface the existing entry.
    return this._json('/api/saved-searches', 'POST', payload);
  }

  async renameSavedSearch(id, name) {
    return this.request(`/api/saved-searches/${id}`, {
      method: 'PATCH',
      body: JSON.stringify({ name }),
    });
  }

  async deleteSavedSearch(id) {
    return this.request(`/api/saved-searches/${id}`, { method: 'DELETE' });
  }

  // Event albums
  async getEventAlbums() {
    return this.request('/api/event-albums');
  }

  async createEventAlbum(payload) {
    return this.request('/api/event-albums', {
      method: 'POST',
      body: JSON.stringify(payload),
    });
  }

  async updateEventAlbum(id, payload) {
    return this.request(`/api/event-albums/${id}`, {
      method: 'PATCH',
      body: JSON.stringify(payload),
    });
  }

  async deleteEventAlbum(id) {
    return this.request(`/api/event-albums/${id}`, { method: 'DELETE' });
  }

  async getEventAlbumPhotos(id, params = {}, options = {}) {
    const searchParams = new URLSearchParams();
    if (params.page !== undefined) searchParams.set('page', params.page);
    if (params.limit !== undefined) searchParams.set('limit', params.limit);
    if (params.sort) searchParams.set('sort', params.sort);
    if (params.order) searchParams.set('order', params.order);
    const qs = searchParams.toString();
    return this.request(`/api/event-albums/${id}/photos${qs ? `?${qs}` : ''}`, options);
  }

  // ===========================================================================
  // Batch selection actions
  // ===========================================================================

  async batchDelete(hashes) {
    return this.request('/api/photos/batch/delete', {
      method: 'POST',
      body: JSON.stringify({ hashes }),
    });
  }

  async batchSetFavorite(hashes, isFavorite) {
    return this.request('/api/photos/batch/favorite', {
      method: 'POST',
      body: JSON.stringify({ hashes, is_favorite: isFavorite }),
    });
  }

  async batchDateShift(hashes, days) {
    return this.request('/api/photos/batch/date-shift', {
      method: 'POST',
      body: JSON.stringify({ hashes, days }),
    });
  }

  /**
   * Downloads the export archive as a blob. Bypasses `request` (which would
   * `.text()` the body): the caller needs the raw bytes plus the
   * Content-Disposition filename. A 400 (some photos unexportable) carries a
   * JSON body with `error` and `failed` — surfaced as a plain Error message.
   */
  async batchExport(hashes) {
    const response = await fetch(`${this.baseUrl}/api/photos/batch/export`, {
      method: 'POST',
      headers: this.defaultHeaders,
      body: JSON.stringify({ hashes }),
    });
    if (!response.ok) {
      let message = `HTTP ${response.status}`;
      try {
        const err = await response.json();
        message = err?.error || message;
      } catch {
        // non-JSON error body
      }
      throw new Error(message);
    }
    const disposition = response.headers.get('content-disposition') || '';
    const match = /filename="?([^";]+)"?/.exec(disposition);
    const blob = await response.blob();
    return { blob, filename: match?.[1] || 'turbopix-export.zip' };
  }

  async batchRemoveHousekeepingCandidates(hashes) {
    return this.request('/api/housekeeping/candidates/batch-remove', {
      method: 'POST',
      body: JSON.stringify({ hashes }),
    });
  }

  async batchAcceptCollages(ids) {
    return this.request('/api/collages/batch-accept', {
      method: 'POST',
      body: JSON.stringify({ ids }),
    });
  }

  async batchRejectCollages(ids) {
    return this.request('/api/collages/batch-reject', {
      method: 'POST',
      body: JSON.stringify({ ids }),
    });
  }

  /** Like request() but resolves parsed JSON and rejects with an Error carrying `.status` and `.data`. */
  async _json(endpoint, method, payload) {
    const response = await fetch(`${this.baseUrl}${endpoint}`, {
      method,
      headers: this.defaultHeaders,
      body: JSON.stringify(payload),
    });
    const text = await response.text();
    let data = null;
    if (text) {
      try {
        data = JSON.parse(text);
      } catch {
        data = text;
      }
    }
    if (!response.ok) {
      const error = new Error(data?.error || `HTTP ${response.status}`);
      error.status = response.status;
      error.data = data;
      throw error;
    }
    return data;
  }
}

export const api = new TurboPixAPI();
