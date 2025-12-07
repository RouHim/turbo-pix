use chrono::{DateTime, Duration, NaiveDate, Utc};
use image::{DynamicImage, ImageBuffer, Rgba, RgbaImage};
use log::{error, info};
use rusqlite::{params, Row};
use rusttype::{point, Font, Scale};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::db::Photo;
use crate::db_pool::DbPool;
use crate::file_scanner::PhotoFile;
use crate::photo_processor::PhotoProcessor;
use crate::raw_processor;

/// Collage entity representing a generated photo collage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collage {
    pub id: i64,
    pub date: String, // Format: YYYY-MM-DD
    pub file_path: String,
    pub thumbnail_path: Option<String>,
    pub photo_count: i32,
    pub photo_hashes: Vec<String>, // JSON array of hashes
    pub accepted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl Collage {
    /// Parse from SQLite row
    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        let photo_hashes_json: String = row.get(5)?;
        let photo_hashes: Vec<String> =
            serde_json::from_str(&photo_hashes_json).unwrap_or_default();

        Ok(Collage {
            id: row.get(0)?,
            date: row.get(1)?,
            file_path: row.get(2)?,
            thumbnail_path: row.get(3)?,
            photo_count: row.get(4)?,
            photo_hashes,
            accepted_at: row
                .get::<_, Option<String>>(6)?
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc)),
            created_at: row
                .get::<_, Option<String>>(7)?
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(Utc::now),
        })
    }

    /// List all pending collages
    pub fn list_pending(pool: &DbPool) -> Result<Vec<Self>, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, date, file_path, thumbnail_path, photo_count, photo_hashes,
                    accepted_at, created_at
             FROM collages
             WHERE accepted_at IS NULL
             ORDER BY date DESC",
        )?;

        let collages = stmt
            .query_map([], Self::from_row)?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(collages)
    }

    /// Get collage by ID
    pub fn get_by_id(pool: &DbPool, id: i64) -> Result<Option<Self>, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, date, file_path, thumbnail_path, photo_count, photo_hashes,
                    accepted_at, created_at
             FROM collages
             WHERE id = ?",
        )?;

        match stmt.query_row([id], Self::from_row) {
            Ok(collage) => Ok(Some(collage)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(Box::new(e)),
        }
    }

    /// Check if collage exists for date
    /// Insert new collage
    pub fn insert(
        pool: &DbPool,
        date: &str,
        file_path: &str,
        thumbnail_path: Option<&str>,
        photo_count: i32,
        photo_hashes: &[String],
    ) -> Result<i64, Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        let photo_hashes_json = serde_json::to_string(photo_hashes)?;

        conn.execute(
            "INSERT INTO collages (date, file_path, thumbnail_path, photo_count, photo_hashes)
             VALUES (?, ?, ?, ?, ?)",
            params![
                date,
                file_path,
                thumbnail_path,
                photo_count,
                photo_hashes_json
            ],
        )?;

        Ok(conn.last_insert_rowid())
    }

    /// Mark collage as accepted and update file path
    pub fn accept(
        pool: &DbPool,
        id: i64,
        new_file_path: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        conn.execute(
            "UPDATE collages SET accepted_at = CURRENT_TIMESTAMP, file_path = ? WHERE id = ?",
            [new_file_path, &id.to_string()],
        )?;
        Ok(())
    }

    /// Delete collage
    pub fn delete(pool: &DbPool, id: i64) -> Result<(), Box<dyn std::error::Error>> {
        let conn = pool.get()?;
        conn.execute("DELETE FROM collages WHERE id = ?", [id])?;
        Ok(())
    }
}

/// Photo cluster representing photos taken on the same day
#[derive(Debug)]
struct PhotoCluster {
    date: NaiveDate,
    photos: Vec<Photo>,
}

const MAX_PHOTOS_PER_COLLAGE: usize = 4;
const COLLAGE_WIDTH: u32 = 3840;
const COLLAGE_HEIGHT: u32 = 2160;
const COLLAGE_PADDING: u32 = 60;
const COLLAGE_HEADER_HEIGHT: u32 = 240;
const COLLAGE_GUTTER: u32 = 20;
const FRAME_THICKNESS: u32 = 8;

/// Photo orientation classification
#[derive(Debug, Clone, Copy, PartialEq)]
enum Orientation {
    Portrait,  // aspect < 0.9
    Landscape, // aspect > 1.1
    Square,    // 0.9 <= aspect <= 1.1
}

/// Photo analysis information
#[derive(Debug, Clone)]
struct PhotoInfo {
    aspect_ratio: f32,
    orientation: Orientation,
}

impl PhotoInfo {
    fn new(width: u32, height: u32) -> Self {
        let aspect_ratio = width as f32 / height as f32;
        let orientation = if aspect_ratio < 0.9 {
            Orientation::Portrait
        } else if aspect_ratio > 1.1 {
            Orientation::Landscape
        } else {
            Orientation::Square
        };
        PhotoInfo {
            aspect_ratio,
            orientation,
        }
    }
}

/// Layout template types
#[derive(Debug, Clone, Copy, PartialEq)]
enum LayoutTemplate {
    Single,
    TwoSideBySide,
    TwoStacked,
    ThreeFocal,   // 60/40 split
    ThreeLinear,  // 33/33/33 horizontal
    ThreePyramid, // Top 50%, bottom 25/25
    FourGrid,     // 2x2 grid
    FourFocal,    // 50% focal + 3 small
}

/// Collage layout configuration
struct CollageLayout {
    photo_count: usize,
    photo_cells: Vec<Rect>,
}

impl CollageLayout {
    /// Calculate optimal layout using smart template selection based on photo characteristics
    fn calculate(photos: &[&Photo]) -> Self {
        let photo_count = photos.len().clamp(1, MAX_PHOTOS_PER_COLLAGE);

        // Analyze photos to get aspect ratios and orientations
        let photo_infos = analyze_photos(photos);

        // Select the best template based on photo characteristics
        let template = select_best_template(photo_count, &photo_infos);

        // Use padded content area to leave room for header and framing
        let content_width = COLLAGE_WIDTH.saturating_sub(COLLAGE_PADDING * 2);
        let content_height =
            COLLAGE_HEIGHT.saturating_sub(COLLAGE_HEADER_HEIGHT + COLLAGE_PADDING * 2);

        let start_x = COLLAGE_PADDING;
        let start_y = COLLAGE_HEADER_HEIGHT + COLLAGE_PADDING;

        // Generate cells using the selected template
        let photo_cells =
            generate_template_cells(template, content_width, content_height, start_x, start_y);

        CollageLayout {
            photo_count,
            photo_cells,
        }
    }
}

#[derive(Clone, Copy)]
struct Rect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl Rect {
    fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn max_x(&self) -> u32 {
        self.x.saturating_add(self.width)
    }

    fn max_y(&self) -> u32 {
        self.y.saturating_add(self.height)
    }
}

/// Analyze photos to extract aspect ratios and orientations
fn analyze_photos(photos: &[&Photo]) -> Vec<PhotoInfo> {
    photos
        .iter()
        .map(|photo| {
            // Extract dimensions from photo metadata or use defaults
            let (width, height) = match (photo.width, photo.height) {
                (Some(w), Some(h)) if w > 0 && h > 0 => (w as u32, h as u32),
                _ => (3, 2), // Default landscape aspect for missing metadata
            };
            PhotoInfo::new(width, height)
        })
        .collect()
}

/// Generate layout cells for a specific template
fn generate_template_cells(
    template: LayoutTemplate,
    content_width: u32,
    content_height: u32,
    start_x: u32,
    start_y: u32,
) -> Vec<Rect> {
    match template {
        LayoutTemplate::Single => {
            vec![Rect::new(start_x, start_y, content_width, content_height)]
        }
        LayoutTemplate::TwoSideBySide => {
            let cell_width = (content_width.saturating_sub(COLLAGE_GUTTER)) / 2;
            vec![
                Rect::new(start_x, start_y, cell_width, content_height),
                Rect::new(
                    start_x + cell_width + COLLAGE_GUTTER,
                    start_y,
                    cell_width,
                    content_height,
                ),
            ]
        }
        LayoutTemplate::TwoStacked => {
            let cell_height = (content_height.saturating_sub(COLLAGE_GUTTER)) / 2;
            vec![
                Rect::new(start_x, start_y, content_width, cell_height),
                Rect::new(
                    start_x,
                    start_y + cell_height + COLLAGE_GUTTER,
                    content_width,
                    cell_height,
                ),
            ]
        }
        LayoutTemplate::ThreeFocal => {
            // 60/40 split: large left, two stacked right
            let left_width = (content_width * 60) / 100;
            let right_width = content_width.saturating_sub(left_width + COLLAGE_GUTTER);
            let right_cell_height = (content_height.saturating_sub(COLLAGE_GUTTER)) / 2;
            vec![
                Rect::new(start_x, start_y, left_width, content_height),
                Rect::new(
                    start_x + left_width + COLLAGE_GUTTER,
                    start_y,
                    right_width,
                    right_cell_height,
                ),
                Rect::new(
                    start_x + left_width + COLLAGE_GUTTER,
                    start_y + right_cell_height + COLLAGE_GUTTER,
                    right_width,
                    right_cell_height,
                ),
            ]
        }
        LayoutTemplate::ThreeLinear => {
            // Three columns using golden ratio proportions
            let cell_width = (content_width.saturating_sub(COLLAGE_GUTTER * 2)) / 3;
            vec![
                Rect::new(start_x, start_y, cell_width, content_height),
                Rect::new(
                    start_x + cell_width + COLLAGE_GUTTER,
                    start_y,
                    cell_width,
                    content_height,
                ),
                Rect::new(
                    start_x + (cell_width + COLLAGE_GUTTER) * 2,
                    start_y,
                    cell_width,
                    content_height,
                ),
            ]
        }
        LayoutTemplate::ThreePyramid => {
            // Top 50%, bottom two 25% each
            let top_height = content_height / 2;
            let bottom_height = content_height.saturating_sub(top_height + COLLAGE_GUTTER);
            let bottom_width = (content_width.saturating_sub(COLLAGE_GUTTER)) / 2;
            vec![
                Rect::new(start_x, start_y, content_width, top_height),
                Rect::new(
                    start_x,
                    start_y + top_height + COLLAGE_GUTTER,
                    bottom_width,
                    bottom_height,
                ),
                Rect::new(
                    start_x + bottom_width + COLLAGE_GUTTER,
                    start_y + top_height + COLLAGE_GUTTER,
                    bottom_width,
                    bottom_height,
                ),
            ]
        }
        LayoutTemplate::FourGrid => {
            // Standard 2x2 grid
            let cell_width = (content_width.saturating_sub(COLLAGE_GUTTER)) / 2;
            let cell_height = (content_height.saturating_sub(COLLAGE_GUTTER)) / 2;
            vec![
                Rect::new(start_x, start_y, cell_width, cell_height),
                Rect::new(
                    start_x + cell_width + COLLAGE_GUTTER,
                    start_y,
                    cell_width,
                    cell_height,
                ),
                Rect::new(
                    start_x,
                    start_y + cell_height + COLLAGE_GUTTER,
                    cell_width,
                    cell_height,
                ),
                Rect::new(
                    start_x + cell_width + COLLAGE_GUTTER,
                    start_y + cell_height + COLLAGE_GUTTER,
                    cell_width,
                    cell_height,
                ),
            ]
        }
        LayoutTemplate::FourFocal => {
            // 50% focal on left, 3 small stacked on right
            let focal_width = content_width / 2;
            let right_width = content_width.saturating_sub(focal_width + COLLAGE_GUTTER);
            let right_cell_height = (content_height.saturating_sub(COLLAGE_GUTTER * 2)) / 3;
            vec![
                Rect::new(start_x, start_y, focal_width, content_height),
                Rect::new(
                    start_x + focal_width + COLLAGE_GUTTER,
                    start_y,
                    right_width,
                    right_cell_height,
                ),
                Rect::new(
                    start_x + focal_width + COLLAGE_GUTTER,
                    start_y + right_cell_height + COLLAGE_GUTTER,
                    right_width,
                    right_cell_height,
                ),
                Rect::new(
                    start_x + focal_width + COLLAGE_GUTTER,
                    start_y + (right_cell_height + COLLAGE_GUTTER) * 2,
                    right_width,
                    right_cell_height,
                ),
            ]
        }
    }
}

/// Score a template based on photo characteristics
fn score_template(template: LayoutTemplate, photo_infos: &[PhotoInfo]) -> f32 {
    let cells = generate_template_cells(
        template,
        COLLAGE_WIDTH.saturating_sub(COLLAGE_PADDING * 2),
        COLLAGE_HEIGHT.saturating_sub(COLLAGE_HEADER_HEIGHT + COLLAGE_PADDING * 2),
        0,
        0,
    );

    let mut total_score = 0.0;
    let count = photo_infos.len().min(cells.len());

    // Calculate aspect ratio compatibility (40% weight)
    for (info, cell) in photo_infos.iter().zip(cells.iter()).take(count) {
        let cell_aspect = cell.width as f32 / cell.height as f32;
        let diff = (info.aspect_ratio - cell_aspect).abs();
        let aspect_score = 1.0 - (diff / 2.0).min(1.0); // Normalize to 0-1
        total_score += aspect_score * 0.4;
    }

    // Orientation match score (30% weight)
    let landscape_count = photo_infos
        .iter()
        .filter(|i| i.orientation == Orientation::Landscape)
        .count();
    let portrait_count = photo_infos
        .iter()
        .filter(|i| i.orientation == Orientation::Portrait)
        .count();

    let orientation_score = match template {
        LayoutTemplate::TwoStacked | LayoutTemplate::ThreePyramid => {
            // Favor these for portrait photos
            if portrait_count > landscape_count {
                1.0
            } else {
                0.5
            }
        }
        LayoutTemplate::ThreeLinear => {
            // Favor for all landscape
            if landscape_count == photo_infos.len() {
                1.0
            } else {
                0.6
            }
        }
        _ => 0.7, // Default moderate score
    };
    total_score += orientation_score * 0.3;

    // Space utilization (30% weight)
    let mut utilization = 0.0;
    for (info, cell) in photo_infos.iter().zip(cells.iter()).take(count) {
        let cell_aspect = cell.width as f32 / cell.height as f32;
        let aspect_diff = (info.aspect_ratio - cell_aspect).abs() / cell_aspect;
        // Better utilization when aspects are close
        utilization += if aspect_diff < 0.20 { 1.0 } else { 0.7 };
    }
    total_score += (utilization / count as f32) * 0.3;

    total_score / count as f32
}

/// Select the best template for the given photos
fn select_best_template(photo_count: usize, photo_infos: &[PhotoInfo]) -> LayoutTemplate {
    let templates = match photo_count {
        1 => vec![LayoutTemplate::Single],
        2 => vec![LayoutTemplate::TwoSideBySide, LayoutTemplate::TwoStacked],
        3 => vec![
            LayoutTemplate::ThreeFocal,
            LayoutTemplate::ThreeLinear,
            LayoutTemplate::ThreePyramid,
        ],
        _ => vec![LayoutTemplate::FourGrid, LayoutTemplate::FourFocal],
    };

    templates
        .iter()
        .map(|&template| (template, score_template(template, photo_infos)))
        .max_by(|(_, score_a), (_, score_b)| {
            score_a
                .partial_cmp(score_b)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(template, _)| template)
        .unwrap_or(templates[0])
}

fn blend_pixel(base: &mut Rgba<u8>, overlay: &Rgba<u8>) {
    let alpha = overlay[3] as f32 / 255.0;
    if alpha <= 0.0 {
        return;
    }

    let inv_alpha = 1.0 - alpha;
    for idx in 0..3 {
        base[idx] = (overlay[idx] as f32 * alpha + base[idx] as f32 * inv_alpha)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    base[3] = 255;
}

fn stroke_rect(canvas: &mut RgbaImage, rect: &Rect, thickness: u32, color: Rgba<u8>) {
    if thickness == 0 {
        return;
    }

    for t in 0..thickness {
        let left = rect.x.saturating_add(t);
        let right = rect
            .max_x()
            .saturating_sub(1 + t)
            .min(canvas.width().saturating_sub(1));
        let top = rect.y.saturating_add(t);
        let bottom = rect
            .max_y()
            .saturating_sub(1 + t)
            .min(canvas.height().saturating_sub(1));

        if left >= right || top >= bottom {
            break;
        }

        for x in left..=right {
            {
                let top_pixel = canvas.get_pixel_mut(x, top);
                if color[3] == 255 {
                    *top_pixel = color;
                } else {
                    blend_pixel(top_pixel, &color);
                }
            }

            {
                let bottom_pixel = canvas.get_pixel_mut(x, bottom);
                if color[3] == 255 {
                    *bottom_pixel = color;
                } else {
                    blend_pixel(bottom_pixel, &color);
                }
            }
        }

        for y in top..=bottom {
            {
                let left_pixel = canvas.get_pixel_mut(left, y);
                if color[3] == 255 {
                    *left_pixel = color;
                } else {
                    blend_pixel(left_pixel, &color);
                }
            }

            {
                let right_pixel = canvas.get_pixel_mut(right, y);
                if color[3] == 255 {
                    *right_pixel = color;
                } else {
                    blend_pixel(right_pixel, &color);
                }
            }
        }
    }
}

fn format_date_label(date_str: &str) -> String {
    NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .map(|date| date.format("%A, %B %d, %Y").to_string())
        .unwrap_or_else(|_| date_str.to_string())
}

fn load_font() -> Result<Font<'static>, Box<dyn std::error::Error>> {
    let candidates: &[(&[u8], &str)] = &[
        (
            include_bytes!("../static/fonts/Questrial-Regular.ttf"),
            "Questrial Regular",
        ),
        (
            include_bytes!("../static/fonts/JetBrainsMono-Regular.ttf"),
            "JetBrains Mono Regular",
        ),
    ];

    for (bytes, name) in candidates {
        if let Some(font) = Font::try_from_bytes(bytes) {
            info!("Loaded collage font: {}", name);
            return Ok(font);
        } else {
            error!("Failed to parse collage font candidate: {}", name);
        }
    }

    Err("No collage font could be loaded".into())
}

fn draw_text(
    canvas: &mut RgbaImage,
    text: &str,
    font: &Font,
    scale: Scale,
    x: u32,
    y: u32,
    color: Rgba<u8>,
) {
    let v_metrics = font.v_metrics(scale);
    let glyphs: Vec<_> = font
        .layout(text, scale, point(0.0, v_metrics.ascent))
        .collect();

    for glyph in glyphs {
        if let Some(bb) = glyph.pixel_bounding_box() {
            glyph.draw(|gx, gy, gv| {
                let px = x as i32 + gx as i32 + bb.min.x;
                let py = y as i32 + gy as i32 + bb.min.y;

                if px < 0 || py < 0 || px >= canvas.width() as i32 || py >= canvas.height() as i32 {
                    return;
                }

                let alpha = (gv * color[3] as f32).round() as u8;
                let overlay = Rgba([color[0], color[1], color[2], alpha]);
                let pixel = canvas.get_pixel_mut(px as u32, py as u32);
                blend_pixel(pixel, &overlay);
            });
        }
    }
}

/// Find photo clusters (dates with ≥10 photos) in the last 365 days
fn find_photo_clusters(pool: &DbPool) -> Result<Vec<PhotoCluster>, Box<dyn std::error::Error>> {
    let conn = pool.get()?;

    // Get cutoff date (365 days ago)
    let cutoff_date = (Utc::now() - Duration::days(365)).to_rfc3339();

    // Find dates with ≥10 photos
    let mut stmt = conn.prepare(
        "SELECT DATE(taken_at) as photo_date, COUNT(*) as count
         FROM photos
         WHERE taken_at IS NOT NULL
           AND taken_at >= ?
         GROUP BY photo_date
         HAVING count >= 10
         ORDER BY photo_date DESC",
    )?;

    let dates: Vec<String> = stmt
        .query_map([&cutoff_date], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    let mut clusters = Vec::new();

    for date_str in dates {
        // Parse date
        let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")?;

        // Get all photos for this date
        let mut photo_stmt = conn.prepare(
            "SELECT * FROM photos
             WHERE DATE(taken_at) = ?
             ORDER BY taken_at",
        )?;

        let photos = photo_stmt
            .query_map([&date_str], Photo::from_row)?
            .collect::<Result<Vec<_>, _>>()?;

        if photos.len() >= 10 {
            clusters.push(PhotoCluster { date, photos });
        }
    }

    Ok(clusters)
}

/// Create collage image from photos
fn create_collage_image(
    photos: &[&Photo],
    layout: &CollageLayout,
    date_label: &str,
) -> Result<RgbaImage, Box<dyn std::error::Error>> {
    // Unified clean background - single color, no gradients or panels
    let mut canvas: RgbaImage =
        ImageBuffer::from_pixel(COLLAGE_WIDTH, COLLAGE_HEIGHT, Rgba([248, 250, 252, 255]));

    let font = load_font().map_err(|e| format!("Failed to load collage font: {}", e))?;

    // Draw date label at top with padding
    draw_text(
        &mut canvas,
        &format_date_label(date_label),
        &font,
        Scale { x: 140.0, y: 140.0 },
        COLLAGE_PADDING + 20,
        COLLAGE_PADDING + 20,
        Rgba([40, 50, 65, 255]),
    );

    for (idx, photo) in photos.iter().take(layout.photo_count).enumerate() {
        let cell = &layout.photo_cells[idx];

        // Load image: decode RAW files using raw_processor, otherwise use standard image loading
        let mut img = if raw_processor::is_raw_file(Path::new(&photo.file_path)) {
            match raw_processor::decode_raw_to_dynamic_image(Path::new(&photo.file_path)) {
                Ok(img) => img,
                Err(e) => {
                    error!("Failed to decode RAW file {}: {}", photo.file_path, e);
                    continue;
                }
            }
        } else {
            // For non-RAW files, use thumbnail if available, otherwise use original
            let image_path = photo.thumbnail_path.as_deref().unwrap_or(&photo.file_path);
            match image::open(image_path) {
                Ok(img) => img,
                Err(e) => {
                    error!("Failed to load image {}: {}", image_path, e);
                    continue;
                }
            }
        };

        // Apply orientation correction if needed
        if let Some(orientation) = photo.orientation {
            img = match orientation {
                3 => img.rotate180(),
                6 => img.rotate90(),
                8 => img.rotate270(),
                _ => img,
            };
        }

        // Always crop to fill cells for uniform, consistent appearance
        let resized = img.resize_to_fill(
            cell.width,
            cell.height,
            image::imageops::FilterType::Lanczos3,
        );
        let x_offset = cell.x;
        let y_offset = cell.y;

        // Convert to RGBA and manually copy pixels
        // Note: Using manual pixel copying instead of image::imageops::overlay
        // to ensure proper rendering of RAW-decoded images
        let rgba_img = resized.to_rgba8();

        for dy in 0..rgba_img.height() {
            for dx in 0..rgba_img.width() {
                let canvas_x = x_offset + dx;
                let canvas_y = y_offset + dy;

                if canvas_x < canvas.width() && canvas_y < canvas.height() {
                    let pixel = rgba_img.get_pixel(dx, dy);
                    canvas.put_pixel(canvas_x, canvas_y, *pixel);
                }
            }
        }

        let frame_rect = Rect::new(x_offset, y_offset, resized.width(), resized.height());
        stroke_rect(
            &mut canvas,
            &frame_rect,
            FRAME_THICKNESS,
            Rgba([80, 95, 115, 255]),
        );
    }

    Ok(canvas)
}

fn chunk_photos(photos: &[Photo]) -> Vec<Vec<&Photo>> {
    const MIN_PHOTOS_PER_COLLAGE: usize = 2;

    // Fill buckets sequentially (4, 4, 4, ..., remainder)
    // This maximizes the number of full 4-photo collages
    photos
        .chunks(MAX_PHOTOS_PER_COLLAGE)
        .map(|chunk| chunk.iter().collect())
        .filter(|bucket: &Vec<_>| bucket.len() >= MIN_PHOTOS_PER_COLLAGE)
        .collect()
}

/// Generate collages for all detected clusters
pub async fn generate_collages(
    pool: &DbPool,
    data_path: &Path,
) -> Result<usize, Box<dyn std::error::Error>> {
    info!("Starting collage generation...");

    // Create staging directory
    let staging_dir = data_path.join("collages").join("staging");
    std::fs::create_dir_all(&staging_dir)?;

    // Find clusters
    let clusters = find_photo_clusters(pool)?;
    info!("Found {} photo clusters to process", clusters.len());

    let mut generated_count = 0;

    for cluster in clusters {
        let date_str = cluster.date.format("%Y-%m-%d").to_string();
        let chunks = chunk_photos(&cluster.photos);

        if chunks.is_empty() {
            info!(
                "No photos found for {}; skipping collage generation",
                date_str
            );
            continue;
        }

        info!(
            "Generating {} collages for {} ({} photos total)",
            chunks.len(),
            date_str,
            cluster.photos.len()
        );

        for (collage_idx, chunk) in chunks.iter().enumerate() {
            // Calculate layout for the current chunk using smart template selection
            let layout = CollageLayout::calculate(chunk);

            // Create collage image
            let collage_img = match create_collage_image(chunk, &layout, &date_str) {
                Ok(img) => img,
                Err(e) => {
                    error!(
                        "Failed to create collage {} for {}: {}",
                        collage_idx + 1,
                        date_str,
                        e
                    );
                    continue;
                }
            };

            // Save collage
            let filename = format!("collage_{}_{}.jpg", date_str, collage_idx + 1);
            let file_path = staging_dir.join(&filename);
            let img = DynamicImage::ImageRgba8(collage_img);

            if let Err(e) = img.save_with_format(&file_path, image::ImageFormat::Jpeg) {
                error!("Failed to save collage to {:?}: {}", file_path, e);
                continue;
            }

            // For now, skip thumbnail generation for collages
            // Thumbnails can be generated on-demand later if needed
            let thumbnail_path: Option<String> = None;

            // Save to database
            let photo_hashes: Vec<String> = chunk.iter().map(|p| p.hash_sha256.clone()).collect();

            match Collage::insert(
                pool,
                &date_str,
                &file_path.to_string_lossy(),
                thumbnail_path.as_deref(),
                chunk.len() as i32,
                &photo_hashes,
            ) {
                Ok(_) => {
                    info!(
                        "Successfully created collage {} for {}",
                        collage_idx + 1,
                        date_str
                    );
                    generated_count += 1;
                }
                Err(e) => {
                    error!("Failed to insert collage into database: {}", e);
                    // Clean up file
                    let _ = std::fs::remove_file(&file_path);
                }
            }
        }
    }

    info!(
        "Collage generation complete: {} collages created",
        generated_count
    );
    Ok(generated_count)
}

/// Move accepted collage to photos directory and trigger indexing
pub async fn accept_collage(
    pool: &DbPool,
    collage_id: i64,
    data_path: &Path,
    semantic_search: std::sync::Arc<crate::semantic_search::SemanticSearchEngine>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Get collage
    let collage = Collage::get_by_id(pool, collage_id)?.ok_or("Collage not found")?;

    // Create destination directory (separate from staging to avoid premature indexing)
    let dest_dir = data_path.join("collages").join("accepted");
    std::fs::create_dir_all(&dest_dir)?;

    // Move file
    let source = PathBuf::from(&collage.file_path);
    let filename = source.file_name().ok_or("Invalid file path")?;
    let dest = dest_dir.join(filename);

    std::fs::rename(&source, &dest)?;

    // Move thumbnail if exists
    if let Some(thumb_path) = &collage.thumbnail_path {
        let thumb_source = PathBuf::from(thumb_path);
        if thumb_source.exists() {
            if let Some(thumb_filename) = thumb_source.file_name() {
                let thumb_dest = dest_dir.join(thumb_filename);
                let _ = std::fs::rename(&thumb_source, &thumb_dest);
            }
        }
    }

    // Mark as accepted and update file path
    Collage::accept(pool, collage_id, &dest.to_string_lossy())?;

    // Index the collage into photos table immediately
    if let Err(e) = index_collage_file(pool, &dest, semantic_search).await {
        error!("Failed to index collage into photos table: {}", e);
        // Don't fail the whole operation if indexing fails
    }

    Ok(dest)
}

/// Reject and delete collage
pub async fn reject_collage(
    pool: &DbPool,
    collage_id: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Get collage
    let collage = Collage::get_by_id(pool, collage_id)?.ok_or("Collage not found")?;

    // Delete files
    let file_path = PathBuf::from(&collage.file_path);
    if file_path.exists() {
        std::fs::remove_file(&file_path)?;
    }

    if let Some(thumb_path) = &collage.thumbnail_path {
        let thumb_file = PathBuf::from(thumb_path);
        if thumb_file.exists() {
            let _ = std::fs::remove_file(&thumb_file);
        }
    }

    // Delete from database
    Collage::delete(pool, collage_id)?;

    Ok(())
}

/// Index a single collage file into the photos table
async fn index_collage_file(
    pool: &DbPool,
    file_path: &Path,
    semantic_search: std::sync::Arc<crate::semantic_search::SemanticSearchEngine>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Get file metadata
    let metadata = fs::metadata(file_path)?;
    let size = metadata.len();
    let modified = metadata.modified().ok().map(|t| {
        let duration = t
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(std::time::Duration::from_secs(0));
        DateTime::from_timestamp(duration.as_secs() as i64, 0).unwrap_or_else(Utc::now)
    });

    // Create PhotoFile
    let photo_file = PhotoFile {
        path: file_path.to_path_buf(),
        size,
        modified,
        metadata,
    };

    // Process the file
    let processor = PhotoProcessor::new(Vec::new(), semantic_search);
    let processed_photo = processor
        .process_file_metadata_only(&photo_file)
        .await
        .ok_or("Failed to process collage file")?;

    // Convert to Photo and insert into database
    let photo: Photo = processed_photo.into();
    let mut conn = pool.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    photo.create_or_update_with_connection(&tx)?;
    tx.commit()?;

    info!("Collage indexed into photos table: {}", file_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    /// Helper to create a mock Photo for testing
    fn mock_photo(file_path: &str, thumbnail_path: Option<&str>) -> Photo {
        Photo {
            hash_sha256: "test_hash".to_string(),
            file_path: file_path.to_string(),
            filename: file_path
                .split('/')
                .next_back()
                .unwrap_or("test.jpg")
                .to_string(),
            file_size: 1024,
            mime_type: Some("image/x-canon-cr2".to_string()),
            taken_at: Some(Utc::now()),
            width: Some(6000),
            height: Some(4000),
            orientation: Some(1),
            duration: None,
            thumbnail_path: thumbnail_path.map(String::from),
            has_thumbnail: thumbnail_path.map(|_| true),
            blurhash: None,
            is_favorite: None,
            semantic_vector_indexed: None,
            metadata: serde_json::json!({}),
            date_modified: Utc::now(),
            date_indexed: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_raw_photo_detection() {
        // Given: A RAW photo file path
        let raw_path = "/path/to/IMG_9899.CR2";
        let photo = mock_photo(raw_path, None);

        // When: Checking if it's a RAW file
        let is_raw = raw_processor::is_raw_file(Path::new(&photo.file_path));

        // Then: Should be detected as RAW
        assert!(is_raw, "CR2 files should be detected as RAW");
    }

    #[test]
    fn test_non_raw_photo_detection() {
        // Given: A JPEG photo file path
        let jpeg_path = "/path/to/photo.jpg";
        let photo = mock_photo(jpeg_path, None);

        // When: Checking if it's a RAW file
        let is_raw = raw_processor::is_raw_file(Path::new(&photo.file_path));

        // Then: Should NOT be detected as RAW
        assert!(!is_raw, "JPEG files should not be detected as RAW");
    }

    #[test]
    fn test_non_raw_photo_uses_thumbnail_when_available() {
        // Given: A JPEG photo with a thumbnail
        let jpeg_path = "/path/to/photo.jpg";
        let thumb_path = "/path/to/thumbnails/photo_small.jpg";
        let photo = mock_photo(jpeg_path, Some(thumb_path));

        // When: Determining which path to use (for non-RAW files)
        let is_raw = raw_processor::is_raw_file(Path::new(&photo.file_path));
        let image_path = if !is_raw {
            photo.thumbnail_path.as_deref().unwrap_or(&photo.file_path)
        } else {
            &photo.file_path
        };

        // Then: Should use thumbnail for non-RAW photos when available
        assert_eq!(
            image_path, thumb_path,
            "Should use thumbnail for non-RAW photos when available"
        );
    }

    #[test]
    fn test_collage_layout_one_photo() {
        let photo = mock_photo("test1.jpg", None);
        let photos = vec![&photo];
        let layout = CollageLayout::calculate(&photos);
        assert_eq!(layout.photo_count, 1);
        assert_eq!(layout.photo_cells.len(), 1);
    }

    #[test]
    fn test_collage_layout_two_photos() {
        let photo1 = mock_photo("test1.jpg", None);
        let photo2 = mock_photo("test2.jpg", None);
        let photos = vec![&photo1, &photo2];
        let layout = CollageLayout::calculate(&photos);
        assert_eq!(layout.photo_count, 2);
        assert_eq!(layout.photo_cells.len(), 2);
    }

    #[test]
    fn test_collage_layout_three_photos() {
        let photo1 = mock_photo("test1.jpg", None);
        let photo2 = mock_photo("test2.jpg", None);
        let photo3 = mock_photo("test3.jpg", None);
        let photos = vec![&photo1, &photo2, &photo3];
        let layout = CollageLayout::calculate(&photos);
        assert_eq!(layout.photo_count, 3);
        assert_eq!(layout.photo_cells.len(), 3);
        // Template is selected based on photo characteristics, so we just verify count
    }

    #[test]
    fn test_collage_layout_four_photos() {
        let photo1 = mock_photo("test1.jpg", None);
        let photo2 = mock_photo("test2.jpg", None);
        let photo3 = mock_photo("test3.jpg", None);
        let photo4 = mock_photo("test4.jpg", None);
        let photos = vec![&photo1, &photo2, &photo3, &photo4];
        let layout = CollageLayout::calculate(&photos);
        assert_eq!(layout.photo_count, 4);
        assert_eq!(layout.photo_cells.len(), 4);
    }

    #[test]
    fn test_collage_layout_exceeds_max() {
        // Should clamp to MAX_PHOTOS_PER_COLLAGE (4)
        let photos_vec: Vec<Photo> = (0..10)
            .map(|i| mock_photo(&format!("test{}.jpg", i), None))
            .collect();
        let photo_refs: Vec<&Photo> = photos_vec.iter().collect();
        let layout = CollageLayout::calculate(&photo_refs);
        assert_eq!(layout.photo_count, 4);
        assert_eq!(layout.photo_cells.len(), 4);
    }

    #[test]
    fn test_chunk_photos_ten_photos() {
        let photos: Vec<Photo> = (0..10)
            .map(|i| mock_photo(&format!("/photo_{}.jpg", i), None))
            .collect();

        let chunks = chunk_photos(&photos);

        // Sequential filling: [4, 4, 2] = 3 collages
        assert_eq!(chunks.len(), 3, "Should create 3 collages for 10 photos");
        assert_eq!(chunks[0].len(), 4, "First collage should have 4 photos");
        assert_eq!(chunks[1].len(), 4, "Second collage should have 4 photos");
        assert_eq!(chunks[2].len(), 2, "Third collage should have 2 photos");
    }

    #[test]
    fn test_chunk_photos_fifteen_photos() {
        let photos: Vec<Photo> = (0..15)
            .map(|i| mock_photo(&format!("/photo_{}.jpg", i), None))
            .collect();

        let chunks = chunk_photos(&photos);

        // Sequential filling: [4, 4, 4, 3] = 4 collages
        assert_eq!(chunks.len(), 4, "Should create 4 collages for 15 photos");
        assert_eq!(chunks[0].len(), 4, "First collage should have 4 photos");
        assert_eq!(chunks[1].len(), 4, "Second collage should have 4 photos");
        assert_eq!(chunks[2].len(), 4, "Third collage should have 4 photos");
        assert_eq!(chunks[3].len(), 3, "Fourth collage should have 3 photos");
    }

    #[test]
    fn test_chunk_photos_six_photos() {
        let photos: Vec<Photo> = (0..6)
            .map(|i| mock_photo(&format!("/photo_{}.jpg", i), None))
            .collect();

        let chunks = chunk_photos(&photos);

        // Sequential filling: [4, 2] = 2 collages
        assert_eq!(chunks.len(), 2, "Should create 2 collages for 6 photos");
        assert_eq!(chunks[0].len(), 4, "First collage should have 4 photos");
        assert_eq!(chunks[1].len(), 2, "Second collage should have 2 photos");
    }

    #[test]
    fn test_chunk_photos_five_photos() {
        let photos: Vec<Photo> = (0..5)
            .map(|i| mock_photo(&format!("/photo_{}.jpg", i), None))
            .collect();

        let chunks = chunk_photos(&photos);

        // Sequential filling: [4, 1] but 1-photo collages are filtered out
        assert_eq!(chunks.len(), 1, "Should create 1 collage for 5 photos");
        assert_eq!(chunks[0].len(), 4, "First collage should have 4 photos");
    }

    #[test]
    fn test_chunk_photos_one_photo() {
        let photos: Vec<Photo> = (0..1)
            .map(|i| mock_photo(&format!("/photo_{}.jpg", i), None))
            .collect();

        let chunks = chunk_photos(&photos);

        // 1 photo is below MIN_PHOTOS_PER_COLLAGE (2), so filtered out
        assert_eq!(chunks.len(), 0, "Should create 0 collages for 1 photo");
    }
}
