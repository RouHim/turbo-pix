import fs from 'fs/promises';
import path from 'path';

const PROJECT_ROOT = path.resolve(process.cwd());
const SERVER_PID_FILE = path.join(PROJECT_ROOT, 'test-server.pid');

export default async function globalTeardown() {
  console.log('🛑 Stopping TurboPix E2E test environment...');

  try {
    // Read server PID
    const pid = await fs.readFile(SERVER_PID_FILE, 'utf-8');
    const pidNumber = parseInt(pid.trim(), 10);

    if (!pidNumber || isNaN(pidNumber)) {
      console.warn('⚠️  Invalid PID in file, skipping server shutdown');
      return;
    }

    console.log(`🔫 Sending SIGTERM to server (PID: ${pidNumber})...`);

    // Kill the server process
    try {
      process.kill(pidNumber, 'SIGTERM');
      console.log('✅ Server shutdown signal sent');
    } catch (error) {
      if (error.code === 'ESRCH') {
        console.warn('⚠️  Server process not found (already stopped?)');
      } else {
        console.error('❌ Error killing server process:', error);
      }
    }

    // Wait for graceful shutdown
    console.log('⏳ Waiting for graceful shutdown...');
    await new Promise((resolve) => setTimeout(resolve, 2000));

    // Force kill if still running
    try {
      process.kill(pidNumber, 0); // Check if process exists
      console.warn('⚠️  Server still running, sending SIGKILL...');
      process.kill(pidNumber, 'SIGKILL');
    } catch (error) {
      if (error.code === 'ESRCH') {
        // Process is gone, good
        console.log('✅ Server stopped successfully');
      }
    }

    // Remove PID file
    try {
      await fs.unlink(SERVER_PID_FILE);
      console.log('🗑️  Removed PID file');
    } catch (error) {
      // File might not exist, that's okay
    }

    console.log('✅ E2E test environment cleaned up!');
  } catch (error) {
    if (error.code === 'ENOENT') {
      console.warn('⚠️  PID file not found - server may not have started properly');
    } else {
      console.error('❌ Error during teardown:', error);
    }
  }
}
