use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use omw_document::{NodeKind, parse, task::task_items};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::db;
use super::{
    FileProjection, IndexAlias, IndexArea, IndexBatch, IndexTask, IngestReport, grams, normalize,
    validate_batch,
};
use crate::{ScanEntryKind, ScanReport, filesystem_identity};
pub(crate) struct BuiltBatch {
    pub batch: IndexBatch,
    pub seen_logical_paths: Vec<String>,
    pub skipped_files: usize,
    pub diagnostics: Vec<String>,
}

pub(crate) fn build_batch(generation: u64, scan: &ScanReport) -> BuiltBatch {
    let mut files: HashMap<PathBuf, FileProjection> = HashMap::new();
    let mut seen = HashSet::new();
    let mut skipped_files = 0;
    let mut diagnostics = scan
        .diagnostics()
        .iter()
        .map(|diagnostic| format!("{}：{}", diagnostic.path.display(), diagnostic.message))
        .collect::<Vec<_>>();

    for entry in scan
        .entries()
        .iter()
        .filter(|entry| entry.kind() == ScanEntryKind::File)
    {
        let logical_path = logical_path_string(entry.logical_path());
        seen.insert(logical_path.clone());
        let alias = IndexAlias::new(
            logical_path,
            area_for(entry.logical_path()),
            entry.is_excluded(),
        );
        let key = entry.target_path().to_path_buf();
        if let Some(file) = files.get_mut(&key) {
            file.aliases.push(alias);
            continue;
        }
        match project_file(entry.target_path(), generation, alias) {
            Ok(file) => {
                files.insert(key, file);
            }
            Err(error) => {
                skipped_files += 1;
                diagnostics.push(format!("{}：{error}", entry.logical_path().display()));
            }
        }
    }

    BuiltBatch {
        batch: IndexBatch {
            generation,
            files: files
                .into_values()
                .map(normalize_alias_path_projection)
                .collect(),
        },
        seen_logical_paths: seen.into_iter().collect(),
        skipped_files,
        diagnostics,
    }
}

fn normalize_alias_path_projection(mut projection: FileProjection) -> FileProjection {
    let aliases = projection
        .aliases
        .iter()
        .map(|alias| alias.logical_path.as_str())
        .collect::<Vec<_>>();
    projection.path_fold = aliases.join("\n");
    projection
}

fn project_file(path: &Path, generation: u64, alias: IndexAlias) -> Result<FileProjection, String> {
    let metadata = fs::metadata(path).map_err(|error| format!("读取文件元数据失败：{error}"))?;
    let bytes = fs::read(path).map_err(|error| format!("读取文件失败：{error}"))?;
    let content_hash = blake3::hash(&bytes).as_bytes().to_vec();
    let mtime_ticks = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_nanos()).ok())
        .unwrap_or(0);
    let identity = filesystem_identity(path).ok();
    let is_markdown = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));

    let mut projection = FileProjection {
        resolved_path: path.to_string_lossy().into_owned(),
        volume_key: identity.map(|value| format!("{:016x}", value.volume_serial)),
        os_file_id: identity.map(|value| value.file_index.to_le_bytes().to_vec()),
        content_hash,
        byte_len: bytes.len() as u64,
        mtime_ticks,
        parse_version: 2,
        generation,
        aliases: vec![alias],
        path_fold: projection_path(path),
        title_fold: String::new(),
        body_fold: String::new(),
        tasks: Vec::new(),
    };

    if !is_markdown {
        return Ok(projection);
    }

    match parse(&bytes) {
        Ok(document) => {
            projection.body_fold = document.serialize();
            projection.title_fold = heading_projection(&document);
            projection.tasks = task_projection(&document);
        }
        Err(_error) => {
            // Keep path search available even when the document parser rejects
            // the bytes; only the Markdown-specific projections are empty.
            if let Ok(text) = String::from_utf8(bytes) {
                projection.body_fold = text;
            }
        }
    }
    Ok(projection)
}

fn projection_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn heading_projection(document: &omw_document::DocumentSession) -> String {
    document
        .tree()
        .iter()
        .filter_map(|id| {
            if !matches!(document.tree().node(id).kind, NodeKind::Heading { .. }) {
                return None;
            }
            let source = document.source_slice(id)?.text.into_owned();
            let line = source.lines().next()?.trim();
            let line = line.trim_start_matches('#').trim();
            Some(line.trim_end_matches('#').trim().to_owned())
        })
        .filter(|title| !title.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn task_projection(document: &omw_document::DocumentSession) -> Vec<IndexTask> {
    task_items(document.tree())
        .into_iter()
        .enumerate()
        .filter_map(|(ordinal, item)| {
            let source = document.source_slice(item.node)?;
            let raw = source.text.as_ref();
            let (title, start_date, due_date, priority, diagnostics) = parse_task_fields(raw);
            Some(IndexTask {
                ordinal: u32::try_from(ordinal).ok()?,
                source_start: u64::from(source.range.start.0),
                source_end: u64::from(source.range.end.0),
                anchor_hash: blake3::hash(raw.as_bytes()).as_bytes().to_vec(),
                title,
                completed: item.checked,
                start_date,
                due_date,
                priority,
                diagnostics_json: serde_json::to_string(&diagnostics)
                    .unwrap_or_else(|_| "[]".to_owned()),
            })
        })
        .collect()
}

fn parse_task_fields(
    raw: &str,
) -> (
    String,
    Option<String>,
    Option<String>,
    Option<u8>,
    Vec<String>,
) {
    let line = raw.lines().next().unwrap_or(raw);
    let marker = [line.find("[ ]"), line.find("[x]"), line.find("[X]")]
        .into_iter()
        .flatten()
        .min();
    let title_start = marker.map_or(0, |offset| offset + 3);
    let title_source = &line[title_start..];
    let mut title = String::with_capacity(title_source.len());
    let mut start_date = None;
    let mut due_date = None;
    let mut priority = None;
    let mut diagnostics = Vec::new();
    let mut cursor = 0;
    while let Some(open_relative) = title_source[cursor..].find('[') {
        let open = cursor + open_relative;
        title.push_str(&title_source[cursor..open]);
        let Some(close_relative) = title_source[open..].find(']') else {
            title.push_str(&title_source[open..]);
            cursor = title_source.len();
            break;
        };
        let close = open + close_relative;
        let field = &title_source[open + 1..close];
        let recognized = if let Some(value) = field.strip_prefix("start::") {
            match valid_date(value.trim()) {
                Some(value) => start_date = Some(value.to_owned()),
                None => diagnostics.push("invalid start date".to_owned()),
            }
            true
        } else if let Some(value) = field.strip_prefix("due::") {
            match valid_date(value.trim()) {
                Some(value) => due_date = Some(value.to_owned()),
                None => diagnostics.push("invalid due date".to_owned()),
            }
            true
        } else if let Some(value) = field.strip_prefix("priority::") {
            match value
                .trim()
                .parse::<u8>()
                .ok()
                .filter(|value| (1..=9).contains(value))
            {
                Some(value) => priority = Some(value),
                None => diagnostics.push("invalid priority".to_owned()),
            }
            true
        } else {
            false
        };
        if !recognized {
            title.push_str(&title_source[open..=close]);
        }
        cursor = close + 1;
    }
    if cursor < title_source.len() {
        title.push_str(&title_source[cursor..]);
    }
    (
        title.trim().to_owned(),
        start_date,
        due_date,
        priority,
        diagnostics,
    )
}

fn valid_date(value: &str) -> Option<&str> {
    (value.len() == 10
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit()))
    .then_some(value)
}

fn area_for(path: &Path) -> IndexArea {
    match path
        .to_string_lossy()
        .replace('\\', "/")
        .split('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "atlas" => IndexArea::Atlas,
        "efforts" => IndexArea::Efforts,
        "archive" => IndexArea::Archive,
        _ => IndexArea::Other,
    }
}

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

fn logical_path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
