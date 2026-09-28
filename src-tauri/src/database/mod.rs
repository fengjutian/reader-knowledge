use rusqlite::Connection;
use std::{fs, path::Path};
use crate::error::AppError;
pub struct Database { path: std::path::PathBuf }
impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AppError> { let path=path.as_ref().to_owned(); if let Some(parent)=path.parent(){fs::create_dir_all(parent).map_err(|e|AppError::Message(e.to_string()))?;} let db=Self{path}; db.connect()?.execute_batch(include_str!("schema.sql"))?; Ok(db) }
    pub fn connect(&self)->Result<Connection,AppError>{Ok(Connection::open(&self.path)?)}
}
