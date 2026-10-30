//! Pi RPC integration: codec, wire protocol, reducer, launch and client adapters.

pub mod codec;
pub mod wire;
pub mod reducer;
pub mod launch;
pub mod session;
pub mod client;
pub mod shutdown;
pub mod context;

pub use codec::{ByteFramer, CompleteFrame, FramerError};
pub use wire::{
    Outgoing, Command, CommandBody, Response, Event, UiRequest, UiResponse, Incoming, Disposition,
    AssistantMessageEvent, ContentBlock,
};
pub use reducer::{SessionState, MessageAccum, reduce, PendingRequests};
pub use launch::{PiLaunch, LaunchError, build_launch, discover_pi};
pub use session::NewOrResume;
pub use client::{PiClient, PiCommandSender, PiUpdate, Generation};
pub use shutdown::{ShutdownOutcome, ShutdownRequest, ABORT_WAIT, STDIN_CLOSE_WAIT};
pub use context::{ContextItem, wrap_context_for_prompt, display_path};
