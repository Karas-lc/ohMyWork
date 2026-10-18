use gpui_kit::component::{button::Button, toolbar::Toolbar};
use gpui_kit::{App, ClickEvent, Entity, IntoElement, Window};

use super::view::EditorSurface;

#[derive(Clone, Copy)]
pub(crate) enum EditorToolbarCommand {
    Save,
    Undo,
    Redo,
    Bold,
    Italic,
    Strikethrough,
    PasteMarkdown,
}
pub(crate) fn editor_toolbar(view: Entity<EditorSurface>) -> impl IntoElement {
    let save_view = view.clone();
    let undo_view = view.clone();
    let redo_view = view.clone();
    let bold_view = view.clone();
    let italic_view = view.clone();
    let strike_view = view.clone();
    let markdown_view = view;
    Toolbar::new("markdown-editor-toolbar")
        .content(
            Button::new("editor-save")
                .label("Save")
                .tooltip("Save (Ctrl+S)")
                .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    save_view.update(cx, |editor, cx| {
                        editor.handle_toolbar_command(EditorToolbarCommand::Save, cx)
                    });
                }),
        )
        .content(
            Button::new("editor-undo")
                .label("Undo")
                .tooltip("Undo (Ctrl+Z)")
                .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    undo_view.update(cx, |editor, cx| {
                        editor.handle_toolbar_command(EditorToolbarCommand::Undo, cx)
                    });
                }),
        )
        .content(
            Button::new("editor-redo")
                .label("Redo")
                .tooltip("Redo (Ctrl+Shift+Z)")
                .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    redo_view.update(cx, |editor, cx| {
                        editor.handle_toolbar_command(EditorToolbarCommand::Redo, cx)
                    });
                }),
        )
        .content(
            Button::new("editor-bold")
                .label("Bold")
                .tooltip("Bold")
                .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    bold_view.update(cx, |editor, cx| {
                        editor.handle_toolbar_command(EditorToolbarCommand::Bold, cx)
                    });
                }),
        )
        .content(
            Button::new("editor-italic")
                .label("Italic")
                .tooltip("Italic")
                .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    italic_view.update(cx, |editor, cx| {
                        editor.handle_toolbar_command(EditorToolbarCommand::Italic, cx)
                    });
                }),
        )
        .content(
            Button::new("editor-strike")
                .label("Strike")
                .tooltip("Strikethrough")
                .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    strike_view.update(cx, |editor, cx| {
                        editor.handle_toolbar_command(EditorToolbarCommand::Strikethrough, cx)
                    });
                }),
        )
        .content(
            Button::new("editor-paste-markdown")
                .label("Paste Markdown")
                .tooltip("Paste Markdown as structure")
                .on_click(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    markdown_view.update(cx, |editor, cx| {
                        editor.handle_toolbar_command(EditorToolbarCommand::PasteMarkdown, cx)
                    });
                }),
        )
}
