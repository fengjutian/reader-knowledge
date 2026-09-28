use crate::error::AppError;
use rusqlite::Connection;
use std::{fs, path::Path, time::Duration};
pub struct Database {
    path: std::path::PathBuf,
}
impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AppError> {
        let path = path.as_ref().to_owned();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| AppError::Message(e.to_string()))?;
        }
        let db = Self { path };
        db.connect()?.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        db.connect()?.execute_batch(include_str!("schema.sql"))?;
        Ok(db)
    }
    pub fn connect(&self) -> Result<Connection, AppError> {
        let connection = Connection::open(&self.path)?;
        connection.busy_timeout(Duration::from_secs(3))?;
        connection.execute_batch("PRAGMA foreign_keys=ON;")?;
        Ok(connection)
    }
}
