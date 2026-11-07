#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use peazip_core::{Limits, MemoryEntry};

fn start() -> Headless {
    let mut app = Headless::start(EngineConfig::new(Root::of::<peazip_ui::App>())).unwrap();
    app.wait_for("Open a ZIP archive to view its contents.")
        .unwrap();
    app
}

#[test]
fn cancel_keeps_the_initial_state_and_open_available() {
    let mut app = start();
    app.on_pick_paths(vec![]).unwrap();
    app.press("role:button[Open ZIP…]").unwrap();
    app.wait_for("Open a ZIP archive to view its contents.")
        .unwrap();
    assert!(app.screen().enabled("role:button[Open ZIP…]").unwrap());
    assert_eq!(app.screen().file_requests().len(), 1);
}

#[test]
fn lists_real_zip_then_reports_invalid_zip_and_recovers() {
    let path = std::env::temp_dir().join(format!("peazip-ui-{}.zip", std::process::id()));
    let bytes = peazip_core::create_zip(
        &[MemoryEntry {
            path: "hello.txt".into(),
            is_dir: false,
            data: b"hello".to_vec(),
        }],
        Limits::default(),
    )
    .unwrap();
    std::fs::write(&path, bytes).unwrap();
    let mut app = start();
    app.on_pick_paths(vec![path.clone()]).unwrap();
    app.press("role:button[Open ZIP…]").unwrap();
    app.wait_for("hello.txt").unwrap();
    app.wait_for("1 entries").unwrap();
    app.wait_for("text:5").unwrap();
    assert!(app.screen().enabled("role:button[Open ZIP…]").unwrap());

    std::fs::write(&path, b"not a zip").unwrap();
    app.press("role:button[Open ZIP…]").unwrap();
    app.wait_for("text:/^Could not open ZIP: invalid ZIP:/")
        .unwrap();
    assert!(app.screen().shows("hello.txt"));
    assert!(app.screen().enabled("role:button[Open ZIP…]").unwrap());

    std::fs::remove_file(&path).unwrap();
    app.press("role:button[Open ZIP…]").unwrap();
    app.wait_for("text:/^Could not open ZIP:.*os error/")
        .unwrap();
    assert!(app.screen().enabled("role:button[Open ZIP…]").unwrap());
}

#[test]
fn rejects_an_oversized_archive_and_allows_another_open() {
    let path = std::env::temp_dir().join(format!("peazip-ui-large-{}.zip", std::process::id()));
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(Limits::default().max_archive_bytes + 1)
        .unwrap();
    drop(file);
    let mut app = start();
    app.on_pick_paths(vec![path.clone()]).unwrap();
    app.press("role:button[Open ZIP…]").unwrap();
    app.wait_for("text:/^Could not open ZIP: ZIP limit exceeded: archive bytes/")
        .unwrap();
    assert!(app.screen().enabled("role:button[Open ZIP…]").unwrap());
    std::fs::remove_file(path).unwrap();
}
