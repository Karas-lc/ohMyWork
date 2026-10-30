use std::{path::PathBuf, sync::Arc};

use editor::{EditorSurface, documents::{DocumentStore, PageDocument}, parse};
use gpui_kit::{AppContext as _, WindowOptions};
use workspace::{
    BootstrapOptions, BootstrapView, Keymap, SystemDirectoryPicker, load_keymap,
    register_keymap_bindings,
};

pub fn run(editor_demo: bool) {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let app_data = project::AppDataPaths::from_local_app_data().ok();
            let keymap = app_data
                .as_ref()
                .map(load_keymap)
                .unwrap_or_else(Keymap::new);
            register_keymap_bindings(&keymap, cx);

            if editor_demo {
                let fixture = parse(
                    b"# \xe4\xb8\x80\xe7\xba\xa7\xe6\xa0\x87\xe9\xa2\x98 \xe4\xb8\xad\xe6\x96\x87\xf0\x9f\x99\x82\n\
## \xe4\xba\x8c\xe7\xba\xa7\xe6\xa0\x87\xe9\xa2\x98\n\
### \xe4\xb8\x89\xe7\xba\xa7\xe6\xa0\x87\xe9\xa2\x98\n\
#### \xe5\x9b\x9b\xe7\xba\xa7\xe6\xa0\x87\xe9\xa2\x98\n\
##### \xe4\xba\x94\xe7\xba\xa7\xe6\xa0\x87\xe9\xa2\x98\n\
###### \xe5\x85\xad\xe7\xba\xa7\xe6\xa0\x87\xe9\xa2\x98\n\
\n\
\xe6\xad\xa3\xe6\x96\x87 **\xe7\xb2\x97\xe4\xbd\x93** \xe3\x80\x81*\xe6\x96\x9c\xe4\xbd\x93*\xe3\x80\x81~~\xe5\x88\xa0\xe9\x99\xa4\xe7\xba\xbf~~ \xe5\x92\x8c `inline code`\xe3\x80\x82\n\
\n\
| \xe5\x90\x8d\xe7\xa7\xb0 | \xe6\x95\xb0\xe9\x87\x8f | \xe5\xa4\x87\xe6\xb3\xa8 |\n\
| :--- | ---: | :---: |\n\
| \xe8\x8b\xb9\xe6\x9e\x9c | 12 | **\xe6\x96\xb0\xe9\xb2\x9c** |\n\
| \xe7\xa9\xba\xe6\xa0\xbc |    | `a\\|b` |\n\
| \xe7\x9f\xad\xe8\xa1\x8c |\n\
\n\
- [ ] \xe6\xb5\x8b\xe8\xaf\x95\xe4\xbb\xbb\xe5\x8a\xa1 [priority:: 3]\n\
\n\
\xe6\x9c\x80\xe5\x90\x8e\xe6\xad\xa3\xe6\x96\x87\xef\xbc\x8c\xe4\xbf\x9d\xe6\x8c\x81\xe6\x9c\xaa\xe4\xbf\x9d\xe5\xad\x98\xe3\x80\x82",
                )
                .expect("built-in editor fixture parses");
                let document = cx.new(|_| PageDocument::from_document(&fixture));
                let documents = cx.new(|cx| {
                    DocumentStore::new(project::WriteQueue::new(2), cx)
                });
                let result = gpui_kit::open_window(WindowOptions::default(), cx, {
                    let document = document.clone();
                    let documents = documents.clone();
                    move |_, cx| cx.new(|cx| EditorSurface::new(document, documents, cx))
                });
                if let Err(error) = result {
                    eprintln!("无法打开编辑器演示窗口：{error}");
                    cx.quit();
                }
                return;
            }

            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(runtime) => Arc::new(runtime),
                Err(error) => {
                    eprintln!("无法启动后台运行时：{error}");
                    cx.quit();
                    return;
                }
            };
            let preview_worker = match preview_worker_path() {
                Ok(path) => path,
                Err(error) => {
                    eprintln!("无法定位预览工作进程：{error}");
                    cx.quit();
                    return;
                }
            };
            let result = gpui_kit::open_window(WindowOptions::default(), cx, {
                let options = BootstrapOptions {
                    app_data,
                    picker: Arc::new(SystemDirectoryPicker),
                    runtime,
                    preview_worker,
                };
                move |window, cx| cx.new(|cx| BootstrapView::new(options, window, cx))
            });
            if let Err(error) = result {
                eprintln!("无法打开 ohMyWork 窗口：{error}");
                cx.quit();
            }
        });
}

fn preview_worker_path() -> Result<PathBuf, std::io::Error> {
    let executable = std::env::current_exe()?;
    let directory = executable.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "current executable has no parent directory",
        )
    })?;
    Ok(directory.join(format!("omw-preview-worker{}", std::env::consts::EXE_SUFFIX)))
}
