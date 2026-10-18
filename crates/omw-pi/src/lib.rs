//! Pi RPC integration (plan 15 protocol + 16 launch).
//! Version source: official docs at https://pi.dev/docs/latest/ (rpc, json, rpc-commands...)
//! corresponding to @earendil-works/pi-coding-agent 0.83.0.
//! See AGENTS.md, docs/pi-integration.md, ADR-0010, plans 15/16/17.
//!
//! Includes: JSONL codec/wire/reducer + launch (discover/spawn), client (Tokio I/O + gen), session mode, orderly shutdown.
//! No UI, no Pi config mutation, no model prompts injected.

// Protocol core (from plan 15) remains unchanged: codec + wire + reducer for framing, typing, state.
// Launch/client added in 16; Extension UI and full context in 17.
pub mod codec;
pub mod wire;
pub mod reducer;
pub mod launch;
pub mod session;
pub mod client;
pub mod shutdown;
pub mod context;




pub use codec::{ByteFramer, FramerError, CompleteFrame};
pub use wire::{
    Outgoing,
    Command, CommandBody, Response, Event, UiRequest, UiResponse, Incoming, Disposition,
    AssistantMessageEvent, ContentBlock,
};
pub use reducer::{SessionState, MessageAccum, reduce, PendingRequests};
pub use launch::{PiLaunch, LaunchError, build_launch, discover_pi};
pub use session::NewOrResume;
pub use client::{PiClient, PiCommandSender, PiUpdate, Generation};
pub use shutdown::{ShutdownOutcome, ShutdownRequest, ABORT_WAIT, STDIN_CLOSE_WAIT};
pub use context::{ContextItem, wrap_context_for_prompt, display_path};
