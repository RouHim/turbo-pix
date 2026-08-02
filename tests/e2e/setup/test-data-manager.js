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
      if (photo.filename) {
        this.photoHashes.set(photo.filename, photo.hash_sha256);
      }
    });

    if (this.photoHashes.size === 0) {
      throw new Error(
        'TestDataManager: no photos matched after fetch — the seed may be broken ' +
          'or the photos API schema changed (expected photo.filename / photo.hash_sha256)'
      );
    }

    return this.photoHashes;
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

  getPhotoHash(fileName) {
    return this.photoHashes.get(fileName);
  }

  async getFirstPhotoHash() {
    if (this.photoHashes.size === 0) {
      await this.fetchTestPhotoHashes();
    }

    const firstEntry = this.photoHashes.entries().next().value;
    if (!firstEntry) {
      throw new Error(
        'TestDataManager: photoHashes is empty — cannot determine the first photo hash'
      );
    }
    return firstEntry[1];
  }
}
