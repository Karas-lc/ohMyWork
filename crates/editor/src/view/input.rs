//! 原生输入处理：实现 EntityInputHandler，直接操作 EditorSurface 状态。
//! 所有确定输入走 replace_source_range -> 单一 PageCommand::ReplaceSource。
//! preedit 仅 overlay 在表面 rope，不进入 PageDocument。
//! 端口自 Velotype（Apache-2.0，commit 07a24a9b5b953539dede8e61a1f7d9d0adbba590）：
//!   floor_char_boundary, normalize_utf8_range, utf16_to_utf8, utf8_to_utf16,
//!   utf16_range_to_utf8, utf8_range_to_utf16 （保留 surrogate 扩展规则）。

use std::ops::Range;
use std::rc::Rc;

use gpui_kit::UTF16Selection;
use gpui_kit::{
    App, Bounds, Context, EntityId, EntityInputHandler, Pixels, Point, Window,
    point, px, size,
};
use ropey::Rope;

use super::projection::{LeafId, ParsedProjection};
use super::{Composition, EditorSurface};
use crate::documents::{PageCommand, PageError, SourceRope, SourceSelection};
use crate::parse::parse_for_view;

// --- ported utf helpers (verbatim logic from Velotype text_offsets.rs, adapted) ---

/// 返回 <= index 的 char boundary。
pub(crate) fn floor_char_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let mut i = index;
    while !text.is_char_boundary(i) && i > 0 {
        i -= 1;
    }
    i
}

pub(crate) fn normalize_utf8_range(text: &str, range: Range<usize>) -> Range<usize> {
    let start = floor_char_boundary(text, range.start);
    let end = floor_char_boundary(text, range.end);
    start..end.max(start)
}

/// UTF-16 单元到 UTF-8 字节（标量值）。
pub(crate) fn utf16_to_utf8(text: &str, utf16_index: usize) -> usize {
    let mut utf8 = 0;
    let mut utf16 = 0;
    for ch in text.chars() {
        let utf16_len = ch.len_utf16();
        if utf16 + utf16_len > utf16_index {
            break;
        }
        utf8 += ch.len_utf8();
        utf16 += utf16_len;
    }
    utf8
}

pub(crate) fn utf8_to_utf16(text: &str, utf8_index: usize) -> usize {
    let mut utf16 = 0;
    let mut utf8 = 0;
    for ch in text.chars() {
        if utf8 >= utf8_index {
            break;
        }
        utf16 += ch.len_utf16();
        utf8 += ch.len_utf8();
    }
    utf16
}

pub(crate) fn utf16_range_to_utf8(text: &str, range: Range<usize>) -> Range<usize> {
    let start = utf16_to_utf8(text, range.start);
    let end = utf16_to_utf8(text, range.end);
    normalize_utf8_range(text, start..end)
}

pub(crate) fn utf8_range_to_utf16(text: &str, range: Range<usize>) -> Range<usize> {
    let start = utf8_to_utf16(text, range.start);
    let end = utf8_to_utf16(text, range.end);
    start..end
}

// --- replace source (唯一入口) ---

impl EditorSurface {
    /// 唯一修改源的路径：验证后 clone rope 局部替换，提交 PageCommand::ReplaceSource。
    /// 失败则回滚本地 projection/composition。
    pub(crate) fn replace_source_range(
        &mut self,
        range: Range<usize>,
        replacement: &str,
        selection_after: SourceSelection,
        cx: &mut Context<Self>,
    ) -> Result<(), PageError> {
        if self.document.read(cx).is_busy() {
            return Err(PageError::Busy);
        }
        let Some(snapshot) = self.snapshot(cx) else {
            return Err(PageError::NotReady);
        };
        let rev = self.document_revision.unwrap_or(snapshot.revision());
        // 基本 UTF-8 边界检查
        let source_text = snapshot.source().to_string();
        if range.start > source_text.len() || range.end > source_text.len() {
            return Err(PageError::InvalidRange);
        }
        // 局部替换
        let mut new_text = source_text.clone();
        new_text.replace_range(range.clone(), replacement);
        // 只提交一次 ReplaceSource
        let before = self.selection;
        let cmd = PageCommand::ReplaceSource {
            revision: rev,
            text: SourceRope::from_str(&new_text),
            selection_before: before,
            selection_after,
        };
        let surface_id = cx.entity().entity_id();
        match self.document.update(cx, |doc, cx| doc.apply(cmd, cx)) {
            Ok(outcome) => {
                self.document_revision = Some(outcome.revision);
                self.selection = selection_after;
                self.projection = None; // 下一帧重投影
                self.input_error = None;
                // 清除 preedit 如果这是确认
                if self.composition.is_some() {
                    self.composition = None;
                    let _ = self.document.update(cx, |d, _| d.set_composing(surface_id, false));
                }
                Ok(())
            }
            Err(e) => {
                self.input_error = Some(e.to_string());
                self.projection = None;
                self.composition = None;
                Err(e)
            }
        }
    }
}

// --- EntityInputHandler ---

impl EntityInputHandler for EditorSurface {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let source = if let Some(snapshot) = self.snapshot(cx) {
            snapshot.source().to_string()
        } else {
            return None;
        };
        // 含 preedit 的表面 rope 模拟
        let mut display = source;
        if let Some(comp) = &self.composition {
            let r = comp.source_range.clone();
            if r.end <= display.len() {
                display.replace_range(r, &comp.preedit);
            }
        }
        let utf8_range = utf16_range_to_utf8(&display, range_utf16);
        let norm = normalize_utf8_range(&display, utf8_range);
        Some(display[norm].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        None
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.composition.as_ref().map(|c| {
            let src = self.snapshot(cx).map(|s| s.source().to_string()).unwrap_or_default();
            utf8_range_to_utf16(&src, c.source_range.clone())
        })
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(comp) = self.composition.take() {
            self.selection = comp.pre_selection;
            let surface_id = cx.entity().entity_id();
            let _ = self.document.update(cx, |d, _| d.set_composing(surface_id, false));
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = match self.snapshot(cx) {
            Some(s) => s.source().to_string(),
            None => return,
        };
        let utf8_range = match range_utf16 {
            Some(r) => utf16_range_to_utf8(&source, r),
            None => self.selection.anchor..self.selection.head,
        };
        let norm = normalize_utf8_range(&source, utf8_range);
        let after = SourceSelection {
            anchor: norm.start + text.len(),
            head: norm.start + text.len(),
        };
        let _ = self.replace_source_range(norm, text, after, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = match self.snapshot(cx) {
            Some(s) => s.source().to_string(),
            None => return,
        };
        let utf8_r = match range_utf16 {
            Some(r) => utf16_range_to_utf8(&source, r),
            None => self.selection.anchor..self.selection.head,
        };
        let norm = normalize_utf8_range(&source, utf8_r);
        if self.composition.is_none() {
            self.composition = Some(Composition {
                base_revision: self.document_revision.unwrap_or(0),
                base_epoch: self.external_epoch.unwrap_or(0),
                source_range: norm.clone(),
                preedit: text.to_string(),
                ime_selection: new_selected_range_utf16.clone().unwrap_or_default(),
                pre_selection: self.selection,
            });
            let surface_id = cx.entity().entity_id();
            let _ = self.document.update(cx, |d, _| d.set_composing(surface_id, true));
        } else if let Some(c) = &mut self.composition {
            c.preedit = text.to_string();
            c.ime_selection = new_selected_range_utf16.unwrap_or_default();
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        if let Some(_proj) = &self.projection {
            Some(Bounds {
                origin: element_bounds.origin,
                size: size(element_bounds.size.width / 2.0, px(20.)),
            })
        } else {
            None
        }
    }


    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        // TODO: use geometry to map point to display offset then source
        None
    }
}
