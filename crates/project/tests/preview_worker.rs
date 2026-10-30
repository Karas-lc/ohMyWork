use project::preview::{PreviewClient, PreviewResponse};
use std::path::PathBuf;

#[test]
fn worker_decodes_and_returns_exact_rgba_pixels() {
    let test_exe = std::env::current_exe().unwrap();
    let debug_dir = test_exe.parent().and_then(|deps| deps.parent()).unwrap();
    let worker = debug_dir.join(format!("omw-preview-worker{}", std::env::consts::EXE_SUFFIX));
    assert!(worker.is_file(), "preview worker must be built beside the test executable: {}", worker.display());

    let directory = tempfile::tempdir().unwrap();
    let image_path = directory.path().join("pixels.png");
    let expected = [
        255, 0, 0, 255, 0, 255, 0, 255,
        0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let image = image::RgbaImage::from_raw(2, 2, expected.to_vec()).unwrap();
    image.save(&image_path).unwrap();

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let (mut client, handle) = PreviewClient::spawn(PathBuf::from(&worker)).unwrap();
        let opened = handle.open(image_path).await.unwrap();
        let document_id = match opened.response {
            PreviewResponse::Opened { document_id, page_count, .. } => {
                assert_eq!(page_count, 1);
                document_id
            }
            response => panic!("expected Opened response, got {response:?}"),
        };

        let rendered = handle.render(document_id, 0, [2, 2], 7).await.unwrap();
        match rendered.response {
            PreviewResponse::Bitmap { document_id: actual_id, page, generation, width, height, stride, .. } => {
                assert_eq!(actual_id, document_id);
                assert_eq!(page, 0);
                assert_eq!(generation, 7);
                assert_eq!((width, height, stride), (2, 2, 8));
            }
            response => panic!("expected Bitmap response, got {response:?}"),
        }
        assert_eq!(rendered.bitmap.as_deref(), Some(expected.as_slice()));
        handle.close(document_id).await.unwrap();
        drop(client);
    });
}
