#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use std::{path::Path, process::Command, time::Duration};

use nsh_core::{Checked, Size, file_dialog::FileRequest, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use peazip_core::{ArchiveFormat, Limits};

const LEVEL: &str = "LZMA2 compression level";

fn start(width: f32) -> Headless {
    let mut config = EngineConfig::new(Root::of::<peazip_ui::App>());
    config.viewport = Size::new(px(width), px(600.0));
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    app
}

fn open(app: &mut Headless) {
    app.press("role:button[Create archive…]").unwrap();
    app.wait_for("role:button[Continue]").unwrap();
}

fn choose(app: &mut Headless, label: &str) {
    app.press(&format!("role:radio[{label}]")).unwrap();
}

fn selected(app: &Headless, label: &str) -> bool {
    app.screen()
        .accessibility_state(&format!("role:radio[{label}]"))
        .unwrap()
        .checked
        == Some(Checked::True)
}

fn level(app: &mut Headless, value: &str) {
    app.clear(LEVEL).unwrap();
    if !value.is_empty() {
        app.type_text(LEVEL, value).unwrap();
    }
}

fn closed(app: &mut Headless) {
    app.wait_for_gone("role:button[Continue]", Duration::from_secs(5))
        .unwrap();
}

#[test]
fn validates_only_visible_options_and_discards_cancelled_drafts_at_both_widths() {
    for width in [360.0, 1024.0] {
        let mut app = start(width);
        open(&mut app);
        assert!(selected(&app, "ZIP"));
        assert!(selected(&app, "Automatic"));
        choose(&mut app, "7z");
        assert!(selected(&app, "LZMA2"));
        assert_eq!(app.screen().input_text(LEVEL).unwrap(), Some("3"));
        assert!(
            app.screen()
                .shows("Compresses files with LZMA2 at the selected level.")
        );
        for invalid in ["", "0", "10", "256", "abc", "-1"] {
            level(&mut app, invalid);
            assert!(
                !app.screen().enabled("role:button[Continue]").unwrap(),
                "{invalid:?}"
            );
            app.press_key("enter").unwrap();
            assert!(app.screen().file_requests().is_empty());
        }
        choose(&mut app, "Store");
        assert!(!app.screen().matches(LEVEL).unwrap());
        assert!(app.screen().enabled("role:button[Continue]").unwrap());
        choose(&mut app, "LZMA2");
        assert!(!app.screen().enabled("role:button[Continue]").unwrap());
        for format in ["ZIP", "TAR", "TAR.GZ"] {
            choose(&mut app, format);
            assert!(!app.screen().matches(LEVEL).unwrap());
            assert!(app.screen().enabled("role:button[Continue]").unwrap());
        }
        choose(&mut app, "7z");
        for valid in ["1", "9"] {
            level(&mut app, valid);
            assert!(app.screen().enabled("role:button[Continue]").unwrap());
        }
        app.press("role:button[Cancel]").unwrap();
        closed(&mut app);
        open(&mut app);
        assert!(selected(&app, "ZIP"));
        assert!(selected(&app, "Automatic"));
        choose(&mut app, "7z");
        assert!(selected(&app, "LZMA2"));
        assert_eq!(app.screen().input_text(LEVEL).unwrap(), Some("3"));
        assert!(app.screen().file_requests().is_empty());
        app.press("role:button[Cancel]").unwrap();
    }
}

#[test]
fn confirmed_format_method_and_level_survive_picker_cancellation() {
    let mut app = start(1024.0);
    open(&mut app);
    choose(&mut app, "7z");
    level(&mut app, "9");
    app.on_pick_paths(vec![]).unwrap();
    app.press("role:button[Continue]").unwrap();
    closed(&mut app);
    open(&mut app);
    assert!(selected(&app, "7z"));
    assert!(selected(&app, "LZMA2"));
    assert_eq!(app.screen().input_text(LEVEL).unwrap(), Some("9"));
    level(&mut app, "1");
    choose(&mut app, "ZIP");
    app.press("role:button[Cancel]").unwrap();
    closed(&mut app);
    open(&mut app);
    assert!(selected(&app, "7z"));
    assert_eq!(app.screen().input_text(LEVEL).unwrap(), Some("9"));
    level(&mut app, "0");
    choose(&mut app, "Store");
    app.on_pick_paths(vec![]).unwrap();
    app.press("role:button[Continue]").unwrap();
    closed(&mut app);
    open(&mut app);
    assert!(selected(&app, "7z"));
    assert!(selected(&app, "Store"));
    assert!(!app.screen().matches(LEVEL).unwrap());
    assert!(app.screen().enabled("role:button[Continue]").unwrap());
    assert_eq!(app.screen().file_requests().len(), 2);
    app.press("role:button[Cancel]").unwrap();
}

fn check_optional_7zip(path: &Path, method: &str) {
    for tool in ["7zz", "7z", "7za"] {
        let listing = match Command::new(tool).args(["l", "-slt"]).arg(path).output() {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => panic!("{tool}: {error}"),
        };
        assert!(
            listing.status.success(),
            "{}",
            String::from_utf8_lossy(&listing.stderr)
        );
        let listing = String::from_utf8(listing.stdout).unwrap();
        assert!(listing.contains(&format!("Method = {method}")), "{listing}");
        assert!(listing.contains("Solid = -"), "{listing}");
        let result = Command::new(tool).arg("t").arg(path).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
}

#[test]
fn app_passes_store_and_lzma2_to_native_and_saves_recognizable_archives() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.bin");
    let payload: Vec<u8> = (0..=255).cycle().take(4096).collect();
    std::fs::write(&source, &payload).unwrap();
    for (method, compression_level) in [("Store", None), ("LZMA2", Some("1")), ("LZMA2", Some("9"))]
    {
        let output = temp.path().join(format!(
            "{method}-{}.7z",
            compression_level.unwrap_or("stored")
        ));
        let mut app = start(1024.0);
        app.on_pick_paths(vec![source.clone()]).unwrap();
        app.on_pick_save_path(Some(output.clone())).unwrap();
        open(&mut app);
        choose(&mut app, "7z");
        choose(&mut app, method);
        if let Some(value) = compression_level {
            level(&mut app, value);
        }
        assert!(app.screen().file_requests().is_empty());
        app.press("role:button[Continue]").unwrap();
        app.wait_for("Created 7Z: 1 entries").unwrap();
        let requests = app.screen().file_requests();
        assert_eq!(requests.len(), 2);
        assert!(matches!(requests[0], FileRequest::OpenDialog(_)));
        let FileRequest::SaveDialog(save) = &requests[1] else {
            panic!("expected save dialog")
        };
        assert_eq!(save.file_name.as_deref(), Some("Archive.7z"));
        assert_eq!(save.filters.len(), 1);
        assert_eq!(save.filters[0].extensions, ["7z"]);
        let bytes = std::fs::read(&output).unwrap();
        assert_eq!(
            peazip_core::detect_format(&bytes).unwrap(),
            ArchiveFormat::SevenZip
        );
        let entries = peazip_core::extract_archive(&bytes, Limits::default()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "source.bin");
        assert_eq!(entries[0].data, payload);
        check_optional_7zip(&output, if method == "Store" { "Copy" } else { "LZMA2" });
        assert_eq!(std::fs::read(&source).unwrap(), payload);
    }
}
