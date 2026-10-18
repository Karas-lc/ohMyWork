use gpui_kit::{AppContext as _, WindowOptions};
use omw_app::BootstrapView;
use omw_app::editor::{EditorDocument, EditorSurface};
use omw_document::parse;

fn main() {
    let editor_demo = std::env::args().any(|argument| argument == "--editor-demo");
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            // Plan 24: register stable keymap defaults + user (if present). Load uses last-known-good on bad.
            let km = if let Ok(p) = omw_local::AppDataPaths::from_local_app_data() {
                omw_app::load_keymap(&p)
            } else {
                omw_app::Keymap::new()
            };
            omw_app::register_keymap_bindings(&km, cx);
            if editor_demo {
                let fixture = parse(
                    b"First paragraph with **markup**.\n\nSecond paragraph for selection.\n\n\
```raw\n<not-editable>\n```\n\nFinal paragraph.",
                )
                .expect("built-in editor fixture parses");
                let document = cx.new(|_| EditorDocument::from_document(&fixture));
                let result = gpui_kit::open_window(WindowOptions::default(), cx, {
                    let document = document.clone();
                    move |_, cx| cx.new(|cx| EditorSurface::from_document(document.clone(), cx))
                });
                if let Err(error) = result {
                    eprintln!("无法打开编辑器演示窗口：{error}");
                    cx.quit();
                }
            } else if let Err(error) =
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    cx.new(|cx| BootstrapView::new(window, cx))
                })
            {
                eprintln!("无法打开 ohMyWork 窗口：{error}");
                cx.quit();
            }
        });
}
