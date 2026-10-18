use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileIdentity {
    pub volume_serial: u64,
    pub file_index: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskVersion {
    pub blake3: [u8; 32],
    pub len: u64,
    pub file_identity: Option<FileIdentity>,
}

impl DiskVersion {
    pub fn same_content(&self, other: &Self) -> bool {
        self.blake3 == other.blake3 && self.len == other.len
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskSnapshot {
    pub bytes: Vec<u8>,
    pub version: DiskVersion,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitAck {
    pub path: PathBuf,
    pub request_revision: u64,
    pub old_version: Option<DiskVersion>,
    pub new_version: DiskVersion,
    pub bytes_written: u64,
}

#[derive(Debug)]
pub enum AtomicWriteError {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    ExternalChanged {
        path: PathBuf,
        expected: Box<Option<DiskVersion>>,
        actual: Box<Option<DiskVersion>>,
    },
}

impl fmt::Display for AtomicWriteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(formatter, "{operation} {} 失败：{source}", path.display()),
            Self::ExternalChanged { path, .. } => write!(
                formatter,
                "文件 {} 在保存期间发生外部变化；未覆盖磁盘版本",
                path.display()
            ),
        }
    }
}

impl std::error::Error for AtomicWriteError {}

pub fn read_snapshot(path: impl AsRef<Path>) -> Result<DiskSnapshot, AtomicWriteError> {
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|source| AtomicWriteError::Io {
        operation: "读取文件",
        path: path.to_path_buf(),
        source,
    })?;
    let metadata = fs::metadata(path).map_err(|source| AtomicWriteError::Io {
        operation: "读取文件版本",
        path: path.to_path_buf(),
        source,
    })?;
    Ok(DiskSnapshot {
        version: version_for(&bytes, &metadata),
        bytes,
    })
}

pub fn read_version(path: impl AsRef<Path>) -> Result<Option<DiskVersion>, AtomicWriteError> {
    let path = path.as_ref();
    match read_snapshot(path) {
        Ok(snapshot) => Ok(Some(snapshot.version)),
        Err(AtomicWriteError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

pub fn atomic_write(
    path: impl AsRef<Path>,
    expected: Option<&DiskVersion>,
    content: &[u8],
    request_revision: u64,
) -> Result<CommitAck, AtomicWriteError> {
    let path = path.as_ref();
    let actual = read_version(path)?;
    if actual.as_ref() != expected {
        return Err(AtomicWriteError::ExternalChanged {
            path: path.to_path_buf(),
            expected: Box::new(expected.cloned()),
            actual: Box::new(actual),
        });
    }

    let temporary = temporary_path(path);
    let write_result = write_temporary(&temporary, content);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }

    let actual_before_replace = read_version(path)?;
    if actual_before_replace.as_ref() != expected {
        let _ = fs::remove_file(&temporary);
        return Err(AtomicWriteError::ExternalChanged {
            path: path.to_path_buf(),
            expected: Box::new(expected.cloned()),
            actual: Box::new(actual_before_replace),
        });
    }

    if let Err(error) = replace_destination(&temporary, path, expected.is_some()) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }

    let committed = read_snapshot(path)?;
    Ok(CommitAck {
        path: path.to_path_buf(),
        request_revision,
        old_version: expected.cloned(),
        new_version: committed.version,
        bytes_written: content.len() as u64,
    })
}

fn version_for(bytes: &[u8], metadata: &fs::Metadata) -> DiskVersion {
    DiskVersion {
        blake3: *blake3::hash(bytes).as_bytes(),
        len: bytes.len() as u64,
        file_identity: file_identity(metadata),
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let nonce = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let name = path
        .file_name()
        .map_or_else(|| "document".into(), |name| name.to_os_string());
    path.with_file_name(format!(
        ".{}.ohmywork-{time:x}-{nonce:x}.tmp",
        name.to_string_lossy()
    ))
}

fn write_temporary(path: &Path, content: &[u8]) -> Result<(), AtomicWriteError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| AtomicWriteError::Io {
            operation: "创建原子写盘临时文件",
            path: path.to_path_buf(),
            source,
        })?;
    use io::Write as _;
    file.write_all(content)
        .and_then(|_| file.sync_all())
        .map_err(|source| AtomicWriteError::Io {
            operation: "写入并刷新原子写盘临时文件",
            path: path.to_path_buf(),
            source,
        })
}

fn replace_destination(
    temporary: &Path,
    destination: &Path,
    destination_exists: bool,
) -> Result<(), AtomicWriteError> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;
        use windows::{
            Win32::Storage::FileSystem::{
                MOVEFILE_WRITE_THROUGH, MoveFileExW, REPLACEFILE_WRITE_THROUGH, ReplaceFileW,
            },
            core::PCWSTR,
        };
        let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let result = if destination_exists {
            unsafe {
                ReplaceFileW(
                    PCWSTR(to.as_ptr()),
                    PCWSTR(from.as_ptr()),
                    PCWSTR::null(),
                    REPLACEFILE_WRITE_THROUGH,
                    None,
                    None,
                )
            }
        } else {
            unsafe {
                MoveFileExW(
                    PCWSTR(from.as_ptr()),
                    PCWSTR(to.as_ptr()),
                    MOVEFILE_WRITE_THROUGH,
                )
            }
        };
        result.map_err(|error| AtomicWriteError::Io {
            operation: if destination_exists {
                "Windows 原子替换文件"
            } else {
                "Windows 原子移动新文件"
            },
            path: destination.to_path_buf(),
            source: io::Error::from_raw_os_error(error.code().0),
        })
    }
    #[cfg(not(windows))]
    {
        if destination_exists {
            fs::rename(temporary, destination).map_err(|source| AtomicWriteError::Io {
                operation: "原子替换文件",
                path: destination.to_path_buf(),
                source,
            })
        } else {
            fs::hard_link(temporary, destination)
                .and_then(|_| fs::remove_file(temporary))
                .map_err(|source| AtomicWriteError::Io {
                    operation: "原子创建新文件",
                    path: destination.to_path_buf(),
                    source,
                })
        }
    }
}

#[cfg(windows)]
fn file_identity(_metadata: &fs::Metadata) -> Option<FileIdentity> {
    // Snapshot versioning intentionally remains content-based on Windows.
    None
}

#[cfg(unix)]
fn file_identity(metadata: &fs::Metadata) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt as _;
    Some(FileIdentity {
        volume_serial: metadata.dev(),
        file_index: metadata.ino(),
    })
}

#[cfg(not(any(windows, unix)))]
fn file_identity(_metadata: &fs::Metadata) -> Option<FileIdentity> {
    None
}

#[cfg(windows)]
pub(crate) fn file_identity_for_traversal(
    path: &Path,
    _metadata: &fs::Metadata,
) -> Option<FileIdentity> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows::Win32::{
        Foundation::CloseHandle,
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            GetFileInformationByHandle, OPEN_EXISTING,
        },
    };
    use windows::core::PCWSTR;

    let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    wide.push(0);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_DELETE | FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
        .ok()?
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    let result = unsafe { GetFileInformationByHandle(handle, &mut info) };
    unsafe {
        let _ = CloseHandle(handle);
    }
    result.ok()?;
    Some(FileIdentity {
        volume_serial: info.dwVolumeSerialNumber as u64,
        file_index: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    })
}

#[cfg(not(windows))]
pub(crate) fn file_identity_for_traversal(
    _path: &Path,
    metadata: &fs::Metadata,
) -> Option<FileIdentity> {
    file_identity(metadata)
}
