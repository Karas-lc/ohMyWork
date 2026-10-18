use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use super::migrate;
use rusqlite::{Connection, OptionalExtension};
pub(crate) fn open_or_rebuild(path: &Path) -> Result<Connection, String> {
    match open_once(path) {
        Ok(connection) => Ok(connection),
        Err(first) => {
            remove_database_files(path).map_err(|cleanup| {
                format!("索引数据库损坏（{first}），且重建前清理失败：{cleanup}")
            })?;
            open_once(path)
                .map_err(|second| format!("索引数据库损坏（{first}），重建失败：{second}"))
        }
    }
}

fn open_once(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open(path).map_err(|error| error.to_string())?;
    configure(&connection)?;
    migrate::run(&connection)?;
    Ok(connection)
}

fn configure(connection: &Connection) -> Result<(), String> {
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| format!("启用 WAL 失败：{error}"))?;
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(|error| format!("启用 foreign_keys 失败：{error}"))?;
    connection
        .busy_timeout(Duration::from_millis(2_000))
        .map_err(|error| format!("设置 busy_timeout 失败：{error}"))?;
    connection
        .pragma_update(None, "cache_size", -32_768)
        .map_err(|error| format!("设置 SQLite cache 失败：{error}"))?;
    Ok(())
}

fn remove_database_files(path: &Path) -> Result<(), String> {
    for suffix in ["", "-wal", "-shm"] {
        let candidate = PathBuf::from(format!("{}{}", path.display(), suffix));
        match fs::remove_file(&candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("删除 {} 失败：{error}", candidate.display())),
        }
    }
    Ok(())
}

pub(crate) fn stats(connection: &Connection) -> Result<crate::IndexStats, String> {
    let files = count(connection, "files")?;
    let aliases = count(connection, "aliases")?;
    let tasks = count(connection, "tasks")?;
    let journal_mode: String = connection
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .map_err(|error| format!("读取 journal_mode 失败：{error}"))?;
    let foreign_keys: i64 = connection
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .map_err(|error| format!("读取 foreign_keys 失败：{error}"))?;
    let busy_timeout_ms: i64 = connection
        .pragma_query_value(None, "busy_timeout", |row| row.get(0))
        .map_err(|error| format!("读取 busy_timeout 失败：{error}"))?;
    let cache_size: i64 = connection
        .pragma_query_value(None, "cache_size", |row| row.get(0))
        .map_err(|error| format!("读取 cache_size 失败：{error}"))?;
    Ok(crate::IndexStats {
        files,
        aliases,
        tasks,
        journal_mode,
        foreign_keys: foreign_keys != 0,
        busy_timeout_ms,
        cache_size,
    })
}

fn count(connection: &Connection, table: &str) -> Result<usize, String> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    connection
        .query_row(&sql, [], |row| row.get::<_, i64>(0))
        .map(|value| value.max(0) as usize)
        .map_err(|error| format!("统计 {table} 失败：{error}"))
}
pub(crate) fn next_generation(connection: &Connection) -> Result<u64, String> {
    let value = global_generation(connection)?;
    u64::try_from(value)
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| "索引代次溢出".to_owned())
}

pub(crate) fn current_max_generation(connection: &Connection) -> Result<u64, String> {
    u64::try_from(global_generation(connection)?).map_err(|_| "索引中存在负代次".to_owned())
}

pub(crate) fn global_generation(connection: &Connection) -> Result<i64, String> {
    connection
        .query_row(
            "SELECT MAX(generation) FROM (
                 SELECT COALESCE(MAX(generation), 0) AS generation FROM files
                 UNION ALL
                 SELECT generation FROM index_state WHERE key = 'completed_generation'
             )",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("读取全局索引代次失败：{error}"))
}

pub(crate) fn mark_completed_generation(
    transaction: &rusqlite::Transaction<'_>,
    generation: u64,
) -> Result<(), String> {
    transaction
        .execute(
            "UPDATE index_state SET generation = ?1 WHERE key = 'completed_generation'",
            [i64::try_from(generation).map_err(|_| "generation 溢出")?],
        )
        .map(|_| ())
        .map_err(|error| format!("保存完成扫描代次失败：{error}"))
}

pub(crate) fn file_record(
    connection: &Connection,
    logical_path: &str,
) -> Result<Option<crate::IndexFileRecord>, String> {
    let Some((id, resolved_path, generation)) = connection
        .query_row(
            "SELECT f.id, f.resolved_path, f.generation
             FROM files f JOIN aliases a ON a.file_id = f.id
             WHERE a.logical_path = ?1",
            [logical_path],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("查询索引文件失败：{error}"))?
    else {
        return Ok(None);
    };

    let mut statement = connection
        .prepare(
            "SELECT logical_path, area, excluded FROM aliases
             WHERE file_id = ?1 ORDER BY logical_path",
        )
        .map_err(|error| format!("查询索引别名失败：{error}"))?;
    let aliases = statement
        .query_map([id], |row| {
            Ok(crate::IndexAlias {
                logical_path: row.get(0)?,
                area: crate::IndexArea::parse(&row.get::<_, String>(1)?).ok_or_else(|| {
                    rusqlite::Error::InvalidColumnType(
                        1,
                        "area".to_owned(),
                        rusqlite::types::Type::Text,
                    )
                })?,
                excluded: row.get::<_, i64>(2)? != 0,
            })
        })
        .map_err(|error| format!("读取索引别名失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("读取索引别名失败：{error}"))?;
    Ok(Some(crate::IndexFileRecord {
        id,
        resolved_path,
        generation: u64::try_from(generation).map_err(|_| "索引中存在负代次".to_owned())?,
        aliases,
    }))
}
