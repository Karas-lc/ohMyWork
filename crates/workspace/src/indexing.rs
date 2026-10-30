use std::{collections::{HashMap, HashSet}, fs, path::{Path, PathBuf}, time::UNIX_EPOCH};

use editor::{DocumentSession, NodeKind, parse, task::task_items};
use project::{
    FileProjection, IndexAlias, IndexArea, IndexBatch, IndexError, IndexStore, IndexTask,
    ScanEntryKind, ScanIngestReport, ScanOptions, ScanReport, filesystem_identity, scan_workspace,
};

pub fn index_workspace(index: &IndexStore, root: impl Into<PathBuf>, options: ScanOptions) -> Result<ScanIngestReport, IndexError> {
    let generation = index.next_generation()?;
    let root = root.into();
    let scan = scan_workspace(root, options).map_err(|error| IndexError::Database(error.to_string()))?;
    let built = build_batch(generation, &scan);
    let indexed_files = built.batch.files().len();
    let skipped_files = built.skipped_files;
    let diagnostics = built.diagnostics;
    index.ingest(built.batch)?;
    index.complete_generation(generation, built.seen_logical_paths)?;
    Ok(ScanIngestReport { generation, indexed_files, skipped_files, diagnostics })
}

struct BuiltBatch {
    pub batch: IndexBatch,
    pub seen_logical_paths: Vec<String>,
    pub skipped_files: usize,
    pub diagnostics: Vec<String>,
}

fn build_batch(generation: u64, scan: &ScanReport) -> BuiltBatch {
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
        batch: { let mut batch = IndexBatch::new(generation); for file in files.into_values() { batch.push(normalize_alias_path_projection(file)); } batch },
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

fn heading_projection(document: &DocumentSession) -> String {
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

fn task_projection(document: &DocumentSession) -> Vec<IndexTask> {
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

fn logical_path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

