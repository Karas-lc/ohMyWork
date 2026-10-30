use std::{path::PathBuf, rc::Rc};

use gpui_kit::{
    AppContext as _, ClickEvent, Context, Entity, InteractiveElement, IntoElement,
    ParentElement as _, Pixels, Render, Role, SharedString, Size, StatefulInteractiveElement,
    Styled as _, Subscription, TestSupportExt as _, Window,
    component::ActiveTheme,
    component::{button::Button, input::{Input, InputEvent, InputState}},
    div,
    prelude::FluentBuilder,
    px, size,
    component::{v_virtual_list, VirtualListScrollHandle},
};

use crate::queries::search::SearchModel;
use project::SearchResult;

pub struct SearchView {
    model: Entity<SearchModel>,
    session: Entity<crate::session::WorkspaceSession>,
    input: Entity<InputState>,
    scroll: VirtualListScrollHandle,
    row_sizes: Rc<Vec<Size<Pixels>>>,
    _subscriptions: Vec<Subscription>,
}

impl SearchView {
    pub fn new(model: Entity<SearchModel>, session: Entity<crate::session::WorkspaceSession>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("搜索文件名、标题或正文"));
        let input_subscription = cx.subscribe_in(&input, window, Self::on_input_event);
        let model_subscription = cx.observe(&model, |this, _, cx| {
            let count = this.model.read(cx).rows().len();
            this.row_sizes = Rc::new(vec![size(px(640.), px(68.)); count]);
            cx.notify();
        });
        Self {
            model,
            session,
            input,
            scroll: VirtualListScrollHandle::new(),
            row_sizes: Rc::new(Vec::new()),
            _subscriptions: vec![input_subscription, model_subscription],
        }
    }



    fn on_input_event(&mut self, _: &Entity<InputState>, event: &InputEvent, _: &mut Window, cx: &mut Context<Self>) {
        if matches!(event, InputEvent::Change) { self.search(cx); }
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value().to_string();
        let include_archive = self.model.read(cx).include_archive();
        self.model.update(cx, |model, cx| model.search(query, include_archive, cx));
    }

    fn toggle_archive(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let include_archive = !self.model.read(cx).include_archive();
        let query = self.input.read(cx).value().to_string();
        self.model.update(cx, |model, cx| model.search(query, include_archive, cx));
    }

    fn load_more(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| model.load_more(cx));
    }
}

impl Render for SearchView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let row_sizes = self.row_sizes.clone();
        let has_more = model.has_more();
        let status: SharedString = model.status().into();
        let include_archive = model.include_archive();
        div().size_full().flex().flex_col().gap_3().p_4().bg(cx.theme().background).text_color(cx.theme().foreground)
            .child(div().flex().gap_2()
                .child(Input::new(&self.input).id("search-input").aria_label("搜索文件名、标题或正文").flex_1())
                .child(Button::new("search-include-archive").label(if include_archive { "隐藏 Archive" } else { "包含 Archive" }).on_click(cx.listener(Self::toggle_archive))))
            .child(div().id("search-status").role(Role::Status).test_support().aria_label(status.clone()).child(status))
            .child(div().id("search-results").flex_1().child(v_virtual_list(cx.entity(), "search-results-list", row_sizes,
                |this, range, _, cx| {
                    let model = this.model.read(cx);
                    let session = this.session.clone();
                    range.filter_map(|index| {
                        model.rows().get(index).cloned().map(|row| result_row(index, row, session.clone()))
                    }).collect::<Vec<_>>()
                }).track_scroll(&self.scroll)))
            .when(has_more, |this| this.child(Button::new("search-load-more").label("加载更多").on_click(cx.listener(Self::load_more))))
    }
}

fn result_row(index: usize, result: SearchResult, session: Entity<crate::session::WorkspaceSession>) -> impl IntoElement {
    let archive = if result.archive { " · Archive" } else { "" };
    let title = if result.title.is_empty() { String::new() } else { format!("\n{}", result.title) };
    let snippet = result.snippet.as_ref().map(|snippet| format!("\n{snippet}")).unwrap_or_default();
    div().id(("search-result", index)).test_support().flex().flex_col().p_2()
        .on_click({
            let path = result.logical_path.clone();
            move |_, _, cx| session.update(cx, |session, cx| session.request_open(PathBuf::from(path.clone()), cx))
        })
        .child(format!("{}{}{}{}", result.logical_path, archive, title, snippet))
}
