use std::collections::HashSet;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::db;
use super::{IndexBatch, IngestReport, grams, normalize, validate_batch};

pub(crate) fn replace_batch(
    connection: &mut Connection,
    batch: IndexBatch,
) -> Result<IngestReport, String> {
    validate_batch(&batch).map_err(|error| error.to_string())?;
    let current_generation = db::global_generation(connection)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("开始索引事务失败：{error}"))?;
    let mut accepted = 0;
    let mut skipped_stale = 0;
    for projection in &batch.files {
        let path_fold = normalize::normalize(&projection.path_fold).text;
        let title_fold = normalize::normalize(&projection.title_fold).text;
        let body_fold = normalize::normalize(&projection.body_fold).text;
        let short_grams = grams::encode_fields(&[&path_fold, &title_fold, &body_fold]);
        let existing: Option<(i64, i64, Vec<u8>, i64)> = transaction
            .query_row(
                "SELECT id, generation, content_hash, parse_version
                 FROM files WHERE resolved_path = ?1",
                [projection.resolved_path.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|error| format!("读取旧索引版本失败：{error}"))?;
        if projection.generation < u64::try_from(current_generation).unwrap_or(u64::MAX)
            || existing.as_ref().is_some_and(|(_, generation, _, _)| {
                *generation > i64::try_from(projection.generation).unwrap_or(i64::MAX)
            })
        {
            skipped_stale += 1;
            continue;
        }
        let reuse_projection =
            existing
                .as_ref()
                .is_some_and(|(_, _, content_hash, parse_version)| {
                    content_hash == &projection.content_hash
                        && *parse_version == i64::from(projection.parse_version)
                });
        let file_id = if let Some((id, _, _, _)) = existing {
            transaction
                .execute(
                    "UPDATE files SET volume_key = ?1, os_file_id = ?2, content_hash = ?3,
                     byte_len = ?4, mtime_ticks = ?5, parse_version = ?6, generation = ?7
                     WHERE id = ?8",
                    params![
                        projection.volume_key,
                        projection.os_file_id,
                        projection.content_hash,
                        i64::try_from(projection.byte_len).map_err(|_| "byte_len 溢出")?,
                        projection.mtime_ticks,
                        i64::from(projection.parse_version),
                        i64::try_from(projection.generation).map_err(|_| "generation 溢出")?,
                        id,
                    ],
                )
                .map_err(|error| format!("更新文件索引失败：{error}"))?;
            id
        } else {
            transaction
                .execute(
                    "INSERT INTO files
                     (resolved_path, volume_key, os_file_id, content_hash, byte_len,
                      mtime_ticks, parse_version, generation)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        projection.resolved_path,
                        projection.volume_key,
                        projection.os_file_id,
                        projection.content_hash,
                        i64::try_from(projection.byte_len).map_err(|_| "byte_len 溢出")?,
                        projection.mtime_ticks,
                        i64::from(projection.parse_version),
                        i64::try_from(projection.generation).map_err(|_| "generation 溢出")?,
                    ],
                )
                .map_err(|error| format!("插入文件索引失败：{error}"))?;
            transaction.last_insert_rowid()
        };

        if !reuse_projection {
            transaction
                .execute("DELETE FROM search_long WHERE rowid = ?1", [file_id])
                .map_err(|error| format!("替换长文本索引失败：{error}"))?;
            transaction
                .execute("DELETE FROM search_short WHERE rowid = ?1", [file_id])
                .map_err(|error| format!("替换短文本索引失败：{error}"))?;
            transaction
                .execute("DELETE FROM tasks WHERE file_id = ?1", [file_id])
                .map_err(|error| format!("替换 Task 索引失败：{error}"))?;
        }
        transaction
            .execute("DELETE FROM aliases WHERE file_id = ?1", [file_id])
            .map_err(|error| format!("替换别名索引失败：{error}"))?;

        for alias in &projection.aliases {
            transaction
                .execute(
                    "INSERT INTO aliases(logical_path, file_id, area, excluded)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(logical_path) DO UPDATE SET
                       file_id = excluded.file_id, area = excluded.area, excluded = excluded.excluded",
                    params![
                        alias.logical_path,
                        file_id,
                        alias.area.as_str(),
                        if alias.excluded { 1_i64 } else { 0_i64 },
                    ],
                )
                .map_err(|error| format!("写入文件别名失败：{error}"))?;
        }
        if reuse_projection {
            accepted += 1;
            continue;
        }
        for task in &projection.tasks {
            transaction
                .execute(
                    "INSERT INTO tasks
                     (file_id, ordinal, source_start, source_end, anchor_hash, title,
                      completed, start_date, due_date, priority, diagnostics_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        file_id,
                        i64::from(task.ordinal),
                        i64::try_from(task.source_start).map_err(|_| "source_start 溢出")?,
                        i64::try_from(task.source_end).map_err(|_| "source_end 溢出")?,
                        task.anchor_hash,
                        task.title,
                        if task.completed { 1_i64 } else { 0_i64 },
                        task.start_date,
                        task.due_date,
                        task.priority.map(i64::from),
                        task.diagnostics_json,
                    ],
                )
                .map_err(|error| format!("写入 Task 索引失败：{error}"))?;
        }
        transaction
            .execute(
                "INSERT INTO search_long(rowid, path_fold, title_fold, body_fold)
                 VALUES (?1, ?2, ?3, ?4)",
                params![file_id, path_fold, title_fold, body_fold],
            )
            .map_err(|error| format!("写入长文本索引失败：{error}"))?;
        transaction
            .execute(
                "INSERT INTO search_short(rowid, grams) VALUES (?1, ?2)",
                params![file_id, short_grams],
            )
            .map_err(|error| format!("写入短文本索引失败：{error}"))?;
        accepted += 1;
    }
    cleanup_orphans(&transaction)?;
    transaction
        .commit()
        .map_err(|error| format!("提交索引事务失败：{error}"))?;
    Ok(IngestReport {
        accepted,
        skipped_stale,
    })
}

pub(crate) fn complete_generation(
    connection: &mut Connection,
    generation: u64,
    seen_logical_paths: Vec<String>,
) -> Result<(), String> {
    let current = crate::index::db::current_max_generation(connection)?;
    if generation < current {
        return Err(format!("扫描代次 {generation} 已落后于当前代次 {current}"));
    }
    let seen = seen_logical_paths.into_iter().collect::<HashSet<_>>();
    let transaction = connection
        .transaction()
        .map_err(|error| format!("开始完成扫描事务失败：{error}"))?;
    let mut statement = transaction
        .prepare("SELECT logical_path FROM aliases")
        .map_err(|error| format!("读取旧别名失败：{error}"))?;
    let stale = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| format!("枚举旧别名失败：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("枚举旧别名失败：{error}"))?;
    drop(statement);
    for logical_path in stale {
        if !seen.contains(&logical_path) {
            transaction
                .execute(
                    "DELETE FROM aliases WHERE logical_path = ?1",
                    [logical_path],
                )
                .map_err(|error| format!("删除消失别名失败：{error}"))?;
        }
    }
    cleanup_orphans(&transaction)?;
    db::mark_completed_generation(&transaction, generation)?;
    transaction
        .commit()
        .map_err(|error| format!("提交扫描完成事务失败：{error}"))
}

fn cleanup_orphans(transaction: &Transaction<'_>) -> Result<(), String> {
    let orphaned = {
        let mut statement = transaction
            .prepare(
                "SELECT f.id FROM files f
                 LEFT JOIN aliases a ON a.file_id = f.id
                 WHERE a.file_id IS NULL",
            )
            .map_err(|error| format!("枚举孤立文件失败：{error}"))?;
        statement
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(|error| format!("枚举孤立文件失败：{error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("枚举孤立文件失败：{error}"))?
    };
    for id in orphaned {
        transaction
            .execute("DELETE FROM search_long WHERE rowid = ?1", [id])
            .map_err(|error| format!("删除孤立长文本索引失败：{error}"))?;
        transaction
            .execute("DELETE FROM search_short WHERE rowid = ?1", [id])
            .map_err(|error| format!("删除孤立短文本索引失败：{error}"))?;
        transaction
            .execute("DELETE FROM files WHERE id = ?1", [id])
            .map_err(|error| format!("删除孤立文件索引失败：{error}"))?;
    }
    Ok(())
}

