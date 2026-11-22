//! Shared checks for model files pinned by revision and SHA-256.

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

/// Fails unless the file's SHA-256 matches the pinned digest.
pub(crate) fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open model {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if actual != expected {
        anyhow::bail!(
            "Model checksum mismatch for {}: expected {}, got {}",
            path.display(),
            expected,
            actual
        );
    }
    Ok(())
}
