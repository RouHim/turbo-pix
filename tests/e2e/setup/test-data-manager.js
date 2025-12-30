/**
 * Test data management utilities
 * Provides helpers for working with test photos and their metadata
 */
export class TestDataManager {
  /**
   * Map of test file names to their SHA256 hashes
   * These hashes will be populated after the first indexing run
   *
   * To populate these hashes:
   * 1. Run the tests once (they will index the photos)
   * 2. Query the API: GET http://localhost:18473/api/photos
   * 3. Match filenames to hashes
   * 4. Update this mapping
   */
  static testPhotos = {
    // TODO: Populate these hashes after first test run
    cat: null, // cat.jpg
    car: null, // car.jpg
    img_9377: null, // IMG_9377.jpg
    img_9899_cr2: null, // IMG_9899.CR2
    video_large: null, // PXL_20251018_124956882.mp4
    sample_with_exif: null, // sample_with_exif.jpg
    test_image_1: null, // test_image_1.jpg
    test_image_3: null, // test_image_3.jpg
    test_video: null, // test_video.mp4
    test_video_2: null, // test_video_2.mp4
  };

  /**
   * Get test photo hash by name
   * Throws an error if hash is not yet populated
   */
  static getPhotoHash(name) {
    const hash = this.testPhotos[name];
    if (!hash) {
      throw new Error(
        `Photo hash for '${name}' not yet populated. Run tests once to populate hashes.`
      );
    }
    return hash;
  }

  /**
   * Fetch all photos from the API and return them
   */
  static async fetchAllPhotos(baseURL = 'http://localhost:18473') {
    const response = await fetch(`${baseURL}/api/photos?limit=100`);
    if (!response.ok) {
      throw new Error(`Failed to fetch photos: ${response.status} ${response.statusText}`);
    }
    const data = await response.json();
    return data.photos || [];
  }

  /**
   * Fetch hashes for test photos from the API
   * This can be used to auto-populate the testPhotos mapping
   */
  static async fetchTestPhotoHashes(baseURL = 'http://localhost:18473') {
    const photos = await this.fetchAllPhotos(baseURL);

    const hashMap = {};
    for (const photo of photos) {
      const filename = photo.filename.toLowerCase();

      if (filename === 'cat.jpg') {
        hashMap.cat = photo.hash_sha256;
      } else if (filename === 'car.jpg') {
        hashMap.car = photo.hash_sha256;
      } else if (filename === 'img_9377.jpg') {
        hashMap.img_9377 = photo.hash_sha256;
      } else if (filename === 'img_9899.cr2') {
        hashMap.img_9899_cr2 = photo.hash_sha256;
      } else if (filename === 'pxl_20251018_124956882.mp4') {
        hashMap.video_large = photo.hash_sha256;
      } else if (filename === 'sample_with_exif.jpg') {
        hashMap.sample_with_exif = photo.hash_sha256;
      } else if (filename === 'test_image_1.jpg') {
        hashMap.test_image_1 = photo.hash_sha256;
      } else if (filename === 'test_image_3.jpg') {
        hashMap.test_image_3 = photo.hash_sha256;
      } else if (filename === 'test_video.mp4') {
        hashMap.test_video = photo.hash_sha256;
      } else if (filename === 'test_video_2.mp4') {
        hashMap.test_video_2 = photo.hash_sha256;
      }
    }

    return hashMap;
  }

  /**
   * Print test photo hashes to console for easy copying to testPhotos
   */
  static async printTestPhotoHashes(baseURL = 'http://localhost:18473') {
    const hashes = await this.fetchTestPhotoHashes(baseURL);
    console.log('Test Photo Hashes:');
    console.log('Copy these to TestDataManager.testPhotos:\n');
    console.log('static testPhotos = {');
    for (const [key, hash] of Object.entries(hashes)) {
      console.log(`  ${key}: '${hash}',`);
    }
    console.log('};');
    return hashes;
  }

  /**
   * Add a photo to favorites
   */
  static async addToFavorites(photoHash, baseURL = 'http://localhost:18473') {
    const response = await fetch(`${baseURL}/api/photos/${photoHash}/favorite`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ is_favorite: true }),
    });
    if (!response.ok) {
      throw new Error(`Failed to favorite photo: ${response.statusText}`);
    }
  }

  /**
   * Remove a photo from favorites
   */
  static async removeFromFavorites(photoHash, baseURL = 'http://localhost:18473') {
    const response = await fetch(`${baseURL}/api/photos/${photoHash}/favorite`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ is_favorite: false }),
    });
    if (!response.ok) {
      throw new Error(`Failed to unfavorite photo: ${response.statusText}`);
    }
  }

  /**
   * Seed favorites by adding multiple photos to favorites
   */
  static async seedFavorites(photoHashes, baseURL = 'http://localhost:18473') {
    for (const hash of photoHashes) {
      await this.addToFavorites(hash, baseURL);
    }
  }

  /**
   * Clear all favorites
   */
  static async clearAllFavorites(baseURL = 'http://localhost:18473') {
    const photos = await this.fetchAllPhotos(baseURL);
    for (const photo of photos) {
      if (photo.is_favorite) {
        await this.removeFromFavorites(photo.hash_sha256, baseURL);
      }
    }
  }

  /**
   * Delete a photo by hash
   */
  static async deletePhoto(photoHash, baseURL = 'http://localhost:18473') {
    const response = await fetch(`${baseURL}/api/photos/${photoHash}`, {
      method: 'DELETE',
    });
    if (!response.ok) {
      throw new Error(`Failed to delete photo: ${response.statusText}`);
    }
  }

  /**
   * Get indexing status
   */
  static async getIndexingStatus(baseURL = 'http://localhost:18473') {
    const response = await fetch(`${baseURL}/api/indexing/status`);
    if (!response.ok) {
      throw new Error(`Failed to get indexing status: ${response.statusText}`);
    }
    return response.json();
  }

  /**
   * Wait for indexing to complete
   */
  static async waitForIndexing(
    baseURL = 'http://localhost:18473',
    maxRetries = 60,
    delayMs = 1000
  ) {
    for (let i = 0; i < maxRetries; i++) {
      try {
        const status = await this.getIndexingStatus(baseURL);
        if (!status.is_indexing) {
          return true;
        }
      } catch (e) {
        // Continue waiting
      }
      await new Promise((resolve) => setTimeout(resolve, delayMs));
    }
    return false;
  }
}
