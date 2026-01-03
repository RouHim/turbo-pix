import { readFile, unlink } from 'fs/promises';
import { existsSync } from 'fs';

const PID_FILE = 'test-server.pid';
const GRACEFUL_SHUTDOWN_DELAY_MS = 2000;

async function killServer() {
  console.log('\n=== TurboPix E2E Test Teardown ===\n');

  if (!existsSync(PID_FILE)) {
    console.log('No PID file found, server may have already stopped');
    return;
  }

  try {
    const pidContent = await readFile(PID_FILE, 'utf-8');
    const pid = parseInt(pidContent.trim(), 10);

    if (isNaN(pid)) {
      console.error('Invalid PID in file:', pidContent);
      return;
    }

    console.log(`Attempting to stop server (PID: ${pid})...`);

    try {
      process.kill(pid, 'SIGTERM');
      console.log('Sent SIGTERM signal');

      await new Promise((resolve) => setTimeout(resolve, GRACEFUL_SHUTDOWN_DELAY_MS));

      try {
        process.kill(pid, 0);
        console.log('Server still running after graceful shutdown, forcing kill...');
        process.kill(pid, 'SIGKILL');
        console.log('Sent SIGKILL signal');
      } catch {
        console.log('Server stopped gracefully');
      }
    } catch (error) {
      if (error.code === 'ESRCH') {
        console.log('Server process not found, already stopped');
      } else {
        console.error('Error stopping server:', error.message);
      }
    }

    await unlink(PID_FILE);
    console.log('Removed PID file');
  } catch (error) {
    console.error('Error during teardown:', error.message);
  }

  console.log('\n=== Teardown Complete ===\n');
}

export default killServer;
