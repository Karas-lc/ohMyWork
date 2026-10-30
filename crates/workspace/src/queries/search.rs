use std::{sync::Arc, time::Duration};

use gpui_kit::{AppContext as _, Context};
use project::{IndexError, IndexStore, SearchCancelToken, SearchPage, SearchRequest, SearchResult};

const PAGE_SIZE: usize = 50;
const DEBOUNCE: Duration = Duration::from_millis(20);

pub struct SearchModel {
    index: Arc<IndexStore>,
    query: String,
    include_archive: bool,
    rows: Vec<SearchResult>,
    has_more: bool,
    status: String,
    generation: u64,
    cancel: Option<SearchCancelToken>,
}

impl SearchModel {
    pub fn new(index: Arc<IndexStore>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            index,
            query: String::new(),
            include_archive: false,
            rows: Vec::new(),
            has_more: false,
            status: "索引尚未就绪".into(),
            generation: 0,
            cancel: None,
        };
        this.search(String::new(), false, cx);
        this
    }

    pub fn rows(&self) -> &[SearchResult] { &self.rows }
    pub fn include_archive(&self) -> bool { self.include_archive }
    pub fn has_more(&self) -> bool { self.has_more }
    pub fn status(&self) -> &str { &self.status }

    pub fn search(&mut self, query: String, include_archive: bool, cx: &mut Context<Self>) {
        self.query = query;
        self.include_archive = include_archive;
        self.start(false, cx);
    }

    pub fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.has_more { self.start(true, cx); }
    }

    fn start(&mut self, append: bool, cx: &mut Context<Self>) {
        if let Some(cancel) = self.cancel.take() { cancel.cancel(); }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let cancel = SearchCancelToken::new();
        self.cancel = Some(cancel.clone());
        let query = self.query.clone();
        let include_archive = self.include_archive;
        let offset = if append { self.rows.len() } else { 0 };
        if !append {
            self.rows.clear();
            self.has_more = false;
        }
        self.status = "搜索中…".into();
        cx.notify();
        let index = self.index.clone();
        let worker_cancel = cancel.clone();
        let task = cx.background_spawn(async move {
            std::thread::sleep(DEBOUNCE);
            if worker_cancel.is_cancelled() { return Err(IndexError::SearchCancelled); }
            let mut request = SearchRequest::new(query, include_archive, offset, PAGE_SIZE);
            request.cancel = worker_cancel;
            index.search(request)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || cancel.is_cancelled() { return; }
                match result {
                    Ok(page) => this.apply_page(page),
                    Err(IndexError::SearchCancelled) => return,
                    Err(error) => this.status = format!("搜索失败：{error}"),
                }
                cx.notify();
            });
        }).detach();
    }

    fn apply_page(&mut self, page: SearchPage) {
        self.rows.extend(page.results);
        self.has_more = page.has_more;
        self.status = if page.index_ready {
            if self.rows.is_empty() {
                "没有匹配结果".into()
            } else {
                format!("已显示 {} 条结果{}", self.rows.len(), if self.has_more { "，可继续加载" } else { "" })
            }
        } else {
            "索引尚未就绪".into()
        };
    }
}

impl Drop for SearchModel {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancel { cancel.cancel(); }
    }
}
