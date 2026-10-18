use crate::{
    ByteFramer,
    launch::{LaunchError, PiLaunch},
    reducer::SessionState,
    shutdown::{ABORT_WAIT, STDIN_CLOSE_WAIT, ShutdownOutcome, ShutdownRequest},
    wire::{Command, Event, Incoming, Outgoing, Response, UiRequest, UiResponse},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    process::{Child, Command as TokioCommand},
    sync::mpsc,
    task::JoinHandle,
};
pub type Generation = u64;
#[derive(Debug, Clone)]
pub enum PiUpdate {
    State {
        generation: Generation,
        state: SessionState,
    },
    Event {
        generation: Generation,
        event: Event,
    },
    Response {
        generation: Generation,
        response: Response,
    },
    UiRequest {
        generation: Generation,
        request: UiRequest,
    },
    Error {
        generation: Generation,
        error: String,
    },
    Stderr {
        generation: Generation,
        line: String,
    },
    Disconnected {
        generation: Generation,
    },
}
#[derive(Clone)]
pub struct PiCommandSender {
    tx: mpsc::Sender<Outgoing>,
    generation: Arc<AtomicU64>,
}
impl PiCommandSender {
    pub fn current_gen(&self) -> Generation {
        self.generation.load(Ordering::SeqCst)
    }
    pub async fn send(&self, cmd: Command) -> Result<(), String> {
        self.tx
            .send(Outgoing::Command(cmd))
            .await
            .map_err(|e| e.to_string())
    }
    pub fn try_send(&self, cmd: Command) -> Result<(), String> {
        self.tx
            .try_send(Outgoing::Command(cmd))
            .map_err(|e| e.to_string())
    }
    pub async fn send_ui_response(&self, response: UiResponse) -> Result<(), String> {
        self.tx
            .send(Outgoing::UiResponse(response))
            .await
            .map_err(|e| e.to_string())
    }
    pub fn try_send_ui_response(&self, response: UiResponse) -> Result<(), String> {
        self.tx
            .try_send(Outgoing::UiResponse(response))
            .map_err(|e| e.to_string())
    }
}
pub struct PiClient {
    generation: Arc<AtomicU64>,
    cmd_tx: mpsc::Sender<Outgoing>,
    update_rx: mpsc::Receiver<PiUpdate>,
    child: Option<Child>,
    writer: Option<JoinHandle<()>>,
    readers: Vec<JoinHandle<()>>,
    runtime: tokio::runtime::Handle,
}
impl PiClient {
    pub fn spawn(
        launch: PiLaunch,
        rt: &tokio::runtime::Handle,
    ) -> Result<(Self, PiCommandSender), LaunchError> {
        crate::launch::validate_cwd(&launch.cwd)?;
        let _enter = rt.enter();
        let mut child = TokioCommand::new(&launch.executable)
            .args(&launch.args)
            .current_dir(&launch.cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| LaunchError::CwdNotAccessible {
                path: launch.cwd.clone(),
                reason: e.to_string(),
            })?;
        let missing = || LaunchError::CwdNotAccessible {
            path: launch.cwd.clone(),
            reason: "Pi process pipe missing".into(),
        };
        let mut stdin = child.stdin.take().ok_or_else(missing)?;
        let mut stdout = child.stdout.take().ok_or_else(missing)?;
        let mut stderr = child.stderr.take().ok_or_else(missing)?;
        let (tx, mut commands) = mpsc::channel::<Outgoing>(32);
        let (updates, update_rx) = mpsc::channel(128);
        static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
        let gen_id = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        let generation = Arc::new(AtomicU64::new(gen_id));
        let out = updates.clone();
        let reader = rt.spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buffer = [0u8; 8192];
            let mut framer = ByteFramer::new();
            let mut state = SessionState::Starting;
            let mut accum = crate::reducer::MessageAccum::default();
            let mut pending = crate::reducer::PendingRequests::default();
            let _ = out
                .send(PiUpdate::State {
                    generation: gen_id,
                    state: state.clone(),
                })
                .await;
            loop {
                let n = match stdout.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let frames = match framer.feed(&buffer[..n]) {
                    Ok(f) => f,
                    Err(_) => {
                        let _ = out
                            .send(PiUpdate::Error {
                                generation: gen_id,
                                error: "Pi JSONL frame exceeds limits".into(),
                            })
                            .await;
                        break;
                    }
                };
                for frame in frames {
                    match frame.parse_incoming() {
                        Ok(Incoming::Event(event)) => {
                            state = crate::reducer::reduce_event(
                                state,
                                &event,
                                &mut accum,
                                &mut pending,
                            );
                            let _ = out
                                .send(PiUpdate::Event {
                                    generation: gen_id,
                                    event,
                                })
                                .await;
                            let _ = out
                                .send(PiUpdate::State {
                                    generation: gen_id,
                                    state: state.clone(),
                                })
                                .await;
                        }
                        Ok(Incoming::Response(response)) => {
                            let _ = out
                                .send(PiUpdate::Response {
                                    generation: gen_id,
                                    response,
                                })
                                .await;
                        }
                        Ok(Incoming::UiRequest(request)) => {
                            let _ = out
                                .send(PiUpdate::UiRequest {
                                    generation: gen_id,
                                    request,
                                })
                                .await;
                        }
                        Ok(Incoming::UiResponse(_)) => {}
                        Err(_) => {
                            let _ = out
                                .send(PiUpdate::Error {
                                    generation: gen_id,
                                    error: "Invalid Pi JSONL record".into(),
                                })
                                .await;
                        }
                    }
                }
            }
            let _ = out
                .send(PiUpdate::Disconnected { generation: gen_id })
                .await;
        });
        let err_reader = rt.spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = [0u8; 4096];
            while let Ok(n) = stderr.read(&mut buf).await {
                if n == 0 {
                    break;
                }
            }
        });
        let writer_out = updates;
        let writer = rt.spawn(async move {
            use tokio::io::AsyncWriteExt;
            while let Some(command) = commands.recv().await {
                let value = match command {
                    Outgoing::Command(c) => serde_json::to_vec(&c),
                    Outgoing::UiResponse(r) => serde_json::to_vec(&r),
                };
                match value {
                    Ok(mut bytes) => {
                        bytes.push(b'\n');
                        if stdin.write_all(&bytes).await.is_err() || stdin.flush().await.is_err() {
                            let _ = writer_out
                                .send(PiUpdate::Error {
                                    generation: gen_id,
                                    error: "Pi stdin closed".into(),
                                })
                                .await;
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = writer_out
                            .send(PiUpdate::Error {
                                generation: gen_id,
                                error: "Could not encode Pi command".into(),
                            })
                            .await;
                    }
                }
            }
        });
        let sender = PiCommandSender {
            tx: tx.clone(),
            generation: generation.clone(),
        };
        Ok((
            Self {
                generation,
                cmd_tx: tx,
                update_rx,
                child: Some(child),
                writer: Some(writer),
                readers: vec![reader, err_reader],
                runtime: rt.clone(),
            },
            sender,
        ))
    }
    pub fn current_generation(&self) -> Generation {
        self.generation.load(Ordering::SeqCst)
    }
    pub fn try_recv_update(&mut self) -> Option<PiUpdate> {
        self.update_rx.try_recv().ok()
    }
    pub async fn recv_update(&mut self) -> Option<PiUpdate> {
        self.update_rx.recv().await
    }
    pub async fn shutdown(&mut self, request: ShutdownRequest) -> ShutdownOutcome {
        if self.child.is_none() {
            return ShutdownOutcome::AlreadyStopped;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let _ = self
            .cmd_tx
            .send(Outgoing::Command(Command::abort(Some(id.clone()))))
            .await;
        let deadline = tokio::time::Instant::now() + ABORT_WAIT;
        while let Ok(Some(update)) = tokio::time::timeout_at(deadline, self.update_rx.recv()).await
        {
            if matches!(&update,PiUpdate::Response{response:Response{id:Some(response_id),..},..}if response_id==&id)
            {
                break;
            }
            if matches!(update, PiUpdate::Disconnected { .. }) {
                break;
            }
        }
        if let Some(writer) = self.writer.take() {
            writer.abort();
            let _ = writer.await;
        }
        if let Some(child) = &mut self.child {
            match tokio::time::timeout(STDIN_CLOSE_WAIT, child.wait()).await {
                Ok(Ok(_)) => {
                    self.child = None;
                    for task in self.readers.drain(..) {
                        task.abort();
                    }
                    return ShutdownOutcome::Clean;
                }
                _ if request.force_after_timeout => {
                    let _ = child.kill().await;
                    self.child = None;
                    for task in self.readers.drain(..) {
                        task.abort();
                    }
                    return ShutdownOutcome::TimedOut;
                }
                _ => return ShutdownOutcome::TimedOut,
            }
        }
        ShutdownOutcome::Clean
    }
    pub async fn verify_connection(&self, sender: &PiCommandSender) -> Result<(), String> {
        sender
            .send(Command {
                id: Some(format!("verify-{}", self.current_generation())),
                body: crate::wire::CommandBody::GetState,
            })
            .await
    }
}
impl Drop for PiClient {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let tx = self.cmd_tx.clone();
            let writer = self.writer.take();
            let readers = std::mem::take(&mut self.readers);
            self.runtime.spawn(async move {
                let _ = tx.send(Outgoing::Command(Command::abort(None))).await;
                tokio::time::sleep(Duration::from_millis(100)).await;
                if let Some(writer) = writer {
                    writer.abort();
                    let _ = writer.await;
                }
                let _ = tokio::time::timeout(STDIN_CLOSE_WAIT, child.wait()).await;
                for reader in readers {
                    reader.abort();
                }
            });
        }
    }
}
