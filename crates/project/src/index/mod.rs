mod db;
mod grams;
mod ingest;
mod migrate;
mod normalize;
mod query;

use std::{
    collections::HashSet,
    fmt, fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread::{self, JoinHandle},
};

use crate::AppDataPaths;

const WORKER_NAME: &str = "ohmywork-index";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexArea {
    Atlas,
    Efforts,
    Archive,
    Other,
}

impl IndexArea {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "atlas" => Some(Self::Atlas),
            "efforts" => Some(Self::Efforts),
            "archive" => Some(Self::Archive),
            "other" => Some(Self::Other),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Atlas => "atlas",
            Self::Efforts => "efforts",
            Self::Archive => "archive",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexAlias {
    pub logical_path: String,
    pub area: IndexArea,
    pub excluded: bool,
}

impl IndexAlias {
    pub fn new(logical_path: impl Into<String>, area: IndexArea, excluded: bool) -> Self {
        Self {
            logical_path: logical_path.into(),
            area,
            excluded,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexTask {
    pub ordinal: u32,
    pub source_start: u64,
    pub source_end: u64,
    pub anchor_hash: Vec<u8>,
    pub title: String,
    pub completed: bool,
    pub start_date: Option<String>,
    pub due_date: Option<String>,
    pub priority: Option<u8>,
    pub diagnostics_json: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileProjection {
    pub resolved_path: String,
    pub volume_key: Option<String>,
    pub os_file_id: Option<Vec<u8>>,
    pub content_hash: Vec<u8>,
    pub byte_len: u64,
    pub mtime_ticks: i64,
    pub parse_version: u32,
    pub generation: u64,
    pub aliases: Vec<IndexAlias>,
    pub path_fold: String,
    pub title_fold: String,
    pub body_fold: String,
    pub tasks: Vec<IndexTask>,
}

impl FileProjection {
    pub fn new(resolved_path: impl Into<String>, generation: u64) -> Self {
        Self {
            resolved_path: resolved_path.into(),
            volume_key: None,
            os_file_id: None,
            content_hash: Vec::new(),
            byte_len: 0,
            mtime_ticks: 0,
            parse_version: 2,
            generation,
            aliases: Vec::new(),
            path_fold: String::new(),
            title_fold: String::new(),
            body_fold: String::new(),
            tasks: Vec::new(),
        }
    }

    pub fn with_alias(mut self, alias: IndexAlias) -> Self {
        self.aliases.push(alias);
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexBatch {
    generation: u64,
    files: Vec<FileProjection>,
}

impl IndexBatch {
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            files: Vec::new(),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn push(&mut self, projection: FileProjection) {
        self.files.push(projection);
    }

    pub fn files(&self) -> &[FileProjection] {
        &self.files
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngestReport {
    pub accepted: usize,
    pub skipped_stale: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanIngestReport {
    pub generation: u64,
    pub indexed_files: usize,
    pub skipped_files: usize,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexStats {
    pub files: usize,
    pub aliases: usize,
    pub tasks: usize,
    pub journal_mode: String,
    pub foreign_keys: bool,
    pub busy_timeout_ms: i64,
    pub cache_size: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexFileRecord {
    pub id: i64,
    pub resolved_path: String,
    pub generation: u64,
    pub aliases: Vec<IndexAlias>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchMatchKind {
    Path,
    Title,
    Body,
}

impl SearchMatchKind {
    pub fn rank(self) -> u8 {
        match self {
            Self::Path => 0,
            Self::Title => 1,
            Self::Body => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchResult {
    pub resolved_path: String,
    pub logical_path: String,
    pub title: String,
    pub snippet: Option<String>,
    pub archive: bool,
    pub match_kind: SearchMatchKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchPage {
    pub results: Vec<SearchResult>,
    pub has_more: bool,
    pub index_ready: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SearchCancelToken(Arc<AtomicBool>);

impl SearchCancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug)]
pub struct SearchRequest {
    pub query: String,
    pub include_archive: bool,
    pub offset: usize,
    pub limit: usize,
    pub cancel: SearchCancelToken,
}

impl SearchRequest {
    pub fn new(query: impl Into<String>, include_archive: bool, offset: usize, limit: usize) -> Self {
        Self {
            query: query.into(),
            include_archive,
            offset,
            limit,
            cancel: SearchCancelToken::new(),
        }
    }
}

#[derive(Debug)]
enum SearchError {
    Cancelled,
    Database(String),
}

impl From<SearchError> for IndexError {
    fn from(error: SearchError) -> Self {
        match error {
            SearchError::Cancelled => Self::SearchCancelled,
            SearchError::Database(error) => Self::Database(error),
        }
    }
}

#[derive(Debug)]
pub enum IndexError {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    Database(String),
    SearchCancelled,
    InvalidProjection(String),
    StaleGeneration {
        requested: u64,
        current: u64,
    },
    Closed,
    WorkerPanic,
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => {
                write!(f, "{operation} {} 失败：{source}", path.display())
            }
            Self::Database(error) => write!(f, "SQLite 索引失败：{error}"),
            Self::SearchCancelled => f.write_str("搜索已取消"),
            Self::InvalidProjection(error) => write!(f, "索引投影无效：{error}"),
            Self::StaleGeneration { requested, current } => {
                write!(f, "索引代次 {requested} 已过期，当前代次为 {current}")
            }
            Self::Closed => f.write_str("SQLite 索引线程已关闭"),
            Self::WorkerPanic => f.write_str("SQLite 索引线程异常退出"),
        }
    }
}

impl std::error::Error for IndexError {}

enum Command {
    Ingest(IndexBatch, Sender<Result<IngestReport, String>>),
    Complete(u64, Vec<String>, Sender<Result<(), String>>),
    NextGeneration(Sender<Result<u64, String>>),
    Stats(Sender<Result<IndexStats, String>>),
    File(String, Sender<Result<Option<IndexFileRecord>, String>>),
    Search(SearchRequest, Sender<Result<SearchPage, SearchError>>),
    Shutdown,
}

pub struct IndexStore {
    path: PathBuf,
    tx: Sender<Command>,
    worker: Option<JoinHandle<()>>,
}

impl IndexStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, IndexError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| IndexError::Io {
                operation: "创建索引目录",
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let (command_tx, command_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker_path = path.clone();
        let worker = thread::Builder::new()
            .name(WORKER_NAME.to_owned())
            .spawn(move || worker_main(worker_path, command_rx, ready_tx))
            .map_err(|error| IndexError::Database(format!("启动索引线程失败：{error}")))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                path,
                tx: command_tx,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(IndexError::Database(error))
            }
            Err(_) => {
                let _ = worker.join();
                Err(IndexError::WorkerPanic)
            }
        }
    }

    pub fn open_app_data(paths: &AppDataPaths) -> Result<Self, IndexError> {
        paths
            .ensure_root()
            .map_err(|error| IndexError::Database(error.to_string()))?;
        Self::open(paths.index_file())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn ingest(&self, batch: IndexBatch) -> Result<IngestReport, IndexError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Command::Ingest(batch, reply_tx))
            .map_err(|_| IndexError::Closed)?;
        recv_database(reply_rx)
    }

    pub fn complete_generation(
        &self,
        generation: u64,
        seen_logical_paths: impl IntoIterator<Item = String>,
    ) -> Result<(), IndexError> {
        if generation == 0 || generation > i64::MAX as u64 {
            return Err(IndexError::InvalidProjection(
                "generation 必须在 1..=i64::MAX".to_owned(),
            ));
        }
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Command::Complete(
                generation,
                seen_logical_paths.into_iter().collect(),
                reply_tx,
            ))
            .map_err(|_| IndexError::Closed)?;
        recv_database(reply_rx)
    }

    pub fn next_generation(&self) -> Result<u64, IndexError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Command::NextGeneration(reply_tx))
            .map_err(|_| IndexError::Closed)?;
        recv_database(reply_rx)
    }

    pub fn stats(&self) -> Result<IndexStats, IndexError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Command::Stats(reply_tx))
            .map_err(|_| IndexError::Closed)?;
        recv_database(reply_rx)
    }

    pub fn file_for_alias(
        &self,
        logical_path: impl Into<String>,
    ) -> Result<Option<IndexFileRecord>, IndexError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Command::File(logical_path.into(), reply_tx))
            .map_err(|_| IndexError::Closed)?;
        recv_database(reply_rx)
    }

    pub fn search(&self, request: SearchRequest) -> Result<SearchPage, IndexError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Command::Search(request, reply_tx))
            .map_err(|_| IndexError::Closed)?;
        match reply_rx.recv() {
            Ok(Ok(page)) => Ok(page),
            Ok(Err(error)) => Err(error.into()),
            Err(_) => Err(IndexError::WorkerPanic),
        }
    }

}

impl Drop for IndexStore {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn recv_database<T>(receiver: Receiver<Result<T, String>>) -> Result<T, IndexError> {
    match receiver.recv() {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(IndexError::Database(error)),
        Err(_) => Err(IndexError::WorkerPanic),
    }
}

fn worker_main(
    path: PathBuf,
    commands: Receiver<Command>,
    ready: mpsc::SyncSender<Result<(), String>>,
) {
    let mut connection = match db::open_or_rebuild(&path) {
        Ok(connection) => connection,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        return;
    }
    for command in commands {
        match command {
            Command::Ingest(batch, reply) => {
                let _ = reply.send(ingest::replace_batch(&mut connection, batch));
            }
            Command::Complete(generation, seen, reply) => {
                let _ = reply.send(ingest::complete_generation(
                    &mut connection,
                    generation,
                    seen,
                ));
            }
            Command::NextGeneration(reply) => {
                let _ = reply.send(db::next_generation(&connection));
            }
            Command::Stats(reply) => {
                let _ = reply.send(db::stats(&connection));
            }
            Command::File(logical_path, reply) => {
                let _ = reply.send(db::file_record(&connection, &logical_path));
            }
            Command::Search(request, reply) => {
                let _ = reply.send(query::search(&connection, request));
            }
            Command::Shutdown => break,
        }
    }
}

pub(crate) fn validate_batch(batch: &IndexBatch) -> Result<(), IndexError> {
    if batch.generation == 0 || batch.generation > i64::MAX as u64 {
        return Err(IndexError::InvalidProjection(
            "generation 必须在 1..=i64::MAX".to_owned(),
        ));
    }
    let mut paths = HashSet::new();
    for file in &batch.files {
        if file.resolved_path.is_empty() {
            return Err(IndexError::InvalidProjection(
                "resolved_path 不能为空".to_owned(),
            ));
        }
        if !paths.insert(&file.resolved_path) {
            return Err(IndexError::InvalidProjection(format!(
                "批次包含重复 resolved_path：{}",
                file.resolved_path
            )));
        }
        if file.generation != batch.generation {
            return Err(IndexError::InvalidProjection(
                "文件代次必须与批次代次一致".to_owned(),
            ));
        }
        if file.content_hash.len() != 32 {
            return Err(IndexError::InvalidProjection(
                "content_hash 必须是 32 字节 BLAKE3 摘要".to_owned(),
            ));
        }
        let mut aliases = HashSet::new();
        for alias in &file.aliases {
            if alias.logical_path.is_empty() || !aliases.insert(&alias.logical_path) {
                return Err(IndexError::InvalidProjection(
                    "同一文件的 logical alias 不能为空且不得重复".to_owned(),
                ));
            }
        }
        for task in &file.tasks {
            if task.source_start > task.source_end {
                return Err(IndexError::InvalidProjection(
                    "Task source range 无效".to_owned(),
                ));
            }
            if task
                .priority
                .is_some_and(|priority| !(1..=9).contains(&priority))
            {
                return Err(IndexError::InvalidProjection(
                    "Task priority 必须在 1..=9".to_owned(),
                ));
            }
        }
    }
    Ok(())
}
