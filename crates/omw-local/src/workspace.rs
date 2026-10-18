use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use uuid::Uuid;

const METADATA_DIRECTORY: &str = ".ohmywork";
const ID_FILE: &str = "id";
const PARTITIONS: [&str; 3] = ["Atlas", "Efforts", "Archive"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspacePaths {
    pub root: PathBuf,
    pub atlas: PathBuf,
    pub efforts: PathBuf,
    pub archive: PathBuf,
    pub metadata: PathBuf,
    pub id_file: PathBuf,
}

impl WorkspacePaths {
    fn new(root: PathBuf) -> Self {
        let metadata = root.join(METADATA_DIRECTORY);
        Self {
            atlas: root.join(PARTITIONS[0]),
            efforts: root.join(PARTITIONS[1]),
            archive: root.join(PARTITIONS[2]),
            id_file: metadata.join(ID_FILE),
            metadata,
            root,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    paths: WorkspacePaths,
    uuid: Uuid,
}

#[derive(Debug)]
pub enum WorkspaceError {
    RootMustBeAbsolute(PathBuf),
    RootNotDirectory(PathBuf),
    RootIo {
        path: PathBuf,
        source: io::Error,
    },
    EntryMustBeDirectory {
        path: PathBuf,
        name: &'static str,
    },
    EntryIsSymlink {
        path: PathBuf,
        name: &'static str,
    },
    MetadataMustBeDirectory(PathBuf),
    IdIsSymlink(PathBuf),
    InvalidId {
        path: PathBuf,
        reason: String,
    },
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RootMustBeAbsolute(path) => write!(
                formatter,
                "Workspace 根目录必须是绝对路径：{}",
                path.display()
            ),
            Self::RootNotDirectory(path) => {
                write!(formatter, "Workspace 根目录不是目录：{}", path.display())
            }
            Self::RootIo { path, source } => write!(
                formatter,
                "访问 Workspace 根目录 {} 失败：{source}",
                path.display()
            ),
            Self::EntryMustBeDirectory { path, name } => {
                write!(formatter, "{name} 已存在但不是目录：{}", path.display())
            }
            Self::EntryIsSymlink { path, name } => {
                write!(formatter, "{name} 不能是符号链接：{}", path.display())
            }
            Self::MetadataMustBeDirectory(path) => {
                write!(formatter, ".ohmywork 已存在但不是目录：{}", path.display())
            }
            Self::IdIsSymlink(path) => {
                write!(formatter, ".ohmywork/id 不能是符号链接：{}", path.display())
            }
            Self::InvalidId { path, reason } => write!(
                formatter,
                "Workspace 身份文件 {} 无效：{reason}",
                path.display()
            ),
            Self::Io {
                operation,
                path,
                source,
            } => write!(formatter, "{operation} {} 失败：{source}", path.display()),
        }
    }
}

impl std::error::Error for WorkspaceError {}

pub(crate) struct WorkspaceProvision {
    workspace: Workspace,
    created_directories: Vec<PathBuf>,
    created_id: bool,
}

impl WorkspaceProvision {
    pub(crate) fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub(crate) fn rollback(mut self) {
        if self.created_id {
            let _ = fs::remove_file(&self.workspace.paths.id_file);
        }
        for directory in self.created_directories.drain(..).rev() {
            let _ = fs::remove_dir(&directory);
        }
    }

    pub(crate) fn commit(self) -> Workspace {
        self.workspace
    }
}

impl Workspace {
    pub fn initialize(root: impl Into<PathBuf>) -> Result<Self, WorkspaceError> {
        let provision = Self::provision(root)?;
        Ok(provision.commit())
    }

    pub(crate) fn provision(
        root: impl Into<PathBuf>,
    ) -> Result<WorkspaceProvision, WorkspaceError> {
        let root = validate_root(root.into())?;
        let paths = WorkspacePaths::new(root);
        preflight(&paths)?;

        let mut created_directories = Vec::new();
        for (name, directory) in [
            ("Atlas", &paths.atlas),
            ("Efforts", &paths.efforts),
            ("Archive", &paths.archive),
            (".ohmywork", &paths.metadata),
        ] {
            match fs::symlink_metadata(directory) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => {
                    rollback_directories(&created_directories);
                    return Err(if name == ".ohmywork" {
                        WorkspaceError::MetadataMustBeDirectory(directory.clone())
                    } else {
                        WorkspaceError::EntryMustBeDirectory {
                            path: directory.clone(),
                            name,
                        }
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    match fs::create_dir(directory) {
                        Ok(()) => created_directories.push(directory.clone()),
                        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {}
                        Err(source) => {
                            rollback_directories(&created_directories);
                            return Err(WorkspaceError::Io {
                                operation: "创建 Workspace 目录",
                                path: directory.clone(),
                                source,
                            });
                        }
                    }
                }
                Err(source) => {
                    rollback_directories(&created_directories);
                    return Err(WorkspaceError::Io {
                        operation: "检查 Workspace 目录",
                        path: directory.clone(),
                        source,
                    });
                }
            }
        }

        let (uuid, created_id) = match read_or_create_id(&paths.id_file) {
            Ok(result) => result,
            Err(error) => {
                rollback_directories(&created_directories);
                return Err(error);
            }
        };

        Ok(WorkspaceProvision {
            workspace: Workspace { paths, uuid },
            created_directories,
            created_id,
        })
    }

    pub(crate) fn open(
        root: impl Into<PathBuf>,
        expected_uuid: Uuid,
    ) -> Result<Self, WorkspaceError> {
        let root = validate_root(root.into())?;
        let paths = WorkspacePaths::new(root);
        preflight(&paths)?;
        require_partitions(&paths)?;
        let (uuid, created_id) = read_id(&paths.id_file)?;
        if created_id || uuid != expected_uuid {
            return Err(WorkspaceError::InvalidId {
                path: paths.id_file,
                reason: "与设置中的 Workspace UUID 不匹配".to_owned(),
            });
        }
        Ok(Self { paths, uuid })
    }

    pub fn root(&self) -> &Path {
        &self.paths.root
    }

    pub fn paths(&self) -> &WorkspacePaths {
        &self.paths
    }

    pub fn uuid(&self) -> Uuid {
        self.uuid
    }
}

fn validate_root(root: PathBuf) -> Result<PathBuf, WorkspaceError> {
    if root.as_os_str().is_empty() || !root.is_absolute() {
        return Err(WorkspaceError::RootMustBeAbsolute(root));
    }
    let metadata = fs::metadata(&root).map_err(|source| WorkspaceError::RootIo {
        path: root.clone(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(WorkspaceError::RootNotDirectory(root));
    }
    fs::read_dir(&root).map_err(|source| WorkspaceError::RootIo {
        path: root.clone(),
        source,
    })?;
    fs::canonicalize(&root).map_err(|source| WorkspaceError::RootIo { path: root, source })
}

fn preflight(paths: &WorkspacePaths) -> Result<(), WorkspaceError> {
    for (name, path) in [
        ("Atlas", &paths.atlas),
        ("Efforts", &paths.efforts),
        ("Archive", &paths.archive),
    ] {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(WorkspaceError::EntryIsSymlink {
                        path: path.clone(),
                        name,
                    });
                }
                if !metadata.is_dir() {
                    return Err(WorkspaceError::EntryMustBeDirectory {
                        path: path.clone(),
                        name,
                    });
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(WorkspaceError::Io {
                    operation: "检查 Workspace 分区",
                    path: path.clone(),
                    source,
                });
            }
        }
    }

    match fs::symlink_metadata(&paths.metadata) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                Err(WorkspaceError::MetadataMustBeDirectory(
                    paths.metadata.clone(),
                ))
            } else {
                Ok(())
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(WorkspaceError::Io {
            operation: "检查 Workspace 元数据目录",
            path: paths.metadata.clone(),
            source,
        }),
    }
}

fn require_partitions(paths: &WorkspacePaths) -> Result<(), WorkspaceError> {
    for (name, path) in [
        ("Atlas", &paths.atlas),
        ("Efforts", &paths.efforts),
        ("Archive", &paths.archive),
    ] {
        let metadata = fs::symlink_metadata(path).map_err(|source| WorkspaceError::Io {
            operation: "读取 Workspace 分区",
            path: path.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(WorkspaceError::EntryIsSymlink {
                path: path.clone(),
                name,
            });
        }
        if !metadata.is_dir() {
            return Err(WorkspaceError::EntryMustBeDirectory {
                path: path.clone(),
                name,
            });
        }
    }
    Ok(())
}

fn read_or_create_id(path: &Path) -> Result<(Uuid, bool), WorkspaceError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(WorkspaceError::IdIsSymlink(path.to_path_buf()))
        }
        Ok(_) => read_id_after_create_race(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let uuid = Uuid::new_v4();
            let content = format!("{uuid}\n");
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(mut file) => {
                    use io::Write as _;
                    if let Err(source) = file
                        .write_all(content.as_bytes())
                        .and_then(|_| file.sync_all())
                    {
                        let _ = fs::remove_file(path);
                        return Err(WorkspaceError::Io {
                            operation: "写入 Workspace 身份文件",
                            path: path.to_path_buf(),
                            source,
                        });
                    }
                    Ok((uuid, true))
                }
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                    read_id_after_create_race(path)
                }
                Err(source) => Err(WorkspaceError::Io {
                    operation: "创建 Workspace 身份文件",
                    path: path.to_path_buf(),
                    source,
                }),
            }
        }
        Err(source) => Err(WorkspaceError::Io {
            operation: "检查 Workspace 身份文件",
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn read_id_after_create_race(path: &Path) -> Result<(Uuid, bool), WorkspaceError> {
    for _ in 0..100 {
        match read_id(path) {
            Err(WorkspaceError::InvalidId { reason, .. }) if reason == "必须以换行结尾" => {
                thread::yield_now();
                thread::sleep(Duration::from_millis(1));
            }
            result => return result,
        }
    }
    read_id(path)
}

fn read_id(path: &Path) -> Result<(Uuid, bool), WorkspaceError> {
    let content = fs::read_to_string(path).map_err(|source| WorkspaceError::Io {
        operation: "读取 Workspace 身份文件",
        path: path.to_path_buf(),
        source,
    })?;
    let text = content
        .strip_suffix('\n')
        .ok_or_else(|| WorkspaceError::InvalidId {
            path: path.to_path_buf(),
            reason: "必须以换行结尾".to_owned(),
        })?;
    if text.contains(['\n', '\r']) {
        return Err(WorkspaceError::InvalidId {
            path: path.to_path_buf(),
            reason: "必须只有一行 UUID".to_owned(),
        });
    }
    let uuid = Uuid::parse_str(text).map_err(|error| WorkspaceError::InvalidId {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })?;
    Ok((uuid, false))
}

fn rollback_directories(directories: &[PathBuf]) {
    for directory in directories.iter().rev() {
        let _ = fs::remove_dir(directory);
    }
}
