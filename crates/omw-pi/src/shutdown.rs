//! Orderly Pi RPC shutdown per plan 16 and docs/pi-integration.md.
//! Sequence on user explicit close:
//!   1. If running, send "abort" command (with id).
//!   2. Wait up to 2s for response or agent_settled/abort event.
//!   3. Close child's stdin.
//!   4. Wait up to 3s for process exit.
//!   5. On timeout: report "未退出", only explicit user action may kill the subtree.
//! Never kills all "pi" processes. Only the specific child we spawned.
//! No raw stderr persisted by default.

use std::time::Duration;

pub const ABORT_WAIT: Duration = Duration::from_secs(2);
pub const STDIN_CLOSE_WAIT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShutdownOutcome {
    /// Clean exit within timeouts after abort + stdin close.
    Clean,
    /// Timed out after abort and/or stdin close. User may choose force kill of this specific child.
    TimedOut,
    /// Already stopped.
    AlreadyStopped,
}

/// Marker for the shutdown request. Actual impl in client using tokio::time::timeout.
#[derive(Debug, Clone, Default)]
pub struct ShutdownRequest {
    pub force_after_timeout: bool,
}

impl ShutdownRequest {
    pub fn new() -> Self {
        Self { force_after_timeout: false }
    }

    pub fn with_force(mut self) -> Self {
        self.force_after_timeout = true;
        self
    }
}
