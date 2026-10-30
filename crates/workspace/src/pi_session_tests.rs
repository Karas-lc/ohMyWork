use gpui_kit::test::TestAppContextExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size};
use project::pi::NewOrResume;
use serde_json::Value;
use std::{fs, path::PathBuf, sync::Arc, time::Duration};
use uuid::Uuid;
use crate::{pi::{ExtensionAnswer, PiSession}, views::pi::PiView};

#[gpui_kit::test]
async fn session_uses_fake_rpc_ids_and_accepts_extension_reply(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let test_exe = std::env::current_exe().unwrap();
    let debug_dir = test_exe.parent().and_then(|deps| deps.parent()).unwrap();
    let fake_pi = debug_dir.join(format!("fake_pi{}", std::env::consts::EXE_SUFFIX));
    assert!(fake_pi.is_file(), "fake Pi must be built beside the test executable: {}", fake_pi.display());

    let root = std::env::temp_dir().join(format!("ohmywork-pi-session-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("fake-pi-scenario.txt"), "--out-of-order\n--stream\n--ui\n--settled\n").unwrap();
    let runtime = Arc::new(tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap());
    let (window_handle, session) = cx.update(|cx| {
        let session = cx.new(|cx| PiSession::new(runtime, cx));
        let window = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(800.), px(600.)),
                })),
                ..Default::default()
            },
            cx,
            {
                let session = session.clone();
                move |window, cx| cx.new(|cx| PiView::new(session.clone(), window, cx))
            },
        ).unwrap().0;
        (window, session)
    });
    cx.update_window(window_handle, |_, window, cx| window.render_frame(cx)).unwrap();

    cx.update(|cx| session.update(cx, |session, cx| {
        session.connect(Some(PathBuf::from(&fake_pi)), root.clone(), NewOrResume::New, cx);
    }));
    cx.wait_for(window_handle.into(), Duration::from_secs(5), |_, _| {
        let records = fs::read_to_string(root.join("fake-pi-record.jsonl")).unwrap_or_default();
        records.lines().any(|line| serde_json::from_str::<Value>(line).is_ok_and(|record| {
            record["kind"] == "received" && record["message"]["type"] == "get_state"
        }))
    }).await;

    cx.update(|cx| session.update(cx, |session, cx| session.send("explicit prompt".into(), cx)));
    cx.wait_for(window_handle.into(), Duration::from_secs(5), |_, cx| {
        let session = session.read(cx);
        session.messages().iter().any(|message| message.contains("chunk0 chunk1 chunk2"))
            && session.current_prompt().is_some()
    }).await;
    let prompt = cx.update(|cx| session.read(cx).current_prompt().unwrap());
    assert_eq!(prompt.id, "fake-ui-1");
    cx.update(|cx| session.update(cx, |session, cx| session.reply(prompt.id.clone(), ExtensionAnswer::Select(1), cx)));
    cx.update(|cx| session.update(cx, |session, cx| session.close(false, cx)));
    cx.wait_for(window_handle.into(), Duration::from_secs(5), |_, cx| {
        session.read(cx).status() == "Pi 已关闭"
    }).await;

    let records = fs::read_to_string(root.join("fake-pi-record.jsonl")).unwrap();
    let records = records.lines().map(|line| serde_json::from_str::<Value>(line).unwrap()).collect::<Vec<_>>();
    assert_eq!(records[0]["kind"], "launch");
    assert_eq!(records[0]["cwd"], root.to_string_lossy().as_ref());
    let received = records.iter().filter(|record| record["kind"] == "received").map(|record| &record["message"]).collect::<Vec<_>>();
    let state = received.iter().find(|message| message["type"] == "get_state").expect("connect should query Pi state");
    assert!(state["id"].as_str().is_some_and(|id| !id.is_empty()));
    let prompt = received.iter().find(|message| message["type"] == "prompt").expect("prompt should cross RPC boundary");
    assert!(prompt["id"].as_str().is_some_and(|id| !id.is_empty()));
    assert!(prompt["message"].as_str().unwrap().contains("explicit prompt"));
    assert!(received.iter().any(|message| {
        message["type"] == "extension_ui_response"
            && message["id"] == "fake-ui-1"
            && message["value"] == "B"
    }));
    assert!(received.iter().any(|message| message["type"] == "abort"));
    cx.update_window(window_handle, |_, window, _| window.remove_window()).unwrap();
    let _ = fs::remove_dir_all(&root);

    let root = std::env::temp_dir().join(format!("ohmywork-pi-hang-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("fake-pi-scenario.txt"), "--hang\n").unwrap();
    let runtime = Arc::new(tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap());
    let (window_handle, session) = cx.update(|cx| {
        let session = cx.new(|cx| PiSession::new(runtime, cx));
        let window = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(800.), px(600.)),
                })),
                ..Default::default()
            },
            cx,
            {
                let session = session.clone();
                move |window, cx| cx.new(|cx| PiView::new(session.clone(), window, cx))
            },
        ).unwrap().0;
        (window, session)
    });
    cx.update_window(window_handle, |_, window, cx| window.render_frame(cx)).unwrap();
    cx.update(|cx| session.update(cx, |session, cx| {
        session.connect(Some(fake_pi), root.clone(), NewOrResume::New, cx);
    }));
    cx.wait_for(window_handle.into(), Duration::from_secs(5), |_, cx| {
        session.read(cx).status() == "已连接"
    }).await;
    cx.update(|cx| session.update(cx, |session, cx| session.close(false, cx)));
    cx.wait_for(window_handle.into(), Duration::from_secs(8), |_, cx| {
        session.read(cx).status().starts_with("Pi 未退出")
    }).await;
    assert!(cx.update(|cx| session.read(cx).is_connected()));
    cx.update(|cx| session.update(cx, |session, cx| session.close(true, cx)));
    cx.wait_for(window_handle.into(), Duration::from_secs(15), |_, cx| {
        session.read(cx).status() == "Pi 已终止"
    }).await;
    assert!(!cx.update(|cx| session.read(cx).is_connected()));
    cx.update_window(window_handle, |_, window, _| window.remove_window()).unwrap();
    let _ = fs::remove_dir_all(root);
}
