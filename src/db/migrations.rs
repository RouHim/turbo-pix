use crate::db::schema::*;
use rusqlite::Connection;

pub fn run_migrations(conn: &Connection) -> Result<(), rusqlite::Error> {
    // Create tables
    conn.execute(CREATE_PHOTOS_TABLE, [])?;

    // Create indexes
    conn.execute_batch(CREATE_INDEXES)?;

    Ok(())
}
