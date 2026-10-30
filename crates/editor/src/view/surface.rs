//!
use std::{collections::HashMap, sync::Arc};

use gpui_kit::base::text::editor_host::{
    BlockNode, FlowPosition, InlineNode, LeafGeometry, Paragraph, RenderContext, TextMark,
    collect_leaf_geometry,
};
use gpui_kit::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, ParentElement as _, Pixels, Styled as _, Window, div,
};
use super::projection::{LeafId, LeafKind, ParsedProjection, ProjectedLeaf};

/// 主 Markdown 渲染元素。替代旧 EditorState 驱动的只读+编辑混用。
/// 由 EditorSurface 持有并在 render 中使用。
pub(crate) struct MarkdownElement {
    projection: Arc<ParsedProjection>,
    // active leaf for reveal syntax + selection
    active_leaf: Option<LeafId>,
    // geometry collected this frame (display utf8 ranges)
    // replaced every prepaint
    geometry: HashMap<u64, LeafGeometry>,
}


impl MarkdownElement {
    pub(crate) fn new(projection: Arc<ParsedProjection>, active_leaf: Option<LeafId>) -> Self {
        Self {
            projection,
            active_leaf,
            geometry: HashMap::new(),
        }
    }

}

impl Element for MarkdownElement {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let render_context = RenderContext::from_app(cx);
        let blocks: Vec<_> = self.projection.leaves.iter().map(|leaf| {
            let paragraph = build_paragraph_for_leaf(leaf, self.active_leaf == Some(leaf.id));
            match leaf.kind {
                LeafKind::Heading(level) => BlockNode::Heading {
                    level,
                    children: paragraph,
                    span: None,
                },
                _ => BlockNode::Paragraph(paragraph),
            }
        }).collect();
        let block_refs: Vec<_> = blocks.iter().collect();
        let mut content = div().flex().flex_col().w_full();
        for (index, (leaf, block)) in self.projection.leaves.iter().zip(&blocks).enumerate() {
            content = content.child(block.render_with_id(
                ("markdown-block", leaf.id.0),
                FlowPosition::at(&block_refs, index, index + 1 == blocks.len()),
                &render_context,
                window,
                cx,
            ));
        }
        let mut child = content.into_any_element();
        let layout_id = child.request_layout(window, cx);
        (layout_id, child)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (_, geometry) = collect_leaf_geometry(HashMap::new(), || {
            child.prepaint_at(bounds.origin, window, cx);
        });
        self.geometry = geometry;
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        child.paint(window, cx);
    }
}

impl IntoElement for MarkdownElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

// Helper to build Paragraph from a leaf (for use in BlockNode construction).
pub(crate) fn build_paragraph_for_leaf(leaf: &ProjectedLeaf, _active: bool) -> Paragraph {
    let mut p = Paragraph::default();
    let text = leaf.display_text.clone();
    let text_len = text.len();
    let mut mark_runs = vec![];
    for m in &leaf.inline_marks {
        let mut tm = TextMark::default();
        match m {
            super::projection::InlineMark::Bold => { tm = tm.bold(); }
            super::projection::InlineMark::Italic => { tm = tm.italic(); }
            super::projection::InlineMark::Strikethrough => { tm = tm.strikethrough(); }
            super::projection::InlineMark::Code => { tm = tm.code(); }
        }
        mark_runs.push((0..text_len, tm));
    }
    let node = InlineNode::new(text).marks(mark_runs);
    p.push(node);
    p.with_leaf_key(leaf.id.0 as u64)
}
