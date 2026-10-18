use std::{rc::Rc, sync::Arc, time::Duration};

use gpui_kit::{
    AppContext as _, ClickEvent, Context, Entity, InteractiveElement, IntoElement,
    ParentElement as _, Pixels, Render, Role, SharedString, Size, StatefulInteractiveElement,
    Styled as _, Subscription, TestSupportExt as _, Window,
    base::{VirtualListScrollHandle, v_virtual_list},
    component::ActiveTheme,
    component::{
        button::Button,
        input::{Input, InputEvent, InputState},
    },
    div,
    prelude::FluentBuilder,
    px, size,
};

use omw_local::{
    IndexError, IndexStore, SearchCancelToken, SearchPage, SearchRequest, SearchResult,
};

const PAGE_SIZE: usize = 50;
const DEBOUNCE: Duration = Duration::from_millis(20);

pub struct SearchView {
    index: Arc<IndexStore>,
    input: Entity<InputState>,
    scroll: VirtualListScrollHandle,
    rows: Vec<SearchResult>,
    row_sizes: Rc<Vec<Size<Pixels>>>,
    status: SharedString,
    include_archive: bool,
    generation: u64,
    cancel: Option<SearchCancelToken>,
    has_more: bool,
    _subscriptions: Vec<Subscription>,
}

impl SearchView {
    pub fn new(index: Arc<IndexStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("搜索文件名、标题或正文"));
        let subscriptions = vec![cx.subscribe_in(&input, window, Self::on_input_event)];
        Self {
            index,
            input,
            scroll: VirtualListScrollHandle::new(),
            rows: Vec::new(),
            row_sizes: Rc::new(Vec::new()),
            status: "索引尚未就绪".into(),
            include_archive: false,
            generation: 0,
            cancel: None,
            has_more: false,
            _subscriptions: subscriptions,
        }
    }

    pub fn input(&self) -> Entity<InputState> {
        self.input.clone()
    }

    pub fn results(&self) -> &[SearchResult] {
        &self.rows
    }

    pub fn include_archive(&self) -> bool {
        self.include_archive
    }

    pub fn status(&self) -> &str {
        self.status.as_ref()
    }

    fn on_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            self.start_search(false, cx);
        }
    }

    fn toggle_archive(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.include_archive = !self.include_archive;
        self.start_search(false, cx);
    }

    fn load_more(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.has_more {
            self.start_search(true, cx);
        }
    }

    fn start_search(&mut self, append: bool, cx: &mut Context<Self>) {
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let cancel = SearchCancelToken::new();
        self.cancel = Some(cancel.clone());
        let cancel_for_task = cancel.clone();
        let query = self.input.read(cx).value().to_string();
        let index = self.index.clone();
        let include_archive = self.include_archive;
        let offset = if append { self.rows.len() } else { 0 };
        if !append {
            self.rows.clear();
            self.has_more = false;
        }
        self.status = "搜索中…".into();
        self.rebuild_row_sizes();

        let task = cx.background_spawn({
            let cancel = cancel_for_task;
            async move {
                std::thread::sleep(DEBOUNCE);
                if cancel.is_cancelled() {
                    return Err(IndexError::SearchCancelled);
                }
                let mut request = SearchRequest::new(query, include_archive, offset, PAGE_SIZE);
                request.cancel = cancel;
                index.search(request)
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation || cancel.is_cancelled() {
                    return;
                }
                match result {
                    Ok(page) => this.apply_page(page, append, cx),
                    Err(IndexError::SearchCancelled) => {}
                    Err(error) => {
                        this.status = format!("搜索失败：{error}").into();
                    }
                }
            });
        })
        .detach();
    }

    fn apply_page(&mut self, page: SearchPage, append: bool, _: &mut Context<Self>) {
        if append {
            self.rows.extend(page.results);
        } else {
            self.rows = page.results;
        }
        self.has_more = page.has_more;
        self.status = if page.index_ready {
            if self.rows.is_empty() {
                "没有匹配结果".into()
            } else {
                format!(
                    "已显示 {} 条结果{}",
                    self.rows.len(),
                    if self.has_more {
                        "，可继续加载"
                    } else {
                        ""
                    }
                )
                .into()
            }
        } else {
            "索引尚未就绪".into()
        };
        self.rebuild_row_sizes();
    }

    fn rebuild_row_sizes(&mut self) {
        self.row_sizes = Rc::new(vec![size(px(640.), px(68.)); self.rows.len()]);
    }
}

impl Render for SearchView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row_sizes = self.row_sizes.clone();
        let has_more = self.has_more;
        let status = self.status.clone();
        let include_archive = self.include_archive;
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Input::new(&self.input)
                            .id("search-input")
                            .aria_label("搜索文件名、标题或正文")
                            .flex_1(),
                    )
                    .child(
                        Button::new("search-include-archive")
                            .label(if include_archive {
                                "隐藏 Archive"
                            } else {
                                "包含 Archive"
                            })
                            .on_click(cx.listener(Self::toggle_archive)),
                    ),
            )
            .child(
                div()
                    .id("search-status")
                    .role(Role::Status)
                    .test_support()
                    .aria_label(status.clone())
                    .child(status),
            )
            .child(
                div().id("search-results").flex_1().child(
                    v_virtual_list(
                        cx.entity(),
                        "search-results-list",
                        row_sizes,
                        |this, range, _, _| {
                            range
                                .map(|index| {
                                    let result = this.rows[index].clone();
                                    result_row(index, result)
                                })
                                .collect::<Vec<_>>()
                        },
                    )
                    .track_scroll(&self.scroll),
                ),
            )
            .when(has_more, |this| {
                this.child(
                    Button::new("search-load-more")
                        .label("加载更多")
                        .on_click(cx.listener(Self::load_more)),
                )
            })
    }
}

fn result_row(index: usize, result: SearchResult) -> impl IntoElement {
    let archive = if result.archive { " · Archive" } else { "" };
    let title = if result.title.is_empty() {
        String::new()
    } else {
        format!("\n{}", result.title)
    };
    let snippet = result
        .snippet
        .as_ref()
        .map(|snippet| format!("\n{snippet}"))
        .unwrap_or_default();
    div()
        .id(("search-result", index))
        .test_support()
        .flex()
        .flex_col()
        .p_2()
        .on_click({
            let path = result.logical_path.clone();
            move |_, _, cx| {
                if let Some(service) = cx.try_global::<crate::workbench::Workbench>() {
                    service.emit(crate::workbench::WorkbenchEvent::Open(
                        service.root.join(&path),
                    ));
                }
            }
        })
        .child(format!(
            "{}{}{}{}",
            result.logical_path, archive, title, snippet
        ))
}
