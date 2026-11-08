#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use std::{collections::BTreeMap, path::Path};

use nsh_core::{Checked, Size, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use peazip_core::Limits;

fn start(path: &Path, count: usize) -> Headless {
    let mut config = EngineConfig::new(Root::of::<peazip_ui::App>());
    config.viewport = Size::new(px(1024.0), px(900.0));
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    app.on_pick_paths(vec![path.to_path_buf()]).unwrap();
    app.press("role:button[Open folder…]").unwrap();
    app.wait_for(&format!("{count} of {count} items shown"))
        .unwrap();
    app
}

fn filter(app: &mut Headless, query: &str, shown: usize, total: usize) {
    app.clear("Filter names").unwrap();
    if !query.is_empty() {
        app.type_text("Filter names", query).unwrap();
    }
    app.wait_for(&format!("{shown} of {total} items shown"))
        .unwrap();
    assert_eq!(app.screen().count("role:checkbox").unwrap(), shown);
    assert_eq!(
        app.screen().input_text("Filter names").unwrap(),
        Some(query)
    );
}

fn checked(app: &Headless, name: &str) -> bool {
    app.screen()
        .accessibility_state(&format!("role:checkbox[{name}]"))
        .unwrap()
        .checked
        == Some(Checked::True)
}

#[test]
fn live_literal_unicode_filter_counts_clear_and_submit_never_navigate() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("folder")).unwrap();
    for name in [
        "Case.TXT",
        "other.txt",
        "literal[1].txt",
        "literal1.txt",
        "中文.txt",
        "Äpfel.txt",
    ] {
        std::fs::write(temp.path().join(name), name.as_bytes()).unwrap();
    }
    let mut app = start(temp.path(), 7);
    assert!(!app.screen().enabled("role:button[Clear filter]").unwrap());
    let path = app
        .screen()
        .input_text("Folder path")
        .unwrap()
        .unwrap()
        .to_owned();
    let requests = app.screen().file_requests().len();
    for (query, shown, name) in [
        ("TXT", 6, "Case.TXT"),
        ("literal[1]", 1, "literal[1].txt"),
        ("中文", 1, "中文.txt"),
        ("äP", 1, "Äpfel.txt"),
        (" TXT ", 0, ""),
        ("../", 0, ""),
    ] {
        filter(&mut app, query, shown, 7);
        if shown > 0 {
            assert!(
                app.screen()
                    .matches(&format!("role:checkbox[{name}]"))
                    .unwrap()
            );
        }
        assert_eq!(
            app.screen().enabled("role:button[Select all]").unwrap(),
            shown > 0
        );
        assert_eq!(
            app.screen()
                .enabled("role:button[Invert selection]")
                .unwrap(),
            shown > 0
        );
        assert_eq!(
            app.screen().input_text("Folder path").unwrap(),
            Some(path.as_str())
        );
    }
    filter(&mut app, "case", 1, 7);
    app.press_key("enter").unwrap();
    app.press_key("mod+a").unwrap();
    app.wait_for("1 selected").unwrap();
    assert!(checked(&app, "Case.TXT"));
    app.press_key("x").unwrap();
    assert_eq!(
        app.screen().input_text("Filter names").unwrap(),
        Some("case")
    );
    assert!(app.screen().shows("Names contain the text (case insensitive). Hidden selections remain selected for Open selected, Copy selected and Create ZIP."));
    app.press("role:button[Clear filter]").unwrap();
    app.wait_for("7 of 7 items shown").unwrap();
    assert_eq!(app.screen().input_text("Filter names").unwrap(), Some(""));
    assert!(checked(&app, "Case.TXT"));
    assert!(!app.screen().enabled("role:button[Clear filter]").unwrap());
    app.type_text("Filter names", &"界".repeat(300)).unwrap();
    assert_eq!(
        app.screen()
            .input_text("Filter names")
            .unwrap()
            .unwrap()
            .chars()
            .count(),
        256
    );
    assert_eq!(app.screen().file_requests().len(), requests);
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 7);
}

#[test]
fn visible_select_all_and_invert_keep_hidden_ids_and_sorting_creates_the_selected_real_files() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    for (name, data) in [
        ("shown-a.txt", "a"),
        ("shown-b.txt", "larger unselected"),
        ("hidden.bin", "hidden bytes"),
    ] {
        std::fs::write(source.join(name), data).unwrap();
    }
    let mut app = start(&source, 3);
    app.press("role:checkbox[hidden.bin]").unwrap();
    filter(&mut app, "SHOWN", 2, 3);
    app.wait_for("1 selected").unwrap();
    app.press("role:button[Select all]").unwrap();
    app.wait_for("3 selected").unwrap();
    app.press("role:button[Invert selection]").unwrap();
    app.wait_for("1 selected").unwrap();
    assert!(!checked(&app, "shown-a.txt"));
    assert!(!checked(&app, "shown-b.txt"));
    assert!(app.screen().enabled("role:button[Open selected]").unwrap());
    app.press("role:button[Invert selection]").unwrap();
    app.wait_for("3 selected").unwrap();
    app.press("role:checkbox[shown-b.txt]").unwrap();
    app.wait_for("2 selected").unwrap();
    for sort in [
        "Name",
        "Size (bytes)",
        "Size (bytes)",
        "Type",
        "Type",
        "Name",
    ] {
        app.press(&format!("role:button[{sort}]")).unwrap();
        assert!(checked(&app, "shown-a.txt"));
        assert!(!checked(&app, "shown-b.txt"));
        assert_eq!(
            app.screen().input_text("Filter names").unwrap(),
            Some("SHOWN")
        );
    }
    filter(&mut app, "no-match", 0, 3);
    app.wait_for("2 selected").unwrap();
    assert!(!app.screen().enabled("role:button[Select all]").unwrap());
    assert!(app.screen().enabled("role:button[Create ZIP…]").unwrap());
    let output = temp.path().join("selected.zip");
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press("role:button[Create ZIP…]").unwrap();
    app.wait_for("role:button[Continue]").unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("Created ZIP: 2 entries").unwrap();
    let actual: BTreeMap<_, _> =
        peazip_core::extract_zip(&std::fs::read(output).unwrap(), Limits::default())
            .unwrap()
            .into_iter()
            .map(|entry| (entry.path, entry.data))
            .collect();
    assert_eq!(
        actual,
        BTreeMap::from([
            ("hidden.bin".into(), b"hidden bytes".to_vec()),
            ("shown-a.txt".into(), b"a".to_vec()),
        ])
    );
    assert_eq!(
        std::fs::read(source.join("shown-b.txt")).unwrap(),
        b"larger unselected"
    );
    assert_eq!(app.screen().file_requests().len(), 2);
}

#[test]
fn same_path_refresh_keeps_filter_but_drops_old_snapshot_selection() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("keep-old.txt"), b"old").unwrap();
    std::fs::write(temp.path().join("other.txt"), b"other").unwrap();
    let mut app = start(temp.path(), 2);
    filter(&mut app, "KEEP", 1, 2);
    app.press("role:checkbox[keep-old.txt]").unwrap();
    std::fs::remove_file(temp.path().join("keep-old.txt")).unwrap();
    std::fs::write(temp.path().join("keep-new.txt"), b"new snapshot").unwrap();
    std::fs::write(temp.path().join("outside.bin"), b"hidden").unwrap();
    app.press("role:toolbar >> nth=0 >> role:button[Refresh folder]")
        .unwrap();
    app.wait_for("1 of 3 items shown").unwrap();
    app.wait_for("0 selected").unwrap();
    assert_eq!(
        app.screen().input_text("Filter names").unwrap(),
        Some("KEEP")
    );
    assert!(!app.screen().matches("role:checkbox[keep-old.txt]").unwrap());
    assert!(!checked(&app, "keep-new.txt"));
    assert!(!app.screen().enabled("role:button[Back]").unwrap());
    app.press("role:button[Name]").unwrap();
    app.press("role:checkbox[keep-new.txt]").unwrap();
    app.press("role:button[Clear filter]").unwrap();
    app.wait_for("3 of 3 items shown").unwrap();
    assert!(checked(&app, "keep-new.txt"));
    assert!(!checked(&app, "other.txt"));
    assert!(!checked(&app, "outside.bin"));
    assert_eq!(app.screen().file_requests().len(), 1);
}

#[test]
fn failed_navigation_keeps_filter_and_selection_while_successful_path_changes_clear_both() {
    let temp = tempfile::tempdir().unwrap();
    let child = temp.path().join("child");
    std::fs::create_dir(&child).unwrap();
    std::fs::write(child.join("inside.txt"), b"inside").unwrap();
    std::fs::write(temp.path().join("parent.txt"), b"parent").unwrap();
    let mut app = start(temp.path(), 2);
    app.press("role:checkbox[parent.txt]").unwrap();
    filter(&mut app, "parent", 1, 2);
    app.clear("Folder path").unwrap();
    app.type_text("Folder path", "missing-child").unwrap();
    app.press_key("enter").unwrap();
    app.wait_for("text:/^Could not browse item:/").unwrap();
    assert_eq!(
        app.screen().input_text("Filter names").unwrap(),
        Some("parent")
    );
    assert!(checked(&app, "parent.txt"));
    assert!(app.screen().shows("1 selected"));
    assert!(app.screen().shows("1 of 2 items shown"));
    app.clear("Folder path").unwrap();
    app.type_text("Folder path", "child").unwrap();
    app.press_key("enter").unwrap();
    app.wait_for("role:checkbox[inside.txt]").unwrap();
    assert_eq!(app.screen().input_text("Filter names").unwrap(), Some(""));
    assert!(app.screen().shows("0 selected"));
    filter(&mut app, "INSIDE", 1, 1);
    app.press("role:button[Up]").unwrap();
    app.wait_for("role:checkbox[parent.txt]").unwrap();
    assert_eq!(app.screen().input_text("Filter names").unwrap(), Some(""));
    assert!(app.screen().shows("2 of 2 items shown"));
    app.press("role:button[Back]").unwrap();
    app.wait_for("role:checkbox[inside.txt]").unwrap();
    assert_eq!(app.screen().input_text("Filter names").unwrap(), Some(""));
    assert_eq!(app.screen().file_requests().len(), 1);
}

#[cfg(unix)]
#[test]
fn escaped_control_and_unicode_names_open_by_snapshot_id_instead_of_filter_or_display_text() {
    let temp = tempfile::tempdir().unwrap();
    let raw = "line\n中文.tar";
    let bytes = include_bytes!("../../../tests/fixtures/sample.tar");
    std::fs::write(temp.path().join(raw), bytes).unwrap();
    std::fs::write(temp.path().join("line-n-中文.tar"), b"not selected").unwrap();
    let mut app = start(temp.path(), 2);
    filter(&mut app, "\\n中文", 1, 2);
    app.press("role:checkbox[line\\n中文.tar]").unwrap();
    app.press("role:button[Size (bytes)]").unwrap();
    assert!(checked(&app, "line\\n中文.tar"));
    app.press("role:button[Open selected]").unwrap();
    app.wait_for("5 entries").unwrap();
    assert!(app.screen().matches("Archive path").unwrap());
    assert_eq!(std::fs::read(temp.path().join(raw)).unwrap(), bytes);
    assert!(!temp.path().join("line\\n中文.tar").exists());
    assert_eq!(app.screen().file_requests().len(), 1);
}

#[cfg(target_os = "linux")]
#[test]
fn invalid_utf8_names_filter_the_escaped_label_but_open_the_original_raw_name() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let temp = tempfile::tempdir().unwrap();
    let raw = OsString::from_vec(b"raw-\xff.tar".to_vec());
    let other = OsString::from_vec(b"raw-\xfe.tar".to_vec());
    let bytes = include_bytes!("../../../tests/fixtures/sample.tar");
    std::fs::write(temp.path().join(&raw), bytes).unwrap();
    std::fs::write(temp.path().join(other), b"not selected").unwrap();
    let snapshot = peazip_native::DirectorySnapshot::open(temp.path(), Default::default()).unwrap();
    let label = &snapshot
        .entries()
        .iter()
        .find(|entry| entry.name == raw)
        .unwrap()
        .display_name;
    let mut app = start(temp.path(), 2);
    filter(&mut app, label, 1, 2);
    app.press(&format!("role:checkbox[{label}]")).unwrap();
    app.press("role:button[Name]").unwrap();
    assert!(checked(&app, label));
    app.press("role:button[Open selected]").unwrap();
    app.wait_for("5 entries").unwrap();
    assert_eq!(std::fs::read(temp.path().join(raw)).unwrap(), bytes);
    assert_eq!(app.screen().file_requests().len(), 1);
}
