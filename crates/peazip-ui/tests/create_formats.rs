#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use std::{collections::BTreeMap, path::Path, time::Duration};

use nsh_core::{Checked, Size, file_dialog::FileRequest, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use peazip_core::Limits;

fn start(width: f32) -> Headless {
    let mut config = EngineConfig::new(Root::of::<peazip_ui::App>());
    config.viewport = Size::new(px(width), px(600.0));
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    app
}

fn dialog(app: &mut Headless) {
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

fn level(app: &mut Headless, label: &str, value: &str) {
    app.clear(label).unwrap();
    if !value.is_empty() {
        app.type_text(label, value).unwrap();
    }
}

fn cancel(app: &mut Headless) {
    app.press("role:button[Cancel]").unwrap();
    app.wait_for_gone("role:button[Continue]", Duration::from_secs(5))
        .unwrap();
}

fn folder(app: &mut Headless, path: &Path, count: usize) {
    app.on_pick_paths(vec![path.to_path_buf()]).unwrap();
    app.press("role:button[Open folder…]").unwrap();
    app.wait_for(&format!("{count} items")).unwrap();
}

fn save_request(app: &Headless, before: usize, filename: &str, extensions: &[&str]) {
    let requests = &app.screen().file_requests()[before..];
    assert_eq!(
        requests.len(),
        1,
        "selected filesystem sources require only a save dialog"
    );
    let FileRequest::SaveDialog(save) = &requests[0] else {
        panic!("expected a save dialog")
    };
    assert_eq!(
        save.title.as_deref(),
        Some("Create archive — choose a new filename")
    );
    assert_eq!(save.button_label.as_deref(), Some("Create"));
    assert_eq!(save.file_name.as_deref(), Some(filename));
    assert_eq!(save.filters.len(), 1);
    assert_eq!(save.filters[0].extensions, extensions);
}

fn extracted(path: &Path) -> BTreeMap<String, Vec<u8>> {
    peazip_core::extract_archive(&std::fs::read(path).unwrap(), Limits::default())
        .unwrap()
        .into_iter()
        .map(|entry| (entry.path, entry.data))
        .collect()
}

#[cfg(unix)]
fn check_system_tar(path: &Path, gzip: bool) {
    let listing = std::process::Command::new("tar")
        .arg("-tf")
        .arg(path)
        .output()
        .unwrap();
    assert!(
        listing.status.success(),
        "{}",
        String::from_utf8_lossy(&listing.stderr)
    );
    let names: std::collections::BTreeSet<_> = String::from_utf8(listing.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        names,
        [
            "docs/",
            "docs/empty/",
            "docs/nested/",
            "docs/nested/data.bin",
            "docs/你好.txt",
            "loose.txt"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );
    for (name, expected) in [
        ("docs/nested/data.bin", [0, 127, 255].as_slice()),
        ("docs/你好.txt", "归档内容\n".as_bytes()),
        ("loose.txt", b"selected".as_slice()),
    ] {
        let result = std::process::Command::new("tar")
            .arg("-xOf")
            .arg(path)
            .arg(name)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, expected);
    }
    if gzip {
        let result = std::process::Command::new("gzip")
            .arg("-t")
            .arg(path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn selected_sources_create_all_formats_with_correct_save_metadata_and_real_contents() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir_all(source.join("docs/empty")).unwrap();
    std::fs::create_dir(source.join("docs/nested")).unwrap();
    std::fs::write(source.join("docs/nested/data.bin"), [0, 127, 255]).unwrap();
    std::fs::write(source.join("docs/你好.txt"), "归档内容\n").unwrap();
    std::fs::write(source.join("loose.txt"), b"selected").unwrap();
    std::fs::write(source.join("skip.txt"), b"not selected").unwrap();
    let expected = BTreeMap::from([
        ("docs/".into(), vec![]),
        ("docs/empty/".into(), vec![]),
        ("docs/nested/".into(), vec![]),
        ("docs/nested/data.bin".into(), vec![0, 127, 255]),
        ("docs/你好.txt".into(), "归档内容\n".as_bytes().to_vec()),
        ("loose.txt".into(), b"selected".to_vec()),
    ]);
    for (format, filename, extensions) in [
        ("ZIP", "Archive.zip", vec!["zip"]),
        ("TAR", "Archive.tar", vec!["tar"]),
        ("TAR.GZ", "Archive.tar.gz", vec!["gz", "tgz"]),
    ] {
        let mut app = start(1024.0);
        folder(&mut app, &source, 3);
        app.press("role:checkbox[docs/]").unwrap();
        app.press("role:checkbox[loose.txt]").unwrap();
        let requests = app.screen().file_requests().len();
        let output = temp.path().join(filename);
        app.on_pick_save_path(Some(output.clone())).unwrap();
        dialog(&mut app);
        assert!(selected(&app, "ZIP"));
        choose(&mut app, format);
        if format == "TAR.GZ" {
            assert_eq!(
                app.screen().input_text("GZIP compression level").unwrap(),
                Some("6")
            );
        }
        app.press("role:button[Continue]").unwrap();
        app.wait_for(&format!("Created {format}: 6 entries"))
            .unwrap();
        save_request(&app, requests, filename, &extensions);
        assert_eq!(extracted(&output), expected);
        let bytes = std::fs::read(&output).unwrap();
        match format {
            "ZIP" => assert_eq!(&bytes[..4], b"PK\x03\x04"),
            "TAR" => assert_eq!(&bytes[257..262], b"ustar"),
            _ => assert_eq!(&bytes[..3], &[0x1f, 0x8b, 8]),
        }
        #[cfg(unix)]
        if format != "ZIP" {
            check_system_tar(&output, format == "TAR.GZ");
        }
    }
    assert_eq!(
        std::fs::read(source.join("skip.txt")).unwrap(),
        b"not selected"
    );
    assert_eq!(
        std::fs::read(source.join("docs/nested/data.bin")).unwrap(),
        [0, 127, 255]
    );
}

#[test]
fn narrow_dialog_validates_only_the_current_format_and_cancels_invalid_drafts() {
    let mut app = start(360.0);
    dialog(&mut app);
    choose(&mut app, "Deflate");
    level(&mut app, "Compression level", "0");
    assert!(!app.screen().enabled("role:button[Continue]").unwrap());
    choose(&mut app, "TAR");
    assert!(!app.screen().matches("role:radio[Deflate]").unwrap());
    assert!(!app.screen().matches("Compression level").unwrap());
    assert!(!app.screen().matches("GZIP compression level").unwrap());
    assert!(app.screen().enabled("role:button[Continue]").unwrap());
    choose(&mut app, "TAR.GZ");
    assert_eq!(
        app.screen().input_text("GZIP compression level").unwrap(),
        Some("6")
    );
    for invalid in ["", "0", "10", "256", "abc"] {
        level(&mut app, "GZIP compression level", invalid);
        assert!(
            !app.screen().enabled("role:button[Continue]").unwrap(),
            "{invalid:?}"
        );
        app.press_key("enter").unwrap();
        assert!(app.screen().file_requests().is_empty());
    }
    choose(&mut app, "ZIP");
    assert!(selected(&app, "Deflate"));
    assert_eq!(
        app.screen().input_text("Compression level").unwrap(),
        Some("0")
    );
    assert!(!app.screen().enabled("role:button[Continue]").unwrap());
    choose(&mut app, "Store (no compression)");
    assert!(app.screen().enabled("role:button[Continue]").unwrap());
    choose(&mut app, "TAR.GZ");
    level(&mut app, "GZIP compression level", "9");
    assert!(app.screen().enabled("role:button[Continue]").unwrap());
    cancel(&mut app);
    dialog(&mut app);
    assert!(selected(&app, "ZIP"));
    assert!(selected(&app, "Automatic"));
    assert!(!app.screen().matches("GZIP compression level").unwrap());
    assert!(app.screen().file_requests().is_empty());
    cancel(&mut app);
}

#[test]
fn confirmed_gzip_level_is_remembered_but_cancelled_format_changes_are_discarded() {
    let mut app = start(1024.0);
    dialog(&mut app);
    choose(&mut app, "TAR.GZ");
    level(&mut app, "GZIP compression level", "9");
    app.on_pick_paths(vec![]).unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for_gone("role:button[Continue]", Duration::from_secs(5))
        .unwrap();
    assert_eq!(app.screen().file_requests().len(), 1);
    dialog(&mut app);
    assert!(selected(&app, "TAR.GZ"));
    assert_eq!(
        app.screen().input_text("GZIP compression level").unwrap(),
        Some("9")
    );
    choose(&mut app, "ZIP");
    choose(&mut app, "Store (no compression)");
    cancel(&mut app);
    dialog(&mut app);
    assert!(selected(&app, "TAR.GZ"));
    assert_eq!(
        app.screen().input_text("GZIP compression level").unwrap(),
        Some("9")
    );
    assert_eq!(app.screen().file_requests().len(), 1);
    cancel(&mut app);
}

#[test]
fn cancelling_other_formats_preserves_loaded_archive_and_selection_without_a_picker() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("original.tar");
    let original = include_bytes!("../../../tests/fixtures/sample.tar");
    std::fs::write(&source, original).unwrap();
    let mut app = start(1024.0);
    app.on_pick_paths(vec![source.clone()]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("5 entries").unwrap();
    app.press("role:checkbox[other.txt]").unwrap();
    for format in ["TAR", "TAR.GZ"] {
        dialog(&mut app);
        choose(&mut app, format);
        cancel(&mut app);
        assert!(app.screen().shows("5 entries"));
        assert_eq!(
            app.screen()
                .accessibility_state("role:checkbox[other.txt]")
                .unwrap()
                .checked,
            Some(Checked::True)
        );
        assert_eq!(app.screen().input_text("Archive path").unwrap(), Some("/"));
        assert_eq!(app.screen().file_requests().len(), 1);
    }
    assert_eq!(std::fs::read(source).unwrap(), original);
}

#[test]
fn gzip_ratio_failure_preserves_sources_and_can_retry_tar_then_stored_zip() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let input = source.join("repeated.bin");
    let payload = vec![0; 128 * 1024];
    std::fs::write(&input, &payload).unwrap();
    let output = temp.path().join("rejected.tar.gz");
    let tar = temp.path().join("retry.tar");
    let zip = temp.path().join("retry.zip");
    let mut app = start(1024.0);
    folder(&mut app, &source, 1);
    app.press("role:checkbox[repeated.bin]").unwrap();
    dialog(&mut app);
    choose(&mut app, "TAR.GZ");
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("text:/^Could not create TAR\\.GZ:.*compression ratio/")
        .unwrap();
    assert!(!output.exists());
    assert_eq!(
        app.screen()
            .accessibility_state("role:checkbox[repeated.bin]")
            .unwrap()
            .checked,
        Some(Checked::True)
    );
    assert_eq!(
        app.screen().input_text("Folder path").unwrap(),
        std::fs::canonicalize(&source).unwrap().to_str()
    );
    dialog(&mut app);
    assert!(selected(&app, "TAR.GZ"));
    choose(&mut app, "TAR");
    app.on_pick_save_path(Some(tar.clone())).unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("Created TAR: 1 entries").unwrap();
    assert_eq!(
        extracted(&tar),
        BTreeMap::from([("repeated.bin".into(), payload.clone())])
    );
    dialog(&mut app);
    assert!(selected(&app, "TAR"));
    choose(&mut app, "ZIP");
    choose(&mut app, "Store (no compression)");
    app.on_pick_paths(vec![input.clone()]).unwrap();
    app.on_pick_save_path(Some(zip.clone())).unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("Created ZIP: 1 entries").unwrap();
    let bytes = std::fs::read(&zip).unwrap();
    assert_eq!(u16::from_le_bytes(bytes[8..10].try_into().unwrap()), 0);
    assert_eq!(
        extracted(&zip),
        BTreeMap::from([("repeated.bin".into(), payload.clone())])
    );
    assert_eq!(std::fs::read(input).unwrap(), payload);
    assert_eq!(app.screen().file_requests().len(), 5);
}

#[test]
fn oversized_source_is_rejected_in_both_tar_modes_without_an_output_or_lost_selection() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let input = source.join("too-large.bin");
    let size = Limits::default().max_entry_bytes + 1;
    std::fs::File::create(&input)
        .unwrap()
        .set_len(size)
        .unwrap();
    for format in ["TAR", "TAR.GZ"] {
        let mut app = start(1024.0);
        folder(&mut app, &source, 1);
        app.press("role:checkbox[too-large.bin]").unwrap();
        dialog(&mut app);
        choose(&mut app, format);
        let output = temp.path().join(format!("rejected-{format}"));
        app.on_pick_save_path(Some(output.clone())).unwrap();
        app.press("role:button[Continue]").unwrap();
        app.wait_for("text:/^Could not create .*limit/").unwrap();
        assert!(!output.exists());
        assert_eq!(
            app.screen()
                .accessibility_state("role:checkbox[too-large.bin]")
                .unwrap()
                .checked,
            Some(Checked::True)
        );
        assert_eq!(std::fs::metadata(&input).unwrap().len(), size);
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    }
}
