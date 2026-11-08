#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use nsh_core::{Checked, Size, file_dialog::FileRequest, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use peazip_core::Limits;

fn start() -> Headless {
    let mut config = EngineConfig::new(Root::of::<peazip_ui::App>());
    config.viewport = Size::new(px(1024.0), px(600.0));
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    app
}

fn open(app: &mut Headless) {
    app.press("role:button[Create archive…]").unwrap();
    app.wait_for("role:button[Continue]").unwrap();
}

fn selected(app: &Headless, label: &str) -> bool {
    app.screen()
        .accessibility_state(&format!("role:radio[{label}]"))
        .unwrap()
        .checked
        == Some(Checked::True)
}

fn method(bytes: &[u8]) -> u16 {
    assert_eq!(&bytes[..4], b"PK\x03\x04");
    u16::from_le_bytes(bytes[8..10].try_into().unwrap())
}

#[test]
fn cancel_and_escape_start_no_picker_and_discard_unconfirmed_options() {
    let mut app = start();
    open(&mut app);
    assert!(selected(&app, "Automatic"));
    assert!(app.screen().file_requests().is_empty());
    app.press("role:radio[Store (no compression)]").unwrap();
    app.press("role:button[Cancel]").unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    assert!(!app.screen().matches("role:button[Continue]").unwrap());
    open(&mut app);
    assert!(selected(&app, "Automatic"));
    app.press("role:radio[Deflate]").unwrap();
    assert_eq!(
        app.screen().input_text("Compression level").unwrap(),
        Some("6")
    );
    app.press_key("escape").unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    assert!(!app.screen().matches("role:button[Continue]").unwrap());
    assert!(app.screen().file_requests().is_empty());
    assert!(
        app.screen()
            .enabled("role:button[Create archive…]")
            .unwrap()
    );
}

#[test]
fn automatic_continue_is_required_before_the_source_picker() {
    let mut app = start();
    app.on_pick_paths(vec![]).unwrap();
    open(&mut app);
    assert!(selected(&app, "Automatic"));
    assert!(app.screen().file_requests().is_empty());
    app.press("role:button[Continue]").unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    assert_eq!(app.screen().file_requests().len(), 1);
    assert!(matches!(
        app.screen().file_requests()[0],
        FileRequest::OpenDialog(_)
    ));
}

#[test]
fn deflate_requires_valid_level_and_remembers_only_confirmed_settings() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.bin");
    let output = temp.path().join("deflate.zip");
    let payload: Vec<u8> = (0..=255).cycle().take(4096).collect();
    std::fs::write(&source, &payload).unwrap();
    let mut app = start();
    app.on_pick_paths(vec![source]).unwrap();
    app.on_pick_save_path(Some(output.clone())).unwrap();
    open(&mut app);
    app.press("role:radio[Deflate]").unwrap();
    assert_eq!(
        app.screen().input_text("Compression level").unwrap(),
        Some("6")
    );
    for invalid in ["", "0", "10", "abc"] {
        app.clear("Compression level").unwrap();
        if !invalid.is_empty() {
            app.type_text("Compression level", invalid).unwrap();
        }
        assert!(
            !app.screen().enabled("role:button[Continue]").unwrap(),
            "{invalid:?}"
        );
        assert!(app.screen().file_requests().is_empty());
    }
    app.clear("Compression level").unwrap();
    app.type_text("Compression level", "9").unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("Created ZIP: 1 entries").unwrap();
    let bytes = std::fs::read(&output).unwrap();
    assert_eq!(method(&bytes), 8);
    assert_eq!(
        peazip_core::extract_zip(&bytes, Limits::default()).unwrap()[0].data,
        payload
    );
    open(&mut app);
    assert!(selected(&app, "Deflate"));
    assert_eq!(
        app.screen().input_text("Compression level").unwrap(),
        Some("9")
    );
    app.press("role:radio[Store (no compression)]").unwrap();
    app.press("role:button[Cancel]").unwrap();
    app.wait_for("Created ZIP: 1 entries").unwrap();
    open(&mut app);
    assert!(selected(&app, "Deflate"));
    assert_eq!(
        app.screen().input_text("Compression level").unwrap(),
        Some("9")
    );
    app.press("role:button[Cancel]").unwrap();
    assert_eq!(app.screen().file_requests().len(), 2);
}

#[test]
fn explicit_deflate_ratio_error_does_not_fallback_and_retry_can_choose_store() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("repeated.bin");
    let output = temp.path().join("archive.zip");
    let payload = vec![0; 128 * 1024];
    std::fs::write(&source, &payload).unwrap();
    let mut app = start();
    app.on_pick_paths(vec![source]).unwrap();
    app.on_pick_save_path(Some(output.clone())).unwrap();
    open(&mut app);
    app.press("role:radio[Deflate]").unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("text:/^Could not create ZIP:.*compression ratio/")
        .unwrap();
    assert!(!output.exists());
    open(&mut app);
    assert!(selected(&app, "Deflate"));
    assert_eq!(
        app.screen().input_text("Compression level").unwrap(),
        Some("6")
    );
    app.press("role:radio[Store (no compression)]").unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("Created ZIP: 1 entries").unwrap();
    let bytes = std::fs::read(&output).unwrap();
    assert_eq!(method(&bytes), 0);
    assert_eq!(
        peazip_core::extract_zip(&bytes, Limits::default()).unwrap()[0].data,
        payload
    );
    assert_eq!(app.screen().file_requests().len(), 4);
}

#[test]
fn android_back_cancels_the_modal_without_file_requests() {
    let mut config = EngineConfig::new(Root::of::<peazip_ui::App>());
    config.environment.os = nsh_engine::Os::Android;
    config.environment.idiom = nsh_engine::Idiom::Phone;
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    open(&mut app);
    app.press("role:radio[Store (no compression)]").unwrap();
    app.back().unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    assert!(!app.screen().matches("role:button[Continue]").unwrap());
    assert!(app.screen().file_requests().is_empty());
    open(&mut app);
    assert!(selected(&app, "Automatic"));
    app.press("role:button[Cancel]").unwrap();
}
