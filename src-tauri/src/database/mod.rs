use crate::error::AppError;
use rusqlite::Connection;
use std::{fs, path::Path, time::Duration};

pub mod migrations;

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
        let connection = db.connect()?;
        connection.execute_batch(include_str!("schema.sql"))?;
        // 新增列只能走版本化迁移，SQLite 的 DDL 没有 ADD COLUMN IF NOT EXISTS。
        migrations::apply(&connection)?;
        Ok(db)
    }
    pub fn connect(&self) -> Result<Connection, AppError> {
        let connection = Connection::open(&self.path)?;
        connection.busy_timeout(Duration::from_secs(3))?;
        connection.execute_batch("PRAGMA foreign_keys=ON;")?;
        Ok(connection)
    }

    /// 同一路径的另一个轻量句柄，用于把长任务搬进后台线程。
    /// 不重跑 schema：schema 已经在 open 时执行过了。
    pub fn reopen(&self) -> Result<Self, AppError> {
        Ok(Self { path: self.path.clone() })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn size_bytes(&self) -> u64 {
        fs::metadata(&self.path).map(|metadata| metadata.len()).unwrap_or(0)
    }
}
