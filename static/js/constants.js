// Application Constants

// UI Constants
const MOBILE_BREAKPOINT = 768;

// Media Constants
const VIDEO_EXTENSIONS = ['.mp4', '.mov', '.avi', '.mkv', '.webm', '.m4v'];
const RAW_EXTENSIONS = [
  '.cr2',
  '.cr3',
  '.nef',
  '.nrw',
  '.arw',
  '.srf',
  '.sr2',
  '.raf',
  '.orf',
  '.rw2',
  '.dng',
  '.pef',
];

// Pagination Constants
const DEFAULT_BATCH_SIZE = 50;

// Make constants available globally
window.APP_CONSTANTS = {
  MOBILE_BREAKPOINT,
  VIDEO_EXTENSIONS,
  RAW_EXTENSIONS,
  DEFAULT_BATCH_SIZE,
};
