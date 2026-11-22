//! Process-wide ONNX Runtime setup shared by every model.
//!
//! ORT permits exactly one environment per process, so CLIP and the face
//! pipeline must build their sessions from the same one.

use std::sync::OnceLock;

use anyhow::{Context, Result};
use ort::environment::Environment;

static ENVIRONMENT: OnceLock<Environment> = OnceLock::new();

/// Returns the single process-wide ORT environment, building it on first use.
/// Session creation may fail and be retried; the environment itself is never
/// rebuilt, because ORT rejects a second one.
pub(crate) fn shared_environment() -> Result<Environment> {
    if let Some(environment) = ENVIRONMENT.get() {
        return Ok(environment.clone());
    }

    let built = ort::init()
        .with_name("turbo-pix")
        .build()
        // A concurrent caller may have won the race; reuse its environment.
        .or_else(|error| ENVIRONMENT.get().cloned().ok_or(error))
        .context("Failed to initialize ONNX Runtime")?;

    Ok(ENVIRONMENT.get_or_init(|| built).clone())
}

/// Intra-op threads for model sessions.
///
/// ORT parallelizes a single inference internally, while the indexer also
/// runs several photo tasks concurrently; capping the session threads keeps
/// the two from oversubscribing every core.
pub(crate) fn model_threads() -> usize {
    num_cpus::get().clamp(1, 4)
}

/// Returns the freed heap to the OS when the model it belongs to is gone.
pub(crate) struct HeapTrim;

impl Drop for HeapTrim {
    fn drop(&mut self) {
        release_freed_memory();
    }
}

// `mi_collect` has to come from the crate that builds mimalloc's C library: its
// build script emits the link line, so every binary that links this crate also
// gets the archive. A hand-written `extern "C"` only resolved for binaries that
// already referenced the `mimalloc` crate — the container build's second binary
// did not, and the release build failed to link.
#[cfg(target_env = "musl")]
use libmimalloc_sys::mi_collect;

/// Hands freed heap back to the OS.
///
/// Neither allocator does this on its own: glibc raises its `mmap` threshold
/// after the first large free, so a second load/unload cycle leaves the whole
/// model in the arena (measured: 538 MiB retained), and mimalloc keeps its
/// heap (measured: 569 MiB retained). Without this the idle unload only looks
/// like it works on the first cycle.
///
/// mimalloc collects the calling thread's heap plus any abandoned one, so on
/// musl the reclaim relies on the thread that allocated the weights being
/// gone: tokio's blocking pool retires idle workers after ~10 s, two orders of
/// magnitude below the idle window, so its heap is abandoned long before the
/// unload runs.
fn release_freed_memory() {
    #[cfg(target_env = "musl")]
    unsafe {
        mi_collect(true);
    }
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        libc::malloc_trim(0);
    }
}

/// Resident set size in MiB, for the load/unload log lines.
pub(crate) fn rss_mib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb: u64 = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some(kb / 1024)
}

pub(crate) fn rss_label(rss: Option<u64>) -> String {
    rss.map_or_else(|| "?".to_string(), |mib| mib.to_string())
}
