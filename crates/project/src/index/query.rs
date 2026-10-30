use std::{
    cmp::Ordering,
    fs,
    path::Path,
};

use rusqlite::{Connection, params};

use super::{
    IndexArea, SearchError, SearchMatchKind, SearchPage, SearchRequest, SearchResult, grams,
    normalize,
};

const MAX_LIMIT: usize = 200;
const MAX_SNIPPET_SOURCE_BYTES: u64 = 4 * 1024 * 1024;

struct Candidate {
    id: i64,
    resolved_path: String,
    title_fold: String,
    body_fold: String,
}

pub(crate) fn search(
    connection: &Connection,
    request: SearchRequest,
) -> Result<SearchPage, SearchError> {
    let normalized_query = normalize::normalize(&request.query).text;
    let index_ready = completed_generation(connection)? > 0;
    if normalized_query.is_empty() || normalized_query.contains('\0') {
        return Ok(SearchPage {
            results: Vec::new(),
            has_more: false,
            index_ready,
        });
    }
    if request.cancel.is_cancelled() {
        return Err(SearchError::Cancelled);
    }

    let candidates = load_candidates(connection, &normalized_query, &request)?;
    let mut results = Vec::new();
    for candidate in candidates {
        if request.cancel.is_cancelled() {
            return Err(SearchError::Cancelled);
        }
        let aliases = visible_aliases(connection, candidate.id, request.include_archive)?;
        if aliases.is_empty() {
            continue;
        }
        let path_alias = aliases
            .iter()
            .filter(|alias| normalize::contains(&alias.0, &normalized_query))
            .min_by(|left, right| alias_order(left, right));
        let path_match = path_alias.is_some();
        let title_match = candidate.title_fold.contains(&normalized_query);
        let body_match = candidate.body_fold.contains(&normalized_query);
        let Some(match_kind) = match_kind(path_match, title_match, body_match) else {
            continue;
        };
        let (logical_path, area) = path_alias
            .or_else(|| aliases.first())
            .cloned()
            .ok_or_else(|| SearchError::Database("可见索引别名缺失".to_owned()))?;
        let title = candidate
            .title_fold
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned();
        let snippet = source_snippet(
            &candidate.resolved_path,
            &request.query,
            match_kind,
        );
        results.push(SearchResult {
            resolved_path: candidate.resolved_path,
            logical_path,
            title,
            snippet,
            archive: area == IndexArea::Archive,
            match_kind,
        });
    }

    results.sort_by(|left, right| {
        left.match_kind
            .rank()
            .cmp(&right.match_kind.rank())
            .then_with(|| left.logical_path.cmp(&right.logical_path))
            .then_with(|| left.resolved_path.cmp(&right.resolved_path))
    });

    let limit = request.limit.clamp(1, MAX_LIMIT);
    let start = request.offset.min(results.len());
    let page = results
        .into_iter()
        .skip(start)
        .take(limit + 1)
        .collect::<Vec<_>>();
    let has_more = page.len() > limit;
    Ok(SearchPage {
        results: page.into_iter().take(limit).collect(),
        has_more,
        index_ready,
    })
}

fn load_candidates(
    connection: &Connection,
    normalized_query: &str,
    request: &SearchRequest,
) -> Result<Vec<Candidate>, SearchError> {
    let mut candidates = Vec::new();
    let mut statement = if normalized_query.chars().count() >= 3 {
        let phrase = fts_phrase(normalized_query);
        let mut statement = connection
            .prepare(
                "SELECT f.id, f.resolved_path, sl.title_fold, sl.body_fold
                 FROM files f JOIN search_long sl ON sl.rowid = f.id
                 WHERE search_long MATCH ?1",
            )
            .map_err(|error| SearchError::Database(format!("准备长词搜索失败：{error}")))?;
        let mut rows = statement
            .query([phrase])
            .map_err(|error| SearchError::Database(format!("执行长词搜索失败：{error}")))?;
        while let Some(row) = rows
            .next()
            .map_err(|error| SearchError::Database(format!("读取长词搜索结果失败：{error}")))?
        {
            candidates.push(candidate_from_row(row)?);
            if request.cancel.is_cancelled() {
                return Err(SearchError::Cancelled);
            }
        }
        return Ok(candidates);
    } else {
        connection
            .prepare(
                "SELECT f.id, f.resolved_path, sl.title_fold, sl.body_fold
                 FROM files f
                 JOIN search_long sl ON sl.rowid = f.id
                 JOIN search_short ss ON ss.rowid = f.id
                 WHERE search_short MATCH ?1",
            )
            .map_err(|error| SearchError::Database(format!("准备短词搜索失败：{error}")))?
    };

    let token = fts_phrase(
        &grams::query_token(normalized_query)
            .ok_or_else(|| SearchError::Database("短词 token 生成失败".to_owned()))?,
    );
    let mut rows = statement
        .query([token])
        .map_err(|error| SearchError::Database(format!("执行短词搜索失败：{error}")))?;
    while let Some(row) = rows
        .next()
        .map_err(|error| SearchError::Database(format!("读取短词搜索结果失败：{error}")))?
    {
        candidates.push(candidate_from_row(row)?);
        if request.cancel.is_cancelled() {
            return Err(SearchError::Cancelled);
        }
    }
    Ok(candidates)
}

fn candidate_from_row(row: &rusqlite::Row<'_>) -> Result<Candidate, SearchError> {
    Ok(Candidate {
        id: row
            .get(0)
            .map_err(|error| SearchError::Database(format!("读取搜索文件 ID 失败：{error}")))?,
        resolved_path: row
            .get(1)
            .map_err(|error| SearchError::Database(format!("读取搜索路径失败：{error}")))?,
        title_fold: row
            .get(2)
            .map_err(|error| SearchError::Database(format!("读取搜索标题投影失败：{error}")))?,
        body_fold: row
            .get(3)
            .map_err(|error| SearchError::Database(format!("读取搜索正文投影失败：{error}")))?,
    })
}

fn visible_aliases(
    connection: &Connection,
    file_id: i64,
    include_archive: bool,
) -> Result<Vec<(String, IndexArea)>, SearchError> {
    let mut statement = connection
        .prepare(
            "SELECT logical_path, area FROM aliases
             WHERE file_id = ?1 AND excluded = 0
               AND (?2 OR area <> 'archive')",
        )
        .map_err(|error| SearchError::Database(format!("准备搜索别名失败：{error}")))?;
    let aliases = statement
        .query_map(params![file_id, include_archive], |row| {
            let logical_path: String = row.get(0)?;
            let area = IndexArea::parse(&row.get::<_, String>(1)?).ok_or_else(|| {
                rusqlite::Error::InvalidColumnType(
                    1,
                    "area".to_owned(),
                    rusqlite::types::Type::Text,
                )
            })?;
            Ok((logical_path, area))
        })
        .map_err(|error| SearchError::Database(format!("读取搜索别名失败：{error}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| SearchError::Database(format!("读取搜索别名失败：{error}")))?;
    Ok(aliases)
}

fn alias_order(left: &(String, IndexArea), right: &(String, IndexArea)) -> Ordering {
    area_rank(left.1)
        .cmp(&area_rank(right.1))
        .then_with(|| left.0.cmp(&right.0))
}

fn area_rank(area: IndexArea) -> u8 {
    u8::from(area == IndexArea::Archive)
}

fn match_kind(path: bool, title: bool, body: bool) -> Option<SearchMatchKind> {
    if path {
        Some(SearchMatchKind::Path)
    } else if title {
        Some(SearchMatchKind::Title)
    } else if body {
        Some(SearchMatchKind::Body)
    } else {
        None
    }
}

fn source_snippet(path: &str, query: &str, kind: SearchMatchKind) -> Option<String> {
    if kind == SearchMatchKind::Path || !is_markdown(path) {
        return None;
    }
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > MAX_SNIPPET_SOURCE_BYTES {
        return None;
    }
    let source = fs::read_to_string(path).ok()?;
    normalize::snippet(&source, query, 48)
}

fn is_markdown(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

fn fts_phrase(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn completed_generation(connection: &Connection) -> Result<i64, SearchError> {
    connection
        .query_row(
            "SELECT generation FROM index_state WHERE key = 'completed_generation'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| SearchError::Database(format!("读取搜索索引状态失败：{error}")))
}
