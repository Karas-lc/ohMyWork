use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, ElementInputHandler, InputHandler, Point, Role, TestAppContext,
    WindowBounds, WindowOptions, px, size,
};
use omw_app::editor::{EditorDocument, EditorSurface};
use omw_document::parse;

#[gpui_kit::test]
fn editor_surface_routes_text_input_to_source_preserving_session(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let document = parse(b"**hello**").expect("fixture parses");
    let (shared_document, shared_session) = cx.update(|cx| {
        let document = cx.new(|_| EditorDocument::from_document(&document));
        let session = document.read(cx).session();
        (document, session)
    });
    let (window_handle, editor_entity) = cx.update(|cx| {
        let document = shared_document.clone();
        let editor = cx.new(|cx| EditorSurface::from_document(document.clone(), cx));
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(640.), px(480.)),
                })),
                ..Default::default()
            },
            cx,
            {
                let editor = editor.clone();
                move |_, _| editor.clone()
            },
        )
        .expect("open editor test window")
    });

    cx.update_window(window_handle, |_, window, cx| {
        window.render_frame(cx);
        let editor = window.find("paragraph-editor");
        assert_eq!(editor.role(), Some(Role::TextInput));
        assert_eq!(editor.value(), Some("hello"));
        let bounds = editor.bounds();
        let mut handler = ElementInputHandler::new(bounds, editor_entity.clone());
        assert!(handler.bounds_for_range(0..1, window, cx).is_some());
        let hit = handler.character_index_for_point(
            gpui_kit::point(bounds.origin.x + px(24.), bounds.origin.y + px(12.)),
            window,
            cx,
        );
        assert!(
            hit.is_some_and(|index| (1..=5).contains(&index)),
            "hit test returned {hit:?}"
        );
        window.click("paragraph-editor", cx);
        handler.set_selected_text_range(0..0, window, cx);
        window.press("shift-right", cx);
        assert_eq!(
            handler
                .selected_text_range(false, window, cx)
                .unwrap()
                .range,
            0..1
        );
        window.press("left", cx);
        window.input("!", cx);
        window.press("backspace", cx);
        window.press("ctrl-z", cx);
        let mut handler = ElementInputHandler::new(editor.bounds(), editor_entity.clone());
        handler.replace_and_mark_text_in_range(None, "日本", Some(2..2), window, cx);
        assert_eq!(handler.marked_text_range(window, cx), Some(0..2));
        assert_eq!(
            String::from_utf8(shared_session.lock().unwrap().bytes()).unwrap(),
            "**\\!hello**"
        );
        handler.replace_and_mark_text_in_range(None, "", None, window, cx);
        assert_eq!(handler.marked_text_range(window, cx), None);
        assert_eq!(
            String::from_utf8(shared_session.lock().unwrap().bytes()).unwrap(),
            "**\\!hello**"
        );
        handler.replace_and_mark_text_in_range(None, "日本", Some(2..2), window, cx);
        handler.replace_text_in_range(None, "日本語", window, cx);
        assert_eq!(
            String::from_utf8(shared_session.lock().unwrap().bytes()).unwrap(),
            "**日本語\\!hello**"
        );
        window.press("ctrl-z", cx);
        assert_eq!(
            String::from_utf8(shared_session.lock().unwrap().bytes()).unwrap(),
            "**\\!hello**"
        );
        window.press("ctrl-z", cx);
        assert_eq!(
            String::from_utf8(shared_session.lock().unwrap().bytes()).unwrap(),
            "**hello**"
        );
        window.render_frame(cx);
        let mut handler = ElementInputHandler::new(
            window.find("paragraph-editor").bounds(),
            editor_entity.clone(),
        );
        handler.set_selected_text_range(0..1, window, cx);
        window.press("ctrl-x", cx);
        assert_eq!(
            String::from_utf8(shared_session.lock().unwrap().bytes()).unwrap(),
            "**ello**"
        );
        window.render_frame(cx);
    })
    .expect("drive editor input");

    cx.update(|cx| assert!(shared_document.read(cx).revision() > 0));
    let session = shared_session.lock().expect("session lock");
    assert_eq!(
        String::from_utf8(session.bytes()).expect("UTF-8 fixture"),
        "**ello**"
    );
}

#[gpui_kit::test]
fn shared_document_updates_are_observed_by_each_editor_view(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let document = parse(b"hello").expect("fixture parses");
    let shared = cx.update(|cx| cx.new(|_| EditorDocument::from_document(&document)));
    let open_editor = |cx: &mut gpui_kit::App, shared: gpui_kit::Entity<EditorDocument>| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(640.), px(480.)),
                })),
                ..Default::default()
            },
            cx,
            move |_, cx| cx.new(|cx| EditorSurface::from_document(shared.clone(), cx)),
        )
        .expect("open shared editor")
    };
    let first = cx.update(|cx| open_editor(cx, shared.clone()));
    let second = cx.update(|cx| open_editor(cx, shared.clone()));
    cx.update_window(first.0, |_, window, cx| {
        window.render_frame(cx);
        window.click("paragraph-editor", cx);
        window.input("!", cx);
        window.render_frame(cx);
    })
    .expect("edit first view");
    cx.update_window(second.0, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("paragraph-editor").value(), Some("!hello"));
    })
    .expect("observe second view");
    cx.update(|cx| assert_eq!(shared.read(cx).revision(), 1));
}

#[gpui_kit::test]
fn scrolling_editor_refreshes_after_external_replacement(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let dir = tempfile::tempdir().expect("isolated fixture");
    let path = dir.path().join("note.md");
    let source = (0..40)
        .map(|i| format!("Paragraph {i} 中文🙂\n\n"))
        .collect::<String>();
    std::fs::write(&path, &source).expect("write fixture");
    let baseline = omw_local::read_snapshot(&path).expect("snapshot");
    let parsed = parse(&baseline.bytes).expect("parse fixture");
    let document =
        cx.update(|cx| cx.new(|_| EditorDocument::from_snapshot(&parsed, &path, &baseline)));
    let (handle, surface) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(640.), px(480.)),
                })),
                ..Default::default()
            },
            cx,
            |_, cx| cx.new(|cx| EditorSurface::from_document(document.clone(), cx)),
        )
        .expect("open editor")
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("paragraph-editor", cx);
        window.scroll(
            "paragraph-editor",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-500.))),
            cx,
        );
        window.render_frame(cx);
        let mut input =
            ElementInputHandler::new(window.find("paragraph-editor").bounds(), surface.clone());
        input.set_selected_text_range(0..0, window, cx);
        input.replace_text_in_range(None, "local ", window, cx);
    })
    .expect("scroll and edit");
    assert_eq!(std::fs::read(&path).expect("disk"), source.as_bytes());
    std::fs::write(&path, "External 中文🙂\n\nSecond").expect("external replacement");
    let snapshot = omw_local::read_snapshot(&path).expect("external snapshot");
    cx.update(|cx| {
        document.update(cx, |document, cx| {
            document
                .apply_external_event(omw_app::document_session::ExternalEvent::Snapshot(snapshot))
                .expect("reload");
            cx.notify();
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("paragraph-editor").value(),
            Some("External 中文🙂\nSecond")
        );
        let mut input =
            ElementInputHandler::new(window.find("paragraph-editor").bounds(), surface.clone());
        input.set_selected_text_range(0..0, window, cx);
        input.replace_text_in_range(None, "!", window, cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("paragraph-editor").value(),
            Some("!External 中文🙂\nSecond")
        );
        window.press("ctrl-z", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("paragraph-editor").value(),
            Some("External 中文🙂\nSecond")
        );
    })
    .expect("fresh projection and undo");
}
