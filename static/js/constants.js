// Application Constants

// UI Constants
const MOBILE_BREAKPOINT = 768;
const MONTH_KEYS = [
  'january',
  'february',
  'march',
  'april',
  'may',
  'june',
  'july',
  'august',
  'september',
  'october',
  'november',
  'december',
];
const WEEKDAY_KEYS = ['sunday', 'monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday'];

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
  MONTH_KEYS,
  WEEKDAY_KEYS,
  VIDEO_EXTENSIONS,
  RAW_EXTENSIONS,
  DEFAULT_BATCH_SIZE,
};
