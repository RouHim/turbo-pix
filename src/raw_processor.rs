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
    let (width, height, mut data) = match raw_image.data {
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

    // 3. Get CFA pattern
    let cfa_pattern = parse_cfa_from_rawloader(&raw_image.cfa)?;

    // 4. Apply RAW processing pipeline
    // Step 1: Black level subtraction and normalization
    apply_black_white_levels(
        &mut data,
        width,
        height,
        &raw_image.blacklevels,
        &raw_image.whitelevels,
        cfa_pattern,
    );

    // Step 2: Apply white balance
    apply_white_balance(&mut data, width, height, &raw_image.wb_coeffs, cfa_pattern);

    // Step 3: Demosaic to RGB
    let mut rgb16_data = simple_demosaic_16bit(&data, width, height, cfa_pattern)?;

    // Step 4: Apply saturation boost (enhance colors)
    apply_saturation_boost(&mut rgb16_data, width, height, 2.0);

    // Step 5: Apply gamma correction (sRGB 2.2)
    apply_gamma_correction(&mut rgb16_data);

    // Step 6: Convert to 8-bit
    let rgb8_data: Vec<u8> = rgb16_data.iter().map(|&v| (v >> 8) as u8).collect();

    debug!(
        "RAW processing completed, RGB data size: {}",
        rgb8_data.len()
    );

    let img_buffer: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_raw(width as u32, height as u32, rgb8_data).ok_or_else(|| {
            RawError::DecodeError("Buffer conversion failed: invalid dimensions".to_string())
        })?;

    Ok(DynamicImage::ImageRgb8(img_buffer))
}

/// Apply black level subtraction and normalize to full 16-bit range
fn apply_black_white_levels(
    data: &mut [u16],
    width: usize,
    height: usize,
    blacklevels: &[u16],
    whitelevels: &[u16],
    _cfa: bayer::CFA,
) {
    // Get average black and white levels (per-channel if available)
    let black = if blacklevels.is_empty() {
        0u16
    } else {
        (blacklevels.iter().filter(|&&b| b > 0).sum::<u16>() as f32
            / blacklevels.iter().filter(|&&b| b > 0).count().max(1) as f32) as u16
    };

    let white = if whitelevels.is_empty() {
        65535u16
    } else {
        whitelevels[0]
    };

    let range = (white - black) as f32;

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let pixel = data[idx];

            // Subtract black level
            let normalized = pixel.saturating_sub(black);

            // Scale to full 16-bit range
            let scaled = ((normalized as f32 / range) * 65535.0).min(65535.0) as u16;
            data[idx] = scaled;
        }
    }
}

/// Apply white balance coefficients
fn apply_white_balance(
    data: &mut [u16],
    width: usize,
    height: usize,
    wb_coeffs: &[f32],
    cfa: bayer::CFA,
) {
    // Extract RGB multipliers from wb_coeffs
    // wb_coeffs format: [R, G, B, G2] where G2 might be NaN
    if wb_coeffs.len() < 3 {
        warn!("Insufficient white balance coefficients, skipping WB");
        return;
    }

    // Normalize so green = 1.0 (typical reference)
    let g_ref = wb_coeffs[1];
    if g_ref <= 0.0 || !g_ref.is_finite() {
        warn!("Invalid green WB coefficient, skipping WB");
        return;
    }

    let r_mult = wb_coeffs[0] / g_ref;
    let g_mult = 1.0;
    let b_mult = wb_coeffs[2] / g_ref;

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let pixel = data[idx] as f32;

            // Apply multiplier based on CFA position
            let multiplier = match cfa {
                bayer::CFA::RGGB => match (y % 2, x % 2) {
                    (0, 0) => r_mult, // R pixel
                    (0, 1) => g_mult, // G pixel
                    (1, 0) => g_mult, // G pixel
                    (1, 1) => b_mult, // B pixel
                    _ => unreachable!(),
                },
                _ => 1.0, // Fallback for other CFA patterns
            };

            let adjusted = (pixel * multiplier).min(65535.0) as u16;
            data[idx] = adjusted;
        }
    }
}

/// Apply saturation boost to enhance colors
/// Uses luminance-preserving saturation adjustment in RGB space
fn apply_saturation_boost(
    rgb_data: &mut [u16],
    width: usize,
    height: usize,
    saturation_factor: f32,
) {
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) * 3;

            let r = rgb_data[idx] as f32;
            let g = rgb_data[idx + 1] as f32;
            let b = rgb_data[idx + 2] as f32;

            // Calculate luminance (rec. 709 coefficients)
            let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;

            // Apply saturation adjustment
            // New color = luminance + saturation_factor * (original_color - luminance)
            let r_adjusted = luminance + saturation_factor * (r - luminance);
            let g_adjusted = luminance + saturation_factor * (g - luminance);
            let b_adjusted = luminance + saturation_factor * (b - luminance);

            // Clamp to valid range
            rgb_data[idx] = r_adjusted.clamp(0.0, 65535.0) as u16;
            rgb_data[idx + 1] = g_adjusted.clamp(0.0, 65535.0) as u16;
            rgb_data[idx + 2] = b_adjusted.clamp(0.0, 65535.0) as u16;
        }
    }
}

/// Apply sRGB gamma correction (gamma 2.2)
fn apply_gamma_correction(rgb_data: &mut [u16]) {
    // Apply sRGB gamma: out = in^(1/2.2)
    const GAMMA: f32 = 1.0 / 2.2;

    for pixel in rgb_data.iter_mut() {
        let normalized = *pixel as f32 / 65535.0;
        let gamma_corrected = normalized.powf(GAMMA);
        *pixel = (gamma_corrected * 65535.0).min(65535.0) as u16;
    }
}

/// 16-bit demosaic algorithm (nearest-neighbor)
#[allow(clippy::needless_range_loop)]
fn simple_demosaic_16bit(
    data: &[u16],
    width: usize,
    height: usize,
    cfa: bayer::CFA,
) -> Result<Vec<u16>, RawError> {
    let mut rgb_data = vec![0u16; width * height * 3];

    // Helper to get pixel value safely
    let get_pixel = |x: usize, y: usize| -> u16 {
        if x < width && y < height {
            data[y * width + x]
        } else {
            0
        }
    };

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let pixel = get_pixel(x, y);

            // Determine RGB values based on CFA pattern and position
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
        assert!(
            result.is_ok(),
            "Failed to decode CR2 file: {:?}",
            result.err()
        );

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
