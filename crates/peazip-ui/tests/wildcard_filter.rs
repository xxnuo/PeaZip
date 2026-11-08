#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use nsh_core::{Checked, MountItem, Size, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use peazip_core::{Limits, MemoryEntry};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn start() -> Headless {
    let mut config = EngineConfig::new(Root::of::<peazip_ui::App>());
    config.viewport = Size::new(px(1024.0), px(900.0));
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    app
}

fn load(source: &Path) -> Headless {
    let entries: Vec<_> = [
        ("docs/", b"".as_slice()),
        ("docs/empty/", b""),
        ("docs/Alpha.txt", b"alpha"),
        ("docs/Alpine.txt", b"alpine"),
        ("docs/beta.bin", b"beta"),
        ("bracket[ab].txt", b"brackets"),
        ("outside.txt", b"outside"),
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
    let mut app = start();
    app.press("role:button[All entries]").unwrap();
    app.on_pick_paths(vec![source.to_path_buf()]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("7 entries").unwrap();
    app
}

fn checked(app: &Headless, selector: &str) -> bool {
    app.screen().accessibility_state(selector).unwrap().checked == Some(Checked::True)
}

fn mode(app: &mut Headless, enabled: bool) {
    let selector = "role:switch[Use wildcards]";
    if checked(app, selector) != enabled {
        app.press(selector).unwrap();
    }
    assert_eq!(checked(app, selector), enabled);
}

fn rows(app: &Headless, group: &str) -> usize {
    app.screen()
        .count(&format!("role:group[{group}] >> role:checkbox"))
        .unwrap()
}

fn query(app: &mut Headless, value: &str, shown: usize, group: &str) {
    app.clear("Filter names").unwrap();
    if !value.is_empty() {
        app.type_text("Filter names", value).unwrap();
    }
    assert_eq!(rows(app, group), shown, "query {value:?}");
}

#[test]
fn literal_is_default_and_star_question_and_implicit_substring_masks_are_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = load(&temp.path().join("source.zip"));
    assert!(!checked(&app, "role:switch[Use wildcards]"));
    query(&mut app, "Al*ha", 0, "Archive contents");
    mode(&mut app, true);
    assert_eq!(rows(&app, "Archive contents"), 1);
    assert!(
        app.screen()
            .matches("role:checkbox[docs/Alpha.txt]")
            .unwrap()
    );
    query(&mut app, "Al?ha", 1, "Archive contents");
    query(&mut app, "AL*TXT", 2, "Archive contents");
    query(&mut app, "pha", 1, "Archive contents");
    mode(&mut app, false);
    query(&mut app, ".*", 0, "Archive contents");
    query(&mut app, "[ab]", 1, "Archive contents");
    assert!(app.screen().shows("bracket[ab].txt"));
}

#[test]
fn character_classes_ranges_and_regex_metacharacters_follow_the_mask_grammar() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = load(&temp.path().join("source.zip"));
    mode(&mut app, true);
    for (mask, count) in [
        ("Al[p]ha", 1),
        ("Al[!p]ha", 0),
        ("Al[!x]ha", 1),
        ("[a-z]", 7),
        ("[z-a]", 7),
        ("[^a]", 4),
        ("[*?]", 0),
        (".*", 5),
        ("(?s:.*)", 0),
        ("docs\\Alpha", 0),
    ] {
        query(&mut app, mask, count, "Archive contents");
        assert!(
            !app.screen().matches("text:/^Invalid name mask:/").unwrap(),
            "valid mask {mask:?}"
        );
    }
}

#[test]
fn invalid_masks_disable_displayed_without_starting_a_picker_but_all_still_extracts() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = load(&temp.path().join("source.zip"));
    mode(&mut app, true);
    for mask in ["[", "[]", "[!]", "[a-]", "[]]"] {
        query(&mut app, mask, 0, "Archive contents");
        app.wait_for("text:/^Invalid name mask:/").unwrap();
    }
    let requests = app.screen().file_requests().len();
    let output = temp.path().join("all");
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press("role:button[Extract…]").unwrap();
    app.wait_for("role:button[Continue]").unwrap();
    assert!(checked(&app, "role:radio[Displayed entries]"));
    assert!(!app.screen().enabled("role:button[Continue]").unwrap());
    assert_eq!(app.screen().file_requests().len(), requests);
    app.press("role:radio[All entries]").unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("Extracted 7 entries").unwrap();
    assert_eq!(
        std::fs::read(output.join("outside.txt")).unwrap(),
        b"outside"
    );
    assert_eq!(
        std::fs::read(output.join("bracket[ab].txt")).unwrap(),
        b"brackets"
    );
    assert!(output.join("docs/empty").is_dir());
    assert_eq!(app.screen().file_requests().len(), requests + 1);
}

#[test]
fn toggles_and_navigation_keep_mode_and_hidden_selection_and_displayed_exports_real_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.zip");
    let mut app = load(&source);
    app.press("role:checkbox[outside.txt]").unwrap();
    query(&mut app, "Al*", 0, "Archive contents");
    mode(&mut app, true);
    assert_eq!(rows(&app, "Archive contents"), 2);
    app.press("role:button[Select all]").unwrap();
    assert!(app.screen().shows("3 selected"));
    mode(&mut app, false);
    assert_eq!(rows(&app, "Archive contents"), 0);
    assert!(app.screen().shows("3 selected"));
    mode(&mut app, true);
    app.press("role:button[Invert selection]").unwrap();
    assert!(app.screen().shows("1 selected"));
    app.press("role:button[Clear filter]").unwrap();
    assert!(checked(&app, "role:switch[Use wildcards]"));
    assert!(checked(&app, "role:checkbox[outside.txt]"));
    assert!(!checked(&app, "role:checkbox[docs/Alpha.txt]"));
    query(&mut app, "docs*", 5, "Archive contents");
    app.press("role:button[Folder view]").unwrap();
    app.press("role:button[Open folder docs/]").unwrap();
    assert_eq!(app.screen().input_text("Filter names").unwrap(), Some(""));
    assert!(checked(&app, "role:switch[Use wildcards]"));
    query(&mut app, "Al*", 2, "Archive contents");
    let output = temp.path().join("displayed");
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press("role:button[Extract…]").unwrap();
    app.wait_for("role:button[Continue]").unwrap();
    app.press("role:radio[Displayed entries]").unwrap();
    std::fs::remove_file(source).unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("Extracted 2 entries").unwrap();
    assert_eq!(
        std::fs::read(output.join("docs/Alpha.txt")).unwrap(),
        b"alpha"
    );
    assert_eq!(
        std::fs::read(output.join("docs/Alpine.txt")).unwrap(),
        b"alpine"
    );
    assert!(!output.join("outside.txt").exists());
    assert!(!output.join("docs/beta.bin").exists());
    assert!(app.screen().shows("1 selected"));
    app.press("role:button[Archive root]").unwrap();
    assert!(checked(&app, "role:checkbox[outside.txt]"));
    assert!(checked(&app, "role:switch[Use wildcards]"));
}

#[test]
fn filesystem_mode_is_independent_and_matching_ids_drive_selection_inversion_and_zip() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("source.zip");
    let mut app = load(&archive);
    mode(&mut app, true);
    let source = temp.path().join("files");
    std::fs::create_dir_all(source.join("child")).unwrap();
    std::fs::write(source.join("child/Alpha-child.txt"), b"child").unwrap();
    for (name, data) in [
        ("Alpha.txt", b"alpha".as_slice()),
        ("Alpine.txt", b"alpine"),
        ("beta.bin", b"beta"),
        ("outside.txt", b"outside"),
    ] {
        std::fs::write(source.join(name), data).unwrap();
    }
    std::fs::copy(&archive, source.join("archive.zip")).unwrap();
    app.on_pick_paths(vec![source]).unwrap();
    app.press("role:button[Open folder…]").unwrap();
    app.wait_for("6 items").unwrap();
    assert!(!checked(&app, "role:switch[Use wildcards]"));
    query(&mut app, "chil*", 0, "Folder contents");
    mode(&mut app, true);
    assert_eq!(rows(&app, "Folder contents"), 1);
    app.press("role:button[Open folder child]").unwrap();
    app.wait_for("role:checkbox[Alpha-child.txt]").unwrap();
    assert_eq!(app.screen().input_text("Filter names").unwrap(), Some(""));
    assert!(checked(&app, "role:switch[Use wildcards]"));
    app.press("role:button[Up]").unwrap();
    app.wait_for("6 items").unwrap();
    assert!(checked(&app, "role:switch[Use wildcards]"));
    app.press("role:checkbox[outside.txt]").unwrap();
    mode(&mut app, false);
    query(&mut app, "Al*", 0, "Folder contents");
    mode(&mut app, true);
    assert_eq!(rows(&app, "Folder contents"), 2);
    app.press("role:button[Select all]").unwrap();
    assert!(app.screen().shows("3 selected"));
    app.press("role:button[Invert selection]").unwrap();
    assert!(app.screen().shows("1 selected"));
    app.press("role:button[Select all]").unwrap();
    assert!(app.screen().shows("3 selected"));
    mode(&mut app, false);
    assert_eq!(rows(&app, "Folder contents"), 0);
    assert!(app.screen().shows("3 selected"));
    let output = temp.path().join("selected.zip");
    let requests = app.screen().file_requests().len();
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press("role:button[Create archive…]").unwrap();
    app.wait_for("role:button[Continue]").unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("Created ZIP: 3 entries").unwrap();
    let contents: BTreeMap<_, _> =
        peazip_core::extract_zip(&std::fs::read(output).unwrap(), Limits::default())
            .unwrap()
            .into_iter()
            .map(|entry| (entry.path, entry.data))
            .collect();
    assert_eq!(
        contents,
        BTreeMap::from([
            ("Alpha.txt".into(), b"alpha".to_vec()),
            ("Alpine.txt".into(), b"alpine".to_vec()),
            ("outside.txt".into(), b"outside".to_vec())
        ])
    );
    assert!(
        checked(&app, "role:switch[Use wildcards]"),
        "archive mode must not inherit the filesystem's false mode"
    );
    assert_eq!(app.screen().file_requests().len(), requests + 1);
}

#[test]
fn wildcard_mode_is_disabled_while_testing_an_archive_and_its_value_is_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = load(&temp.path().join("source.zip"));
    mode(&mut app, true);
    app.press("role:checkbox[outside.txt]").unwrap();
    let target = app
        .screen()
        .press_target("role:switch[Use wildcards]")
        .unwrap()
        .unwrap();
    let disabled = Arc::new(AtomicBool::new(false));
    let captured = disabled.clone();
    app.observe(move |_, batch| {
        for item in &batch.items {
            if let MountItem::ViewProps { id, props } = item
                && *id == target
                && props.accessibility.state.disabled
            {
                captured.store(true, Ordering::SeqCst);
            }
        }
    });
    let requests = app.screen().file_requests().len();
    app.press("role:button[Test archive]").unwrap();
    app.wait_for("text:/^Tested /").unwrap();
    assert!(
        disabled.load(Ordering::SeqCst),
        "the live mode control must become disabled during archive work"
    );
    assert!(app.screen().enabled("role:switch[Use wildcards]").unwrap());
    assert!(checked(&app, "role:switch[Use wildcards]"));
    assert!(checked(&app, "role:checkbox[outside.txt]"));
    assert_eq!(app.screen().file_requests().len(), requests);
}

#[test]
fn create_and_extract_modals_explicitly_disable_mode_without_changing_it() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = load(&temp.path().join("source.zip"));
    mode(&mut app, true);
    app.press("role:checkbox[outside.txt]").unwrap();
    query(&mut app, "Al*", 2, "Archive contents");
    let target = app
        .screen()
        .press_target("role:switch[Use wildcards]")
        .unwrap()
        .unwrap();
    let disabled = Arc::new(AtomicBool::new(false));
    let captured = disabled.clone();
    app.observe(move |_, batch| {
        for item in &batch.items {
            if let MountItem::ViewProps { id, props } = item
                && *id == target
                && props.accessibility.state.disabled
            {
                captured.store(true, Ordering::SeqCst);
            }
        }
    });
    let requests = app.screen().file_requests().len();
    for action in ["Create archive…", "Extract…"] {
        disabled.store(false, Ordering::SeqCst);
        app.press(&format!("role:button[{action}]")).unwrap();
        app.wait_for("role:button[Continue]").unwrap();
        assert!(
            disabled.load(Ordering::SeqCst),
            "{action} must publish the mode's disabled state"
        );
        assert!(!app.screen().enabled("role:switch[Use wildcards]").unwrap());
        assert!(app.press("role:switch[Use wildcards]").is_err());
        assert_eq!(app.screen().file_requests().len(), requests);
        app.press("role:button[Cancel]").unwrap();
        app.wait_for("7 entries").unwrap();
        assert!(app.screen().enabled("role:switch[Use wildcards]").unwrap());
        assert!(checked(&app, "role:switch[Use wildcards]"));
        assert_eq!(
            app.screen().input_text("Filter names").unwrap(),
            Some("Al*")
        );
        assert!(app.screen().shows("1 selected"));
    }
}
