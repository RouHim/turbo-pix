use image::{DynamicImage, ImageBuffer, Rgb};
use log::{debug, warn};
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RawError {
    #[error("Failed to decode RAW file: {0}")]
    DecodeError(String),
    #[error("Unsupported RAW format")]
    UnsupportedFormat,
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Decode a RAW image file to a DynamicImage
pub fn decode_raw_to_dynamic_image(path: &Path) -> Result<DynamicImage, RawError> {
    debug!("Decoding RAW file: {}", path.display());

    // 1. Use rawloader to decode
    let raw_image = rawloader::decode_file(path)
        .map_err(|e| RawError::DecodeError(format!("{:?}", e)))?;

    // 2. Extract bayer pattern data
    let (width, height, data) = match raw_image.data {
        rawloader::RawImageData::Integer(data) => (raw_image.width, raw_image.height, data),
        rawloader::RawImageData::Float(data) => {
            // Convert float data to u16
            let int_data: Vec<u16> = data
                .iter()
                .map(|&f| (f.clamp(0.0, 65535.0) as u16))
                .collect();
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
    // Note: bayer crate requires specific CFA pattern format
    let cfa = match raw_image.cfa {
        rawloader::CFA::Empty => {
            warn!("No CFA pattern found, using default RGGB");
            bayer::CFA::RGGB
        }
        rawloader::CFA::Standard(ref pattern) => {
            // Convert rawloader CFA to bayer CFA
            parse_cfa_pattern(pattern)?
        }
    };

    let rgb_data = bayer::demosaic::run_demosaic(
        &data,
        width,
        height,
        cfa,
        bayer::RasterDepth::Depth16,
        bayer::Demosaic::Linear, // Fast linear interpolation
    )
    .map_err(|e| RawError::DecodeError(format!("Demosaic failed: {:?}", e)))?;

    debug!("Demosaic completed, RGB data size: {}", rgb_data.len());

    // 4. Convert to image::DynamicImage
    // bayer outputs u16 data, we need to convert to u8 for DynamicImage
    let rgb8_data: Vec<u8> = rgb_data.iter().map(|&val| (val >> 8) as u8).collect();

    let img_buffer: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_raw(width as u32, height as u32, rgb8_data).ok_or_else(|| {
            RawError::DecodeError("Buffer conversion failed: invalid dimensions".to_string())
        })?;

    Ok(DynamicImage::ImageRgb8(img_buffer))
}

/// Parse CFA pattern from rawloader format to bayer format
fn parse_cfa_pattern(pattern: &str) -> Result<bayer::CFA, RawError> {
    match pattern {
        "RGGB" => Ok(bayer::CFA::RGGB),
        "BGGR" => Ok(bayer::CFA::BGGR),
        "GRBG" => Ok(bayer::CFA::GRBG),
        "GBRG" => Ok(bayer::CFA::GBRG),
        _ => {
            warn!("Unknown CFA pattern '{}', using RGGB", pattern);
            Ok(bayer::CFA::RGGB)
        }
    }
}

/// Check if a file is a RAW image file based on extension
pub fn is_raw_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_lowercase().as_str(),
                "cr2" | "cr3" | "nef" | "nrw" | "arw" | "srf" | "sr2" | "raf" | "orf" | "rw2"
                    | "dng" | "pef"
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

    #[test]
    fn test_parse_cfa_pattern() {
        assert!(matches!(
            parse_cfa_pattern("RGGB").unwrap(),
            bayer::CFA::RGGB
        ));
        assert!(matches!(
            parse_cfa_pattern("BGGR").unwrap(),
            bayer::CFA::BGGR
        ));
        assert!(matches!(
            parse_cfa_pattern("GRBG").unwrap(),
            bayer::CFA::GRBG
        ));
        assert!(matches!(
            parse_cfa_pattern("GBRG").unwrap(),
            bayer::CFA::GBRG
        ));

        // Unknown pattern should default to RGGB
        assert!(matches!(
            parse_cfa_pattern("UNKNOWN").unwrap(),
            bayer::CFA::RGGB
        ));
    }

    #[test]
    fn test_decode_nonexistent_file() {
        let result = decode_raw_to_dynamic_image(&PathBuf::from("/nonexistent/file.cr2"));
        assert!(result.is_err());
    }
}
