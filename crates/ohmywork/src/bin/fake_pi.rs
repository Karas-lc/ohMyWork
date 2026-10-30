//! JSONL test process for the Pi RPC boundary. Not packaged for users.
use serde_json::{Value, json};
use std::{
    env,
    fs::{self, OpenOptions},
    io::{self, BufRead, Write},
    path::Path,
    thread,
    time::Duration,
};

#[derive(Default)]
struct Scenario {
    handled: bool,
    stream: bool,
    ui: bool,
    out_of_order: bool,
    settled: bool,
    hang: bool,
}

impl Scenario {
    fn from_args_and_file(cwd: &Path) -> Self {
        let mut scenario = Self::default();
        for flag in env::args().skip(1).chain(
            fs::read_to_string(cwd.join("fake-pi-scenario.txt"))
                .unwrap_or_default()
                .lines()
                .map(str::to_owned),
        ) {
            match flag.trim() {
                "--handled" => scenario.handled = true,
                "--stream" => scenario.stream = true,
                "--ui" => scenario.ui = true,
                "--out-of-order" => scenario.out_of_order = true,
                "--settled" => scenario.settled = true,
                "--hang" => scenario.hang = true,
                _ => {}
            }
        }
        scenario
    }
}

fn main() {
    let cwd = match env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            eprintln!("fake-pi: cannot read cwd: {error}");
            std::process::exit(2);
        }
    };
    let scenario = Scenario::from_args_and_file(&cwd);
    let record_path = cwd.join("fake-pi-record.jsonl");
    if let Err(error) = record(&record_path, &json!({"kind": "launch", "cwd": cwd})) {
        eprintln!("fake-pi: cannot write launch record: {error}");
        std::process::exit(2);
    }

    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("fake-pi: stdin read failed: {error}");
                break;
            }
        };
        let message: Value = match serde_json::from_str::<Value>(&line) {
            Ok(message) if message.is_object() => message,
            Ok(_) => {
                eprintln!("fake-pi: ignored non-object JSON record");
                continue;
            }
            Err(error) => {
                eprintln!("fake-pi: ignored malformed JSON: {error}");
                continue;
            }
        };
        if let Err(error) = record(&record_path, &json!({"kind": "received", "message": message})) {
            eprintln!("fake-pi: cannot write received record: {error}");
            continue;
        }
        if message.get("type").and_then(Value::as_str) == Some("extension_ui_response") {
            continue;
        }

        let command = message.get("type").and_then(Value::as_str).unwrap_or("");
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        match command {
            "get_state" => emit(&mut stdout, &json!({
                "id": id, "type": "response", "command": "get_state", "success": true,
                "data": {"sessionFile": cwd.join("fake-session.jsonl"), "model": {"provider": "fake", "id": "fake-model"}}
            })),
            "prompt" => handle_prompt(&mut stdout, &scenario, id),
            "abort" if scenario.hang => loop { thread::sleep(Duration::from_secs(60)); },
            "abort" => {
                emit(&mut stdout, &json!({"id": id, "type": "response", "command": "abort", "success": true}));
                emit(&mut stdout, &json!({"type": "agent_settled", "aborted": true}));
                break;
            }
            _ => emit(&mut stdout, &json!({
                "id": id, "type": "response", "command": command, "success": false,
                "error": format!("unsupported fake command: {command}")
            })),
        }
    }
    if scenario.hang {
        loop { thread::sleep(Duration::from_secs(60)); }
    }
}

fn handle_prompt(stdout: &mut impl Write, scenario: &Scenario, id: Value) {
    let response = json!({
        "id": id, "type": "response", "command": "prompt", "success": true,
        "data": {"disposition": if scenario.handled { "handled" } else { "started" }}
    });
    if scenario.out_of_order && !scenario.handled {
        emit(stdout, &json!({"type": "agent_start"}));
    }
    emit(stdout, &response);
    if scenario.handled {
        if scenario.settled { emit(stdout, &json!({"type": "agent_settled", "aborted": false})); }
        return;
    }
    if !scenario.out_of_order { emit(stdout, &json!({"type": "agent_start"})); }
    emit(stdout, &json!({"type": "message_start", "message": {"role": "assistant", "content": []}}));
    if scenario.stream {
        for index in 0..3 {
            emit(stdout, &json!({
                "type": "message_update", "usage": {},
                "assistantMessageEvent": {"type": "text_delta", "contentIndex": 0, "delta": format!("chunk{index} ")}
            }));
        }
        emit(stdout, &json!({
            "type": "message_end", "message": {"role": "assistant", "content": [{"type": "text", "text": "chunk0 chunk1 chunk2 "}]}
        }));
    }
    if scenario.ui {
        emit(stdout, &json!({
            "type": "extension_ui_request", "id": "fake-ui-1", "method": "select",
            "title": "Pick an option", "options": ["A", "B"]
        }));
    }
    if scenario.settled { emit(stdout, &json!({"type": "agent_settled", "aborted": false})); }
}

fn emit(stdout: &mut impl Write, value: &Value) {
    if serde_json::to_writer(&mut *stdout, value).is_err() || stdout.write_all(b"\n").is_err() {
        eprintln!("fake-pi: stdout write failed");
        std::process::exit(2);
    }
    let _ = stdout.flush();
}

fn record(path: &Path, value: &Value) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")
}
