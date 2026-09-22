import { exec, spawn } from 'child_process';
import { promisify } from 'util';
import { copyFile, mkdir, readlink, rm, utimes, writeFile } from 'fs/promises';
import { existsSync } from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const execAsync = promisify(exec);

const TEST_DATA_DIR = 'test-e2e-data';
// Sibling worktrees run this same suite; TURBO_PIX_E2E_PORT keeps their servers
// (and Playwright's baseURL) on distinct ports instead of racing one port.
const SERVER_PORT = process.env.TURBO_PIX_E2E_PORT ?? '18473';
const MAX_HEALTH_RETRIES = 30;
const MAX_INDEXING_RETRIES = 600;
const MAX_DB_RETRIES = 30;
const RETRY_DELAY_MS = 1000;
const RECENT_PHOTO_COUNT = 10;
const ARCHIVE_PHOTO_COUNT = 4;
const CLUSTER_DAYS_AGO = 7;
const ARCHIVE_DAYS_AGO = 400;
// Legacy seeds: six photos spread over six decades give the timeline real
// decade/year granularity, a populated gap-free modern cluster, and one empty
// decade (the 1990s) so gaps are exercised. Dates are written by
// updateTestPhotoDates() — the source image's own EXIF/mtime never matters.
const LEGACY_PHOTOS = [
  ['legacy_01.jpg', '1962-03-15T12:00:00.000Z'],
  ['legacy_02.jpg', '1974-09-02T12:00:00.000Z'],
  ['legacy_03.jpg', '1985-06-20T12:00:00.000Z'],
  ['legacy_04.jpg', '2004-11-05T12:00:00.000Z'],
  ['legacy_05.jpg', '2012-03-15T12:00:00.000Z'],
  ['legacy_06.jpg', '2019-07-01T12:00:00.000Z'],
];
const DB_PATH = path.join(TEST_DATA_DIR, 'database', 'turbo-pix.db');
const REPO_ROOT = fileURLToPath(new URL('../../..', import.meta.url));

// Reap only THIS checkout's stale server: the previous machine-wide pattern
// killed a sibling worktree's server mid-run. The spawned binary's argv is
// relative (`target/debug/turbo-pix`), so a path-anchored pkill pattern can
// never match it — the checkout is identified from /proc/<pid>/exe instead.
// The pattern itself stays narrow so it cannot match the Playwright runner
// (whose argv contains the repo root but never the binary path).
const SERVER_BINARY_PATTERN = 'target/(debug|release)/turbo-pix';
const SERVER_TARGET_PREFIX = `${path.join(REPO_ROOT, 'target')}${path.sep}`;

if (new RegExp(SERVER_BINARY_PATTERN).test(process.argv.join(' '))) {
  throw new Error(`Server reap pattern matches the Playwright runner: ${SERVER_BINARY_PATTERN}`);
}

async function reapStaleServers() {
  const { stdout } = await execAsync(`pgrep -f '${SERVER_BINARY_PATTERN}' || true`).catch(() => ({
    stdout: '',
  }));
  const mine = [];
  for (const pid of stdout.split(/\s+/).filter(Boolean)) {
    try {
      // A binary rebuilt since the process started reads `.../turbo-pix (deleted)`.
      const exe = (await readlink(`/proc/${pid}/exe`)).replace(/ \(deleted\)$/, '');
      if (exe.startsWith(SERVER_TARGET_PREFIX) && exe.endsWith(`${path.sep}turbo-pix`)) {
        mine.push(pid);
      }
    } catch {
      // Exited between pgrep and readlink, or not visible to this user.
    }
  }

  if (mine.length > 0) {
    await execAsync(`kill -9 ${mine.join(' ')}`).catch(() => {});
    console.log(`Reaped stale server process(es): ${mine.join(', ')}`);
  }
}

async function buildBinary() {
  console.log('Building TurboPix binary...');
  try {
    await execAsync('npm run build');
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

  const legacySource = path.join('test-data', 'test_image_1.jpg');
  if (!existsSync(legacySource)) {
    throw new Error(`Missing legacy source image at ${legacySource}`);
  }
  for (const [filename] of LEGACY_PHOTOS) {
    const filePath = path.join(photosDir, filename);
    await copyFile(legacySource, filePath);
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

  // Camera-EXIF fixture: the metadata EXIF test needs a photo whose EXIF
  // carries Make/Model. Its EXIF taken_at is 2024-01-01 — NOT pinned to the
  // archive era — so its sort position is not relied on anywhere: the EXIF
  // test targets it by hash (metadata.e2e.spec.js), and no photos[0]-based
  // test depends on the newest card being a particular file. If this
  // fixture's date ever moves past the cluster seed dates, re-check
  // photos[0]-based tests.
  const exifSrc = path.join('test-data', 'sample_with_exif.jpg');
  const exifDest = path.join(photosDir, 'sample_with_exif.jpg');
  if (existsSync(exifSrc)) {
    await copyFile(exifSrc, exifDest);
    await utimes(exifDest, recentDate, recentDate);
  } else {
    console.warn(`EXIF fixture not found at ${exifSrc}`);
  }

  const videoSrc = path.join('test-data', 'test_video.mp4');
  const videoDest = path.join(photosDir, 'test_video.mp4');
  if (existsSync(videoSrc)) {
    await copyFile(videoSrc, videoDest);
    await utimes(videoDest, recentDate, recentDate);
  } else {
    console.warn(`Video fixture not found at ${videoSrc}`);
  }

  const hevcVideoSrc = path.join('test-data', 'test_video_hevc.mp4');
  const hevcVideoDest = path.join(photosDir, 'test_video_hevc.mp4');
  if (existsSync(hevcVideoSrc)) {
    await copyFile(hevcVideoSrc, hevcVideoDest);
    await utimes(hevcVideoDest, recentDate, recentDate);
  } else {
    console.warn(`HEVC video fixture not found at ${hevcVideoSrc}`);
  }

  // Streaming-playback fixtures (video-streaming.e2e.spec.js): the same 20 s
  // h264+aac content in Matroska (remux case) and as an h264 + AC-3 MP4
  // (audio-only conversion case). Same pinned date as the videos above, so
  // nothing here displaces the first video card with the existing h264
  // fixtures. The progressive MP4 twin of the mkv (test_video_long.mp4) is NOT
  // seeded: no spec references it, and the matrix's direct-play row for a
  // progressive h264+aac MP4 is covered by test_video.mp4.
  for (const fixture of ['test_video_long.mkv', 'test_video_ac3.mp4']) {
    const source = path.join('test-data', fixture);
    const destination = path.join(photosDir, fixture);
    if (existsSync(source)) {
      await copyFile(source, destination);
      await utimes(destination, recentDate, recentDate);
    } else {
      console.warn(`Video fixture not found at ${source}`);
    }
  }

  // Capability-matrix fixtures (video-streaming.e2e.spec.js): 10-bit h264, a
  // silent h264, an h264 with two audio tracks, a progressive-less h264 and a
  // legacy MPEG-4/AVI rip. The dates pinned here only fix each file's
  // `date_modified` (the conversion cache key); the sort order the videos view
  // and the first-card specs see is pinned in updateTestPhotoDates, because a
  // video's `taken_at` falls back to its birth time, not its mtime.
  const matrixFixtures = [
    ['test_video_moov_end.mp4', CLUSTER_DAYS_AGO + 1],
    ['test_video_10bit.mp4', CLUSTER_DAYS_AGO + 2],
    ['test_video_multitrack.mp4', CLUSTER_DAYS_AGO + 3],
    ['test_video_noaudio.mp4', CLUSTER_DAYS_AGO + 4],
    ['test_video_legacy.avi', CLUSTER_DAYS_AGO + 5],
  ];
  for (const [fixture, daysAgo] of matrixFixtures) {
    const source = path.join('test-data', fixture);
    const destination = path.join(photosDir, fixture);
    if (existsSync(source)) {
      await copyFile(source, destination);
      const date = new Date(Date.now() - daysAgo * 24 * 60 * 60 * 1000);
      await utimes(destination, date, date);
    } else {
      console.warn(`Video fixture not found at ${source}`);
    }
  }

  console.log('Generated test media ready');
}

/**
 * Indexing faststarts every video in place (photo_processor's
 * `maybe_fix_moov_for_video`), so the progressive-less fixture is progressive
 * again before any spec runs and the matrix's moov-layout row would never meet
 * a non-progressive MP4. Re-seed the fixture after indexing — the state a file
 * reaches the decision in when the in-place fix could not run (a read-only
 * library, a file that arrived after the scan) — so the serve-time remux
 * decision is exercised instead of silently answering `direct`. The stored
 * capability record carries no `capability_version` yet (the indexer writes
 * codec/container facts only), so the decision probes the restored file rather
 * than answering from the record.
 */
async function reseedNonProgressiveFixture() {
  const source = path.join('test-data', 'test_video_moov_end.mp4');
  const destination = path.join(TEST_DATA_DIR, 'photos', 'test_video_moov_end.mp4');
  if (!existsSync(source)) {
    console.warn(`Progressive-less fixture not found at ${source}`);
    return;
  }
  const date = new Date(Date.now() - (CLUSTER_DAYS_AGO + 1) * 24 * 60 * 60 * 1000);
  await copyFile(source, destination);
  await utimes(destination, date, date);
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
  console.log('Waiting for indexing phases to complete (metadata + geo_resolution)...');

  for (let i = 0; i < maxRetries; i++) {
    try {
      const indexingResponse = await fetch(`${baseURL}/api/indexing/status`);
      if (indexingResponse.ok) {
        const data = await indexingResponse.json();
        const phases = Array.isArray(data.phases) ? data.phases : [];
        const metadataPhase = phases.find((phase) => phase.id === 'metadata');
        const geoPhase = phases.find((phase) => phase.id === 'geo_resolution');

        const metadataComplete = data.is_complete === true || metadataPhase?.state === 'done';
        const geoComplete = data.is_complete === true || geoPhase?.state === 'done';

        if (metadataComplete && geoComplete) {
          console.log(
            `All indexing phases ready - ${data.photos_indexed} photos indexed (metadata=done, geo_resolution=done)`
          );
          return true;
        }

        if (i % 10 === 0) {
          const metadataState = metadataPhase?.state || 'unknown';
          const geoState = geoPhase?.state || 'unknown';
          console.log(
            `Indexing progress: metadata=${metadataState}, geo_resolution=${geoState}, is_indexing=${data.is_indexing} (${data.photos_indexed} photos)`
          );
        }
      }
    } catch (error) {
      console.error('Error checking indexing status:', error.message);
    }

    await new Promise((resolve) => setTimeout(resolve, RETRY_DELAY_MS));
  }

  throw new Error(
    `Indexing phases did not complete after ${maxRetries} retries (${maxRetries * RETRY_DELAY_MS}ms)`
  );
}

// A generous budget: the fixture seeds 24 photos across six decades and sibling worktrees share the same cores, so the final phase (housekeeping) can outlast a two-minute wait. A longer wait, not a weaker check.
const INDEXING_COMPLETE_RETRIES = 600;

async function waitForIndexingComplete(baseURL, maxRetries = INDEXING_COMPLETE_RETRIES) {
  console.log('Waiting for indexing to fully complete (is_complete)...');

  // The housekeeping phase runs LAST and starts with DELETE FROM
  // housekeeping_candidates, so the seeded candidate only survives once the
  // whole indexing run (incl. semantic_vectors, collages, housekeeping) has
  // finished. waitForIndexing() above deliberately returns early on
  // metadata + geo_resolution — this wait is only for the deterministic seed.
  for (let i = 0; i < maxRetries; i++) {
    try {
      const indexingResponse = await fetch(`${baseURL}/api/indexing/status`);
      if (indexingResponse.ok) {
        const data = await indexingResponse.json();
        if (data.is_complete === true) {
          console.log(`Indexing fully complete - ${data.photos_indexed} photos indexed`);
          return true;
        }

        if (i % 10 === 0) {
          console.log(
            `Indexing not complete yet - ${data.photos_indexed} photos, is_indexing=${data.is_indexing}`
          );
        }
      }
    } catch (error) {
      console.error('Error checking indexing completion:', error.message);
    }

    await new Promise((resolve) => setTimeout(resolve, RETRY_DELAY_MS));
  }

  throw new Error(
    `Indexing did not reach is_complete after ${maxRetries} retries (${maxRetries * RETRY_DELAY_MS}ms)`
  );
}

async function updateTestPhotoDates(baseURL) {
  const recentDate = new Date(Date.now() - CLUSTER_DAYS_AGO * 24 * 60 * 60 * 1000);
  const archiveDate = new Date(Date.now() - ARCHIVE_DAYS_AGO * 24 * 60 * 60 * 1000);
  const recentTakenAt = recentDate.toISOString();
  const archiveTakenAt = archiveDate.toISOString();

  const legacySql = LEGACY_PHOTOS.map(
    ([filename, takenAt]) =>
      `UPDATE photos SET taken_at = '${takenAt}', updated_at = CURRENT_TIMESTAMP ` +
      `WHERE filename = '${filename}';`
  ).join(' ');
  // The seeded videos carry no embedded creation_time (the hevc fixture is the
  // exception), so their `taken_at` falls back to the file's BIRTH time — the
  // moment this setup copied them, i.e. the seeding order — and `utimes` cannot
  // pin it. That would put the last-copied fixture (the legacy AVI) at the top
  // of the videos view and ahead of nothing at all in the photos view. Pin
  // every seeded video to a distinct day older than the cluster seed instead:
  // `test_video.mp4` stays the newest video (the fixture the older specs open
  // as the first card), the matrix fixtures keep a stable order among
  // themselves, and no video can displace the cluster photos from `photos[0]`.
  const videoTakenAt = [
    ['test_video.mp4', CLUSTER_DAYS_AGO + 1],
    ['test_video_long.mkv', CLUSTER_DAYS_AGO + 3],
    ['test_video_ac3.mp4', CLUSTER_DAYS_AGO + 4],
    ['test_video_moov_end.mp4', CLUSTER_DAYS_AGO + 5],
    ['test_video_10bit.mp4', CLUSTER_DAYS_AGO + 6],
    ['test_video_multitrack.mp4', CLUSTER_DAYS_AGO + 7],
    ['test_video_noaudio.mp4', CLUSTER_DAYS_AGO + 8],
    ['test_video_legacy.avi', CLUSTER_DAYS_AGO + 9],
  ]
    .map(([filename, daysAgo]) => {
      const takenAt = new Date(Date.now() - daysAgo * 24 * 60 * 60 * 1000).toISOString();
      return (
        `UPDATE photos SET taken_at = '${takenAt}', updated_at = CURRENT_TIMESTAMP ` +
        `WHERE filename = '${filename}'; `
      );
    })
    .join('');

  const sql =
    `PRAGMA busy_timeout=5000; ` +
    `UPDATE photos SET taken_at = '${recentTakenAt}', updated_at = CURRENT_TIMESTAMP ` +
    `WHERE filename LIKE 'cluster_%'; ` +
    `UPDATE photos SET taken_at = '${archiveTakenAt}', updated_at = CURRENT_TIMESTAMP ` +
    `WHERE filename LIKE 'archive_%'; ` +
    legacySql +
    videoTakenAt;

  try {
    await execAsync(`sqlite3 "${DB_PATH}" "${sql}"`);
  } catch (error) {
    throw new Error(`Failed to update photo dates: ${error.message}`);
  }
}

async function waitForPhotosTable() {
  const sql = "SELECT name FROM sqlite_master WHERE type='table' AND name='photos';";

  for (let i = 0; i < MAX_DB_RETRIES; i += 1) {
    try {
      const { stdout } = await execAsync(`sqlite3 "${DB_PATH}" "${sql}"`);
      if (stdout.trim() === 'photos') {
        return;
      }
    } catch (error) {
      console.warn('Failed to check photos table:', error.message);
    }

    await new Promise((resolve) => setTimeout(resolve, RETRY_DELAY_MS));
  }

  throw new Error('Photos table not ready after retries');
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
  const matchingRecent = clusterPhotos.filter((photo) => photo.taken_at?.startsWith(recentPrefix));
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

  if (!existsSync(collageSource)) {
    throw new Error(`Missing collage source image at ${collageSource}`);
  }

  // Seed TWO pending collages: the viewer-accept test consumes one, and the
  // arrow-key navigation test needs >= 2 to run. Distinct signatures + staging
  // paths so both rows insert (the collages table has no unique constraint).
  const collageSeeds = [
    { filename: 'collage_seed_01.jpg', signature: 'seed-collage-01' },
    { filename: 'collage_seed_02.jpg', signature: 'seed-collage-02' },
  ];

  for (const seed of collageSeeds) {
    const collagePath = path.join(TEST_DATA_DIR, 'collages', 'staging', seed.filename);
    await copyFile(collageSource, collagePath);

    const sql =
      `PRAGMA busy_timeout=5000; ` +
      `INSERT OR IGNORE INTO collages ` +
      `(date, file_path, thumbnail_path, photo_count, photo_hashes, signature) ` +
      `VALUES ('${new Date().toISOString().split('T')[0]}', ` +
      `'${collagePath}', NULL, 6, '[]', '${seed.signature}');`;

    try {
      await execAsync(`sqlite3 "${DB_PATH}" "${sql}"`);
    } catch (error) {
      throw new Error(`Failed to seed pending collages: ${error.message}`);
    }
  }
}

async function startServer() {
  console.log('Starting TurboPix server...');

  const env = {
    ...process.env,
    TURBO_PIX_DATA_PATH: TEST_DATA_DIR,
    TURBO_PIX_PHOTO_PATHS: path.join(TEST_DATA_DIR, 'photos'),
    TURBO_PIX_PORT: SERVER_PORT,
    // Dedicated per-run transcode cache inside the wiped test dir: the
    // default /tmp/turbo-pix survives between runs, so a previously
    // transcoded HEVC file would short-circuit the transcode flow and the
    // HEVC spec would never see the transcode toast (its whole point).
    TRANSCODE_CACHE_DIR: path.join(TEST_DATA_DIR, 'transcode-cache'),
    RUST_LOG: 'info',
  };

  const serverProcess = spawn('cargo', ['run', '--bin', 'turbo-pix'], {
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  });

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

  // Kill THIS checkout's stale dev server so the health check / port binding
  // can't race a leftover process (AGENTS.md 'E2E port collision' learning).
  await reapStaleServers();

  try {
    await buildBinary();

    await setupTestDataDirectory();
    await seedTestMedia();

    await startServer();

    const baseURL = `http://localhost:${SERVER_PORT}`;
    await waitForHealthCheck(baseURL);

    await waitForIndexing(baseURL);
    await waitForPhotosTable();
    await updateTestPhotoDates(baseURL);
    await verifyTestPhotoDates(baseURL);
    // The housekeeping phase runs after metadata+geo as the LAST indexing
    // phase and starts with DELETE FROM housekeeping_candidates — wait for
    // full completion so the seeded candidate is not wiped by the scan.
    await waitForIndexingComplete(baseURL);
    // Indexing rewrites progressive-less videos in place; restore the fixture
    // to its moov-at-the-end state so the serve-time layout decision is
    // testable (see reseedNonProgressiveFixture).
    await reseedNonProgressiveFixture();
    await ensureHousekeepingCandidate(baseURL);
    await seedPendingCollages();

    console.log('\n=== Setup Complete ===\n');
  } catch (error) {
    console.error('\n=== Setup Failed ===');
    console.error(error);
    throw error;
  }
}
