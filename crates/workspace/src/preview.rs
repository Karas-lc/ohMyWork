use gpui_kit::Context;
use project::preview::{PreviewClient, PreviewHandle, PreviewReply, PreviewResponse};
use std::{path::PathBuf, sync::Arc};

pub struct PreviewBitmap { pub generation: u64, pub width: u32, pub height: u32, pub data: Vec<u8> }

pub struct PreviewSession {
    client: Option<PreviewClient>,
    handle: Option<PreviewHandle>,
    process_lifetime: u64,
    document_id: Option<u64>,
    page: u32,
    page_count: u32,
    zoom: f32,
    generation: u64,
    frame: Option<PreviewBitmap>,
    status: String,
}
impl PreviewSession {
    pub fn new(path: PathBuf, worker_path: PathBuf, runtime: Arc<tokio::runtime::Runtime>, cx: &mut Context<Self>) -> Self {
        let mut this = Self { client: None, handle: None, process_lifetime: 1, document_id: None, page: 0, page_count: 0, zoom: 1.0, generation: 0, frame: None, status: "正在加载…".into() };
        let _enter = runtime.enter();
        match PreviewClient::spawn(worker_path) {
            Ok((mut client, handle)) => {
                let exit_receiver = client.take_exit();
                let lifetime = this.process_lifetime;
                let open_handle = handle.clone();
                cx.spawn(async move |entity, cx| {
                    let reply = open_handle.open(path).await;
                    let _ = entity.update(cx, |session, cx| {
                        if session.process_lifetime != lifetime { return; }
                        match reply {
                            Ok(reply) => session.accept_open(reply, cx),
                            Err(error) => session.status = error,
                        }
                        cx.notify();
                    });
                }).detach();
                if let Some(mut exit) = exit_receiver {
                    let lifetime = this.process_lifetime;
                    cx.spawn(async move |entity, cx| {
                        let _ = (&mut exit).await;
                        let _ = entity.update(cx, |session, cx| {
                            if session.process_lifetime == lifetime {
                                session.process_lifetime = session.process_lifetime.wrapping_add(1);
                                session.status = "预览进程已退出；关闭并重新打开文件可重试".into();
                                session.document_id = None;
                                session.frame = None;
                                cx.notify();
                            }
                        });
                    }).detach();
                }
                this.handle = Some(handle);
                this.client = Some(client);
            }
            Err(error) => this.status = error,
        }
        this
    }
    fn accept_open(&mut self, reply: PreviewReply, cx: &mut Context<Self>) {
        match reply.response {
            PreviewResponse::Opened { document_id, page_count, .. } => {
                self.document_id = Some(document_id);
                self.page_count = page_count;
                self.page = 0;
                self.render(cx);
            }
            PreviewResponse::Error { code, message, .. } => self.status = format!("{code}: {message}"),
            _ => self.status = "预览协议返回错误响应".into(),
        }
    }
    fn render(&mut self, cx: &mut Context<Self>) {
        let (Some(handle), Some(document)) = (self.handle.clone(), self.document_id) else { return; };
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let lifetime = self.process_lifetime;
        let page = self.page;
        let size = [(800.0 * self.zoom) as u32, (1000.0 * self.zoom) as u32];
        self.status = "正在渲染…".into();
        cx.spawn(async move |entity, cx| {
            let result = handle.render(document, page, size, generation).await;
            let _ = entity.update(cx, |session, cx| {
                if session.process_lifetime != lifetime || session.generation != generation { return; }
                match result {
                    Ok(reply) => session.accept_render(reply, generation),
                    Err(error) => session.status = error,
                }
                cx.notify();
            });
        }).detach();
    }
    fn accept_render(&mut self, reply: PreviewReply, generation: u64) {
        match (reply.response, reply.bitmap) {
            (PreviewResponse::Bitmap { document_id, page, generation: tag, width, height, .. }, Some(data))
                if Some(document_id) == self.document_id && page == self.page && tag == generation => {
                    self.frame = Some(PreviewBitmap { generation, width, height, data });
                    self.status = "预览已就绪".into();
                }
            (PreviewResponse::Error { code, message, .. }, _) => self.status = format!("{code}: {message}"),
            _ => self.status = "预览协议返回错误响应".into(),
        }
    }
    pub fn previous(&mut self, cx: &mut Context<Self>) { if self.page > 0 { self.page -= 1; self.render(cx); } }
    pub fn next(&mut self, cx: &mut Context<Self>) { if self.page + 1 < self.page_count { self.page += 1; self.render(cx); } }
    pub fn zoom_in(&mut self, cx: &mut Context<Self>) { let value = (self.zoom * 1.25).min(2.0); if value != self.zoom { self.zoom = value; self.render(cx); } }
    pub fn zoom_out(&mut self, cx: &mut Context<Self>) { let value = (self.zoom / 1.25).max(0.25); if value != self.zoom { self.zoom = value; self.render(cx); } }
    pub fn status(&self) -> &str { &self.status }
    pub fn page(&self) -> u32 { self.page }
    pub fn page_count(&self) -> u32 { self.page_count }
    pub fn zoom(&self) -> f32 { self.zoom }
    pub fn take_frame(&mut self) -> Option<PreviewBitmap> { self.frame.take() }
}
