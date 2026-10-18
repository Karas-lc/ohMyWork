//! Controllable fake Pi for pure protocol tests (plan 15).
//! Same stdin/stdout JSONL boundary as real `pi --mode rpc`.
//! Invoked as test target only; not packaged.
//! Usage: fake_pi [flags...]
//! Flags (any contain):
//!   --split-frames   write partial records (test framer split reads)
//!   --out-of-order   emit events before command response
//!   --stream         emit incremental message_update deltas then end
//!   --handled        prompt response disposition=handled, no run events
//!   --settled        emit agent_settled (after run or immediate)
//!   --ui             emit one extension_ui_request (select) then expect response
//!   --stderr-flood   spam stderr (backpressure test)
//!   --crash          exit 1 after some output
//!   --hang           never exit (after setup)
//! All output uses strict LF JSONL. Reads stdin but ignores for minimal.

use std::env;
use std::io::{self, BufRead, Write};
use std::thread;
use std::time::Duration;

fn main() {
    let args: Vec<String> = env::args().collect();
    let has = |f: &str| args.iter().any(|a| a.contains(f));

    let split = has("--split");
    let ooo = has("--out-of-order");
    let stream = has("--stream");
    let handled = has("--handled");
    let settled = has("--settled");
    let ui = has("--ui");
    let flood = has("--stderr");
    let crash = has("--crash");
    let hang = has("--hang");

    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();

    // Read one command (simplified; real would loop)
    let _ = stdin.read_line(&mut String::new()); // consume prompt etc, ignore

    if flood {
        for i in 0..100 {
            let _ = writeln!(stderr, "FLOOD {} stderr diagnostics to test backpressure", i);
        }
    }

    // Emit a response for the (assumed) prompt
    let resp = if handled {
        r#"{"id":"p1","type":"response","command":"prompt","success":true,"data":{"disposition":"handled"}}"#
    } else {
        r#"{"id":"p1","type":"response","command":"prompt","success":true,"data":{"disposition":"started"}}"#
    };

    if ooo {
        // out of order: event before response
        emit(&mut stdout, split, r#"{"type":"agent_start"}"#);
        emit(&mut stdout, split, resp);
    } else {
        emit(&mut stdout, split, resp);
        if !handled {
            emit(&mut stdout, split, r#"{"type":"agent_start"}"#);
        }
    }

    if stream && !handled {
        // incremental
        for i in 0..3 {
            let delta = format!(r#"{{"type":"message_update","usage":{{}},"assistantMessageEvent":{{"type":"text_delta","contentIndex":0,"delta":"chunk{} "}}}}"#, i);
            emit(&mut stdout, split, &delta);
            if split {
                thread::sleep(Duration::from_millis(5));
            }
        }
        emit(&mut stdout, split, r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"chunk0 chunk1 chunk2 "}]}}"#);
        emit(&mut stdout, split, r#"{"type":"agent_end","willRetry":false}"#);
    }

    if ui {
        // emit ui request; caller test must reply on stdin for full, here just emit
        emit(&mut stdout, split, r#"{"type":"extension_ui_request","id":"ui-1","method":"select","title":"Pick","options":["A","B"]}"#);
        // In real test, the harness would read this, send response back via its stdin to fake.
    }

    if settled {
        emit(&mut stdout, split, r#"{"type":"agent_settled","aborted":false}"#);
    }

    if crash {
        let _ = stdout.flush();
        std::process::exit(1);
    }

    if hang {
        // never exit
        loop {
            thread::sleep(Duration::from_secs(60));
        }
    }

    let _ = stdout.flush();
}

fn emit(out: &mut impl Write, split: bool, json: &str) {
    if split {
        // simulate partial write for framer test (receiver sees split across reads)
        let bytes = format!("{}\n", json).into_bytes();
        let mid = bytes.len() / 2;
        let _ = out.write_all(&bytes[..mid]);
        let _ = out.flush();
        thread::sleep(Duration::from_millis(1));
        let _ = out.write_all(&bytes[mid..]);
    } else {
        let _ = writeln!(out, "{}", json);
    }
    let _ = out.flush();
}
