export class TestDataManager {
  constructor(baseURL = 'http://localhost:18473') {
    this.baseURL = baseURL;
    this.photoHashes = new Map();
  }

  async fetchAllPhotos() {
    const response = await fetch(`${this.baseURL}/api/photos?limit=100`);
    if (!response.ok) {
      throw new Error(`Failed to fetch photos: ${response.statusText}`);
    }
    const data = await response.json();
    return data.photos || [];
  }

  async fetchTestPhotoHashes() {
    const photos = await this.fetchAllPhotos();

    photos.forEach((photo) => {
      if (photo.file_name) {
        this.photoHashes.set(photo.file_name, photo.hash_sha256);
      }
    });

    return this.photoHashes;
  }

  async printTestPhotoHashes() {
    const hashes = await this.fetchTestPhotoHashes();

    console.log('\n=== Test Photo Hashes ===\n');
    hashes.forEach((hash, fileName) => {
      console.log(`${fileName}: ${hash}`);
    });
    console.log('\n========================\n');
  }

  async addToFavorites(photoHash) {
    const response = await fetch(`${this.baseURL}/api/photos/${photoHash}/favorite`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ is_favorite: true }),
    });

    if (!response.ok) {
      throw new Error(`Failed to add photo to favorites: ${response.statusText}`);
    }

    return await response.json();
  }

  async removeFromFavorites(photoHash) {
    const response = await fetch(`${this.baseURL}/api/photos/${photoHash}/favorite`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ is_favorite: false }),
    });

    if (!response.ok) {
      throw new Error(`Failed to remove photo from favorites: ${response.statusText}`);
    }

    return await response.json();
  }

  async seedFavorites(photoHashes) {
    console.log(`Seeding ${photoHashes.length} favorites...`);

    const promises = photoHashes.map((hash) => this.addToFavorites(hash));
    await Promise.all(promises);

    console.log('Favorites seeded successfully');
  }

  async clearAllFavorites() {
    console.log('Clearing all favorites...');

    const photos = await this.fetchAllPhotos();
    const favoritePhotos = photos.filter((photo) => photo.is_favorite);

    const promises = favoritePhotos.map((photo) => this.removeFromFavorites(photo.hash_sha256));

    await Promise.all(promises);

    console.log(`Cleared ${favoritePhotos.length} favorites`);
  }

  async deletePhoto(photoHash) {
    const response = await fetch(`${this.baseURL}/api/photos/${photoHash}`, {
      method: 'DELETE',
    });

    if (!response.ok) {
      throw new Error(`Failed to delete photo: ${response.statusText}`);
    }

    return await response.json();
  }

  async getIndexingStatus() {
    const response = await fetch(`${this.baseURL}/api/indexing/status`);

    if (!response.ok) {
      throw new Error(`Failed to get indexing status: ${response.statusText}`);
    }

    return await response.json();
  }

  async waitForIndexing(maxRetries = 180, delayMs = 1000) {
    console.log('Waiting for indexing to complete...');

    for (let i = 0; i < maxRetries; i++) {
      try {
        const status = await this.getIndexingStatus();

        if (status.is_complete) {
          console.log('Indexing complete');
          return true;
        }

        if (i % 10 === 0) {
          const progress = Math.round(status.progress * 100);
          console.log(`Indexing progress: ${progress}%`);
        }
      } catch (error) {
        console.error('Error checking indexing status:', error.message);
      }

      await new Promise((resolve) => setTimeout(resolve, delayMs));
    }

    throw new Error('Indexing did not complete within timeout');
  }

  getPhotoHash(fileName) {
    return this.photoHashes.get(fileName);
  }

  async getFirstPhotoHash() {
    if (this.photoHashes.size === 0) {
      await this.fetchTestPhotoHashes();
    }

    const firstEntry = this.photoHashes.entries().next().value;
    return firstEntry ? firstEntry[1] : null;
  }

  async getFirstVideoHash() {
    const photos = await this.fetchAllPhotos();
    const video = photos.find((photo) => photo.mime_type?.startsWith('video/'));
    return video?.hash_sha256 || null;
  }
}
