use omw_preview_worker::{
    limits::check_bitmap_size,
    protocol::{
        FrameHeader, PROTOCOL_VERSION, PreviewRequest, PreviewResponse,
        decode_header_len_and_header, encode_frame,
    },
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    process::Command,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
pub type Generation = u64;
#[derive(Clone, Debug)]
pub enum PreviewUpdate {
    Opened {
        generation: Generation,
        document_id: u64,
        page_count: u32,
        title: Option<String>,
    },
    PageInfo {
        generation: Generation,
        page: u32,
        width: u32,
        height: u32,
    },
    Bitmap {
        generation: Generation,
        document_id: u64,
        page: u32,
        generation_tag: u64,
        width: u32,
        height: u32,
        data: Vec<u8>,
    },
    Error {
        generation: Generation,
        code: String,
        message: String,
    },
    Closed {
        generation: Generation,
        document_id: u64,
    },
    Disconnected {
        generation: Generation,
    },
}
enum PreviewCmd {
    Request(
        PreviewRequest,
        oneshot::Sender<Result<PreviewResponse, String>>,
    ),
    Shutdown,
}
#[derive(Clone)]
pub struct PreviewHandle {
    tx: mpsc::Sender<PreviewCmd>,
    generation: Arc<AtomicU64>,
    next: Arc<AtomicU64>,
}
impl PreviewHandle {
    pub fn generation(&self) -> Generation {
        self.generation.load(Ordering::SeqCst)
    }
    async fn request(&self, request: PreviewRequest) -> Result<PreviewResponse, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(PreviewCmd::Request(request, tx))
            .await
            .map_err(|_| "preview worker stopped")?;
        rx.await
            .map_err(|_| "preview worker disconnected".to_string())?
    }
    pub async fn open(&self, path: PathBuf) -> Result<PreviewResponse, String> {
        self.request(PreviewRequest::Open {
            path,
            request_id: self.next.fetch_add(1, Ordering::Relaxed),
        })
        .await
    }
    pub async fn render(
        &self,
        document_id: u64,
        page: u32,
        target_size: [u32; 2],
        gen_tag: u64,
    ) -> Result<PreviewResponse, String> {
        check_bitmap_size(target_size[0], target_size[1]).map_err(|(_, e)| e)?;
        self.request(PreviewRequest::Render {
            document_id,
            page,
            target_size,
            generation: gen_tag,
            request_id: self.next.fetch_add(1, Ordering::Relaxed),
        })
        .await
    }
    pub async fn close(&self, document_id: u64) -> Result<PreviewResponse, String> {
        self.request(PreviewRequest::Close {
            document_id,
            request_id: self.next.fetch_add(1, Ordering::Relaxed),
        })
        .await
    }
}
pub struct PreviewClient {
    handle: PreviewHandle,
    task: JoinHandle<()>,
    pub update_rx: Option<mpsc::Receiver<PreviewUpdate>>,
}
impl Drop for PreviewClient {
    fn drop(&mut self) {
        let _ = self.handle.tx.try_send(PreviewCmd::Shutdown);
        self.task.abort();
    }
}
fn channels() -> (
    PreviewHandle,
    mpsc::Receiver<PreviewCmd>,
    mpsc::Sender<PreviewUpdate>,
    mpsc::Receiver<PreviewUpdate>,
) {
    let (tx, rx) = mpsc::channel(8);
    let (updates, update_rx) = mpsc::channel(2);
    (
        PreviewHandle {
            tx,
            generation: Arc::new(AtomicU64::new(1)),
            next: Arc::new(AtomicU64::new(1)),
        },
        rx,
        updates,
        update_rx,
    )
}
impl PreviewClient {
    pub fn spawn(worker_path: PathBuf) -> Result<(Self, PreviewHandle), String> {
        let mut child = Command::new(worker_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("启动预览进程失败: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("preview stdin unavailable")?;
        let mut stdout = child.stdout.take().ok_or("preview stdout unavailable")?;
        let (handle, mut commands, updates, update_rx) = channels();
        let task = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Some(command) = commands.recv().await {
                let PreviewCmd::Request(request, reply) = command else {
                    break;
                };
                let result = tokio::time::timeout(Duration::from_secs(10), async {
                    let request_id = match &request {
                        PreviewRequest::Open { request_id, .. }
                        | PreviewRequest::Render { request_id, .. }
                        | PreviewRequest::Close { request_id, .. }
                        | PreviewRequest::Shutdown { request_id } => *request_id,
                    };
                    let payload = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
                    let header = FrameHeader {
                        version: PROTOCOL_VERSION,
                        request_id,
                        kind: "request".into(),
                        payload_len: payload.len() as u32,
                    };
                    stdin
                        .write_all(&encode_frame(&header, &payload))
                        .await
                        .map_err(|e| e.to_string())?;
                    stdin.flush().await.map_err(|e| e.to_string())?;
                    let mut prefix = [0u8; 4];
                    stdout
                        .read_exact(&mut prefix)
                        .await
                        .map_err(|e| e.to_string())?;
                    let length = u32::from_le_bytes(prefix) as usize;
                    if length == 0 || length > 65536 {
                        return Err("invalid preview header size".into());
                    }
                    let mut header = vec![0u8; length + 4];
                    header[..4].copy_from_slice(&prefix);
                    stdout
                        .read_exact(&mut header[4..])
                        .await
                        .map_err(|e| e.to_string())?;
                    let (header, _) = decode_header_len_and_header(&header)?;
                    if header.version != PROTOCOL_VERSION
                        || header.request_id != request_id
                        || header.payload_len as usize > 64 * 1024 * 1024 + 65536
                    {
                        return Err("invalid preview response envelope".into());
                    }
                    let mut payload = vec![0; header.payload_len as usize];
                    stdout
                        .read_exact(&mut payload)
                        .await
                        .map_err(|e| e.to_string())?;
                    let (response, data) = if header.kind == "bitmap" {
                        if payload.len() < 4 {
                            return Err("truncated bitmap metadata".into());
                        }
                        let size = u32::from_le_bytes(
                            payload[..4].try_into().map_err(|_| "metadata size")?,
                        ) as usize;
                        if size > 65536 || size + 4 > payload.len() {
                            return Err("invalid bitmap metadata".into());
                        }
                        let response: PreviewResponse =
                            serde_json::from_slice(&payload[4..4 + size])
                                .map_err(|e| e.to_string())?;
                        let data = payload.split_off(4 + size);
                        (response, Some(data))
                    } else {
                        (
                            serde_json::from_slice(&payload).map_err(|e| e.to_string())?,
                            None,
                        )
                    };
                    match &response {
                        PreviewResponse::Opened {
                            document_id,
                            page_count,
                            title,
                            ..
                        } => {
                            let _ = updates
                                .send(PreviewUpdate::Opened {
                                    generation: 1,
                                    document_id: *document_id,
                                    page_count: *page_count,
                                    title: title.clone(),
                                })
                                .await;
                        }
                        PreviewResponse::Bitmap {
                            document_id,
                            page,
                            generation,
                            width,
                            height,
                            stride,
                            ..
                        } => {
                            let size = check_bitmap_size(*width, *height).map_err(|(_, e)| e)?;
                            let data = data.ok_or("bitmap payload missing")?;
                            if *stride as u64 != *width as u64 * 4 || data.len() as u64 != size {
                                return Err("invalid bitmap stride or length".into());
                            }
                            let _ = updates
                                .send(PreviewUpdate::Bitmap {
                                    generation: 1,
                                    document_id: *document_id,
                                    page: *page,
                                    generation_tag: *generation,
                                    width: *width,
                                    height: *height,
                                    data,
                                })
                                .await;
                        }
                        PreviewResponse::Error { code, message, .. } => {
                            let _ = updates
                                .send(PreviewUpdate::Error {
                                    generation: 1,
                                    code: code.clone(),
                                    message: message.clone(),
                                })
                                .await;
                        }
                        PreviewResponse::Closed { document_id, .. } => {
                            let _ = updates
                                .send(PreviewUpdate::Closed {
                                    generation: 1,
                                    document_id: *document_id,
                                })
                                .await;
                        }
                        _ => {}
                    }
                    Ok::<_, String>(response)
                })
                .await;
                match result {
                    Ok(Ok(response)) => {
                        let _ = reply.send(Ok(response));
                    }
                    Ok(Err(error)) => {
                        let _ = reply.send(Err(error));
                        break;
                    }
                    Err(_) => {
                        let _ = reply
                            .send(Err("preview worker timeout; reopen file to restart".into()));
                        break;
                    }
                }
            }
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = updates
                .send(PreviewUpdate::Disconnected { generation: 1 })
                .await;
        });
        Ok((
            Self {
                handle: handle.clone(),
                task,
                update_rx: Some(update_rx),
            },
            handle,
        ))
    }
    #[cfg(test)]
    pub fn fake() -> (Self, PreviewHandle) {
        let (handle, mut commands, _updates, update_rx) = channels();
        let task = tokio::spawn(async move {
            while let Some(command) = commands.recv().await {
                if let PreviewCmd::Request(_, reply) = command {
                    let _ = reply.send(Err("fake preview is not a production decoder".into()));
                } else {
                    break;
                }
            }
        });
        (
            Self {
                handle: handle.clone(),
                task,
                update_rx: Some(update_rx),
            },
            handle,
        )
    }
}
