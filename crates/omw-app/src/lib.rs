pub mod bootstrap;
pub mod workbench;
pub use bootstrap::{BootstrapView, DirectoryPicker};
pub mod commands;
pub mod diagnostics;
pub mod document_session;
pub mod editor;
pub mod file_tree;
pub mod home;
pub mod keymap;
pub mod search;
pub mod state;
pub mod theme;
pub use commands::{CommandOutcome, DocumentCommand, reduce_batch_close, reduce_document_command};
pub use document_session::{
    CloseAction, CloseDecision, DocumentSaveState, ExternalEvent, ExternalEventOutcome,
    SaveRequestError, SaveResult, SaveStatus, apply_save_result, close_action,
    reduce_external_event, submit_save,
};
pub mod ui;
pub use ui::PiPanel;
pub use ui::panes::{
    HomePanel, MarkdownPanel, PersistedDockState, PreviewPanel, panel_for_tab,
    register_pane_factories,
};

pub use ui::TabRef;
pub use ui::preview::{PreviewClient, PreviewHandle, PreviewUpdate};
pub use ui::shell::ShellView;
pub use ui::tasks::{TaskIntent, TasksView};
pub mod i18n;
pub use i18n::{I18n, Lang, StringId};

pub use keymap::{
    CommandId, KeyContext, Keymap, build_command_palette, load_keymap, parse_user_keymap,
    register_keymap_bindings, reload_keymap,
};
