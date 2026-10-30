use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AppDataPaths, PathsError, Workspace, WorkspaceError};

pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PiLaunchDescription {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub workspace_root: PathBuf,
    pub workspace_uuid: Uuid,
    pub pi_launch: Option<PiLaunchDescription>,
}

#[derive(Debug)]
pub enum SettingsError {
    Paths(PathsError),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    UnsupportedSchema {
        path: PathBuf,
        version: u32,
    },
    InvalidField {
        path: PathBuf,
        field: &'static str,
        reason: String,
    },
}

impl fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Paths(error) => error.fmt(formatter),
            Self::Io {
                operation,
                path,
                source,
            } => write!(formatter, "{operation} {} 失败：{source}", path.display()),
            Self::Json { path, source } => {
                write!(formatter, "解析设置 {} 失败：{source}", path.display())
            }
            Self::UnsupportedSchema { path, version } => write!(
                formatter,
                "设置 {} 使用不支持的 schema 版本 {version}；原文件已保留",
                path.display()
            ),
            Self::InvalidField {
                path,
                field,
                reason,
            } => write!(
                formatter,
                "设置 {} 的 {field} 无效：{reason}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for SettingsError {}

impl From<PathsError> for SettingsError {
    fn from(error: PathsError) -> Self {
        Self::Paths(error)
    }
}

#[derive(Clone, Debug)]
pub struct SettingsStore {
    path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct SettingsFile {
    schema_version: u32,
    workspace_root: PathBuf,
    workspace_uuid: Uuid,
    #[serde(default)]
    pi_launch: Option<PiLaunchDescription>,
}

impl SettingsStore {
    pub fn new(paths: &AppDataPaths) -> Self {
        Self {
            path: paths.settings_file(),
        }
    }

    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Option<Settings>, SettingsError> {
        let content = match fs::read_to_string(&self.path) {
            Ok(content) => content,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(SettingsError::Io {
                    operation: "读取设置",
                    path: self.path.clone(),
                    source,
                });
            }
        };
        let file: SettingsFile =
            serde_json::from_str(&content).map_err(|source| SettingsError::Json {
                path: self.path.clone(),
                source,
            })?;
        if file.schema_version != SETTINGS_SCHEMA_VERSION {
            return Err(SettingsError::UnsupportedSchema {
                path: self.path.clone(),
                version: file.schema_version,
            });
        }
        validate_settings(&self.path, &file.workspace_root, file.workspace_uuid)?;
        Ok(Some(Settings {
            workspace_root: file.workspace_root,
            workspace_uuid: file.workspace_uuid,
            pi_launch: file.pi_launch,
        }))
    }

    pub fn save(&self, settings: &Settings) -> Result<(), SettingsError> {
        validate_settings(
            &self.path,
            &settings.workspace_root,
            settings.workspace_uuid,
        )?;
        let parent = self.path.parent().ok_or_else(|| SettingsError::Io {
            operation: "确定设置目录",
            path: self.path.clone(),
            source: io::Error::new(io::ErrorKind::InvalidInput, "设置路径没有父目录"),
        })?;
        fs::create_dir_all(parent).map_err(|source| SettingsError::Io {
            operation: "创建设置目录",
            path: parent.to_path_buf(),
            source,
        })?;

        let file = SettingsFile {
            schema_version: SETTINGS_SCHEMA_VERSION,
            workspace_root: settings.workspace_root.clone(),
            workspace_uuid: settings.workspace_uuid,
            pi_launch: settings.pi_launch.clone(),
        };
        let content =
            serde_json::to_string_pretty(&file).map_err(|source| SettingsError::Json {
                path: self.path.clone(),
                source,
            })? + "\n";
        let temporary = temporary_path(&self.path);
        let result = write_and_replace(&temporary, &self.path, content.as_bytes());
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceState {
    pub root: PathBuf,
    pub workspace_uuid: Uuid,
    app_data: AppDataPaths,
}

#[derive(Debug)]
pub enum StateError {
    Paths(PathsError),
    Workspace(WorkspaceError),
    Settings(SettingsError),
}

impl fmt::Display for StateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Paths(error) => error.fmt(formatter),
            Self::Workspace(error) => error.fmt(formatter),
            Self::Settings(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for StateError {}

impl From<PathsError> for StateError {
    fn from(error: PathsError) -> Self {
        Self::Paths(error)
    }
}
impl From<WorkspaceError> for StateError {
    fn from(error: WorkspaceError) -> Self {
        Self::Workspace(error)
    }
}
impl From<SettingsError> for StateError {
    fn from(error: SettingsError) -> Self {
        Self::Settings(error)
    }
}

impl WorkspaceState {
    pub fn initialize(
        root: impl Into<PathBuf>,
        app_data: AppDataPaths,
    ) -> Result<Self, StateError> {
        let provision = Workspace::provision(root)?;
        let workspace = provision.workspace().clone();
        let settings = Settings {
            workspace_root: workspace.root().to_path_buf(),
            workspace_uuid: workspace.uuid(),
            pi_launch: None,
        };
        let store = SettingsStore::new(&app_data);
        if let Err(error) = store.save(&settings) {
            provision.rollback();
            return Err(StateError::Settings(error));
        }
        let workspace = provision.commit();
        Ok(Self {
            root: workspace.root().to_path_buf(),
            workspace_uuid: workspace.uuid(),
            app_data,
        })
    }

    pub fn load(app_data: AppDataPaths) -> Result<Option<Self>, StateError> {
        let store = SettingsStore::new(&app_data);
        let Some(settings) = store.load()? else {
            return Ok(None);
        };
        let workspace = Workspace::open(&settings.workspace_root, settings.workspace_uuid)?;
        Ok(Some(Self {
            root: workspace.root().to_path_buf(),
            workspace_uuid: workspace.uuid(),
            app_data,
        }))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn workspace_uuid(&self) -> Uuid {
        self.workspace_uuid
    }
    pub fn app_data(&self) -> &AppDataPaths {
        &self.app_data
    }
}

fn validate_settings(path: &Path, root: &Path, uuid: Uuid) -> Result<(), SettingsError> {
    if root.as_os_str().is_empty() || !root.is_absolute() {
        return Err(SettingsError::InvalidField {
            path: path.to_path_buf(),
            field: "workspace_root",
            reason: "必须是非空绝对路径".to_owned(),
        });
    }
    if uuid.is_nil() {
        return Err(SettingsError::InvalidField {
            path: path.to_path_buf(),
            field: "workspace_uuid",
            reason: "不能是 nil UUID".to_owned(),
        });
    }
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let suffix = format!(".{}.{}.tmp", std::process::id(), nonce);
    let mut temporary = path.to_path_buf();
    let name = path
        .file_name()
        .map_or_else(|| "settings.json".into(), |name| name.to_os_string());
    temporary.set_file_name(format!("{}{}", name.to_string_lossy(), suffix));
    temporary
}

fn write_and_replace(
    temporary: &Path,
    destination: &Path,
    content: &[u8],
) -> Result<(), SettingsError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary)
        .map_err(|source| SettingsError::Io {
            operation: "创建设置临时文件",
            path: temporary.to_path_buf(),
            source,
        })?;
    use io::Write as _;
    file.write_all(content)
        .and_then(|_| file.sync_all())
        .map_err(|source| SettingsError::Io {
            operation: "写入设置临时文件",
            path: temporary.to_path_buf(),
            source,
        })?;
    replace_file(temporary, destination)
}

#[cfg(not(windows))]
fn replace_file(temporary: &Path, destination: &Path) -> Result<(), SettingsError> {
    fs::rename(temporary, destination).map_err(|source| SettingsError::Io {
        operation: "替换设置文件",
        path: destination.to_path_buf(),
        source,
    })
}

#[cfg(windows)]
fn replace_file(temporary: &Path, destination: &Path) -> Result<(), SettingsError> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows::{
        Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        },
        core::PCWSTR,
    };
    let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let moved = unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved.is_ok() {
        Ok(())
    } else {
        Err(SettingsError::Io {
            operation: "原子替换设置文件",
            path: destination.to_path_buf(),
            source: io::Error::last_os_error(),
        })
    }
}
