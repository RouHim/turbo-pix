import { spawn } from 'child_process';
import { promisify } from 'util';
import { exec } from 'child_process';
import fs from 'fs/promises';
import path from 'path';

const execAsync = promisify(exec);

const PROJECT_ROOT = path.resolve(process.cwd());
const TEST_DATA_DIR = path.join(PROJECT_ROOT, 'test-e2e-data');
const SERVER_PID_FILE = path.join(PROJECT_ROOT, 'test-server.pid');

let serverProcess = null;

export default async function globalSetup() {
  console.log('🚀 Starting TurboPix E2E test environment...');

  try {
    // 1. Build the binary (in case static files changed)
    console.log('📦 Building binary...');
    await execAsync('cargo build --bin turbo-pix', {
      cwd: PROJECT_ROOT,
    });
    console.log('✅ Binary built successfully');

    // 2. Clean test data directory
    try {
      await fs.rm(TEST_DATA_DIR, { recursive: true, force: true });
      console.log('🗑️  Removed old test data directory');
    } catch (e) {
      // Directory doesn't exist, that's fine
    }

    // 3. Create test data directory structure
    await fs.mkdir(path.join(TEST_DATA_DIR, 'database'), { recursive: true });
    await fs.mkdir(path.join(TEST_DATA_DIR, 'cache', 'thumbnails'), {
      recursive: true,
    });
    console.log('📁 Created test data directory structure');

    // 4. Set environment variables for test mode
    const env = {
      ...process.env,
      TURBO_PIX_DATA_PATH: TEST_DATA_DIR,
      TURBO_PIX_PHOTO_PATHS: path.join(PROJECT_ROOT, 'test-data'),
      TURBO_PIX_PORT: '18473',
      RUST_LOG: 'info',
    };

    console.log('🔧 Environment configured:');
    console.log(`   DATA_PATH: ${TEST_DATA_DIR}`);
    console.log(`   PHOTO_PATHS: ${path.join(PROJECT_ROOT, 'test-data')}`);
    console.log(`   PORT: 18473`);

    // 5. Start server process
    console.log('🚀 Starting server...');
    serverProcess = spawn('cargo', ['run', '--bin', 'turbo-pix'], {
      cwd: PROJECT_ROOT,
      env,
      detached: false,
      stdio: 'pipe',
    });

    // Log server output
    serverProcess.stdout.on('data', (data) => {
      const output = data.toString().trim();
      if (output) {
        console.log(`[SERVER] ${output}`);
      }
    });

    serverProcess.stderr.on('data', (data) => {
      const output = data.toString().trim();
      if (output) {
        console.error(`[SERVER ERROR] ${output}`);
      }
    });

    serverProcess.on('error', (error) => {
      console.error('❌ Server process error:', error);
    });

    serverProcess.on('exit', (code, signal) => {
      if (code !== null && code !== 0) {
        console.error(`❌ Server exited with code ${code}`);
      } else if (signal) {
        console.log(`ℹ️  Server exited with signal ${signal}`);
      }
    });

    // 6. Wait for health check
    console.log('⏳ Waiting for server to be ready...');
    const serverReady = await waitForServer();
    if (!serverReady) {
      throw new Error('Server failed to start within timeout');
    }
    console.log('✅ Server is ready!');

    // 7. Wait for initial indexing to complete
    await waitForIndexing();

    // 8. Store PID for cleanup
    await fs.writeFile(SERVER_PID_FILE, serverProcess.pid.toString());
    console.log(`💾 Stored server PID: ${serverProcess.pid}`);

    console.log('✅ E2E test environment ready!');
  } catch (error) {
    console.error('❌ Failed to set up E2E test environment:', error);
    // Try to clean up if setup failed
    if (serverProcess) {
      serverProcess.kill('SIGTERM');
    }
    throw error;
  }
}

async function waitForServer(maxRetries = 30, delayMs = 1000) {
  for (let i = 0; i < maxRetries; i++) {
    try {
      const response = await fetch('http://localhost:18473/health');
      if (response.ok) {
        return true;
      }
    } catch (e) {
      // Server not ready yet
    }
    await new Promise((resolve) => setTimeout(resolve, delayMs));
    if ((i + 1) % 5 === 0) {
      console.log(`⏳ Still waiting for server (${i + 1}/${maxRetries})...`);
    }
  }
  return false;
}

async function waitForIndexing(maxRetries = 180, delayMs = 1000) {
  console.log('⏳ Waiting for photo indexing to complete...');

  for (let i = 0; i < maxRetries; i++) {
    try {
      const response = await fetch('http://localhost:18473/api/indexing/status');
      if (response.ok) {
        const status = await response.json();
        if (!status.is_indexing) {
          console.log('✅ Indexing complete!');
          console.log(`   Photos indexed: ${status.photos_processed || 0}`);
          return true;
        } else {
          const progress = status.progress_percent || 0;
          if (i % 5 === 0) {
            console.log(
              `⏳ Indexing in progress: ${progress.toFixed(1)}% (${status.photos_processed}/${status.photos_total})`
            );
          }
        }
      }
    } catch (e) {
      // Endpoint might not be available yet
    }
    await new Promise((resolve) => setTimeout(resolve, delayMs));
  }

  console.warn('⚠️  Indexing timeout - proceeding anyway');
  return false;
}
