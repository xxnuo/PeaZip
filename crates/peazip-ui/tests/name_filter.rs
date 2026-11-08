#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use nsh_core::{Checked, Size, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use peazip_core::{Limits, MemoryEntry};
use std::path::Path;

fn load(source: &Path) -> Headless {
    let entries: Vec<_> = [
        ("docs/", b"".as_slice()),
        ("docs/empty/", b""),
        ("docs/Alpha.TXT", b"alpha"),
        ("docs/你好.txt", b"chinese"),
        ("docs/readme", b"readme"),
        ("outside.txt", b"outside"),
        ("Älpha.bin", b"unicode case"),
    ]
    .into_iter()
    .map(|(path, data)| MemoryEntry {
        path: path.into(),
        is_dir: path.ends_with('/'),
        data: data.into(),
    })
    .collect();
    std::fs::write(
        source,
        peazip_core::create_zip(&entries, Limits::default()).unwrap(),
    )
    .unwrap();
    let mut config = EngineConfig::new(Root::of::<peazip_ui::App>());
    config.viewport = Size::new(px(1024.0), px(900.0));
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    app.press("role:button[All entries]").unwrap();
    app.on_pick_paths(vec![source.to_path_buf()]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("7 entries").unwrap();
    app
}

fn filter(app: &mut Headless, value: &str, shown: usize, total: usize) {
    app.clear("Filter names").unwrap();
    if !value.is_empty() {
        app.type_text("Filter names", value).unwrap();
    }
    app.wait_for(&format!("{shown} of {total} rows shown"))
        .unwrap();
    assert_eq!(app.screen().count("role:checkbox").unwrap(), shown);
    if !value.is_empty() {
        assert!(
            app.screen()
                .matches("text:/Hidden selections are retained\\./")
                .unwrap()
        );
    }
}

fn checked(app: &Headless, path: &str) -> bool {
    app.screen()
        .accessibility_state(&format!("role:checkbox[{path}]"))
        .unwrap()
        .checked
        == Some(Checked::True)
}

fn extract(app: &mut Headless, output: &Path, scope: &str, count: usize) {
    app.on_pick_save_path(Some(output.to_path_buf())).unwrap();
    app.press("role:button[Extract…]").unwrap();
    app.wait_for("role:button[Continue]").unwrap();
    app.press(&format!("role:radio[{scope}]")).unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for(&format!("Extracted {count} entries")).unwrap();
}

#[test]
fn filtering_is_literal_case_insensitive_unicode_bounded_and_clear_restores_rows() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = load(&temp.path().join("names.zip"));
    filter(&mut app, "ALPHA", 1, 7);
    assert!(
        app.screen()
            .matches("role:checkbox[docs/Alpha.TXT]")
            .unwrap()
    );
    filter(&mut app, "älP", 1, 7);
    assert!(app.screen().matches("role:checkbox[Älpha.bin]").unwrap());
    filter(&mut app, "你好", 1, 7);
    assert!(
        app.screen()
            .matches("role:checkbox[docs/你好.txt]")
            .unwrap()
    );
    filter(&mut app, ".*", 0, 7);
    filter(&mut app, &"界".repeat(257), 0, 7);
    assert_eq!(
        app.screen()
            .input_text("Filter names")
            .unwrap()
            .unwrap()
            .chars()
            .count(),
        256
    );
    app.press("role:button[Clear filter]").unwrap();
    assert_eq!(app.screen().input_text("Filter names").unwrap(), Some(""));
    assert_eq!(app.screen().count("role:checkbox").unwrap(), 7);
    assert!(
        !app.screen()
            .matches("text:/Hidden selections are retained\\./")
            .unwrap()
    );
}

#[test]
fn visible_select_all_and_invert_retain_hidden_selection_and_scopes_extract_distinct_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("names.zip");
    let mut app = load(&source);
    app.press("role:checkbox[outside.txt]").unwrap();
    filter(&mut app, "docs/Alpha", 1, 7);
    app.press("role:button[Select all]").unwrap();
    assert!(app.screen().shows("2 selected"));
    assert!(checked(&app, "docs/Alpha.TXT"));
    app.press("role:button[Invert selection]").unwrap();
    assert!(!checked(&app, "docs/Alpha.TXT"));
    assert!(app.screen().shows("1 selected"));
    app.press("role:button[Select all]").unwrap();
    assert!(app.screen().shows("2 selected"));
    let displayed = temp.path().join("displayed");
    app.on_pick_save_path(Some(displayed.clone())).unwrap();
    app.press("role:button[Extract…]").unwrap();
    app.wait_for("role:button[Continue]").unwrap();
    app.press("role:radio[Displayed entries]").unwrap();
    std::fs::remove_file(&source).unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("Extracted 1 entries").unwrap();
    assert_eq!(
        std::fs::read(displayed.join("docs/Alpha.TXT")).unwrap(),
        b"alpha"
    );
    assert!(!displayed.join("outside.txt").exists());
    assert!(app.screen().shows("2 selected"));
    app.press("role:button[Clear filter]").unwrap();
    assert!(checked(&app, "outside.txt"));
    assert!(checked(&app, "docs/Alpha.TXT"));
    assert!(!checked(&app, "docs/你好.txt"));
    filter(&mut app, "docs/Alpha", 1, 7);
    let selected = temp.path().join("selected");
    extract(&mut app, &selected, "Selected entries", 2);
    assert_eq!(
        std::fs::read(selected.join("outside.txt")).unwrap(),
        b"outside"
    );
    assert_eq!(
        std::fs::read(selected.join("docs/Alpha.TXT")).unwrap(),
        b"alpha"
    );
    assert!(!selected.join("docs/你好.txt").exists());
    let all = temp.path().join("all");
    extract(&mut app, &all, "All entries", 7);
    for (path, data) in [
        ("docs/Alpha.TXT", b"alpha".as_slice()),
        ("docs/你好.txt", b"chinese"),
        ("docs/readme", b"readme"),
        ("outside.txt", b"outside"),
        ("Älpha.bin", b"unicode case"),
    ] {
        assert_eq!(std::fs::read(all.join(path)).unwrap(), data);
    }
    assert!(all.join("docs/empty").is_dir());
    assert!(app.screen().shows("2 selected"));
    assert_eq!(
        app.screen().input_text("Filter names").unwrap(),
        Some("docs/Alpha")
    );
}

#[test]
fn folder_names_and_flat_paths_use_different_matching_without_clearing_the_filter() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = load(&temp.path().join("names.zip"));
    filter(&mut app, "docs/Alpha", 1, 7);
    app.press("role:button[Folder view]").unwrap();
    app.wait_for("0 of 3 rows shown").unwrap();
    assert_eq!(
        app.screen().input_text("Filter names").unwrap(),
        Some("docs/Alpha")
    );
    app.press("role:button[All entries]").unwrap();
    app.wait_for("1 of 7 rows shown").unwrap();
    assert!(
        app.screen()
            .matches("role:checkbox[docs/Alpha.TXT]")
            .unwrap()
    );
    filter(&mut app, "docs", 5, 7);
    app.press("role:button[Folder view]").unwrap();
    app.wait_for("1 of 3 rows shown").unwrap();
    assert!(app.screen().matches("role:checkbox[docs/]").unwrap());
    let output = temp.path().join("docs-only");
    extract(&mut app, &output, "Displayed entries", 5);
    assert_eq!(
        std::fs::read(output.join("docs/Alpha.TXT")).unwrap(),
        b"alpha"
    );
    assert_eq!(
        std::fs::read(output.join("docs/你好.txt")).unwrap(),
        b"chinese"
    );
    assert!(output.join("docs/empty").is_dir());
    assert!(!output.join("outside.txt").exists());
    assert!(!output.join("Älpha.bin").exists());
}

#[test]
fn sorting_and_failed_navigation_retain_filter_but_success_and_reopen_clear_it() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("names.zip");
    let mut app = load(&source);
    app.press("role:checkbox[outside.txt]").unwrap();
    app.press("role:button[Folder view]").unwrap();
    filter(&mut app, "docs", 1, 3);
    app.press("role:button[Open folder docs/]").unwrap();
    assert_eq!(app.screen().input_text("Filter names").unwrap(), Some(""));
    assert_eq!(app.screen().count("role:checkbox").unwrap(), 4);
    filter(&mut app, "Alpha", 1, 4);
    for sort in ["Name", "Size (bytes)"] {
        app.press(&format!("role:button[{sort}]")).unwrap();
        assert_eq!(
            app.screen().input_text("Filter names").unwrap(),
            Some("Alpha")
        );
        assert_eq!(app.screen().count("role:checkbox").unwrap(), 1);
    }
    app.clear("Archive path").unwrap();
    app.type_text("Archive path", "missing/").unwrap();
    app.press("role:button[Go]").unwrap();
    app.wait_for("Archive directory not found").unwrap();
    assert_eq!(
        app.screen().input_text("Filter names").unwrap(),
        Some("Alpha")
    );
    assert!(
        app.screen()
            .matches("role:checkbox[docs/Alpha.TXT]")
            .unwrap()
    );
    assert!(app.screen().shows("1 selected"));
    app.clear("Archive path").unwrap();
    app.type_text("Archive path", "docs/empty/").unwrap();
    app.press("role:button[Go]").unwrap();
    assert_eq!(app.screen().input_text("Filter names").unwrap(), Some(""));
    assert_eq!(app.screen().count("role:checkbox").unwrap(), 0);
    app.press("role:button[Archive root]").unwrap();
    assert!(checked(&app, "outside.txt"));
    filter(&mut app, "outside", 1, 3);
    app.on_pick_paths(vec![source]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("7 entries").unwrap();
    assert_eq!(app.screen().input_text("Filter names").unwrap(), Some(""));
    assert_eq!(app.screen().count("role:checkbox").unwrap(), 3);
    assert!(app.screen().shows("0 selected"));
}

#[test]
fn no_match_disables_displayed_until_all_is_explicitly_chosen() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = load(&temp.path().join("names.zip"));
    filter(&mut app, ".*", 0, 7);
    let requests = app.screen().file_requests().len();
    let output = temp.path().join("all");
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press("role:button[Extract…]").unwrap();
    app.wait_for("role:button[Continue]").unwrap();
    assert_eq!(
        app.screen()
            .accessibility_state("role:radio[Displayed entries]")
            .unwrap()
            .checked,
        Some(Checked::True)
    );
    assert!(!app.screen().enabled("role:button[Continue]").unwrap());
    assert_eq!(app.screen().file_requests().len(), requests);
    assert!(!output.exists());
    app.press("role:radio[All entries]").unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("Extracted 7 entries").unwrap();
    assert_eq!(
        std::fs::read(output.join("outside.txt")).unwrap(),
        b"outside"
    );
    assert!(output.join("docs/empty").is_dir());
    assert_eq!(app.screen().input_text("Filter names").unwrap(), Some(".*"));
    assert_eq!(app.screen().count("role:checkbox").unwrap(), 0);
    assert_eq!(app.screen().file_requests().len(), requests + 1);
}
