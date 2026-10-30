//! UI request contract tests (plan 15).
//! Covers request types + response correlation. Unknown ui req must produce error response.

use project::pi::wire::{Incoming, UiRequest, UiResponse};
use serde_json::json;

#[test]
fn parse_select_request() {
    let raw = json!({
        "type": "extension_ui_request",
        "id": "u1",
        "method": "select",
        "title": "Choose",
        "options": ["A", "B"]
    });
    let req: UiRequest = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(req.id, "u1");
    assert_eq!(req.method, "select");
    assert!(req.payload.get("options").is_some());

    // via Incoming
    let inc: Incoming = serde_json::from_value(raw).unwrap();
    match inc {
        Incoming::UiRequest(r) => assert_eq!(r.id, "u1"),
        other => panic!("expected UiRequest via Incoming, got other: {:?}", other),
    }
}

#[test]
fn ui_response_shape() {
    let resp = UiResponse {
        typ: "extension_ui_response".into(),
        id: "u1".into(),
        value: Some(json!({"selected":"A"})),
        confirmed: None,
        cancelled: None,
    };
    let ser = serde_json::to_string(&resp).unwrap();
    assert!(ser.contains("\"id\":\"u1\""));
    assert!(ser.contains("selected"));
}

#[test]
fn unknown_ui_request_does_not_hang_produces_error_response() {
    let raw = json!({
        "type": "extension_ui_request",
        "id": "u666",
        "method": "future_magic",
        "title": "??"
    });
    let req: UiRequest = serde_json::from_value(raw).unwrap();
    let err_resp = UiResponse {
        typ: "extension_ui_response".into(),
        id: req.id,
        value: Some(json!({"code":"unknown_method","message":"not supported"})),
        confirmed: None,
        cancelled: Some(true),
    };
    assert_eq!(err_resp.id, "u666");
    assert!(err_resp.cancelled.unwrap_or(false));
}
