use crate::{preview::PreviewSession, ui::tabs::TabRef};
use gpui_kit::{App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render, Subscription, Window, div, prelude::*};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{button::Button, dock::{BasePanel, Panel, PanelEvent, PanelState}};
use std::{path::PathBuf, sync::Arc};

pub struct PreviewPanel {
    focus: FocusHandle,
    tab_ref: TabRef,
    session: Entity<PreviewSession>,
    bitmap: Option<Arc<gpui_kit::RenderImage>>,
    bitmap_generation: Option<u64>,
    _subscription: Subscription,
}
impl PreviewPanel {
    pub fn new(tab_ref: TabRef, session: Entity<PreviewSession>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&session, |this, _, cx| this.take_frame(cx));
        let mut this = Self { focus: cx.focus_handle(), tab_ref, session, bitmap: None, bitmap_generation: None, _subscription: subscription };
        this.take_frame(cx);
        this
    }
    fn take_frame(&mut self, cx: &mut Context<Self>) {
        if let Some(mut frame) = self.session.update(cx, |session, _| session.take_frame()) {
            if self.bitmap_generation.is_some_and(|previous| frame.generation <= previous) { return; }
            self.bitmap_generation = Some(frame.generation);
            for pixel in frame.data.chunks_exact_mut(4) { pixel.swap(0, 2); }
            if let Some(buffer) = image::RgbaImage::from_raw(frame.width, frame.height, frame.data) {
                self.bitmap = Some(Arc::new(gpui_kit::RenderImage::new(vec![image::Frame::new(buffer)])));
            }
            cx.notify();
        }
    }
    fn update_session(&self, cx: &mut Context<Self>, update: impl FnOnce(&mut PreviewSession, &mut Context<PreviewSession>)) {
        self.session.update(cx, update);
    }
}
impl BasePanel for PreviewPanel {
    fn panel_name(&self) -> &'static str { "PreviewPanel" }
    fn closable(&self, _: &App) -> bool { false }
    fn dump(&self, _: &App) -> PanelState { crate::ui::panes::persisted(self.panel_name(), &self.tab_ref) }
}
impl Panel for PreviewPanel { fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { self.tab_ref.display_name() } }
impl EventEmitter<PanelEvent> for PreviewPanel {}
impl Focusable for PreviewPanel { fn focus_handle(&self, _: &App) -> FocusHandle { self.focus.clone() } }
impl Render for PreviewPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (status, page_info) = {
            let session = self.session.read(cx);
            let info = if session.page_count() == 0 { String::new() } else { format!("{} / {} · {}%", session.page() + 1, session.page_count(), (session.zoom() * 100.0) as u32) };
            (session.status().to_owned(), info)
        };
        let path = match &self.tab_ref { TabRef::Preview { path } => path.clone(), _ => PathBuf::new() };
        let closing = cx.global::<crate::context::WorkspaceContext>().closing();
        let close_path = path.clone();
        let mut body = div().id("preview-pane").test_support().size_full().flex().flex_col().track_focus(&self.focus).child(
            div().flex().gap_2()
                .child(Button::new("close-preview").label("关闭 / Close").on_click(move |_, _, cx| {
                    closing.update(cx, |closing, cx| closing.request_tab(close_path.clone(), cx));
                }))
                .child(Button::new("previous-page").label("上一页").on_click(cx.listener(|this, _, _, cx| this.update_session(cx, |s, cx| s.previous(cx)))))
                .child(Button::new("next-page").label("下一页").on_click(cx.listener(|this, _, _, cx| this.update_session(cx, |s, cx| s.next(cx)))))
                .child(Button::new("zoom-out").label("−").on_click(cx.listener(|this, _, _, cx| this.update_session(cx, |s, cx| s.zoom_out(cx)))))
                .child(Button::new("zoom-in").label("+").on_click(cx.listener(|this, _, _, cx| this.update_session(cx, |s, cx| s.zoom_in(cx)))))
                .child(div().id("preview-page-info").child(page_info))
                .child(div().id("preview-status").role(gpui_kit::Role::Status).test_support().aria_label(status.clone()).child(status))
        );
        if let Some(image) = &self.bitmap {
            body = body.child(div().id("preview-image").test_support().flex_1().min_h_0().child(gpui_kit::img(image.clone()).w_full().h_full().object_fit(gpui_kit::ObjectFit::Contain)));
        }
        body
    }
}
