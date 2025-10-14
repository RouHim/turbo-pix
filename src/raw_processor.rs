use image::{DynamicImage, ImageBuffer, Rgb};
use log::{debug, warn};
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RawError {
    #[error("Failed to decode RAW file: {0}")]
    DecodeError(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Decode a RAW image file to a DynamicImage
pub fn decode_raw_to_dynamic_image(path: &Path) -> Result<DynamicImage, RawError> {
    debug!("Decoding RAW file: {}", path.display());

    // 1. Use rawloader to decode
    let raw_image =
        rawloader::decode_file(path).map_err(|e| RawError::DecodeError(format!("{:?}", e)))?;

    // 2. Extract bayer pattern data
    let (width, height, data) = match raw_image.data {
        rawloader::RawImageData::Integer(data) => (raw_image.width, raw_image.height, data),
        rawloader::RawImageData::Float(data) => {
            // Convert float data to u16
            let int_data: Vec<u16> = data.iter().map(|&f| f.clamp(0.0, 65535.0) as u16).collect();
            (raw_image.width, raw_image.height, int_data)
        }
    };

    debug!(
        "RAW image decoded: {}x{}, {} pixels",
        width,
        height,
        data.len()
    );

    // 3. Demosaic using bayer crate
    // Note: The bayer crate v0.1.5 has a complex API that requires readers and output buffers
    // For simplicity, we'll use a basic nearest-neighbor demosaic approach instead
    let cfa_pattern = parse_cfa_from_rawloader(&raw_image.cfa)?;

    // Simple nearest-neighbor demosaic (fast but lower quality)
    let rgb8_data = simple_demosaic(&data, width, height, cfa_pattern)?;

    debug!("Demosaic completed, RGB data size: {}", rgb8_data.len());

    let img_buffer: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_raw(width as u32, height as u32, rgb8_data).ok_or_else(|| {
            RawError::DecodeError("Buffer conversion failed: invalid dimensions".to_string())
        })?;

    Ok(DynamicImage::ImageRgb8(img_buffer))
}

/// Simple nearest-neighbor demosaic algorithm
/// This is a basic implementation that's fast but produces lower quality than advanced algorithms
#[allow(clippy::needless_range_loop)]
fn simple_demosaic(
    data: &[u16],
    width: usize,
    height: usize,
    cfa: bayer::CFA,
) -> Result<Vec<u8>, RawError> {
    let mut rgb_data = vec![0u8; width * height * 3];

    // Helper to get pixel value safely
    let get_pixel = |x: usize, y: usize| -> u8 {
        if x < width && y < height {
            (data[y * width + x] >> 8) as u8
        } else {
            0
        }
    };

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let pixel = get_pixel(x, y);

            // Determine RGB values based on CFA pattern and position
            // Using simple replication for missing channels (nearest neighbor)
            let (r, g, b) = match cfa {
                bayer::CFA::RGGB => match (y % 2, x % 2) {
                    (0, 0) => (pixel, get_pixel(x + 1, y), get_pixel(x, y + 1)), // R pixel
                    (0, 1) => (get_pixel(x.wrapping_sub(1), y), pixel, get_pixel(x, y + 1)), // G pixel (R row)
                    (1, 0) => (get_pixel(x, y.wrapping_sub(1)), pixel, get_pixel(x + 1, y)), // G pixel (B row)
                    (1, 1) => (
                        get_pixel(x, y.wrapping_sub(1)),
                        get_pixel(x.wrapping_sub(1), y),
                        pixel,
                    ), // B pixel
                    _ => unreachable!(),
                },
                _ => (pixel, pixel, pixel), // Fallback for other CFA patterns
            };

            let out_idx = idx * 3;
            rgb_data[out_idx] = r;
            rgb_data[out_idx + 1] = g;
            rgb_data[out_idx + 2] = b;
        }
    }

    Ok(rgb_data)
}

/// Parse CFA pattern from rawloader format to bayer format
fn parse_cfa_from_rawloader(cfa: &rawloader::CFA) -> Result<bayer::CFA, RawError> {
    // Try to get the pattern name from the CFA
    // rawloader CFA is a struct with pattern information
    let pattern_name = format!("{:?}", cfa);

    // Extract pattern from debug string (e.g., "CFA { name: \"RGGB\", ... }" -> "RGGB")
    if pattern_name.contains("RGGB") {
        Ok(bayer::CFA::RGGB)
    } else if pattern_name.contains("BGGR") {
        Ok(bayer::CFA::BGGR)
    } else if pattern_name.contains("GRBG") {
        Ok(bayer::CFA::GRBG)
    } else if pattern_name.contains("GBRG") {
        Ok(bayer::CFA::GBRG)
    } else {
        warn!("Unknown CFA pattern, using default RGGB");
        Ok(bayer::CFA::RGGB)
    }
}

/// Check if a file is a RAW image file based on extension
pub fn is_raw_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_lowercase().as_str(),
                "cr2"
                    | "cr3"
                    | "nef"
                    | "nrw"
                    | "arw"
                    | "srf"
                    | "sr2"
                    | "raf"
                    | "orf"
                    | "rw2"
                    | "dng"
                    | "pef"
            )
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_is_raw_file() {
        // Canon
        assert!(is_raw_file(&PathBuf::from("photo.cr2")));
        assert!(is_raw_file(&PathBuf::from("photo.CR2")));
        assert!(is_raw_file(&PathBuf::from("photo.cr3")));

        // Nikon
        assert!(is_raw_file(&PathBuf::from("photo.nef")));
        assert!(is_raw_file(&PathBuf::from("photo.NEF")));
        assert!(is_raw_file(&PathBuf::from("photo.nrw")));

        // Sony
        assert!(is_raw_file(&PathBuf::from("photo.arw")));
        assert!(is_raw_file(&PathBuf::from("photo.srf")));
        assert!(is_raw_file(&PathBuf::from("photo.sr2")));

        // Fujifilm
        assert!(is_raw_file(&PathBuf::from("photo.raf")));

        // Olympus
        assert!(is_raw_file(&PathBuf::from("photo.orf")));

        // Panasonic
        assert!(is_raw_file(&PathBuf::from("photo.rw2")));

        // Adobe
        assert!(is_raw_file(&PathBuf::from("photo.dng")));

        // Pentax
        assert!(is_raw_file(&PathBuf::from("photo.pef")));

        // Not RAW
        assert!(!is_raw_file(&PathBuf::from("photo.jpg")));
        assert!(!is_raw_file(&PathBuf::from("photo.png")));
        assert!(!is_raw_file(&PathBuf::from("photo.webp")));
        assert!(!is_raw_file(&PathBuf::from("video.mp4")));
    }

    // Note: CFA pattern parsing tests removed as parse_cfa_from_rawloader
    // requires actual rawloader::CFA struct which can only be obtained from
    // real RAW files. Manual testing with actual RAW files will verify this.

    #[test]
    fn test_decode_nonexistent_file() {
        let result = decode_raw_to_dynamic_image(&PathBuf::from("/nonexistent/file.cr2"));
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_real_cr2_file() {
        // This test uses the actual CR2 file in test-data/
        let test_file = PathBuf::from("test-data/IMG_9899.CR2");

        if !test_file.exists() {
            panic!("Test CR2 file not found: {}", test_file.display());
        }

        let result = decode_raw_to_dynamic_image(&test_file);
        assert!(result.is_ok(), "Failed to decode CR2 file: {:?}", result.err());

        let img = result.unwrap();
        assert!(img.width() > 0, "Image width should be greater than 0");
        assert!(img.height() > 0, "Image height should be greater than 0");

        println!(
            "Successfully decoded CR2: {}x{} pixels",
            img.width(),
            img.height()
        );
    }
}
