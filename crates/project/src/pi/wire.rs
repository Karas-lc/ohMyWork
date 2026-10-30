use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Official command sent to Pi on stdin.
/// id optional but recommended for correlation when >1 outstanding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Command {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(flatten)]
    pub body: CommandBody,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommandBody {
    Prompt {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        images: Option<Vec<ImageContent>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        streaming_behavior: Option<String>, // "steer" | "followUp"
    },
    Abort,
    // Minimal others for lifecycle/ui tests; extend boringly as needed
    GetState,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageContent {
    #[serde(rename = "type")]
    pub typ: String, // "image"
    pub data: String, // base64
    pub mime_type: String,
}

/// Response from command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Response {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub typ: String, // always "response" on wire
    pub command: String,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Started,
    Queued,
    Handled,
}

/// Session event on stdout (no id usually).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    #[serde(rename = "type")]
    pub typ: String,
    #[serde(flatten)]
    pub fields: Value,
    // For typed access in reducer, convenience parsed below.
}

/// Parsed known assistant message events for accumulation (from message_update).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AssistantMessageEvent {
    TextDelta {
        #[serde(rename = "contentIndex")]
        content_index: u32,
        delta: String,
    },
    TextEnd {
        #[serde(rename = "contentIndex")]
        content_index: u32,
        content: String,
    },
    ThinkingDelta {
        #[serde(rename = "contentIndex")]
        content_index: u32,
        delta: String,
    },
    ThinkingEnd {
        #[serde(rename = "contentIndex")]
        content_index: u32,
        content: String,
    },
    ToolcallStart {
        #[serde(rename = "contentIndex")]
        content_index: u32,
        id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
    },
    ToolcallDelta {
        #[serde(rename = "contentIndex")]
        content_index: u32,
        delta: String,
    },
    ToolcallEnd {
        #[serde(rename = "contentIndex")]
        content_index: u32,
        tool_call: Value, // full ToolCall
    },
    // others ignored or extended
    #[serde(other)]
    Other,
}

/// Content block for accum.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentBlock {
    Text(String),
    Thinking(String),
    Tool { id: String, name: String, arguments: Value },
}

/// Incoming record union for framer + parse.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Incoming {
    UiRequest(UiRequest),
    UiResponse(UiResponse),
    Response(Response),
    Event(Event),
}
/// Items the host writes on Pi's stdin: either a Command or an extension UI response.
#[derive(Debug, Clone)]
pub enum Outgoing {
    Command(Command),
    UiResponse(UiResponse),
}


/// Extension UI request (from Pi to host).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiRequest {
    #[serde(rename = "type")]
    pub typ: String, // "extension_ui_request"
    pub id: String,
    pub method: String, // "select" | "confirm" | "input" | "editor" | ...
    #[serde(flatten)]
    pub payload: Value,
}

/// Host reply for dialog UI requests (stdin).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiResponse {
    #[serde(rename = "type")]
    pub typ: String, // "extension_ui_response"
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancelled: Option<bool>,
}

impl Command {
    pub fn prompt(id: Option<String>, message: impl Into<String>) -> Self {
        Command {
            id,
            body: CommandBody::Prompt {
                message: message.into(),
                images: None,
                streaming_behavior: None,
            },
        }
    }

    pub fn abort(id: Option<String>) -> Self {
        Command { id, body: CommandBody::Abort }
    }
}

impl Response {
    pub fn disposition(&self) -> Option<Disposition> {
        if !self.success {
            return None;
        }
        self.data.as_ref().and_then(|d| {
            d.get("disposition")
                .and_then(|v| serde_json::from_value::<Disposition>(v.clone()).ok())
        })
    }
}

impl Event {
    pub fn as_agent_settled(&self) -> Option<bool> {
        if self.typ == "agent_settled" {
            self.fields.get("aborted").and_then(|v| v.as_bool())
        } else {
            None
        }
    }

    pub fn as_message_update(&self) -> Option<(&str, AssistantMessageEvent)> {
        if self.typ == "message_update" {
            if let Some(evt) = self.fields.get("assistantMessageEvent") {
                if let Ok(parsed) = serde_json::from_value::<AssistantMessageEvent>(evt.clone()) {
                    return Some(("message_update", parsed));
                }
            }
        }
        None
    }

    // Add accessors for message_start/end, agent_end etc as needed for reducer.
    pub fn is_type(&self, t: &str) -> bool {
        self.typ == t
    }
}
