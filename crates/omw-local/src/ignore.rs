use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

pub const IGNORE_FILE_NAME: &str = ".ohmyworkignore";
pub const BUILTIN_IGNORE_PATTERNS: &[&str] = &[".git/", ".ohmywork/", "**/.ohmywork-*.tmp"];

#[derive(Clone, Debug)]
pub struct IgnoreRules {
    root: PathBuf,
    matcher: Gitignore,
}

#[derive(Debug)]
pub enum IgnoreError {
    RootNotDirectory {
        path: PathBuf,
        source: io::Error,
    },
    RulesFile {
        path: PathBuf,
        source: ignore::Error,
    },
    BuiltinRule {
        pattern: &'static str,
        source: ignore::Error,
    },
    Build {
        source: ignore::Error,
    },
}

impl fmt::Display for IgnoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RootNotDirectory { path, source } => {
                write!(f, "读取 Workspace 根目录 {} 失败：{source}", path.display())
            }
            Self::RulesFile { path, source } => {
                write!(f, "读取 {} 失败：{source}", path.display())
            }
            Self::BuiltinRule { pattern, source } => {
                write!(f, "构造内置 ignore 规则 {pattern:?} 失败：{source}")
            }
            Self::Build { source } => write!(f, "构造 ignore 匹配器失败：{source}"),
        }
    }
}

impl std::error::Error for IgnoreError {}

impl IgnoreRules {
    pub fn load(root: impl Into<PathBuf>) -> Result<Self, IgnoreError> {
        let root = root.into();
        let metadata = fs::metadata(&root).map_err(|source| IgnoreError::RootNotDirectory {
            path: root.clone(),
            source,
        })?;
        if !metadata.is_dir() {
            return Err(IgnoreError::RootNotDirectory {
                path: root,
                source: io::Error::new(io::ErrorKind::NotADirectory, "不是目录"),
            });
        }

        let root = fs::canonicalize(&root)
            .map_err(|source| IgnoreError::RootNotDirectory { path: root, source })?;
        let mut builder = GitignoreBuilder::new(&root);
        for pattern in BUILTIN_IGNORE_PATTERNS {
            builder
                .add_line(None, pattern)
                .map_err(|source| IgnoreError::BuiltinRule { pattern, source })?;
        }

        let rules_path = root.join(IGNORE_FILE_NAME);
        if rules_path.is_file()
            && let Some(source) = builder.add(&rules_path)
        {
            return Err(IgnoreError::RulesFile {
                path: rules_path,
                source,
            });
        }

        let matcher = builder
            .build()
            .map_err(|source| IgnoreError::Build { source })?;
        Ok(Self { root, matcher })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn rules_path(&self) -> PathBuf {
        self.root.join(IGNORE_FILE_NAME)
    }

    pub fn is_ignored(&self, logical_path: impl AsRef<Path>, is_dir: bool) -> bool {
        matches!(
            self.matcher
                .matched_path_or_any_parents(self.root.join(logical_path), is_dir),
            Match::Ignore(_)
        )
    }

    pub fn matcher(&self) -> &Gitignore {
        &self.matcher
    }
}
