//! Memory bounds for full-resolution image decodes.
//!
//! Every route that decodes an original is reachable from the unauthenticated
//! LAN API, and each decode holds a full-resolution buffer for the duration of
//! the work. The two limits differ by an order of magnitude in buffer size:
//!
//! * A RAW demosaic holds hundreds of MB per decode, so `RAW_DECODE_LIMIT`
//!   stays at four.
//! * A standard-image thumbnail decode holds tens of MB; six permits match the
//!   browser's per-origin connection cap, so a cold grid is not throttled below
//!   what one client can request.

use std::sync::{Arc, LazyLock};
use tokio::sync::Semaphore;

/// Caps concurrent RAW decodes (full-resolution demosaic holds several buffers
/// per request — a 45 MP sensor can be hundreds of MB).
pub static RAW_DECODE_LIMIT: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(4)));

/// Caps concurrent standard-image thumbnail decodes. Six is the browser's
/// HTTP/1.1 per-origin connection cap, so a single client's cold grid loads in
/// one wave while extra clients stay bounded; the sequential face-indexing
/// pass holds at most one permit at a time.
pub static THUMBNAIL_DECODE_LIMIT: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(6)));
