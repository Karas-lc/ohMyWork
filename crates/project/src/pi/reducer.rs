use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use crate::pi::wire::{AssistantMessageEvent, ContentBlock, Disposition, Event, Response};

/// Pure session state per plan contract 6.
/// Transitions only via reduce(); illegal/late events have defined (usually no-op + diag) handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    Stopped,
    Starting,
    Idle,
    Running,
    Closing,
    Failed(String),
}

impl Default for SessionState {
    fn default() -> Self {
        SessionState::Stopped
    }
}

/// Accumulator for one assistant message's blocks, keyed by contentIndex.
/// message_end replaces the accum with authoritative final.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MessageAccum {
    pub blocks: BTreeMap<u32, ContentBlock>,
    pub is_final: bool,
}

impl MessageAccum {
    pub fn apply_event(&mut self, event: &Event) {
        match event.typ.as_str() {
            "message_start" => *self = Self::default(),
            "message_update" => {
                if let Some((_, evt)) = event.as_message_update() {
                    self.apply_delta(evt);
                }
            }
            "message_end" => {
                if let Some(arr) = event.fields.get("message").and_then(|msg| msg.get("content")).and_then(Value::as_array) {
                    let full = arr.iter().filter_map(|c| match c.get("type").and_then(Value::as_str) {
                        Some("text") => Some(ContentBlock::Text(c.get("text").and_then(Value::as_str).unwrap_or("").to_string())),
                        Some("thinking") => Some(ContentBlock::Thinking(c.get("thinking").and_then(Value::as_str).unwrap_or("").to_string())),
                        _ => None,
                    }).collect();
                    self.finalize(full);
                }
            }
            _ => {}
        }
    }

    fn apply_delta(&mut self, evt: AssistantMessageEvent) {
        match evt {
            AssistantMessageEvent::TextDelta { content_index, delta } => {
                let e = self.blocks.entry(content_index).or_insert(ContentBlock::Text(String::new()));
                if let ContentBlock::Text(s) = e {
                    s.push_str(&delta);
                }
            }
            AssistantMessageEvent::ThinkingDelta { content_index, delta } => {
                let e = self.blocks.entry(content_index).or_insert(ContentBlock::Thinking(String::new()));
                if let ContentBlock::Thinking(s) = e {
                    s.push_str(&delta);
                }
            }
            AssistantMessageEvent::ToolcallDelta { content_index, delta } => {
                // simplistic: treat as text for args for now; real would append json
                let e = self.blocks.entry(content_index).or_insert(ContentBlock::Tool {
                    id: String::new(),
                    name: String::new(),
                    arguments: Value::String(delta.clone()),
                });
                if let ContentBlock::Tool { arguments, .. } = e {
                    if let Value::String(s) = arguments {
                        s.push_str(&delta);
                    }
                }
            }
            AssistantMessageEvent::TextEnd { content_index, content } => {
                self.blocks.insert(content_index, ContentBlock::Text(content));
            }
            AssistantMessageEvent::ThinkingEnd { content_index, content } => {
                self.blocks.insert(content_index, ContentBlock::Thinking(content));
            }
            AssistantMessageEvent::ToolcallEnd { content_index, tool_call } => {
                // parse minimal
                let id = tool_call.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = tool_call.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                self.blocks.insert(content_index, ContentBlock::Tool { id, name, arguments: tool_call.get("arguments").cloned().unwrap_or(Value::Null) });
            }
            _ => {}
        }
    }

    fn finalize(&mut self, full_blocks: Vec<ContentBlock>) {
        self.blocks.clear();
        for (i, b) in full_blocks.into_iter().enumerate() {
            self.blocks.insert(i as u32, b);
        }
        self.is_final = true;
    }
}

/// Tracks outstanding command ids (response correlation). Events independent.
#[derive(Debug, Clone, Default)]
pub struct PendingRequests {
    pub open: HashMap<String, String>, // id -> command name snapshot
}

impl PendingRequests {
    pub fn insert(&mut self, id: String, cmd: String) {
        self.open.insert(id, cmd);
    }
    pub fn remove(&mut self, id: &str) -> Option<String> {
        self.open.remove(id)
    }
    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }
}

/// Pure reducer. Takes current state + event/response, returns new state + side effects (accum, pending updates).
/// Never panics on bad input; late/illegal produce deterministic result (usually stay, log via diag).
pub fn reduce(
    mut state: SessionState,
    incoming: &Event, // or Response? use separate calls or union
    response: Option<&Response>,
    accum: &mut MessageAccum,
    pending: &mut PendingRequests,
) -> SessionState {
    // Handle response first (command correlation)
    if let Some(r) = response {
        if let Some(id) = &r.id {
            pending.remove(id);
        }
        if let Some(disp) = r.disposition() {
            match disp {
                Disposition::Handled => {
                    // per contract 5: do not wait for non-existent run
                    if state == SessionState::Running {
                        // stay running or? handled means extension took it, may stay idle-ish but per spec don't assume settled
                    }
                }
                _ => {}
            }
        }
        // success/fail on response doesn't auto change run state; events do.
    }

    // Event driven state + accum
    accum.apply_event(incoming);
    match incoming.typ.as_str() {
        "agent_start" => {
            if matches!(state, SessionState::Idle | SessionState::Stopped) {
                state = SessionState::Running;
            }
        }
        "agent_end" => {
            // per contract: agent_end does NOT mean idle
            // may still have retries/queue -> wait for settled
        }
        "agent_settled" => {
            let aborted = incoming.as_agent_settled().unwrap_or(false);
            if matches!(state, SessionState::Running | SessionState::Closing) {
                state = if aborted { SessionState::Stopped } else { SessionState::Idle };
            }
        }
        "message_start" | "message_update" | "message_end" => {}
        "turn_end" | "tool_execution_end" => {
            // no state change, just observation
        }
        _ => {
            // unknown ordinary event: retain type for diag, ignore for state (per contract 2)
            // caller can inspect incoming.typ
        }
    }

    // Simple illegal transition guard (contract 6): e.g. settled while stopped -> stay
    if matches!(state, SessionState::Stopped) && incoming.is_type("agent_settled") {
        // late settled: stay stopped
    }

    state
}

/// Convenience: reduce using only Event (for pure event stream tests).
pub fn reduce_event(state: SessionState, ev: &Event, accum: &mut MessageAccum, pending: &mut PendingRequests) -> SessionState {
    reduce(state, ev, None, accum, pending)
}
