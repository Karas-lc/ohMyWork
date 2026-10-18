mod atomic_write;
mod identity;
mod ignore;
mod index;
mod paths;
mod scan;
mod state;
mod watch;
mod workspace;
mod write_queue;
pub mod portable;
pub use atomic_write::{
    AtomicWriteError, CommitAck, DiskSnapshot, DiskVersion, FileIdentity, atomic_write,
    read_snapshot, read_version,
};
pub use identity::{
    DocumentKey, filesystem_identity, is_application_temporary, normalize_document_path,
    resolve_filesystem_path,
};
pub use ignore::{BUILTIN_IGNORE_PATTERNS, IGNORE_FILE_NAME, IgnoreError, IgnoreRules};
pub use index::{
    FileProjection, IndexAlias, IndexArea, IndexBatch, IndexError, IndexFileRecord, IndexStats,
    IndexStore, IndexTask, IngestReport, ScanIngestReport, SearchCancelToken, SearchMatchKind,
    SearchPage, SearchRequest, SearchResult,
};
pub use paths::{AppDataPaths, PathsError};
pub use scan::{
    AliasRecord, FileOperationError, LinkError, LinkTarget, ResolvedLink, ScanDiagnostic,
    ScanEntry, ScanEntryKind, ScanError, ScanOptions, ScanReport, create_directory,
    create_markdown, move_entry, rename_entry, resolve_link, reveal_in_system,
    scan_directory_children, scan_workspace,
};
pub use state::{
    PiLaunchDescription, SETTINGS_SCHEMA_VERSION, Settings, SettingsError, SettingsStore,
    StateError, WorkspaceState,
};
pub use watch::{
    ChangeTracker, FileWatcher, WATCH_EVENT_QUEUE_CAPACITY, WatchBatch, WatchChange,
    WatchEventQueue, WatchQueueError,
};
pub use workspace::{Workspace, WorkspaceError, WorkspacePaths};
pub use portable::{
    disable_portable, enable_portable, is_portable_mode, portable_marker_path,
    relativize_paths, absolutize_paths, resolve_ui_state_path, workspace_ui_state_path,
    PortableMarker, PortableSwitchError, PORTABLE_SCHEMA_VERSION,
};
pub use write_queue::{ORDINARY_QUEUE_CAPACITY, QueueError, WriteQueue};
