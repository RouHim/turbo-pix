use clokwerk::{Job, Scheduler, TimeUnits};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use tracing::{error, info};

use crate::db::DbPool;
use crate::indexer::PhotoProcessor;

pub struct PhotoScheduler {
    photo_paths: Vec<PathBuf>,
    db_pool: DbPool,
}

impl PhotoScheduler {
    pub fn new(photo_paths: Vec<PathBuf>, db_pool: DbPool) -> Self {
        Self {
            photo_paths,
            db_pool,
        }
    }

    pub fn start(&self) {
        let mut scheduler = Scheduler::new();

        let photo_paths = self.photo_paths.clone();
        let db_pool = self.db_pool.clone();

        scheduler.every(1.day()).at("00:00").run(move || {
            info!("Starting scheduled photo indexing");

            let processor = PhotoProcessor::new(photo_paths.clone());
            let processed_photos = processor.process_all();

            let mut indexed_count = 0;
            let mut error_count = 0;

            for processed_photo in processed_photos {
                let photo: crate::db::models::Photo = processed_photo.into();
                match photo.create_or_update(&db_pool) {
                    Ok(_) => indexed_count += 1,
                    Err(e) => {
                        error!("Failed to save photo to database: {}", e);
                        error_count += 1;
                    }
                }
            }

            info!(
                "Scheduled indexing completed: {} photos indexed, {} errors",
                indexed_count, error_count
            );
        });

        thread::spawn(move || loop {
            scheduler.run_pending();
            thread::sleep(Duration::from_secs(60));
        });

        info!("Photo scheduler started with midnight indexing (0 0 * * *)");
    }

    pub fn run_manual_scan(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Starting manual photo indexing");

        let processor = PhotoProcessor::new(self.photo_paths.clone());
        let processed_photos = processor.process_all();

        let mut indexed_count = 0;
        let mut error_count = 0;

        for processed_photo in processed_photos {
            let photo: crate::db::models::Photo = processed_photo.into();
            match photo.create_or_update(&self.db_pool) {
                Ok(_) => indexed_count += 1,
                Err(e) => {
                    error!("Failed to save photo to database: {}", e);
                    error_count += 1;
                }
            }
        }

        info!(
            "Manual indexing completed: {} photos indexed, {} errors",
            indexed_count, error_count
        );

        Ok(())
    }
}
