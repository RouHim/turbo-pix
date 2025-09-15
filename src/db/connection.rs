use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use std::path::Path;

pub type DbPool = Pool<SqliteConnectionManager>;

pub fn create_db_pool<P: AsRef<Path>>(db_path: P) -> Result<DbPool, Box<dyn std::error::Error>> {
    let manager = SqliteConnectionManager::file(db_path);
    let pool = Pool::new(manager)?;

    // Run migrations on startup
    let conn = pool.get()?;
    crate::db::migrations::run_migrations(&conn)?;

    Ok(pool)
}

#[allow(dead_code)]
pub fn create_in_memory_pool() -> Result<DbPool, Box<dyn std::error::Error>> {
    let manager = SqliteConnectionManager::memory();
    let pool = Pool::new(manager)?;

    // Run migrations on startup
    let conn = pool.get()?;
    crate::db::migrations::run_migrations(&conn)?;

    Ok(pool)
}
