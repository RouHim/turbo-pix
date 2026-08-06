import { execSync } from 'child_process';
import { copyFileSync, existsSync } from 'fs';
import path from 'path';

const TEST_DATA_DIR = 'test-e2e-data';
const DB_PATH = path.join(TEST_DATA_DIR, 'database', 'turbo-pix.db');

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

  /**
   * Restore N pending collages after a test consumed them (mirrors
   * global-setup's seedPendingCollages). Each call uses fresh signatures and
   * filenames so re-runs insert new rows instead of colliding.
   */
  static reseedPendingCollages(count = 2) {
    const collageSource = path.join('test-data', 'car.jpg');
    if (!existsSync(collageSource)) {
      throw new Error(`Missing collage source image at ${collageSource}`);
    }

    for (let n = 1; n <= count; n += 1) {
      const collagePath = path.join(
        TEST_DATA_DIR,
        'collages',
        'staging',
        `collage_e2e_reseed_${n}.jpg`
      );
      copyFileSync(collageSource, collagePath);
      const signature = `e2e-reseed-${Date.now()}-${n}`;
      const sql =
        `PRAGMA busy_timeout=5000; ` +
        `INSERT OR IGNORE INTO collages ` +
        `(date, file_path, thumbnail_path, photo_count, photo_hashes, signature) ` +
        `VALUES ('${new Date().toISOString().split('T')[0]}', ` +
        `'${collagePath}', NULL, 6, '[]', '${signature}');`;
      execSync(`sqlite3 "${DB_PATH}" "${sql}"`);
    }
  }

  /**
   * Restore one housekeeping candidate after a test consumed it (mirrors
   * global-setup's ensureHousekeepingCandidate): the current first photo.
   */
  static async reseedHousekeepingCandidate() {
    const response = await fetch('http://localhost:18473/api/photos?limit=200');
    if (!response.ok) {
      throw new Error(`Failed to fetch photos for housekeeping reseed: ${response.statusText}`);
    }
    const data = await response.json();
    const photos = data.photos || [];
    const targetPhoto = photos.find((photo) => photo.filename === 'receipt.jpg') || photos[0];
    if (!targetPhoto) {
      throw new Error('No photos available to reseed housekeeping candidates');
    }
    const sql =
      `PRAGMA busy_timeout=5000; ` +
      `INSERT OR IGNORE INTO housekeeping_candidates (photo_hash, reason, score) ` +
      `VALUES ('${targetPhoto.hash_sha256}', 'receipt', 95.0);`;
    execSync(`sqlite3 "${DB_PATH}" "${sql}"`);
  }
}
