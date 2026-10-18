use std::{
    collections::{HashMap, HashSet},
    fmt, fs, io,
    path::{Component, Path, PathBuf},
    process::Command,
};

use crate::{
    DocumentKey, FileIdentity, IgnoreError, IgnoreRules, filesystem_identity,
    resolve_filesystem_path,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanOptions {
    pub include_excluded: bool,
    pub max_entries: usize,
    pub max_depth: usize,
}

impl ScanOptions {
    pub const DEFAULT_MAX_ENTRIES: usize = 100_000;
    pub const DEFAULT_MAX_DEPTH: usize = 256;

    pub fn including_excluded(mut self, include: bool) -> Self {
        self.include_excluded = include;
        self
    }
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            include_excluded: false,
            max_entries: Self::DEFAULT_MAX_ENTRIES,
            max_depth: Self::DEFAULT_MAX_DEPTH,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanEntryKind {
    File,
    Directory,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanEntry {
    logical_path: PathBuf,
    target_path: PathBuf,
    kind: ScanEntryKind,
    excluded: bool,
    alias_of: Option<PathBuf>,
    is_link: bool,
}

impl ScanEntry {
    pub fn logical_path(&self) -> &Path {
        &self.logical_path
    }

    pub fn target_path(&self) -> &Path {
        &self.target_path
    }

    pub fn kind(&self) -> ScanEntryKind {
        self.kind
    }

    pub fn is_directory(&self) -> bool {
        self.kind == ScanEntryKind::Directory
    }

    pub fn is_file(&self) -> bool {
        self.kind == ScanEntryKind::File
    }

    pub fn is_excluded(&self) -> bool {
        self.excluded
    }

    pub fn is_link(&self) -> bool {
        self.is_link
    }

    pub fn alias_of(&self) -> Option<&Path> {
        self.alias_of.as_deref()
    }

    pub fn id(&self) -> String {
        self.logical_path.to_string_lossy().into_owned()
    }

    pub fn document_key(&self) -> DocumentKey {
        DocumentKey::new(&self.target_path)
    }

    fn new(
        logical_path: PathBuf,
        target_path: PathBuf,
        kind: ScanEntryKind,
        excluded: bool,
        alias_of: Option<PathBuf>,
        is_link: bool,
    ) -> Self {
        Self {
            logical_path,
            target_path,
            kind,
            excluded,
            alias_of,
            is_link,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AliasRecord {
    logical_path: PathBuf,
    target_path: PathBuf,
    target_identity: FileIdentity,
    cycle: bool,
}

impl AliasRecord {
    pub fn logical_path(&self) -> &Path {
        &self.logical_path
    }

    pub fn target_path(&self) -> &Path {
        &self.target_path
    }

    pub fn target_identity(&self) -> &FileIdentity {
        &self.target_identity
    }

    pub fn is_cycle(&self) -> bool {
        self.cycle
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanDiagnostic {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanReport {
    entries: Vec<ScanEntry>,
    aliases: Vec<AliasRecord>,
    diagnostics: Vec<ScanDiagnostic>,
}

impl ScanReport {
    pub fn entries(&self) -> &[ScanEntry] {
        &self.entries
    }

    pub fn aliases(&self) -> &[AliasRecord] {
        &self.aliases
    }

    pub fn diagnostics(&self) -> &[ScanDiagnostic] {
        &self.diagnostics
    }
}

#[derive(Debug)]
pub enum ScanError {
    Ignore(IgnoreError),
    Root { path: PathBuf, source: io::Error },
    InvalidLogicalPath(PathBuf),
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ignore(error) => write!(f, "加载 ignore 规则失败：{error}"),
            Self::Root { path, source } => {
                write!(f, "扫描根目录 {} 失败：{source}", path.display())
            }
            Self::InvalidLogicalPath(path) => {
                write!(f, "文件树逻辑路径无效：{}", path.display())
            }
        }
    }
}

impl std::error::Error for ScanError {}

impl From<IgnoreError> for ScanError {
    fn from(value: IgnoreError) -> Self {
        Self::Ignore(value)
    }
}

#[derive(Clone, Debug)]
struct RawChild {
    name: PathBuf,
}

pub fn scan_workspace(
    root: impl Into<PathBuf>,
    options: ScanOptions,
) -> Result<ScanReport, ScanError> {
    let rules = IgnoreRules::load(root.into())?;
    let root = rules.root().to_path_buf();
    let mut report = ScanReport::default();
    let mut state = TraversalState {
        rules: &rules,
        options,
        cache: HashMap::new(),
        active: HashSet::new(),
        first_identity: HashMap::new(),
        report: &mut report,
    };
    visit_directory(&mut state, &root, Path::new(""), 0)?;
    Ok(report)
}

/// Load one directory for lazy UI expansion. It never recursively walks children.
pub fn scan_directory_children(
    root: impl Into<PathBuf>,
    logical_directory: impl AsRef<Path>,
    options: ScanOptions,
) -> Result<ScanReport, ScanError> {
    let rules = IgnoreRules::load(root.into())?;
    let root = rules.root().to_path_buf();
    let logical_directory = validate_logical_path(logical_directory.as_ref())?;
    let directory = root.join(&logical_directory);
    let metadata = fs::metadata(&directory).map_err(|source| ScanError::Root {
        path: directory.clone(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(ScanError::Root {
            path: directory,
            source: io::Error::new(io::ErrorKind::NotADirectory, "不是目录"),
        });
    }

    let mut report = ScanReport::default();
    let children = read_children(&directory, &mut report)?;
    let mut first_identity = HashMap::new();
    for child in children {
        if report.entries.len() >= options.max_entries {
            report.diagnostics.push(ScanDiagnostic {
                path: logical_directory.clone(),
                message: format!("达到文件树条目上限 {}，其余条目未显示", options.max_entries),
            });
            break;
        }
        append_child(
            &rules,
            &root,
            &logical_directory,
            child,
            options,
            &mut report,
            &mut first_identity,
            &HashSet::new(),
        );
    }
    report
        .entries
        .sort_by(|a, b| a.logical_path.cmp(&b.logical_path));
    Ok(report)
}

struct TraversalState<'a> {
    rules: &'a IgnoreRules,
    options: ScanOptions,
    cache: HashMap<FileIdentity, Vec<RawChild>>,
    active: HashSet<FileIdentity>,
    first_identity: HashMap<FileIdentity, PathBuf>,
    report: &'a mut ScanReport,
}

fn visit_directory(
    state: &mut TraversalState<'_>,
    directory: &Path,
    logical_directory: &Path,
    depth: usize,
) -> Result<(), ScanError> {
    if depth > state.options.max_depth {
        state.report.diagnostics.push(ScanDiagnostic {
            path: logical_directory.to_path_buf(),
            message: format!(
                "达到目录深度上限 {}，其余目录未扫描",
                state.options.max_depth
            ),
        });
        return Ok(());
    }

    let identity = match filesystem_identity(directory) {
        Ok(identity) => identity,
        Err(source) => {
            state.report.diagnostics.push(ScanDiagnostic {
                path: logical_directory.to_path_buf(),
                message: format!("无法读取目录身份：{source}"),
            });
            return Ok(());
        }
    };
    if !state.active.insert(identity) {
        state.report.diagnostics.push(ScanDiagnostic {
            path: logical_directory.to_path_buf(),
            message: "检测到链接目录循环，已停止该分支".to_owned(),
        });
        return Ok(());
    }

    state
        .first_identity
        .entry(identity)
        .or_insert_with(|| logical_directory.to_path_buf());
    let children = if let Some(children) = state.cache.get(&identity) {
        children.clone()
    } else {
        let children = match read_children(directory, state.report) {
            Ok(children) => children,
            Err(_) => {
                state.active.remove(&identity);
                return Ok(());
            }
        };
        state.cache.insert(identity, children.clone());
        children
    };

    for child in children {
        if state.report.entries.len() >= state.options.max_entries {
            state.report.diagnostics.push(ScanDiagnostic {
                path: logical_directory.to_path_buf(),
                message: format!(
                    "达到文件树条目上限 {}，其余条目未扫描",
                    state.options.max_entries
                ),
            });
            break;
        }
        let Some(entry) = append_child(
            state.rules,
            state.rules.root(),
            logical_directory,
            child,
            state.options,
            state.report,
            &mut state.first_identity,
            &state.active,
        ) else {
            continue;
        };
        if entry.is_directory() && (!entry.is_excluded() || state.options.include_excluded) {
            let child_identity = match filesystem_identity(entry.target_path()) {
                Ok(identity) => identity,
                Err(source) => {
                    state.report.diagnostics.push(ScanDiagnostic {
                        path: entry.logical_path().to_path_buf(),
                        message: format!("无法读取目录身份：{source}"),
                    });
                    continue;
                }
            };
            if state.active.contains(&child_identity) {
                state.report.aliases.push(AliasRecord {
                    logical_path: entry.logical_path().to_path_buf(),
                    target_path: entry.target_path().to_path_buf(),
                    target_identity: child_identity,
                    cycle: true,
                });
                continue;
            }
            visit_directory(state, entry.target_path(), entry.logical_path(), depth + 1)?;
        }
    }
    state.active.remove(&identity);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn append_child(
    rules: &IgnoreRules,
    root: &Path,
    logical_directory: &Path,
    child: RawChild,
    options: ScanOptions,
    report: &mut ScanReport,
    first_identity: &mut HashMap<FileIdentity, PathBuf>,
    active: &HashSet<FileIdentity>,
) -> Option<ScanEntry> {
    let logical_path = logical_directory.join(&child.name);
    let physical_path = root.join(&logical_path);
    let symlink_metadata = match fs::symlink_metadata(&physical_path) {
        Ok(metadata) => metadata,
        Err(source) => {
            report.diagnostics.push(ScanDiagnostic {
                path: logical_path,
                message: format!("无法读取目录项：{source}"),
            });
            return None;
        }
    };
    let is_link = symlink_metadata.file_type().is_symlink();
    let target_path = match resolve_filesystem_path(&physical_path) {
        Ok(path) => path,
        Err(source) => {
            report.diagnostics.push(ScanDiagnostic {
                path: logical_path.clone(),
                message: format!("链接目标不可读：{source}"),
            });
            physical_path.clone()
        }
    };
    let metadata = match fs::metadata(&physical_path) {
        Ok(metadata) => metadata,
        Err(source) => {
            report.diagnostics.push(ScanDiagnostic {
                path: logical_path.clone(),
                message: format!("无法读取目标：{source}"),
            });
            return None;
        }
    };
    let kind = if metadata.is_dir() {
        ScanEntryKind::Directory
    } else if metadata.is_file() {
        ScanEntryKind::File
    } else {
        ScanEntryKind::Other
    };
    let excluded = rules.is_ignored(&logical_path, kind == ScanEntryKind::Directory);
    if excluded && !options.include_excluded {
        return None;
    }

    let target_identity = filesystem_identity(&physical_path).ok();
    let alias_of = if is_link {
        target_identity
            .as_ref()
            .and_then(|identity| first_identity.get(identity).cloned())
    } else {
        None
    };
    let entry = ScanEntry::new(logical_path, target_path, kind, excluded, alias_of, is_link);
    if let Some(identity) = target_identity {
        first_identity
            .entry(identity)
            .or_insert_with(|| entry.logical_path().to_path_buf());
        if is_link {
            report.aliases.push(AliasRecord {
                logical_path: entry.logical_path().to_path_buf(),
                target_path: entry.target_path().to_path_buf(),
                target_identity: identity,
                cycle: active.contains(&identity),
            });
        }
    }
    report.entries.push(entry.clone());
    Some(entry)
}

fn read_children(directory: &Path, report: &mut ScanReport) -> Result<Vec<RawChild>, ScanError> {
    let entries = fs::read_dir(directory).map_err(|source| ScanError::Root {
        path: directory.to_path_buf(),
        source,
    })?;
    let mut children = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => children.push(RawChild {
                name: entry.file_name().into(),
            }),
            Err(source) => report.diagnostics.push(ScanDiagnostic {
                path: directory.to_path_buf(),
                message: format!("读取单个目录项失败：{source}"),
            }),
        }
    }
    children.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(children)
}

fn validate_logical_path(path: &Path) -> Result<PathBuf, ScanError> {
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::ParentDir
            )
        })
    {
        return Err(ScanError::InvalidLogicalPath(path.to_path_buf()));
    }
    Ok(path.to_path_buf())
}

#[derive(Debug)]
pub enum FileOperationError {
    InvalidPath(PathBuf),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for FileOperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(path) => write!(f, "文件树路径无效：{}", path.display()),
            Self::Io {
                operation,
                path,
                source,
            } => {
                write!(f, "{operation} {} 失败：{source}", path.display())
            }
        }
    }
}

impl std::error::Error for FileOperationError {}

fn safe_workspace_path(
    root: &Path,
    relative: impl AsRef<Path>,
) -> Result<PathBuf, FileOperationError> {
    let relative = relative.as_ref();
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::ParentDir
            )
        })
    {
        return Err(FileOperationError::InvalidPath(relative.to_path_buf()));
    }
    Ok(root.join(relative))
}

pub fn create_directory(
    root: &Path,
    relative: impl AsRef<Path>,
) -> Result<PathBuf, FileOperationError> {
    let path = safe_workspace_path(root, relative)?;
    fs::create_dir(&path).map_err(|source| FileOperationError::Io {
        operation: "创建目录",
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

pub fn create_markdown(
    root: &Path,
    relative: impl AsRef<Path>,
) -> Result<PathBuf, FileOperationError> {
    let path = safe_workspace_path(root, relative)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|source| FileOperationError::Io {
            operation: "创建 Markdown 文件",
            path: path.clone(),
            source,
        })?;
    use io::Write as _;
    file.write_all(b"# ")
        .map_err(|source| FileOperationError::Io {
            operation: "写入 Markdown 文件",
            path: path.clone(),
            source,
        })?;
    file.sync_all().map_err(|source| FileOperationError::Io {
        operation: "同步 Markdown 文件",
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

pub fn move_entry(
    root: &Path,
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<PathBuf, FileOperationError> {
    let source = safe_workspace_path(root, source)?;
    let destination = safe_workspace_path(root, destination)?;
    fs::rename(&source, &destination).map_err(|source_error| FileOperationError::Io {
        operation: "移动文件树项",
        path: source,
        source: source_error,
    })?;
    Ok(destination)
}

pub fn rename_entry(
    root: &Path,
    source: impl AsRef<Path>,
    new_name: &str,
) -> Result<PathBuf, FileOperationError> {
    if new_name.is_empty()
        || new_name == "."
        || new_name == ".."
        || Path::new(new_name).components().count() != 1
        || Path::new(new_name)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(FileOperationError::InvalidPath(PathBuf::from(new_name)));
    }
    let source = safe_workspace_path(root, source)?;
    let destination = source
        .parent()
        .ok_or_else(|| FileOperationError::InvalidPath(source.clone()))?
        .join(new_name);
    fs::rename(&source, &destination).map_err(|source_error| FileOperationError::Io {
        operation: "重命名文件树项",
        path: source,
        source: source_error,
    })?;
    Ok(destination)
}

pub fn reveal_in_system(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        Command::new("explorer.exe")
            .arg(format!("/select,{}", path.display()))
            .spawn()
            .map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg("-R").arg(path).spawn().map(|_| ())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(path.parent().unwrap_or(path))
            .spawn()
            .map(|_| ())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedLink {
    WorkspacePath {
        path: PathBuf,
        fragment: Option<String>,
    },
    Browser {
        url: String,
    },
    System {
        value: String,
    },
}

pub type LinkTarget = ResolvedLink;

#[derive(Debug, PartialEq, Eq)]
pub enum LinkError {
    Empty,
    InvalidCurrentFile(PathBuf),
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("链接目标为空"),
            Self::InvalidCurrentFile(path) => {
                write!(f, "当前文件路径无效：{}", path.display())
            }
        }
    }
}

impl std::error::Error for LinkError {}

pub fn resolve_link(
    workspace_root: &Path,
    current_file: &Path,
    destination: &str,
) -> Result<ResolvedLink, LinkError> {
    let destination = destination.trim().trim_matches('<').trim_matches('>');
    if destination.is_empty() {
        return Err(LinkError::Empty);
    }
    if let Some(scheme_end) = destination.find(':') {
        let scheme = &destination[..scheme_end];
        if !scheme.is_empty()
            && scheme
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
            && !(scheme.len() == 1 && destination.as_bytes().get(scheme_end + 1) == Some(&b'\\'))
        {
            if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
                return Ok(ResolvedLink::Browser {
                    url: destination.to_owned(),
                });
            }
            return Ok(ResolvedLink::System {
                value: destination.to_owned(),
            });
        }
    }

    let (path_text, fragment) = match destination.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment.to_owned())),
        None => (destination, None),
    };
    if path_text.is_empty() {
        return Err(LinkError::Empty);
    }
    let base = if current_file.is_absolute() {
        current_file.to_path_buf()
    } else {
        workspace_root.join(current_file)
    };
    if !base.exists() && base.extension().is_none() {
        return Err(LinkError::InvalidCurrentFile(current_file.to_path_buf()));
    }
    let base = base
        .parent()
        .ok_or_else(|| LinkError::InvalidCurrentFile(current_file.to_path_buf()))?;
    let path = if path_text.starts_with('/') || path_text.starts_with('\\') {
        lexical_join(workspace_root, path_text.trim_start_matches(['/', '\\']))
    } else {
        lexical_join(base, path_text)
    };
    Ok(ResolvedLink::WorkspacePath { path, fragment })
}

fn lexical_join(base: &Path, suffix: &str) -> PathBuf {
    let mut result = base.to_path_buf();
    for component in Path::new(suffix).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            Component::Normal(part) => result.push(part),
            Component::Prefix(_) | Component::RootDir => {}
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_resolution_uses_current_file_and_workspace_root() {
        let root = Path::new("C:/workspace");
        assert_eq!(
            resolve_link(root, Path::new("Atlas/note.md"), "../other.md"),
            Ok(ResolvedLink::WorkspacePath {
                path: PathBuf::from("C:/workspace/other.md"),
                fragment: None,
            })
        );
        assert_eq!(
            resolve_link(root, Path::new("Atlas/note.md"), "/Atlas/other.md#part"),
            Ok(ResolvedLink::WorkspacePath {
                path: PathBuf::from("C:/workspace/Atlas/other.md"),
                fragment: Some("part".to_owned()),
            })
        );
    }
}
