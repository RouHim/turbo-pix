// Centralized Logging System for TurboPix Frontend

class TurboPixLogger {
  constructor(options = {}) {
    this.level = options.level || 'INFO';
    this.context = options.context || {};
    this.enableConsole = options.enableConsole !== false;
    this.enablePersistence = options.enablePersistence !== false;
    this.maxStoredLogs = options.maxStoredLogs || 1000;
    this.sessionId = this.generateSessionId();

    this.levels = {
      DEBUG: 0,
      INFO: 1,
      WARN: 2,
      ERROR: 3,
    };

    // Initialize storage
    if (this.enablePersistence && typeof Storage !== 'undefined') {
      this.loadPersistedLogs();
    }
  }

  generateSessionId() {
    return 'session_' + Date.now() + '_' + Math.random().toString(36).substr(2, 9);
  }

  shouldLog(level) {
    return this.levels[level] >= this.levels[this.level];
  }

  createLogEntry(level, message, data = {}, error = null) {
    const entry = {
      timestamp: new Date().toISOString(),
      level,
      message,
      sessionId: this.sessionId,
      userAgent: navigator.userAgent,
      url: window.location.href,
      context: { ...this.context },
      data: { ...data },
    };

    if (error) {
      entry.error = {
        name: error.name,
        message: error.message,
        stack: error.stack,
        fileName: error.fileName,
        lineNumber: error.lineNumber,
        columnNumber: error.columnNumber,
      };
    }

    return entry;
  }

  log(level, message, data = {}, error = null) {
    if (!this.shouldLog(level)) return;

    const entry = this.createLogEntry(level, message, data, error);

    // Console logging
    if (this.enableConsole) {
      this.logToConsole(level, entry);
    }

    // Persistence
    if (this.enablePersistence) {
      this.persistLog(entry);
    }

    return entry;
  }

  logToConsole(level, entry) {
    const prefix = `[${entry.timestamp}] [${level}] [${entry.context.component || 'App'}]`;
    const message = `${prefix} ${entry.message}`;

    try {
      switch (level) {
        case 'DEBUG':
          console.debug(message, JSON.parse(JSON.stringify(entry.data)));
          break;
        case 'INFO':
          console.info(message, JSON.parse(JSON.stringify(entry.data)));
          break;
        case 'WARN':
          console.warn(message, JSON.parse(JSON.stringify(entry.data)));
          break;
        case 'ERROR':
          console.error(message, entry.error || JSON.parse(JSON.stringify(entry.data)));
          break;
      }
    } catch {
      console.log(message);
    }
  }

  persistLog(entry) {
    if (typeof Storage === 'undefined') return;

    try {
      const logs = this.getStoredLogs();
      logs.push(entry);

      // Keep only the most recent logs
      if (logs.length > this.maxStoredLogs) {
        logs.splice(0, logs.length - this.maxStoredLogs);
      }

      localStorage.setItem('turbopix_logs', JSON.stringify(logs));
    } catch (e) {
      // If storage fails, disable persistence to avoid repeated errors
      console.warn('Log persistence failed, disabling:', e);
      this.enablePersistence = false;
    }
  }

  loadPersistedLogs() {
    if (typeof Storage === 'undefined') return;

    try {
      const stored = localStorage.getItem('turbopix_logs');
      if (stored) {
        this.storedLogs = JSON.parse(stored);
      } else {
        this.storedLogs = [];
      }
    } catch (e) {
      console.warn('Failed to load persisted logs:', e);
      this.storedLogs = [];
    }
  }

  getStoredLogs() {
    return this.storedLogs || [];
  }

  // Convenience methods
  debug(message, data = {}) {
    return this.log('DEBUG', message, data);
  }

  info(message, data = {}) {
    return this.log('INFO', message, data);
  }

  warn(message, data = {}, error = null) {
    return this.log('WARN', message, data, error);
  }

  error(message, error = null, data = {}) {
    return this.log('ERROR', message, data, error);
  }

  // Error boundary helper
  captureError(error, context = {}) {
    this.error('Unhandled Error', error, {
      component: context.component || 'Unknown',
      action: context.action || 'Unknown',
      ...context,
    });
  }
}

// Create module-level logger instance
export const logger = new TurboPixLogger({
  level: 'INFO', // Change to 'DEBUG' for development
  enablePersistence: true,
  maxStoredLogs: 1000,
});

// Global error handlers
window.addEventListener('error', (event) => {
  if (logger) {
    logger.captureError(event.error, {
      component: 'Global',
      filename: event.filename,
      lineno: event.lineno,
      colno: event.colno,
    });
  }
});

window.addEventListener('unhandledrejection', (event) => {
  if (logger) {
    logger.error('Unhandled Promise Rejection', null, {
      reason: event.reason,
      component: 'Global',
    });
  }
});
