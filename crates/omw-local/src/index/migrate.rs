use std::collections::HashSet;

use rusqlite::Connection;

pub(crate) const SCHEMA_VERSION: i64 = 1;
const MIGRATION: &str = include_str!("../../migrations/001_index.sql");

pub(crate) fn run(connection: &Connection) -> Result<(), String> {
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| format!("读取 schema 版本失败：{error}"))?;
    match version {
        0 => connection
            .execute_batch(MIGRATION)
            .map_err(|error| format!("执行索引迁移失败：{error}")),
        SCHEMA_VERSION => validate_schema(connection),
        other => Err(format!("不支持的索引 schema 版本 {other}")),
    }
}

fn validate_schema(connection: &Connection) -> Result<(), String> {
    for (table, columns) in [
        (
            "files",
            &[
                "id",
                "resolved_path",
                "volume_key",
                "os_file_id",
                "content_hash",
                "byte_len",
                "mtime_ticks",
                "parse_version",
                "generation",
            ][..],
        ),
        (
            "aliases",
            &["logical_path", "file_id", "area", "excluded"][..],
        ),
        (
            "tasks",
            &[
                "file_id",
                "ordinal",
                "source_start",
                "source_end",
                "anchor_hash",
                "title",
                "completed",
                "start_date",
                "due_date",
                "priority",
                "diagnostics_json",
            ][..],
        ),
        ("search_long", &["path_fold", "title_fold", "body_fold"][..]),
        ("search_short", &["grams"][..]),
        ("index_state", &["key", "generation"][..]),
    ] {
        validate_table(connection, table, columns)?;
    }
    Ok(())
}

fn validate_table(connection: &Connection, table: &str, required: &[&str]) -> Result<(), String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| format!("校验索引表 {table} 失败：{error}"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("读取索引表 {table} 列失败：{error}"))?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|error| format!("读取索引表 {table} 列失败：{error}"))?;
    for column in required {
        if !columns.contains(*column) {
            return Err(format!("索引表 {table} 缺少列 {column}"));
        }
    }
    Ok(())
}
