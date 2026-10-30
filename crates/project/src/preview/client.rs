use super::{
    check_bitmap_size,
    protocol::{decode_header_len_and_header, encode_frame, FrameHeader, PreviewRequest, PreviewResponse, PROTOCOL_VERSION},
};
use std::{path::PathBuf, sync::{Arc, atomic::{AtomicU64, Ordering}}, time::Duration};
use tokio::{
    process::Command,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};


#[derive(Debug)]
pub struct PreviewReply {
    pub response: PreviewResponse,
    pub bitmap: Option<Vec<u8>>,
}

enum PreviewCmd {
    Request(PreviewRequest, oneshot::Sender<Result<PreviewReply, String>>),
    Shutdown,
}

#[derive(Clone)]
pub struct PreviewHandle {
    tx: mpsc::Sender<PreviewCmd>,
    next: Arc<AtomicU64>,
}
impl PreviewHandle {
    async fn request(&self, request: PreviewRequest) -> Result<PreviewReply, String> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(PreviewCmd::Request(request, tx)).await.map_err(|_| "preview worker stopped".to_string())?;
        rx.await.map_err(|_| "preview worker disconnected".to_string())?
    }
    pub async fn open(&self, path: PathBuf) -> Result<PreviewReply, String> {
        self.request(PreviewRequest::Open { path, request_id: self.next.fetch_add(1, Ordering::Relaxed) }).await
    }
    pub async fn render(&self, document_id: u64, page: u32, target_size: [u32; 2], gen_tag: u64) -> Result<PreviewReply, String> {
        check_bitmap_size(target_size[0], target_size[1]).map_err(|(_, e)| e)?;
        self.request(PreviewRequest::Render { document_id, page, target_size, generation: gen_tag, request_id: self.next.fetch_add(1, Ordering::Relaxed) }).await
    }
    pub async fn close(&self, document_id: u64) -> Result<PreviewReply, String> {
        self.request(PreviewRequest::Close { document_id, request_id: self.next.fetch_add(1, Ordering::Relaxed) }).await
    }
}

pub struct PreviewClient {
    handle: PreviewHandle,
    task: JoinHandle<()>,
    exit_rx: Option<oneshot::Receiver<()>>,
}
impl PreviewClient {
    pub fn spawn(worker_path: PathBuf) -> Result<(Self, PreviewHandle), String> {
        let mut child = Command::new(worker_path).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true).spawn().map_err(|e| format!("启动预览进程失败: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("preview stdin unavailable")?;
        let mut stdout = child.stdout.take().ok_or("preview stdout unavailable")?;
        let (tx, mut commands) = mpsc::channel(8);
        let (exit_tx, exit_rx) = oneshot::channel();
        let handle = PreviewHandle { tx, next: Arc::new(AtomicU64::new(1)) };
        let task = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Some(command) = commands.recv().await {
                let PreviewCmd::Request(request, reply_tx) = command else { break };
                let result = tokio::time::timeout(Duration::from_secs(10), async {
                    let request_id = match &request {
                        PreviewRequest::Open { request_id, .. } | PreviewRequest::Render { request_id, .. } | PreviewRequest::Close { request_id, .. } | PreviewRequest::Shutdown { request_id } => *request_id,
                    };
                    let payload = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
                    let header = FrameHeader { version: PROTOCOL_VERSION, request_id, kind: "request".into(), payload_len: payload.len() as u32 };
                    stdin.write_all(&encode_frame(&header, &payload)).await.map_err(|e| e.to_string())?;
                    stdin.flush().await.map_err(|e| e.to_string())?;
                    let mut prefix = [0u8; 4];
                    stdout.read_exact(&mut prefix).await.map_err(|e| e.to_string())?;
                    let length = u32::from_le_bytes(prefix) as usize;
                    if length == 0 || length > 65536 { return Err("invalid preview header size".into()); }
                    let mut encoded_header = vec![0u8; length + 4];
                    encoded_header[..4].copy_from_slice(&prefix);
                    stdout.read_exact(&mut encoded_header[4..]).await.map_err(|e| e.to_string())?;
                    let (header, _) = decode_header_len_and_header(&encoded_header)?;
                    if header.version != PROTOCOL_VERSION || header.request_id != request_id || header.payload_len as usize > 64 * 1024 * 1024 + 65536 { return Err("invalid preview response envelope".into()); }
                    let mut payload = vec![0; header.payload_len as usize];
                    stdout.read_exact(&mut payload).await.map_err(|e| e.to_string())?;
                    let (response, bitmap) = if header.kind == "bitmap" {
                        if payload.len() < 4 { return Err("truncated bitmap metadata".into()); }
                        let size = u32::from_le_bytes(payload[..4].try_into().map_err(|_| "metadata size")?) as usize;
                        if size > 65536 || size + 4 > payload.len() { return Err("invalid bitmap metadata".into()); }
                        (serde_json::from_slice(&payload[4..4 + size]).map_err(|e| e.to_string())?, Some(payload.split_off(4 + size)))
                    } else { (serde_json::from_slice(&payload).map_err(|e| e.to_string())?, None) };
                    if let PreviewResponse::Bitmap { width, height, stride, .. } = &response {
                        let size = check_bitmap_size(*width, *height).map_err(|(_, e)| e)?;
                        let data = bitmap.as_ref().ok_or("bitmap payload missing")?;
                        if *stride as u64 != *width as u64 * 4 || data.len() as u64 != size { return Err("invalid bitmap stride or length".into()); }
                    }
                    Ok::<_, String>(PreviewReply { response, bitmap })
                }).await;
                match result {
                    Ok(Ok(reply)) => { let _ = reply_tx.send(Ok(reply)); }
                    Ok(Err(error)) => { let _ = reply_tx.send(Err(error)); break; }
                    Err(_) => { let _ = reply_tx.send(Err("preview worker timeout; reopen file to restart".into())); break; }
                }
            }
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = exit_tx.send(());
        });
        Ok((Self { handle: handle.clone(), task, exit_rx: Some(exit_rx) }, handle))
    }
    pub fn take_exit(&mut self) -> Option<oneshot::Receiver<()>> { self.exit_rx.take() }
}
impl Drop for PreviewClient {
    fn drop(&mut self) { let _ = self.handle.tx.try_send(PreviewCmd::Shutdown); self.task.abort(); }
}
