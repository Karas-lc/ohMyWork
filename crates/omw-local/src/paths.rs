use std::{
    env, fmt, fs, io,
    path::{Path, PathBuf},
};

const APP_DIRECTORY_NAME: &str = "ohMyWork";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppDataPaths {
    root: PathBuf,
}

#[derive(Debug)]
pub enum PathsError {
    MissingLocalAppData,
    InvalidBase(PathBuf),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for PathsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingLocalAppData => {
                formatter.write_str("LOCALAPPDATA 未设置，无法确定应用数据目录")
            }
            Self::InvalidBase(path) => {
                write!(formatter, "应用数据目录必须是绝对路径：{}", path.display())
            }
            Self::Io {
                operation,
                path,
                source,
            } => {
                write!(formatter, "{operation} {} 失败：{source}", path.display())
            }
        }
    }
}

impl std::error::Error for PathsError {}

impl AppDataPaths {
    pub fn from_local_app_data() -> Result<Self, PathsError> {
        let value = env::var_os("LOCALAPPDATA").ok_or(PathsError::MissingLocalAppData)?;
        Self::from_base(PathBuf::from(value).join(APP_DIRECTORY_NAME))
    }

    pub fn from_base(root: impl Into<PathBuf>) -> Result<Self, PathsError> {
        let root = root.into();
        if root.as_os_str().is_empty() || !root.is_absolute() {
            return Err(PathsError::InvalidBase(root));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub fn index_file(&self) -> PathBuf {
        self.root.join("index.sqlite")
    }

    pub fn ui_state_file(&self) -> PathBuf {
        self.root.join("ui-state.json")
    }

    pub fn keymap_file(&self) -> PathBuf {
        self.root.join("keymap.jsonc")
    }

    pub fn themes_dir(&self) -> PathBuf {
        self.root.join("themes")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn ensure_root(&self) -> Result<(), PathsError> {
        fs::create_dir_all(&self.root).map_err(|source| PathsError::Io {
            operation: "创建 LocalAppData 目录",
            path: self.root.clone(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_app_data_children_are_stable() {
        let paths =
            AppDataPaths::from_base(PathBuf::from(r"C:\LocalAppData\ohMyWork")).expect("base");
        assert_eq!(
            paths.ui_state_file(),
            PathBuf::from(r"C:\LocalAppData\ohMyWork\ui-state.json")
        );
        assert_eq!(
            paths.keymap_file(),
            PathBuf::from(r"C:\LocalAppData\ohMyWork\keymap.jsonc")
        );
        assert_eq!(
            paths.themes_dir(),
            PathBuf::from(r"C:\LocalAppData\ohMyWork\themes")
        );
        assert_eq!(
            paths.logs_dir(),
            PathBuf::from(r"C:\LocalAppData\ohMyWork\logs")
        );
    }
}

