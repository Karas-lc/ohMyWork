//! Lifecycle, reducer, handled/settled, message accum contract tests (3-6).
//! Pure functions, no process, no real Pi.

use omw_pi::reducer::{reduce, reduce_event, MessageAccum, PendingRequests, SessionState};
use omw_pi::wire::{Event, Response};
use serde_json::{json, Value};

fn make_event(typ: &str, fields: Value) -> Event {
    Event {
        typ: typ.to_string(),
        fields,
    }
}

#[test]
fn handled_disposition_does_not_wait_for_nonexistent_run() {
    let mut accum = MessageAccum::default();
    let mut pend = PendingRequests::default();
    let resp = Response {
        id: Some("p1".into()),
        typ: "response".into(),
        command: "prompt".into(),
        success: true,
        data: Some(json!({"disposition":"handled"})),
        error: None,
    };
    let ev_start = make_event("agent_start", json!({}));
    let state = reduce(SessionState::Starting, &ev_start, Some(&resp), &mut accum, &mut pend);
    assert_ne!(state, SessionState::Running);
}

#[test]
fn agent_end_does_not_imply_idle_settled_does() {
    let mut accum = MessageAccum::default();
    let mut pend = PendingRequests::default();
    let start = make_event("agent_start", json!({}));
    let end = make_event("agent_end", json!({"willRetry": false}));
    let settled = make_event("agent_settled", json!({"aborted": false}));

    let mut s = SessionState::Idle;
    s = reduce_event(s, &start, &mut accum, &mut pend);
    assert_eq!(s, SessionState::Running);

    s = reduce_event(s, &end, &mut accum, &mut pend);
    assert_eq!(s, SessionState::Running);

    s = reduce_event(s, &settled, &mut accum, &mut pend);
    assert_eq!(s, SessionState::Idle);
}

#[test]
fn message_end_replaces_incremental() {
    let mut accum = MessageAccum::default();
    let mut pend = PendingRequests::default();
    let delta = make_event("message_update", json!({
        "usage": {},
        "assistantMessageEvent": {"type":"textDelta","contentIndex":0,"delta":"hel"}
    }));
    // direct enum serde check
    let inner_evt = json!({"type":"textDelta","contentIndex":0,"delta":"hel"});
    let parsed_evt: Result<omw_pi::AssistantMessageEvent, _> = serde_json::from_value(inner_evt.clone());
    assert!(parsed_evt.is_ok(), "direct serde of AssistantMessageEvent failed: {:?}", parsed_evt.err());
    let parsed = delta.as_message_update();
    assert!(parsed.is_some(), "parse failed, fields were: {:?}", delta.fields);
    let end = make_event("message_end", json!({
        "message": {"role":"assistant","content":[{"type":"text","text":"hello full"}]}
    }));

    let mut s = SessionState::Running;
    s = reduce_event(s, &delta, &mut accum, &mut pend);
    let blk = accum.blocks.get(&0);
    assert!(blk.is_some(), "delta should create block");

    s = reduce_event(s, &end, &mut accum, &mut pend);
    let blk = accum.blocks.get(&0).unwrap();
    match blk {
        omw_pi::ContentBlock::Text(t) => assert_eq!(t, "hello full"),
        _ => panic!("expected text"),
    }
    assert!(accum.is_final);
}

#[test]
fn unknown_event_ignored_state_unchanged() {
    let mut accum = MessageAccum::default();
    let mut pend = PendingRequests::default();
    let weird = make_event("future_thing", json!({"foo":42}));
    let s0 = SessionState::Idle;
    let s1 = reduce_event(s0.clone(), &weird, &mut accum, &mut pend);
    assert_eq!(s1, s0);
    assert_eq!(weird.typ, "future_thing");
}

#[test]
fn out_of_order_response_and_pending_cleanup() {
    let mut accum = MessageAccum::default();
    let mut pend = PendingRequests::default();
    pend.insert("cmd42".into(), "prompt".into());

    let resp = Response {
        id: Some("cmd42".into()),
        typ: "response".into(),
        command: "prompt".into(),
        success: true,
        data: Some(json!({})),
        error: None,
    };
    let ev_settled = make_event("agent_settled", json!({}));

    let _s = reduce(
        SessionState::Running,
        &ev_settled,
        Some(&resp),
        &mut accum,
        &mut pend,
    );
    assert!(pend.is_empty());
}

#[test]
fn disconnect_clears_all_pending() {
    let _accum = MessageAccum::default();
    let mut pend = PendingRequests::default();
    pend.insert("a".into(), "x".into());
    pend.insert("b".into(), "y".into());
    pend.open.clear();
    assert!(pend.is_empty());
}
