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
) -> Result<(), String> {
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
                std::fs::read(file_path).map_err(|e| format!("Failed to read JPEG: {}", e))?;
            let mut jpeg = Jpeg::from_bytes(image_bytes.into())
                .map_err(|e| format!("Failed to parse JPEG: {}", e))?;
            jpeg.set_exif(Some(exif_bytes));
            let output_bytes = jpeg.encoder().bytes();
            std::fs::write(&temp_path, output_bytes)
        }
        "png" => {
            let image_bytes =
                std::fs::read(file_path).map_err(|e| format!("Failed to read PNG: {}", e))?;
            let mut png = Png::from_bytes(image_bytes.into())
                .map_err(|e| format!("Failed to parse PNG: {}", e))?;
            png.set_exif(Some(exif_bytes));
            let output_bytes = png.encoder().bytes();
            std::fs::write(&temp_path, output_bytes)
        }
        _ => return Err(format!("Unsupported format: {}", format)),
    };
    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("Failed to write {}: {}", format.to_uppercase(), e));
    }
    if let Err(e) = std::fs::rename(&temp_path, file_path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!(
            "Failed to replace {}: {}",
            format.to_uppercase(),
            e
        ));
    }
    Ok(())
}
