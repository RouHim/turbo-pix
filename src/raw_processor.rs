use image::{DynamicImage, ImageBuffer, Rgb};
use log::{debug, warn};
use std::path::Path;
use thiserror::Error;

/// Caps concurrent RAW decodes (full-resolution demosaic holds several
/// buffers per request — a 45MP sensor can be hundreds of MB). Shared by
/// `get_photo_file` and the thumbnail generator, both reachable through the
/// unauthenticated API.
pub static RAW_DECODE_LIMIT: std::sync::LazyLock<tokio::sync::Semaphore> =
    std::sync::LazyLock::new(|| tokio::sync::Semaphore::new(4));

#[derive(Error, Debug)]
pub enum RawError {
    #[error("Failed to decode RAW file: {0}")]
    DecodeError(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Color Filter Array pattern for Bayer images
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CFA {
    /// Red-Green-Green-Blue pattern
    RGGB,
    /// Blue-Green-Green-Red pattern
    BGGR,
    /// Green-Red-Blue-Green pattern
    GRBG,
    /// Green-Blue-Red-Green pattern
    GBRG,
}

impl CFA {
    /// Shift CFA pattern horizontally (for crop adjustments)
    fn next_x(self) -> Self {
        match self {
            CFA::RGGB => CFA::GRBG,
            CFA::GRBG => CFA::RGGB,
            CFA::BGGR => CFA::GBRG,
            CFA::GBRG => CFA::BGGR,
        }
    }

    /// Shift CFA pattern vertically (for crop adjustments)
    fn next_y(self) -> Self {
        match self {
            CFA::RGGB => CFA::GBRG,
            CFA::GBRG => CFA::RGGB,
            CFA::BGGR => CFA::GRBG,
            CFA::GRBG => CFA::BGGR,
        }
    }

    /// Offset of this pattern's 2x2 basis relative to RGGB's. Every Bayer
    /// pattern is a horizontal/vertical shift of the RGGB basis:
    /// RGGB (0,0), GRBG (1,0), GBRG (0,1), BGGR (1,1). The channel at image
    /// position (x, y) is the RGGB-basis channel at ((x+dx) % 2, (y+dy) % 2).
    fn basis_offset(self) -> (usize, usize) {
        match self {
            CFA::RGGB => (0, 0),
            CFA::GRBG => (1, 0),
            CFA::GBRG => (0, 1),
            CFA::BGGR => (1, 1),
        }
    }

    /// Bayer channel role at image position (x, y) for this pattern.
    fn channel_at(self, x: usize, y: usize) -> BayerChannel {
        let (dx, dy) = self.basis_offset();
        match ((x + dx) % 2, (y + dy) % 2) {
            (0, 0) => BayerChannel::Red,
            (0, 1) | (1, 0) => BayerChannel::Green,
            _ => BayerChannel::Blue,
        }
    }

    /// Basis-space coordinates of position (x, y): the 2x2 block's origin
    /// (0,0) is the pattern's Red pixel, (1,1) its Blue pixel.
    fn basis_coords(self, x: usize, y: usize) -> (usize, usize) {
        let (dx, dy) = self.basis_offset();
        ((x + dx) % 2, (y + dy) % 2)
    }
}

/// Bayer channel role for a 2x2 position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BayerChannel {
    Red,
    Green,
    Blue,
}

/// Decode a RAW image file to a DynamicImage
pub fn decode_raw_to_dynamic_image(path: &Path) -> Result<DynamicImage, RawError> {
    debug!("Decoding RAW file: {}", path.display());

    // 1. Use rawloader to decode
    let raw_image =
        rawloader::decode_file(path).map_err(|e| RawError::DecodeError(format!("{:?}", e)))?;

    // 2. Extract bayer pattern data
    let (mut width, mut height, mut data) = match raw_image.data {
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
    let mut cfa_pattern = parse_cfa_from_rawloader(&raw_image.cfa)?;

    // 4. Crop to active area to remove sensor black borders
    let crop_top = raw_image.crops[0];
    let crop_right = raw_image.crops[1];
    let crop_bottom = raw_image.crops[2];
    let crop_left = raw_image.crops[3];
    let crop_horiz = crop_left.saturating_add(crop_right);
    let crop_vert = crop_top.saturating_add(crop_bottom);

    if crop_horiz > 0 || crop_vert > 0 {
        if crop_horiz >= width || crop_vert >= height {
            warn!(
                "RAW crop values too large (top={}, right={}, bottom={}, left={}); skipping",
                crop_top, crop_right, crop_bottom, crop_left
            );
        } else {
            let crop_width = width.saturating_sub(crop_horiz);
            let crop_height = height.saturating_sub(crop_vert);
            let mut cropped = vec![0u16; crop_width * crop_height];

            for y in 0..crop_height {
                let src_row = (y + crop_top) * width + crop_left;
                let dst_row = y * crop_width;
                let src = &data[src_row..src_row + crop_width];
                let dst = &mut cropped[dst_row..dst_row + crop_width];
                dst.copy_from_slice(src);
            }

            data = cropped;
            width = crop_width;
            height = crop_height;

            if crop_left % 2 == 1 {
                cfa_pattern = cfa_pattern.next_x();
            }
            if crop_top % 2 == 1 {
                cfa_pattern = cfa_pattern.next_y();
            }

            debug!(
                "Applied RAW crops top={} right={} bottom={} left={} -> {}x{}",
                crop_top, crop_right, crop_bottom, crop_left, width, height
            );
        }
    }

    // 5. Apply RAW processing pipeline
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
    _cfa: CFA,
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
fn apply_white_balance(data: &mut [u16], width: usize, height: usize, wb_coeffs: &[f32], cfa: CFA) {
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

            // Apply multiplier based on the channel role at this CFA position
            // (works for all four Bayer patterns, not just RGGB — a wrong
            // multiplier would tint the image).
            let multiplier = match cfa.channel_at(x, y) {
                BayerChannel::Red => r_mult,
                BayerChannel::Green => g_mult,
                BayerChannel::Blue => b_mult,
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
    cfa: CFA,
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
            let (bx, by) = cfa.basis_coords(x, y);

            // Nearest-neighbor demosaic for ALL four Bayer patterns: each
            // channel reads the pixel whose basis position carries that
            // channel. Basis coordinates map to image offsets: Red is at
            // (-bx, -by), Blue at (1-bx, 1-by), and Green is the pixel
            // itself on the green rows, else the nearest green basis
            // position ((1-bx, -by) for Red positions, (-bx, 1-by) for
            // Blue positions — matching the RGGB neighbor choices).
            let (r, g, b) = match cfa.channel_at(x, y) {
                BayerChannel::Red => (
                    pixel,
                    get_pixel(x + 1 - bx, y.wrapping_sub(by)),
                    get_pixel(x + 1 - bx, y + 1 - by),
                ),
                BayerChannel::Green => (
                    get_pixel(x.wrapping_sub(bx), y.wrapping_sub(by)),
                    pixel,
                    get_pixel(x + 1 - bx, y + 1 - by),
                ),
                BayerChannel::Blue => (
                    get_pixel(x.wrapping_sub(bx), y.wrapping_sub(by)),
                    get_pixel(x.wrapping_sub(bx), y + 1 - by),
                    pixel,
                ),
            };

            let out_idx = idx * 3;
            rgb_data[out_idx] = r;
            rgb_data[out_idx + 1] = g;
            rgb_data[out_idx + 2] = b;
        }
    }

    Ok(rgb_data)
}

/// Parse CFA pattern from rawloader format to our CFA enum
fn parse_cfa_from_rawloader(cfa: &rawloader::CFA) -> Result<CFA, RawError> {
    // Try to get the pattern name from the CFA
    // rawloader CFA is a struct with pattern information
    let pattern_name = format!("{:?}", cfa);

    // Extract pattern from debug string (e.g., "CFA { name: \"RGGB\", ... }" -> "RGGB")
    if pattern_name.contains("RGGB") {
        Ok(CFA::RGGB)
    } else if pattern_name.contains("BGGR") {
        Ok(CFA::BGGR)
    } else if pattern_name.contains("GRBG") {
        Ok(CFA::GRBG)
    } else if pattern_name.contains("GBRG") {
        Ok(CFA::GBRG)
    } else {
        warn!("Unknown CFA pattern, using default RGGB");
        Ok(CFA::RGGB)
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

    #[test]
    fn test_channel_at_all_cfa_patterns() {
        // The 2x2 block of each pattern must place R/G/G/B at the right
        // positions: RGGB = [[R,G],[G,B]], BGGR = [[B,G],[G,R]],
        // GRBG = [[G,R],[B,G]], GBRG = [[G,B],[R,G]].
        use BayerChannel::{Blue, Green, Red};
        let block = |cfa: CFA| -> Vec<BayerChannel> {
            (0..2)
                .flat_map(|y| (0..2).map(move |x| cfa.channel_at(x, y)))
                .collect()
        };
        assert_eq!(block(CFA::RGGB), vec![Red, Green, Green, Blue]);
        assert_eq!(block(CFA::BGGR), vec![Blue, Green, Green, Red]);
        assert_eq!(block(CFA::GRBG), vec![Green, Red, Blue, Green]);
        assert_eq!(block(CFA::GBRG), vec![Green, Blue, Red, Green]);

        // Shift consistency: next_x/next_y must match the basis offsets.
        assert_eq!(CFA::RGGB.next_x(), CFA::GRBG);
        assert_eq!(CFA::RGGB.next_y(), CFA::GBRG);
        assert_eq!(CFA::GRBG.next_x(), CFA::RGGB);
        assert_eq!(CFA::GBRG.next_y(), CFA::RGGB);
    }

    #[test]
    fn test_demosaic_all_cfa_patterns_are_colored() {
        // A gray-ish flat field must demosaic to gray (r≈g≈b) for every
        // pattern — the old fallback (pixel,pixel,pixel) also passed this,
        // so additionally a pure-red corner (R pixel on) must produce a red
        // channel in the right place for EVERY pattern, not just RGGB.
        let width = 4usize;
        let height = 4usize;
        for cfa in [CFA::RGGB, CFA::BGGR, CFA::GRBG, CFA::GBRG] {
            // All pixels 1000: demosaic should keep channels within 1% of
            // each other (neighbors all read 1000).
            let data = vec![1000u16; width * height];
            let rgb = simple_demosaic_16bit(&data, width, height, cfa).unwrap();
            // Border pixels lack their 2x2 block's far channels (nearest-
            // neighbor reads out of bounds -> 0), so only interior pixels
            // must be gray.
            for y in 1..height - 1 {
                for x in 1..width - 1 {
                    let (r, g, b) = (
                        rgb[(y * width + x) * 3],
                        rgb[(y * width + x) * 3 + 1],
                        rgb[(y * width + x) * 3 + 2],
                    );
                    assert!(
                        (r as i64 - g as i64).abs() <= 10 && (g as i64 - b as i64).abs() <= 10,
                        "flat field not gray for {cfa:?} at ({x},{y}): ({r},{g},{b})"
                    );
                }
            }

            // Light up ONLY the Red pixel of the 2x2 basis and check the
            // demosaic puts the bright value in the red channel at that
            // position for every pattern.
            let mut data = vec![0u16; width * height];
            let (dx, dy) = cfa.basis_offset();
            for y in 0..height {
                for x in 0..width {
                    if ((x + dx) % 2, (y + dy) % 2) == (0, 0) {
                        data[y * width + x] = 65535;
                    }
                }
            }
            let rgb = simple_demosaic_16bit(&data, width, height, cfa).unwrap();
            // Image position of the pattern's Red pixel (basis (0,0)): the
            // lit cells above are exactly where basis (0,0) lands.
            let (rx, ry) = ((2 - dx) % 2, (2 - dy) % 2);
            let out_idx = (ry * width + rx) * 3;
            assert!(
                rgb[out_idx] > 30000,
                "red channel dim for {cfa:?}: {} at red pixel",
                rgb[out_idx]
            );
            assert!(
                rgb[out_idx + 2] < 1000,
                "blue channel should be dark at red pixel for {cfa:?}: {}",
                rgb[out_idx + 2]
            );
        }
    }

    #[test]
    fn test_white_balance_applies_per_channel_all_cfa_patterns() {
        // A hot R pixel must be scaled by r_mult and NOT by b_mult for every
        // pattern — a pattern-blind multiplier would tint the image.
        let width = 2usize;
        let height = 2usize;
        let wb = [2.0f32, 1.0, 0.5]; // r=2x, g=1x, b=0.5x (normalized below)
        for cfa in [CFA::RGGB, CFA::BGGR, CFA::GRBG, CFA::GBRG] {
            let mut data = vec![1000u16; width * height];
            apply_white_balance(&mut data, width, height, &wb, cfa);
            // r_mult = 2.0/1.0 = 2.0, b_mult = 0.5/1.0 = 0.5
            for y in 0..height {
                for x in 0..width {
                    let expected = match cfa.channel_at(x, y) {
                        BayerChannel::Red => 2000,
                        BayerChannel::Green => 1000,
                        BayerChannel::Blue => 500,
                    };
                    assert_eq!(
                        data[y * width + x],
                        expected,
                        "WB multiplier wrong for {cfa:?} at ({x},{y})"
                    );
                }
            }
        }
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
