//! 版本化迁移。SQLite 的 DDL 不支持 `ADD COLUMN IF NOT EXISTS`，
//! 所以新增列必须走代码：先看 PRAGMA table_info，缺了才 ALTER。
//! 迁移版本记在 app_meta（schema.sql 已经建好这张表），
//! 每次启动执行一次，已执行的版本直接跳过，重复启动不会重复 ALTER。

use crate::error::AppError;
use rusqlite::{params, Connection};

/// 迁移键与当前版本。往后续版本时只追加新 step，不要改旧 step。
const MIGRATION_KEY: &str = "glossary_import_migration";
const CURRENT_VERSION: i64 = 1;

pub fn apply(connection: &Connection) -> Result<(), AppError> {
    let version = applied_version(connection)?;
    if version >= CURRENT_VERSION {
        return Ok(());
    }
    if version < 1 {
        step_1_glossary_import_columns(connection)?;
    }
    set_version(connection, CURRENT_VERSION)
}

fn applied_version(connection: &Connection) -> Result<i64, AppError> {
    connection
        .query_row(
            "SELECT coalesce((SELECT value FROM app_meta WHERE key = ?1), '0')",
            params![MIGRATION_KEY],
            |row| row.get::<_, String>(0),
        )
        .map_err(AppError::from)
        .map(|value| value.parse::<i64>().unwrap_or(0))
}

/// 维基导入所需的新增列。已有的 source / status / wikipedia_snapshot 直接复用，
/// 不再另造含义重复的 source_type / review_status 字段。
fn step_1_glossary_import_columns(connection: &Connection) -> Result<(), AppError> {
    let columns: &[(&str, &str)] = &[
        // INTEGER 而不是 TEXT：维基 page id 要参与数值比较，
        // TEXT 存法会让 `external_page_id = 1001` 永远不成立。
        ("external_page_id", "INTEGER"),
        ("source_revision_id", "INTEGER"),
        ("source_dump_version", "TEXT"),
        ("source_updated_at", "INTEGER"),
        ("source_synced_at", "INTEGER"),
        ("license_code", "TEXT"),
        ("manually_edited", "INTEGER NOT NULL DEFAULT 0"),
        ("source_content_hash", "TEXT"),
        ("published_batch_id", "TEXT"),
        ("normalized_term", "TEXT"),
    ];
    for (name, declaration) in columns {
        add_column_if_missing(connection, "glossary_terms", name, declaration)?;
    }

    // 依赖新列的索引单独建。维基 page id 只在 source='wikipedia' 时唯一，
    // 人工条目的 external_page_id 保持 NULL，不参与唯一性。
    connection.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS ux_glossary_terms_wiki_page
             ON glossary_terms(external_page_id)
             WHERE source='wikipedia' AND external_page_id IS NOT NULL;
         CREATE INDEX IF NOT EXISTS idx_glossary_terms_source ON glossary_terms(source, status);
         CREATE INDEX IF NOT EXISTS idx_glossary_terms_normalized ON glossary_terms(normalized_term);
         CREATE INDEX IF NOT EXISTS idx_glossary_terms_batch ON glossary_terms(published_batch_id);
         CREATE INDEX IF NOT EXISTS idx_glossary_terms_external_revision
             ON glossary_terms(source_revision_id)
             WHERE source_revision_id IS NOT NULL;
         UPDATE glossary_terms
            SET source = 'manual'
          WHERE source IS NULL OR trim(source) = '';
         UPDATE glossary_terms
            SET status = 'confirmed'
          WHERE status IS NULL OR trim(status) = '';
         -- 人工存量数据没有来源页，统一标记成未人工编辑过的基线值。
         UPDATE glossary_terms SET manually_edited = 0 WHERE manually_edited IS NULL;",
    )?;
    Ok(())
}

fn add_column_if_missing(
    connection: &Connection,
    table: &str,
    column: &str,
    declaration: &str,
) -> Result<(), AppError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(());
        }
    }
    connection.execute_batch(&format!(
        "ALTER TABLE {table} ADD COLUMN {column} {declaration};"
    ))?;
    Ok(())
}

fn set_version(connection: &Connection, version: i64) -> Result<(), AppError> {
    connection.execute(
        "INSERT INTO app_meta(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![MIGRATION_KEY, version.to_string()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Connection {
        let connection = Connection::open_in_memory().expect("open in-memory db");
        connection
            .execute_batch(include_str!("schema.sql"))
            .expect("apply schema");
        connection
    }

    #[test]
    fn migration_adds_columns_and_is_idempotent() {
        let connection = memory_db();
        apply(&connection).expect("first apply");
        apply(&connection).expect("second apply must be a no-op");

        let mut statement = connection
            .prepare("PRAGMA table_info(glossary_terms)")
            .expect("prepare");
        let names: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("rows");
        for expected in [
            "external_page_id",
            "source_revision_id",
            "source_dump_version",
            "source_updated_at",
            "source_synced_at",
            "license_code",
            "manually_edited",
            "source_content_hash",
            "published_batch_id",
            "normalized_term",
        ] {
            assert!(names.contains(&expected.to_string()), "缺少列 {expected}");
        }
    }

    #[test]
    fn wikipedia_partial_unique_index_allows_many_manual_rows() {
        let connection = memory_db();
        apply(&connection).expect("apply");
        for index in 0..3 {
            connection
                .execute(
                    "INSERT INTO glossary_terms(term, canonical_name, definition, source, status, updated_at)
                     VALUES(?1, ?1, 'd', 'manual', 'confirmed', 0)",
                    params![format!("人工词条{index}")],
                )
                .expect("insert manual");
        }
        connection
            .execute(
                "INSERT INTO glossary_terms(term, canonical_name, definition, source, status, updated_at, external_page_id)
                 VALUES('w','w','d','wikipedia','pending',0,4242)",
                [],
            )
            .expect("insert wikipedia");
        let duplicate = connection.execute(
            "INSERT INTO glossary_terms(term, canonical_name, definition, source, status, updated_at, external_page_id)
             VALUES('w2','w2','d','wikipedia','pending',0,4242)",
            [],
        );
        assert!(duplicate.is_err(), "同一维基 page id 不允许出现两条");
    }

    #[test]
    fn existing_rows_are_backfilled_as_manual_confirmed() {
        let connection = memory_db();
        connection
            .execute(
                "INSERT INTO glossary_terms(term, canonical_name, definition, source, status, updated_at)
                 VALUES('人工智能','人工智能','AI','manual','confirmed',1)",
                [],
            )
            .expect("seed legacy row");
        apply(&connection).expect("apply");
        let (source, status, edited): (String, String, i64) = connection
            .query_row(
                "SELECT source, status, manually_edited FROM glossary_terms WHERE term='人工智能'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("row");
        assert_eq!(source, "manual");
        assert_eq!(status, "confirmed");
        assert_eq!(edited, 0, "存量名词不标记为人工编辑过");
    }
}
