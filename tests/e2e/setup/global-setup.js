import { exec } from 'child_process';
import { promisify } from 'util';
import { copyFile, mkdir, rm, utimes, writeFile } from 'fs/promises';
import { existsSync } from 'fs';
import path from 'path';

const execAsync = promisify(exec);

const TEST_DATA_DIR = 'test-e2e-data';
const SERVER_PORT = '18473';
const MAX_HEALTH_RETRIES = 30;
const MAX_INDEXING_RETRIES = 180;
const RETRY_DELAY_MS = 1000;
const RECENT_PHOTO_COUNT = 10;
const ARCHIVE_PHOTO_COUNT = 4;
const CLUSTER_DAYS_AGO = 7;
const ARCHIVE_DAYS_AGO = 400;
const DB_PATH = path.join(TEST_DATA_DIR, 'database', 'turbo-pix.db');

async function buildBinary() {
  console.log('Building TurboPix binary...');
  try {
    const { stdout, stderr } = await execAsync('cargo build --bin turbo-pix');
    if (stderr && !stderr.includes('Finished')) {
      console.log('Build output:', stderr);
    }
    console.log('Binary built successfully');
  } catch (error) {
    console.error('Failed to build binary:', error.message);
    throw error;
  }
}

async function setupTestDataDirectory() {
  console.log('Setting up test data directory...');

  if (existsSync(TEST_DATA_DIR)) {
    console.log('Cleaning existing test data directory...');
    await rm(TEST_DATA_DIR, { recursive: true, force: true });
  }

  await mkdir(TEST_DATA_DIR, { recursive: true });
  await mkdir(path.join(TEST_DATA_DIR, 'database'), { recursive: true });
  await mkdir(path.join(TEST_DATA_DIR, 'cache', 'thumbnails'), {
    recursive: true,
  });
  await mkdir(path.join(TEST_DATA_DIR, 'photos'), { recursive: true });
  await mkdir(path.join(TEST_DATA_DIR, 'collages', 'staging'), {
    recursive: true,
  });
  await mkdir(path.join(TEST_DATA_DIR, 'collages', 'thumbnails'), {
    recursive: true,
  });
  await mkdir(path.join(TEST_DATA_DIR, 'collages', 'accepted'), {
    recursive: true,
  });

  console.log('Test data directory created');
}

async function seedTestMedia() {
  console.log('Seeding generated test media...');

  const photosDir = path.join(TEST_DATA_DIR, 'photos');
  const recentDate = new Date(Date.now() - CLUSTER_DAYS_AGO * 24 * 60 * 60 * 1000);
  const archiveDate = new Date(Date.now() - ARCHIVE_DAYS_AGO * 24 * 60 * 60 * 1000);
  const clusterSource = path.join('test-data', 'car.jpg');
  if (!existsSync(clusterSource)) {
    throw new Error(`Missing cluster source image at ${clusterSource}`);
  }

  for (let i = 1; i <= RECENT_PHOTO_COUNT; i += 1) {
    const filename = `cluster_${String(i).padStart(2, '0')}.jpg`;
    const filePath = path.join(photosDir, filename);
    await copyFile(clusterSource, filePath);
    await utimes(filePath, recentDate, recentDate);
  }

  for (let i = 1; i <= ARCHIVE_PHOTO_COUNT; i += 1) {
    const filename = `archive_${String(i).padStart(2, '0')}.jpg`;
    const filePath = path.join(photosDir, filename);
    await copyFile(clusterSource, filePath);
    await utimes(filePath, archiveDate, archiveDate);
  }

  const receiptSrc = path.join('tests', 'e2e', 'fixtures', 'receipt.jpg');
  const receiptDest = path.join(photosDir, 'receipt.jpg');
  if (existsSync(receiptSrc)) {
    await copyFile(receiptSrc, receiptDest);
    await utimes(receiptDest, recentDate, recentDate);
  } else {
    console.warn(`Receipt fixture not found at ${receiptSrc}`);
  }

  const videoSrc = path.join('test-data', 'test_video.mp4');
  const videoDest = path.join(photosDir, 'test_video.mp4');
  if (existsSync(videoSrc)) {
    await copyFile(videoSrc, videoDest);
    await utimes(videoDest, recentDate, recentDate);
  } else {
    console.warn(`Video fixture not found at ${videoSrc}`);
  }

  console.log('Generated test media ready');
}

async function waitForHealthCheck(baseURL, maxRetries = MAX_HEALTH_RETRIES) {
  console.log('Waiting for server health check...');

  for (let i = 0; i < maxRetries; i++) {
    try {
      const response = await fetch(`${baseURL}/health`);
      if (response.ok) {
        console.log('Server is healthy');
        return true;
      }
    } catch (error) {
      // Server not ready yet, continue waiting
    }

    await new Promise((resolve) => setTimeout(resolve, RETRY_DELAY_MS));
  }

  throw new Error(
    `Server failed health check after ${maxRetries} retries (${maxRetries * RETRY_DELAY_MS}ms)`
  );
}

async function waitForIndexing(baseURL, maxRetries = MAX_INDEXING_RETRIES) {
  console.log('Waiting for metadata indexing to complete...');

  for (let i = 0; i < maxRetries; i++) {
    try {
      const indexingResponse = await fetch(`${baseURL}/api/indexing/status`);
      if (indexingResponse.ok) {
        const data = await indexingResponse.json();
        const progress = Math.round(data.progress_percent);

        const metadataComplete =
          data.photos_total > 0 && data.photos_processed >= data.photos_total;

        if (metadataComplete) {
          console.log(
            `Metadata indexing complete - ${data.photos_indexed} photos indexed`
          );
          return true;
        }

        if (i % 10 === 0) {
          const progressStr = isNaN(progress) ? 'computing...' : `${progress}%`;
          console.log(
            `Indexing progress: ${progressStr} (${data.photos_indexed} photos)`
          );
        }
      }
    } catch (error) {
      console.error('Error checking indexing status:', error.message);
    }

    await new Promise((resolve) => setTimeout(resolve, RETRY_DELAY_MS));
  }

  throw new Error(
    `Metadata indexing did not complete after ${maxRetries} retries (${maxRetries * RETRY_DELAY_MS}ms)`
  );
}

async function updateTestPhotoDates(baseURL) {
  const recentDate = new Date(Date.now() - CLUSTER_DAYS_AGO * 24 * 60 * 60 * 1000);
  const archiveDate = new Date(Date.now() - ARCHIVE_DAYS_AGO * 24 * 60 * 60 * 1000);
  const recentTakenAt = recentDate.toISOString();
  const archiveTakenAt = archiveDate.toISOString();

  const sql =
    `PRAGMA busy_timeout=5000; ` +
    `UPDATE photos SET taken_at = '${recentTakenAt}', updated_at = CURRENT_TIMESTAMP ` +
    `WHERE filename LIKE 'cluster_%'; ` +
    `UPDATE photos SET taken_at = '${archiveTakenAt}', updated_at = CURRENT_TIMESTAMP ` +
    `WHERE filename LIKE 'archive_%';`;

  try {
    await execAsync(`sqlite3 "${DB_PATH}" "${sql}"`);
  } catch (error) {
    throw new Error(`Failed to update photo dates: ${error.message}`);
  }
}

async function verifyTestPhotoDates(baseURL) {
  const response = await fetch(`${baseURL}/api/photos?limit=200`);
  if (!response.ok) {
    throw new Error(`Failed to fetch photos for verification: ${response.statusText}`);
  }

  const data = await response.json();
  const photos = data.photos || [];
  const recentDate = new Date(Date.now() - CLUSTER_DAYS_AGO * 24 * 60 * 60 * 1000);
  const archiveDate = new Date(Date.now() - ARCHIVE_DAYS_AGO * 24 * 60 * 60 * 1000);
  const recentPrefix = recentDate.toISOString().split('T')[0];
  const archivePrefix = archiveDate.toISOString().split('T')[0];

  const clusterPhotos = photos.filter((photo) => photo.filename?.startsWith('cluster_'));
  const archivePhotos = photos.filter((photo) => photo.filename?.startsWith('archive_'));
  const matchingRecent = clusterPhotos.filter((photo) =>
    photo.taken_at?.startsWith(recentPrefix)
  );
  const matchingArchive = archivePhotos.filter((photo) =>
    photo.taken_at?.startsWith(archivePrefix)
  );

  console.log(
    `Cluster date verification: ${matchingRecent.length}/${clusterPhotos.length} photos set to ${recentPrefix}`
  );
  console.log(
    `Archive date verification: ${matchingArchive.length}/${archivePhotos.length} photos set to ${archivePrefix}`
  );

  if (matchingRecent.length < RECENT_PHOTO_COUNT) {
    throw new Error(
      `Expected at least ${RECENT_PHOTO_COUNT} cluster photos on ${recentPrefix}, found ${matchingRecent.length}`
    );
  }

  if (matchingArchive.length < ARCHIVE_PHOTO_COUNT) {
    throw new Error(
      `Expected at least ${ARCHIVE_PHOTO_COUNT} archive photos on ${archivePrefix}, found ${matchingArchive.length}`
    );
  }
}

async function ensureHousekeepingCandidate(baseURL) {
  const response = await fetch(`${baseURL}/api/photos?limit=200`);
  if (!response.ok) {
    throw new Error(`Failed to fetch photos for housekeeping seed: ${response.statusText}`);
  }

  const data = await response.json();
  const photos = data.photos || [];
  const receiptPhoto = photos.find((photo) => photo.filename === 'receipt.jpg');
  const targetPhoto = receiptPhoto || photos[0];

  if (!targetPhoto) {
    throw new Error('No photos available to seed housekeeping candidates');
  }

  const sql =
    `PRAGMA busy_timeout=5000; ` +
    `CREATE TABLE IF NOT EXISTS housekeeping_candidates (photo_hash TEXT NOT NULL, reason TEXT NOT NULL, score REAL NOT NULL, PRIMARY KEY (photo_hash)); ` +
    `INSERT OR IGNORE INTO housekeeping_candidates (photo_hash, reason, score) ` +
    `VALUES ('${targetPhoto.hash_sha256}', 'receipt', 95.0);`;

  try {
    await execAsync(`sqlite3 "${DB_PATH}" "${sql}"`);
  } catch (error) {
    throw new Error(`Failed to seed housekeeping candidates: ${error.message}`);
  }
}

async function seedPendingCollages() {
  const collageSource = path.join(TEST_DATA_DIR, 'photos', 'cluster_01.jpg');
  const collagePath = path.join(
    TEST_DATA_DIR,
    'collages',
    'staging',
    'collage_seed_01.jpg'
  );

  if (!existsSync(collageSource)) {
    throw new Error(`Missing collage source image at ${collageSource}`);
  }

  await copyFile(collageSource, collagePath);

  const sql =
    `PRAGMA busy_timeout=5000; ` +
    `INSERT OR IGNORE INTO collages ` +
    `(date, file_path, thumbnail_path, photo_count, photo_hashes, signature) ` +
    `VALUES ('${new Date().toISOString().split('T')[0]}', ` +
    `'${collagePath}', NULL, 6, '[]', 'seed-collage-01');`;

  try {
    await execAsync(`sqlite3 "${DB_PATH}" "${sql}"`);
  } catch (error) {
    throw new Error(`Failed to seed pending collages: ${error.message}`);
  }
}

async function startServer() {
  console.log('Starting TurboPix server...');

  const env = {
    ...process.env,
    TURBO_PIX_DATA_PATH: TEST_DATA_DIR,
    TURBO_PIX_PHOTO_PATHS: path.join(TEST_DATA_DIR, 'photos'),
    TURBO_PIX_PORT: SERVER_PORT,
    RUST_LOG: 'info',
  };

  const serverProcess = exec('cargo run --bin turbo-pix', { env });

  serverProcess.stdout?.on('data', (data) => {
    console.log(`[server] ${data.toString().trim()}`);
  });

  serverProcess.stderr?.on('data', (data) => {
    console.log(`[server] ${data.toString().trim()}`);
  });

  serverProcess.on('error', (error) => {
    console.error('Server process error:', error);
  });

  await writeFile('test-server.pid', serverProcess.pid.toString());
  console.log(`Server started with PID: ${serverProcess.pid}`);

  return serverProcess;
}

export default async function globalSetup() {
  console.log('\n=== TurboPix E2E Test Setup ===\n');

  try {
    await buildBinary();

    await setupTestDataDirectory();
    await seedTestMedia();

    await startServer();

    const baseURL = `http://localhost:${SERVER_PORT}`;
    await waitForHealthCheck(baseURL);

    await waitForIndexing(baseURL);
    await updateTestPhotoDates(baseURL);
    await verifyTestPhotoDates(baseURL);
    await ensureHousekeepingCandidate(baseURL);
    await seedPendingCollages();

    console.log('\n=== Setup Complete ===\n');
  } catch (error) {
    console.error('\n=== Setup Failed ===');
    console.error(error);
    throw error;
  }
}
