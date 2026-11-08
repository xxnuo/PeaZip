#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use nsh_core::{Checked, Size, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use peazip_core::{Limits, MemoryEntry};
use std::path::Path;

fn load(source: &Path, entries: usize) -> Headless {
    let mut config = EngineConfig::new(Root::of::<peazip_ui::App>());
    config.viewport = Size::new(px(1024.0), px(600.0));
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    app.press("role:button[All entries]").unwrap();
    app.on_pick_paths(vec![source.to_path_buf()]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for(&format!("{entries} entries")).unwrap();
    app
}

fn sample(path: &Path) {
    std::fs::write(path, include_bytes!("../../../tests/fixtures/sample.tar")).unwrap();
}

fn open(app: &mut Headless) {
    app.press("role:button[Extract…]").unwrap();
    app.wait_for("role:button[Continue]").unwrap();
}

fn radio(app: &Headless, label: &str) -> bool {
    app.screen()
        .accessibility_state(&format!("role:radio[{label}]"))
        .unwrap()
        .checked
        == Some(Checked::True)
}

fn confirm(app: &mut Headless) {
    app.press("role:button[Continue]").unwrap();
}

#[test]
fn no_selection_defaults_to_displayed_and_cancel_escape_do_not_open_a_picker() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("sample.tar");
    sample(&source);
    let mut app = load(&source, 5);
    assert!(app.screen().enabled("role:button[Extract…]").unwrap());
    let requests = app.screen().file_requests().len();
    open(&mut app);
    assert!(radio(&app, "Displayed entries"));
    assert!(!radio(&app, "All entries"));
    assert!(app.screen().shows("All entries view"));
    assert!(!radio(&app, "Selected entries"));
    assert!(
        !app.screen()
            .enabled("role:radio[Selected entries]")
            .unwrap()
    );
    assert!(radio(&app, "Full paths"));
    app.press("role:radio[Files only (no folders)]").unwrap();
    app.press("role:button[Cancel]").unwrap();
    app.wait_for("5 entries").unwrap();
    open(&mut app);
    assert!(radio(&app, "Full paths"), "cancel must not persist changes");
    app.press_key("escape").unwrap();
    app.wait_for("5 entries").unwrap();
    assert!(!app.screen().matches("role:button[Continue]").unwrap());
    assert_eq!(app.screen().file_requests().len(), requests);
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    assert_eq!(
        std::fs::read(source).unwrap(),
        include_bytes!("../../../tests/fixtures/sample.tar")
    );
}

#[test]
fn selected_scope_extracts_real_selected_bytes_and_keeps_the_selection() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("sample.tar");
    let output = temp.path().join("selected");
    sample(&source);
    let mut app = load(&source, 5);
    app.press("role:checkbox[docs/你好.txt]").unwrap();
    app.on_pick_save_path(Some(output.clone())).unwrap();
    let requests = app.screen().file_requests().len();
    open(&mut app);
    assert!(radio(&app, "Selected entries"));
    assert!(!radio(&app, "Displayed entries"));
    assert!(!radio(&app, "All entries"));
    assert_eq!(app.screen().file_requests().len(), requests);
    confirm(&mut app);
    app.wait_for("Extracted 1 entries").unwrap();
    assert_eq!(
        std::fs::read(output.join("docs/你好.txt")).unwrap(),
        "tar 你好\n".as_bytes()
    );
    assert!(!output.join("other.txt").exists());
    assert!(!output.join("docs/empty").exists());
    assert!(app.screen().shows("1 selected"));
    assert_eq!(app.screen().file_requests().len(), requests + 1);
}

#[test]
fn all_scope_without_selection_extracts_every_file_and_empty_directory() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("sample.tar");
    let output = temp.path().join("all");
    sample(&source);
    let mut app = load(&source, 5);
    app.press("role:button[Folder view]").unwrap();
    app.press("role:button[Open folder docs/]").unwrap();
    app.on_pick_save_path(Some(output.clone())).unwrap();
    open(&mut app);
    assert!(radio(&app, "Displayed entries"));
    app.press("role:radio[All entries]").unwrap();
    assert!(radio(&app, "All entries"));
    confirm(&mut app);
    app.wait_for("Extracted 5 entries").unwrap();
    assert_eq!(
        std::fs::read(output.join("docs/你好.txt")).unwrap(),
        "tar 你好\n".as_bytes()
    );
    assert_eq!(
        std::fs::read(output.join("docs/nested/data.bin")).unwrap(),
        [0, 127, 255]
    );
    assert_eq!(
        std::fs::read(output.join("other.txt")).unwrap(),
        b"not selected\n"
    );
    assert!(output.join("docs/empty").is_dir());
    assert!(app.screen().shows("0 selected"));
}

#[test]
fn displayed_folder_scope_excludes_siblings_keeps_selection_and_uses_the_frozen_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("sample.tar");
    let output = temp.path().join("displayed");
    sample(&source);
    let mut app = load(&source, 5);
    app.press("role:checkbox[other.txt]").unwrap();
    app.press("role:button[Folder view]").unwrap();
    app.press("role:button[Open folder docs/]").unwrap();
    app.on_pick_save_path(Some(output.clone())).unwrap();
    let requests = app.screen().file_requests().len();
    open(&mut app);
    assert!(radio(&app, "Selected entries"));
    app.wait_for("Current view: /docs/").unwrap();
    app.press("role:radio[Displayed entries]").unwrap();
    assert_eq!(app.screen().file_requests().len(), requests);
    std::fs::remove_file(&source).unwrap();
    confirm(&mut app);
    app.wait_for("Extracted 3 entries").unwrap();
    assert_eq!(
        std::fs::read(output.join("docs/你好.txt")).unwrap(),
        "tar 你好\n".as_bytes()
    );
    assert_eq!(
        std::fs::read(output.join("docs/nested/data.bin")).unwrap(),
        [0, 127, 255]
    );
    assert!(output.join("docs/empty").is_dir());
    assert!(!output.join("other.txt").exists());
    assert!(!source.exists());
    assert!(app.screen().shows("1 selected"));
    assert_eq!(
        app.screen().input_text("Archive path").unwrap(),
        Some("docs/")
    );
    app.press("role:button[Archive root]").unwrap();
    assert_eq!(
        app.screen()
            .accessibility_state("role:checkbox[other.txt]")
            .unwrap()
            .checked,
        Some(Checked::True)
    );
    assert_eq!(app.screen().file_requests().len(), requests + 1);
}

#[test]
fn displayed_scope_expands_implicit_directory_rows_without_inventing_entries() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("implicit.zip");
    let output = temp.path().join("displayed");
    let entries: Vec<_> = [
        ("docs/deep/a.txt", b"a".as_slice()),
        ("docs/deep/b.txt", b"b"),
        ("docs/c.txt", b"c"),
        ("other.txt", b"outside"),
    ]
    .into_iter()
    .map(|(path, data)| MemoryEntry {
        path: path.into(),
        is_dir: false,
        data: data.into(),
    })
    .collect();
    std::fs::write(
        &source,
        peazip_core::create_zip(&entries, Limits::default()).unwrap(),
    )
    .unwrap();
    let mut app = load(&source, 4);
    app.press("role:button[Folder view]").unwrap();
    app.press("role:button[Open folder docs/]").unwrap();
    assert_eq!(app.screen().count("role:checkbox").unwrap(), 2);
    assert!(app.screen().matches("role:checkbox[docs/deep/]").unwrap());
    app.on_pick_save_path(Some(output.clone())).unwrap();
    open(&mut app);
    assert!(radio(&app, "Displayed entries"));
    confirm(&mut app);
    app.wait_for("Extracted 3 entries").unwrap();
    for (name, bytes) in [
        ("docs/deep/a.txt", b"a"),
        ("docs/deep/b.txt", b"b"),
        ("docs/c.txt", b"c"),
    ] {
        assert_eq!(std::fs::read(output.join(name)).unwrap(), bytes);
    }
    assert!(!output.join("other.txt").exists());
    assert!(app.screen().shows("0 selected"));
}

#[test]
fn empty_current_directory_defaults_to_displayed_and_requires_explicit_all() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("sample.tar");
    let output = temp.path().join("all");
    sample(&source);
    let mut app = load(&source, 5);
    app.press("role:button[Folder view]").unwrap();
    app.press("role:button[Open folder docs/]").unwrap();
    app.press("role:button[Open folder docs/empty/]").unwrap();
    assert_eq!(app.screen().count("role:checkbox").unwrap(), 0);
    assert!(app.screen().enabled("role:button[Extract…]").unwrap());
    app.on_pick_save_path(Some(output.clone())).unwrap();
    let requests = app.screen().file_requests().len();
    open(&mut app);
    assert!(radio(&app, "Displayed entries"));
    assert!(!radio(&app, "All entries"));
    assert!(!app.screen().enabled("role:button[Continue]").unwrap());
    assert_eq!(app.screen().file_requests().len(), requests);
    assert!(!output.exists());
    app.wait_for("Current view: /docs/empty/").unwrap();
    app.press("role:radio[All entries]").unwrap();
    assert!(app.screen().enabled("role:button[Continue]").unwrap());
    confirm(&mut app);
    app.wait_for("Extracted 5 entries").unwrap();
    assert_eq!(
        std::fs::read(output.join("other.txt")).unwrap(),
        b"not selected\n"
    );
    assert_eq!(
        std::fs::read(output.join("docs/nested/data.bin")).unwrap(),
        [0, 127, 255]
    );
    assert!(output.join("docs/empty").is_dir());
    assert!(app.screen().shows("0 selected"));
    assert_eq!(
        app.screen().input_text("Archive path").unwrap(),
        Some("docs/empty/")
    );
    assert_eq!(app.screen().file_requests().len(), requests + 1);
}

#[test]
fn flat_view_displayed_scope_includes_all_files_and_empty_directories() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("sample.tar");
    let output = temp.path().join("flat-view");
    sample(&source);
    let mut app = load(&source, 5);
    app.on_pick_save_path(Some(output.clone())).unwrap();
    open(&mut app);
    assert!(radio(&app, "Displayed entries"));
    assert!(app.screen().shows("All entries view"));
    confirm(&mut app);
    app.wait_for("Extracted 5 entries").unwrap();
    assert_eq!(
        std::fs::read(output.join("docs/你好.txt")).unwrap(),
        "tar 你好\n".as_bytes()
    );
    assert_eq!(
        std::fs::read(output.join("docs/nested/data.bin")).unwrap(),
        [0, 127, 255]
    );
    assert_eq!(
        std::fs::read(output.join("other.txt")).unwrap(),
        b"not selected\n"
    );
    assert!(output.join("docs/empty").is_dir());
    assert!(app.screen().shows("0 selected"));
}

#[test]
fn files_only_flattens_files_and_omits_empty_directories() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("sample.tar");
    let output = temp.path().join("flat");
    sample(&source);
    let mut app = load(&source, 5);
    app.on_pick_save_path(Some(output.clone())).unwrap();
    open(&mut app);
    app.press("role:radio[Files only (no folders)]").unwrap();
    confirm(&mut app);
    app.wait_for("Extracted 3 entries").unwrap();
    assert_eq!(
        std::fs::read(output.join("你好.txt")).unwrap(),
        "tar 你好\n".as_bytes()
    );
    assert_eq!(
        std::fs::read(output.join("data.bin")).unwrap(),
        [0, 127, 255]
    );
    assert_eq!(
        std::fs::read(output.join("other.txt")).unwrap(),
        b"not selected\n"
    );
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 3);
    assert!(!output.join("docs").exists());
}

#[test]
fn files_only_collision_can_retry_full_paths_using_the_original_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("collision.zip");
    let output = temp.path().join("output");
    let entries: Vec<_> = [
        ("a/shared.txt", b"first".as_slice()),
        ("b/shared.txt", b"second"),
    ]
    .into_iter()
    .map(|(path, data)| MemoryEntry {
        path: path.into(),
        is_dir: false,
        data: data.into(),
    })
    .collect();
    let original = peazip_core::create_zip(&entries, Limits::default()).unwrap();
    std::fs::write(&source, &original).unwrap();
    let mut app = load(&source, 2);
    app.press("role:checkbox[a/shared.txt]").unwrap();
    app.on_pick_save_path(Some(output.clone())).unwrap();
    open(&mut app);
    assert!(radio(&app, "Selected entries"));
    app.press("role:radio[All entries]").unwrap();
    app.press("role:radio[Files only (no folders)]").unwrap();
    confirm(&mut app);
    app.wait_for("text:/^Could not extract archive:/").unwrap();
    assert!(!output.exists());
    assert!(app.screen().shows("1 selected"));
    assert_eq!(
        app.screen()
            .accessibility_state("role:checkbox[a/shared.txt]")
            .unwrap()
            .checked,
        Some(Checked::True)
    );
    std::fs::write(&source, b"changed after listing").unwrap();
    open(&mut app);
    assert!(radio(&app, "Files only (no folders)"));
    assert!(radio(&app, "Selected entries"));
    app.press("role:radio[All entries]").unwrap();
    app.press("role:radio[Full paths]").unwrap();
    confirm(&mut app);
    app.wait_for("Extracted 2 entries").unwrap();
    assert_eq!(
        std::fs::read(output.join("a/shared.txt")).unwrap(),
        b"first"
    );
    assert_eq!(
        std::fs::read(output.join("b/shared.txt")).unwrap(),
        b"second"
    );
    assert!(app.screen().shows("1 selected"));
    assert_eq!(std::fs::read(source).unwrap(), b"changed after listing");
}

#[test]
fn extraction_modal_blocks_shortcuts_and_existing_output_is_not_overwritten() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("sample.tar");
    let output = temp.path().join("existing");
    sample(&source);
    std::fs::create_dir(&output).unwrap();
    std::fs::write(output.join("sentinel"), b"untouched").unwrap();
    let mut app = load(&source, 5);
    app.press("role:checkbox[other.txt]").unwrap();
    app.on_pick_save_path(Some(output.clone())).unwrap();
    let requests = app.screen().file_requests().len();
    open(&mut app);
    for shortcut in [
        "mod+o",
        "mod+n",
        "mod+w",
        "mod+a",
        "mod+shift+a",
        "alt+left",
        "mod+enter",
    ] {
        app.press_key(shortcut).unwrap();
        assert!(app.screen().matches("role:button[Continue]").unwrap());
        assert_eq!(app.screen().file_requests().len(), requests);
    }
    confirm(&mut app);
    app.wait_for("text:/^Could not extract archive:/").unwrap();
    assert_eq!(
        std::fs::read(output.join("sentinel")).unwrap(),
        b"untouched"
    );
    assert_eq!(std::fs::read_dir(output).unwrap().count(), 1);
    assert!(app.screen().shows("1 selected"));
}
