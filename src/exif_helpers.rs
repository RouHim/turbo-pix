//! Shared EXIF reading and writing helpers.
//!
//! The read/write pipeline (kamadak-exif reader, experimental Writer,
//! img-parts JPEG/PNG segment swap) is used by the metadata extractor, the
//! metadata writer, the image editor, and the EXIF API handler. Keeping it in
//! one place prevents the copies from drifting (e.g. format strings diverging
//! between `"jpg" | "jpeg"` and `"jpeg"`).

use std::io::BufRead;
use std::io::Seek;
use std::path::Path;

use exif::Field;
use img_parts::jpeg::Jpeg;
use img_parts::png::Png;
use img_parts::{Bytes, ImageEXIF};

/// Why a JPEG/PNG EXIF write did not happen.
///
/// The split is the caller's decision, not a detail of the message: a medium
/// that refuses the write (a read-only mount, a photo directory this process
/// may not write) is a condition the user can fix, while a file that cannot be
/// parsed or serialized is a server-side failure of this write. Stringifying
/// both into one `String` is what used to make every refusal look like a
/// database error.
#[derive(Debug)]
pub enum ExifWriteError {
    /// The medium refused the read/write: permission denied or a read-only
    /// filesystem.
    ReadOnly(String),
    /// Any other failure: unparsable image, serializer refusal, unexpected
    /// I/O error.
    Failed(String),
}

impl std::fmt::Display for ExifWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadOnly(message) | Self::Failed(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ExifWriteError {}

/// One I/O step of a write, classified by what the caller can do about it.
fn io_failure(action: &str, format: &str, err: std::io::Error) -> ExifWriteError {
    let message = format!("Failed to {action} {}: {err}", format.to_uppercase());
    match err.kind() {
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem => {
            ExifWriteError::ReadOnly(message)
        }
        _ => ExifWriteError::Failed(message),
    }
}

/// Reads EXIF data from an open file-like reader.
///
/// Preserves the original kamadak-exif error so callers can tell a file with
/// no EXIF segment at all (`exif::Error::NotFound`) apart from malformed data.
pub fn read_exif<R: BufRead + Seek>(reader: &mut R) -> Result<exif::Exif, exif::Error> {
    exif::Reader::new().read_from_container(reader)
}

/// Opens `path` and reads its EXIF data.
pub fn read_exif_from_path(path: &Path) -> Result<exif::Exif, exif::Error> {
    let file = std::fs::File::open(path)?;
    read_exif(&mut std::io::BufReader::new(&file))
}

/// Serializes EXIF fields into a TIFF buffer (big-endian, standard EXIF format).
pub fn build_exif_buffer(new_fields: &[Field]) -> Result<Bytes, String> {
    let mut exif_buffer = std::io::Cursor::new(Vec::new());
    let mut writer = exif::experimental::Writer::new();
    for field in new_fields {
        writer.push_field(field);
    }
    writer
        .write(&mut exif_buffer, false)
        .map_err(|e| format!("Failed to generate EXIF data: {}", e))?;
    Ok(Bytes::from(exif_buffer.into_inner()))
}

/// Writes EXIF data into a JPEG or PNG file, replacing the APP1 segment
/// while preserving image content. `format` accepts `jpg`, `jpeg`, or `png`.
///
/// The original file is only touched by an atomic rename: the new bytes go
/// to a unique sibling temp file first, so a crash, power loss, or ENOSPC
/// mid-write can never truncate/corrupt the original photo (the codebase's
/// other write paths — rotate, transcode, MOOV fix — follow the same
/// temp+rename pattern).
static EXIF_TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn write_exif_to_image(
    file_path: &Path,
    format: &str,
    exif_bytes: Bytes,
) -> Result<(), ExifWriteError> {
    // Unique sibling temp path: same directory (so rename is atomic) and
    // unique per call (so concurrent PATCHes on the same photo cannot
    // interleave writes to one shared temp file).
    let temp_path = file_path.with_extension(format!(
        "exif_tmp.{}",
        EXIF_TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));

    let write_result = match format {
        "jpg" | "jpeg" => {
            let image_bytes =
                std::fs::read(file_path).map_err(|e| io_failure("read", "JPEG", e))?;
            let mut jpeg = Jpeg::from_bytes(image_bytes.into())
                .map_err(|e| ExifWriteError::Failed(format!("Failed to parse JPEG: {}", e)))?;
            jpeg.set_exif(Some(exif_bytes));
            let output_bytes = jpeg.encoder().bytes();
            std::fs::write(&temp_path, output_bytes).map_err(|e| io_failure("write", "JPEG", e))
        }
        "png" => {
            let image_bytes = std::fs::read(file_path).map_err(|e| io_failure("read", "PNG", e))?;
            let mut png = Png::from_bytes(image_bytes.into())
                .map_err(|e| ExifWriteError::Failed(format!("Failed to parse PNG: {}", e)))?;
            png.set_exif(Some(exif_bytes));
            let output_bytes = png.encoder().bytes();
            std::fs::write(&temp_path, output_bytes).map_err(|e| io_failure("write", "PNG", e))
        }
        _ => {
            return Err(ExifWriteError::Failed(format!(
                "Unsupported format: {}",
                format
            )))
        }
    };
    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&temp_path, file_path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(io_failure("replace", format, e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

    /// The medium classifications the metadata PATCH route turns into 403
    /// `file_read_only`. A refusing medium is exactly what a read-only photo
    /// mount reports, and it used to be flattened into the generic 500.
    #[test]
    fn io_failure_classifies_a_refusing_medium_as_read_only() {
        for kind in [ErrorKind::PermissionDenied, ErrorKind::ReadOnlyFilesystem] {
            let error = io_failure("write", "JPEG", std::io::Error::from(kind));
            match error {
                ExifWriteError::ReadOnly(message) => {
                    assert!(message.contains("Failed to write JPEG"), "{message}");
                }
                other => panic!("{kind:?} must classify as ReadOnly, got {other:?}"),
            }
        }
    }

    #[test]
    fn io_failure_keeps_other_io_errors_as_server_faults() {
        let error = io_failure("write", "PNG", std::io::Error::from(ErrorKind::NotFound));
        assert!(
            matches!(error, ExifWriteError::Failed(_)),
            "a missing file is not an actionable read-only medium"
        );
    }
}
