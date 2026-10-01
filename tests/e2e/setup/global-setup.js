import { exec, execFileSync, spawn } from 'child_process';
import { promisify } from 'util';
import { copyFile, mkdir, readlink, rm, utimes, writeFile } from 'fs/promises';
import { existsSync, renameSync, rmSync } from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';
import { fetchAllPhotos } from './photo-pages.js';

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
// updateTestPhotoDates() — the source image's own EXIF/mtime never matters —
// but the copies must be real JPEGs, because that date is written INTO the file
// through the PATCH endpoint (see seedTestMedia).
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

// Every video the harness pins, and how many days before "now" its date is. A
// video's taken_at comes out of the container, so these offsets are
// load-bearing (the videos view's order, the first-card specs, the capability
// matrix); the seeding, the file-level tag check and the API assertion all read
// this one list so an offset cannot drift in one place only.
const VIDEO_FIXTURES = [
  ['test_video.mp4', CLUSTER_DAYS_AGO + 1],
  ['test_video_long.mkv', CLUSTER_DAYS_AGO + 3],
  ['test_video_ac3.mp4', CLUSTER_DAYS_AGO + 4],
  ['test_video_moov_end.mp4', CLUSTER_DAYS_AGO + 5],
  ['test_video_10bit.mp4', CLUSTER_DAYS_AGO + 6],
  ['test_video_multitrack.mp4', CLUSTER_DAYS_AGO + 7],
  ['test_video_noaudio.mp4', CLUSTER_DAYS_AGO + 8],
  ['test_video_legacy.avi', CLUSTER_DAYS_AGO + 9],
];
// The one fixture that must reach the specs with its moov at the END: it is the
// premise of the capability matrix's serve-time remux row. Everything else is
// seeded progressive.
const NON_PROGRESSIVE_FIXTURE = 'test_video_moov_end.mp4';

function videoDate(daysAgo) {
  return new Date(Date.now() - daysAgo * 24 * 60 * 60 * 1000);
}

function pinnedVideoDate(filename) {
  const entry = VIDEO_FIXTURES.find(([name]) => name === filename);
  if (!entry) {
    throw new Error(`No pinned date for video fixture ${filename}`);
  }
  return videoDate(entry[1]);
}

// ffmpeg/ffprobe are hard dependencies of the suite: the server shells out to
// them and the fixtures below are remuxed with them. Resolved once so the
// seeding helpers and the tag probes agree on the binaries; the env overrides
// let a developer point the suite at a specific build.
const ffmpegPath = process.env.FFMPEG_PATH || 'ffmpeg';
const ffprobePath = process.env.FFPROBE_PATH || 'ffprobe';

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

/**
 * A video's taken_at must come out of the container; the DB pin it used to
 * come from is gone. ffmpeg writes the tag ffprobe reads (creation_time for
 * mov/mp4/matroska, the generic `date` tag for AVI which has no
 * creation_time). No -f: the muxer is inferred from the destination extension
 * (`mkv` needs the `matroska` muxer, which only inference gets right).
 *
 * `faststart` defaults to true because a plain `-c copy` to an `.mp4`
 * destination writes the moov at the END, and the scan then faststarts such a
 * file in place with `-c copy -movflags +faststart`
 * (video_processor::fix_moov_atom), which replaces the file. That rewrite does
 * carry the container's global tags over (`-map_metadata 0`, added with the
 * file-only dates), but it is still real work on a fixture, and a regression in
 * it would be invisible to `verifyTestPhotoDates` — which only reads the API,
 * whose value the scan extracted before the rewrite — hence the file-level
 * `verifyPinnedVideoFiles`. Seeding every pinned video progressive keeps the
 * scan off the file.
 * The moov-at-end fixture opts out (see reseedNonProgressiveFixture), since
 * there the layout IS the premise; the matroska muxer accepts and ignores
 * movflags, and the AVI muxer never gets them.
 */
function pinVideoDate(source, destination, date, { faststart = true } = {}) {
  const iso = date.toISOString();
  const tag = path.extname(destination) === '.avi' ? 'date' : 'creation_time';
  execFileSync(ffmpegPath, [
    '-v',
    'error',
    '-y',
    '-i',
    source,
    '-c',
    'copy',
    '-metadata',
    `${tag}=${iso}`,
    ...(faststart && tag === 'creation_time' ? ['-movflags', '+faststart'] : []),
    destination,
  ]);
}

/** The first non-empty format tag value of a container (creation_time or date). */
function probeVideoDate(filePath) {
  const tags = execFileSync(ffprobePath, [
    '-v',
    'error',
    '-show_entries',
    'format_tags=creation_time,date',
    '-of',
    'default=noprint_wrappers=1:nokey=1',
    filePath,
  ])
    .toString()
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean);
  return tags[0] ?? '';
}

/**
 * The pinned date has to survive in the FILE, not only in the DB: the DB value
 * is extracted before the scan may rewrite a video in place, so an API-only
 * check is blind to a dropped tag. Reads every pinned video off disk and
 * compares the container tag's day with the fixture list's offset.
 */
function verifyPinnedVideoFiles() {
  for (const [filename, daysAgo] of VIDEO_FIXTURES) {
    const filePath = path.join(TEST_DATA_DIR, 'photos', filename);
    const expectedPrefix = videoDate(daysAgo).toISOString().split('T')[0];
    const tag = probeVideoDate(filePath);
    if (!tag.startsWith(expectedPrefix)) {
      throw new Error(
        `Pinned container date missing in ${filename}: expected ${expectedPrefix}, got '${tag || '(no tag)'}'`
      );
    }
  }
  console.log(`Pinned container dates verified in ${VIDEO_FIXTURES.length} video files`);
}

/**
 * Seed the multi-track fixture as a progressive all-stream twin of its source.
 *
 * The source ships with its moov at the end, so the indexing pass would
 * faststart it in place with `-c copy -movflags +faststart` and no `-map`:
 * ffmpeg's default stream selection keeps at most one stream per type and
 * silently drops the AC-3 track, leaving a plain h264+aac file that makes the
 * multi-track spec's "the second (AC-3) track must not be picked" assertion
 * vacuous. Remuxing here with `-map 0` keeps h264+aac+ac3 and moves the moov to
 * the front, so `fix_moov_atom`'s progressive check is already satisfied and
 * the indexing pass leaves every stream alone. The same remux carries the
 * pinned creation_time (see pinVideoDate), which `-map 0` forces us to set
 * here rather than through that helper.
 *
 * Fails loudly: a seed that silently lost a track would make the spec assert
 * against the wrong file, which is exactly the state this avoids.
 */
function seedMultitrackFixture(source, destination, date) {
  const iso = date.toISOString();
  // Mux into a per-process staging sibling and rename it into place: two runs
  // sharing this worktree (a re-run racing its predecessor, a sibling spec run)
  // would otherwise write the same destination, and one of them would probe the
  // other's half-written file — "moov atom not found" on a fixture that is fine.
  const staging = `${destination}.${process.pid}.tmp`;
  try {
    execFileSync(
      ffmpegPath,
      [
        '-v',
        'error',
        '-y',
        '-i',
        source,
        '-map',
        '0',
        '-c',
        'copy',
        '-movflags',
        '+faststart',
        '-metadata',
        `creation_time=${iso}`,
        // The staging name carries no media extension, so name the muxer: it
        // also keeps a leaked staging file from looking like indexable media.
        '-f',
        'mp4',
        staging,
      ],
      { stdio: ['ignore', 'ignore', 'pipe'] }
    );
    renameSync(staging, destination);
  } catch (error) {
    rmSync(staging, { force: true });
    throw new Error(
      `Failed to seed ${destination} from ${source} with ${ffmpegPath}: ` +
        `${error.stderr?.toString().trim() || error.message}`
    );
  }

  const streams = execFileSync(ffprobePath, [
    '-v',
    'error',
    '-show_entries',
    'stream=codec_type',
    '-of',
    'default=noprint_wrappers=1:nokey=1',
    destination,
  ])
    .toString()
    .trim()
    .split('\n');
  if (streams.length !== 3) {
    throw new Error(
      `Seeded ${destination} with ${streams.length} stream(s) [${streams.join(', ')}], ` +
        'expected 3 (video, audio, audio)'
    );
  }
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

  // The legacy copies must be EXIF-writable: `updateTestPhotoDates` pins their
  // dates through the PATCH endpoint, which writes the date INTO the file. The
  // old source here was a 13-byte non-JPEG placeholder ("fake image 1"), which
  // the EXIF writer rejects ("file signature didn't match the expected
  // signature") and no extractor can read a date out of; the filename-date
  // fallback can never express these years either (parse_date_from_filename
  // rejects pre-1990). So seed them from the same real JPEG as the cluster and
  // archive copies — the smallest source in test-data (34 KB, ~0.68 MP) and
  // one that carries no camera Make/Model, so the metadata spec's "photo with
  // camera EXIF" locator stays unambiguous. Its EXIF date is overwritten by the
  // PATCH below; its mtime never mattered.
  const legacySource = clusterSource;
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

  // Every pinned video is seeded through pinVideoDate (or seedMultitrackFixture
  // for the multi-track one), which writes the date into the container and — by
  // default — leaves the file progressive, so the scan's in-place faststart
  // rewrite never touches it. See VIDEO_FIXTURES for the offsets, and
  // pinVideoDate for why progressive matters. `test_video.mp4` stays the newest
  // video (the fixture the older specs open as the first card) and no video can
  // displace the cluster photos from `photos[0]`. The progressive MP4 twin of
  // the mkv (test_video_long.mp4) is NOT seeded: no spec references it, and the
  // matrix's direct-play row for a progressive h264+aac MP4 is covered by
  // test_video.mp4.
  for (const [fixture, daysAgo] of VIDEO_FIXTURES) {
    const source = path.join('test-data', fixture);
    const destination = path.join(photosDir, fixture);
    if (!existsSync(source)) {
      console.warn(`Video fixture not found at ${source}`);
      continue;
    }
    const date = videoDate(daysAgo);
    if (fixture === 'test_video_multitrack.mp4') {
      // Remuxed to keep both audio tracks, not copied: see
      // seedMultitrackFixture.
      seedMultitrackFixture(source, destination, date);
    } else {
      pinVideoDate(source, destination, date);
    }
    // The mtime still keys the conversion cache; the date now also lives in the
    // container.
    await utimes(destination, date, date);
  }

  // The HEVC fixture is NOT date-pinned: its source already carries a
  // creation_time (2020-01-01), old enough to stay behind every pinned video in
  // the videos view. Copying keeps that tag.
  const hevcVideoSrc = path.join('test-data', 'test_video_hevc.mp4');
  const hevcVideoDest = path.join(photosDir, 'test_video_hevc.mp4');
  if (existsSync(hevcVideoSrc)) {
    await copyFile(hevcVideoSrc, hevcVideoDest);
    await utimes(hevcVideoDest, recentDate, recentDate);
  } else {
    console.warn(`HEVC video fixture not found at ${hevcVideoSrc}`);
  }

  // Metadata-editing fixture (video-metadata.e2e.spec.js): the only seeded
  // video that already carries a QuickTime location carrier, so a coordinate
  // save exercises replacing a carrier instead of the absence of one. Seeded
  // byte-for-byte — it is already faststart, and pinning its date through the
  // container (pinVideoDate) would remux it and move or drop the carriers. Its
  // `taken_at` comes from its own `com.apple.quicktime.creationdate`
  // (2024-05-01), older than every video pinned in VIDEO_FIXTURES, so it cannot
  // displace the first video card; the mtime below only keys the conversion
  // cache.
  const keysVideoSrc = path.join('test-data', 'test_video_quicktime_keys.mp4');
  const keysVideoDest = path.join(photosDir, 'test_video_quicktime_keys.mp4');
  if (existsSync(keysVideoSrc)) {
    await copyFile(keysVideoSrc, keysVideoDest);
    const date = videoDate(CLUSTER_DAYS_AGO + 6);
    await utimes(keysVideoDest, date, date);
  } else {
    console.warn(`Video fixture not found at ${keysVideoSrc}`);
  }

  // Second metadata-editing fixture (video-metadata.e2e.spec.js): FR-001
  // accepts MOV next to MP4 and M4V, and a renamed copy proves that gate end
  // to end — the editor offers the file, the request reaches the writer and
  // ffprobe reads the new instant back out of the container — without adding a
  // binary to the repository. test_video.mp4 again: it carries no location
  // carrier and no date carrier, so the copy stays out of the map's marker
  // set (only the API-derived unlocated count moves) and out of the date
  // carriers the metadata cases assert. Its `taken_at` has to be pinned in
  // updateTestPhotoDates, see there.
  const movSrc = path.join('test-data', 'test_video.mp4');
  const movDest = path.join(photosDir, 'test_video_mov.mov');
  if (existsSync(movSrc)) {
    await copyFile(movSrc, movDest);
    const date = new Date(Date.now() - (CLUSTER_DAYS_AGO + 10) * 24 * 60 * 60 * 1000);
    await utimes(movDest, date, date);
  } else {
    console.warn(`Video fixture not found at ${movSrc}`);
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
 *
 * Seeded through pinVideoDate rather than copyFile, with `faststart: false`: a
 * plain `-c copy` to an MP4 destination writes the moov at the END (the whole
 * premise of this fixture) and pins the container's creation_time in the same
 * pass. The seed itself is progressive (VIDEO_FIXTURES + pinVideoDate's default)
 * so the scan leaves the tag alone; this reseed is what makes the file
 * non-progressive for the specs.
 */
async function reseedNonProgressiveFixture() {
  const source = path.join('test-data', NON_PROGRESSIVE_FIXTURE);
  const destination = path.join(TEST_DATA_DIR, 'photos', NON_PROGRESSIVE_FIXTURE);
  if (!existsSync(source)) {
    console.warn(`Progressive-less fixture not found at ${source}`);
    return;
  }
  const date = pinnedVideoDate(NON_PROGRESSIVE_FIXTURE);
  // `faststart: false`: this fixture exists to reach the specs with its moov at
  // the END, and a plain `-c copy` to an MP4 destination is what writes it that
  // way (the pinned creation_time is written in the same pass).
  pinVideoDate(source, destination, date, { faststart: false });
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

/** Every photo of the library; `failureMessage` labels this site's error. */
async function listAllPhotos(baseURL, failureMessage) {
  return fetchAllPhotos(async (requestPath) => {
    const response = await fetch(`${baseURL}${requestPath}`);
    if (!response.ok) throw new Error(`${failureMessage}: ${response.statusText}`);
    return response.json();
  });
}

async function updateTestPhotoDates(baseURL) {
  const photos = await listAllPhotos(baseURL, 'Failed to list photos');
  // PATCH writes the file, so this is the product's own write path; the old
  // sqlite3 block (and the video rows in it) is gone. Videos need no PATCH —
  // their dates were pinned in seedTestMedia.
  const recentDate = new Date(Date.now() - CLUSTER_DAYS_AGO * 24 * 60 * 60 * 1000);
  const archiveDate = new Date(Date.now() - ARCHIVE_DAYS_AGO * 24 * 60 * 60 * 1000);
  const legacy = LEGACY_PHOTOS.map(([filename, takenAt]) => ({
    match: (p) => p.filename === filename,
    takenAt,
  }));
  const groups = [
    { match: (p) => p.filename?.startsWith('cluster_'), takenAt: recentDate.toISOString() },
    { match: (p) => p.filename?.startsWith('archive_'), takenAt: archiveDate.toISOString() },
    ...legacy,
  ];
  for (const { match, takenAt } of groups) {
    for (const photo of photos.filter(match)) {
      const patched = await fetch(`${baseURL}/api/photos/${photo.hash_sha256}/metadata`, {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ taken_at: takenAt }),
      });
      if (!patched.ok) throw new Error(`Failed to pin ${photo.filename}: ${patched.status}`);
    }
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
  const photos = await listAllPhotos(baseURL, 'Failed to fetch photos for verification');

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

  // The videos carry no DB pin any more (updateTestPhotoDates PATCHes photos
  // only), so their `taken_at` is the container tag seedTestMedia wrote; the
  // newest entry of VIDEO_FIXTURES (test_video.mp4) sorts first. Checked here
  // because a remux that silently failed to write the tag would leave the
  // fixture with its birth time — the regression this assertion exists to
  // catch, and one a photos-only verification would not see. That the tag also
  // survives in the FILE (which this API check cannot see) is asserted by
  // verifyPinnedVideoFiles.
  const [newestVideo, newestVideoDaysAgo] = VIDEO_FIXTURES[0];
  const firstVideo = photos.find((photo) => /\.(mp4|mkv|avi)$/i.test(photo.filename ?? ''));
  if (!firstVideo) {
    throw new Error('No video fixture in the indexed library');
  }
  const videoPrefix = videoDate(newestVideoDaysAgo).toISOString().split('T')[0];
  if (firstVideo.filename !== newestVideo || !firstVideo.taken_at?.startsWith(videoPrefix)) {
    throw new Error(
      `Expected first video ${newestVideo} on ${videoPrefix}, ` +
        `got ${firstVideo.filename} at ${firstVideo.taken_at}`
    );
  }
  console.log(`Video date verification: ${firstVideo.filename} set to ${firstVideo.taken_at}`);
}

async function ensureHousekeepingCandidate(baseURL) {
  const photos = await listAllPhotos(baseURL, 'Failed to fetch photos for housekeeping seed');

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
    // The scan faststarts videos in place; the rewrite carries the container
    // tags over (`-map_metadata 0`), and this first file-level check is what
    // would catch it if that stopped being true.
    verifyPinnedVideoFiles();
    // Indexing rewrites progressive-less videos in place; restore the fixture
    // to its moov-at-the-end state so the serve-time layout decision is
    // testable (see reseedNonProgressiveFixture).
    await reseedNonProgressiveFixture();
    // The reseed is the last writer of a pinned fixture, so the check runs
    // again here: this is the state the specs read.
    verifyPinnedVideoFiles();
    await ensureHousekeepingCandidate(baseURL);
    await seedPendingCollages();

    console.log('\n=== Setup Complete ===\n');
  } catch (error) {
    console.error('\n=== Setup Failed ===');
    console.error(error);
    throw error;
  }
}
