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
            FATAL: 4
        };

        this.reverseLevels = Object.keys(this.levels);

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
            data: { ...data }
        };

        if (error) {
            entry.error = {
                name: error.name,
                message: error.message,
                stack: error.stack,
                fileName: error.fileName,
                lineNumber: error.lineNumber,
                columnNumber: error.columnNumber
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

        switch (level) {
            case 'DEBUG':
                console.debug(message, entry.data);
                break;
            case 'INFO':
                console.info(message, entry.data);
                break;
            case 'WARN':
                console.warn(message, entry.data);
                break;
            case 'ERROR':
            case 'FATAL':
                console.error(message, entry.error || entry.data);
                break;
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

    exportLogs() {
        const logs = this.getStoredLogs();
        const dataStr = JSON.stringify(logs, null, 2);
        const dataBlob = new Blob([dataStr], { type: 'application/json' });

        const link = document.createElement('a');
        link.href = URL.createObjectURL(dataBlob);
        link.download = `turbopix_logs_${new Date().toISOString().split('T')[0]}.json`;
        link.click();
    }

    clearStoredLogs() {
        this.storedLogs = [];
        if (typeof Storage !== 'undefined') {
            localStorage.removeItem('turbopix_logs');
        }
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

    fatal(message, error = null, data = {}) {
        return this.log('FATAL', message, data, error);
    }

    // Context management
    withContext(newContext) {
        const logger = new TurboPixLogger({
            level: this.level,
            enableConsole: this.enableConsole,
            enablePersistence: this.enablePersistence,
            maxStoredLogs: this.maxStoredLogs
        });
        logger.context = { ...this.context, ...newContext };
        logger.sessionId = this.sessionId;
        logger.storedLogs = this.storedLogs;
        return logger;
    }

    withComponent(component) {
        return this.withContext({ component });
    }

    // Performance tracking
    startTimer(name) {
        const startTime = performance.now();
        return {
            end: (data = {}) => {
                const duration = performance.now() - startTime;
                this.info(`Timer: ${name}`, { duration, ...data });
                return duration;
            }
        };
    }

    // User action tracking
    trackUserAction(action, data = {}) {
        this.info(`User Action: ${action}`, {
            action,
            timestamp: Date.now(),
            ...data
        });
    }

    // Error boundary helper
    captureError(error, context = {}) {
        this.error('Unhandled Error', error, {
            component: context.component || 'Unknown',
            action: context.action || 'Unknown',
            ...context
        });
    }
}

// Global error handler
window.addEventListener('error', (event) => {
    if (window.logger) {
        window.logger.captureError(event.error, {
            component: 'Global',
            filename: event.filename,
            lineno: event.lineno,
            colno: event.colno
        });
    }
});

window.addEventListener('unhandledrejection', (event) => {
    if (window.logger) {
        window.logger.error('Unhandled Promise Rejection', null, {
            reason: event.reason,
            component: 'Global'
        });
    }
});

// Create global logger instance
window.logger = new TurboPixLogger({
    level: 'INFO', // Change to 'DEBUG' for development
    enablePersistence: true,
    maxStoredLogs: 1000
});

// Export for use in other modules
if (typeof module !== 'undefined' && module.exports) {
    module.exports = TurboPixLogger;
}