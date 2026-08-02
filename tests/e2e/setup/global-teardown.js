import { readFile, unlink } from 'fs/promises';
import { execSync } from 'child_process';
import { existsSync } from 'fs';

const PID_FILE = 'test-server.pid';
const GRACEFUL_SHUTDOWN_DELAY_MS = 2000;

// The PID file tracks the `cargo run` wrapper (global-setup writes
// serverProcess.pid), so signaling it may leave the actual server binary
// alive. Kill the binary directly too — the deliberately narrow pattern is
// documented in AGENTS.md; a broader `-f turbo-pix` would match the Playwright
// runner itself (its argv contains the repo path).
const SERVER_BINARY_PATTERN = 'target/(debug|release)/turbo-pix';

function killServerBinary() {
  try {
    execSync(`pkill -9 -f '${SERVER_BINARY_PATTERN}'`, { stdio: 'ignore' });
    console.log('Killed server binary (pkill)');
  } catch {
    // pkill exits non-zero when no process matches — that is the expected case
    // after a successful graceful shutdown.
    console.log('No server binary process matched pkill');
  }
}

async function killServer() {
  console.log('\n=== TurboPix E2E Test Teardown ===\n');

  if (!existsSync(PID_FILE)) {
    console.log('No PID file found, server may have already stopped');
  } else {
    try {
      const pidContent = await readFile(PID_FILE, 'utf-8');
      const pid = parseInt(pidContent.trim(), 10);

      if (isNaN(pid)) {
        console.error('Invalid PID in file:', pidContent);
      } else {
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
      }
    } catch (error) {
      console.error('Error during teardown:', error.message);
    }
  }

  // The tracked PID is cargo's, not the binary's — make sure the server is
  // really gone so the port is released before the next run. Runs even when
  // no PID file exists (e.g. an orphaned binary from a crashed run).
  killServerBinary();

  console.log('\n=== Teardown Complete ===\n');
}

export default killServer;
