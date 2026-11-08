#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use nsh_core::{
    Checked, Command, EventKind, EventPayload, MountItem, PayloadValue, Size, prelude::px,
};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

fn open_folder(path: &Path, count: usize) -> Headless {
    let mut config = EngineConfig::new(Root::of::<peazip_ui::App>());
    config.viewport = Size::new(px(1024.0), px(900.0));
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    app.on_pick_paths(vec![path.to_path_buf()]).unwrap();
    app.press("role:button[Open folder…]").unwrap();
    app.wait_for(&format!("{count} items")).unwrap();
    app
}

fn open_dialog(app: &mut Headless) {
    app.press("role:button[New folder…]").unwrap();
    app.wait_for("Folder name").unwrap();
}

fn name(app: &mut Headless, value: &str) {
    app.clear("Folder name").unwrap();
    if !value.is_empty() {
        app.type_text("Folder name", value).unwrap();
    }
}

fn closed(app: &mut Headless) {
    app.wait_for_gone("role:button[Create]", Duration::from_secs(5))
        .unwrap();
}

fn keep_selected(app: &Headless) {
    assert_eq!(
        app.screen()
            .accessibility_state("role:checkbox[keep.txt]")
            .unwrap()
            .checked,
        Some(Checked::True)
    );
    assert!(app.screen().shows("1 selected"));
}

#[test]
fn cancel_and_single_escape_from_focused_name_do_not_write_or_start_pickers() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("keep.txt"), b"original").unwrap();
    let mut app = open_folder(temp.path(), 1);
    app.press("role:checkbox[keep.txt]").unwrap();
    let selected_name = Arc::new(Mutex::new(None));
    let observed = Arc::clone(&selected_name);
    app.observe(move |_, batch| {
        for item in &batch.items {
            if let MountItem::Command {
                id,
                command: Command::SetSelection { start: 0, end: 10 },
                ..
            } = item
            {
                *observed.lock().unwrap() = Some(*id);
            }
        }
    });
    open_dialog(&mut app);
    assert_eq!(
        app.screen().input_text("Folder name").unwrap(),
        Some("New folder")
    );
    assert!(app.screen().enabled("role:button[Create]").unwrap());
    assert!(selected_name.lock().unwrap().is_some());
    // Headless ignores SetSelection; direct typing still verifies initial focus.
    app.press_key("x").unwrap();
    assert_eq!(
        app.screen().input_text("Folder name").unwrap(),
        Some("New folderx")
    );
    app.press("role:button[Cancel]").unwrap();
    closed(&mut app);
    keep_selected(&app);
    app.press_key("mod+shift+n").unwrap();
    app.wait_for("Folder name").unwrap();
    app.press_key("y").unwrap();
    assert_eq!(
        app.screen().input_text("Folder name").unwrap(),
        Some("New foldery")
    );
    // Headless drops Escape on focused inputs, so dispatch the platform event.
    app.engine().control().dispatch_event(
        selected_name.lock().unwrap().unwrap(),
        EventKind::KeyPress,
        EventPayload::Map(vec![("key", PayloadValue::Str("Escape".into()))]),
    );
    closed(&mut app);
    keep_selected(&app);
    assert_eq!(app.screen().file_requests().len(), 1);
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    assert_eq!(
        std::fs::read(temp.path().join("keep.txt")).unwrap(),
        b"original"
    );
}

#[test]
fn invalid_names_disable_create_and_modal_blocks_shared_shortcuts() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("keep.txt"), b"original").unwrap();
    let mut app = open_folder(temp.path(), 1);
    open_dialog(&mut app);
    for invalid in [
        "",
        ".",
        "..",
        "a/b",
        "a\\b",
        "CON",
        "trailing.",
        "space ",
        &"界".repeat(342),
    ] {
        name(&mut app, invalid);
        assert!(
            !app.screen().enabled("role:button[Create]").unwrap(),
            "{invalid:?}"
        );
        app.press_key("enter").unwrap();
        assert!(app.screen().matches("Folder name").unwrap());
    }
    for shortcut in ["mod+o", "mod+n", "mod+shift+n", "mod+w", "mod+i"] {
        app.press_key(shortcut).unwrap();
        assert!(app.screen().matches("Folder name").unwrap());
        assert!(!app.screen().matches("role:button[Continue]").unwrap());
        assert_eq!(app.screen().file_requests().len(), 1);
    }
    app.press("role:button[Cancel]").unwrap();
    closed(&mut app);
    assert!(app.screen().shows("0 selected"));
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[test]
fn unicode_enter_success_clears_filter_and_selection_without_adding_history() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("keep.txt"), b"original").unwrap();
    let mut app = open_folder(temp.path(), 1);
    app.press("role:checkbox[keep.txt]").unwrap();
    app.type_text("Filter names", "keep").unwrap();
    open_dialog(&mut app);
    name(&mut app, "新建目录");
    app.press_key("enter").unwrap();
    closed(&mut app);
    app.wait_for("role:checkbox[新建目录/]").unwrap();
    assert!(temp.path().join("新建目录").is_dir());
    assert_eq!(app.screen().input_text("Filter names").unwrap(), Some(""));
    assert!(app.screen().shows("0 selected"));
    assert!(!app.screen().enabled("role:button[Back]").unwrap());
    assert_eq!(app.screen().file_requests().len(), 1);
    assert_eq!(
        std::fs::read(temp.path().join("keep.txt")).unwrap(),
        b"original"
    );
}

#[test]
fn collision_keeps_dialog_and_selection_and_allows_corrected_retry() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("keep.txt"), b"original").unwrap();
    std::fs::create_dir(temp.path().join("taken")).unwrap();
    std::fs::write(temp.path().join("taken/sentinel"), b"untouched").unwrap();
    let mut app = open_folder(temp.path(), 2);
    app.press("role:checkbox[keep.txt]").unwrap();
    open_dialog(&mut app);
    name(&mut app, "taken");
    app.press("role:button[Create]").unwrap();
    app.wait_for("An item with this name already exists. Choose another name.")
        .unwrap();
    assert!(app.screen().matches("Folder name").unwrap());
    assert!(app.screen().enabled("role:button[Create]").unwrap());
    keep_selected(&app);
    name(&mut app, "retry");
    app.press("role:button[Create]").unwrap();
    closed(&mut app);
    app.wait_for("role:checkbox[retry/]").unwrap();
    assert!(temp.path().join("retry").is_dir());
    assert_eq!(
        std::fs::read(temp.path().join("taken/sentinel")).unwrap(),
        b"untouched"
    );
    assert_eq!(
        std::fs::read(temp.path().join("keep.txt")).unwrap(),
        b"original"
    );
    assert_eq!(app.screen().file_requests().len(), 1);
}

#[cfg(unix)]
#[test]
fn creation_uses_frozen_parent_capability_after_ambient_path_is_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("parent");
    let moved = temp.path().join("moved");
    std::fs::create_dir(&parent).unwrap();
    let mut app = open_folder(&parent, 0);
    open_dialog(&mut app);
    std::fs::rename(&parent, &moved).unwrap();
    std::fs::create_dir(&parent).unwrap();
    std::fs::write(parent.join("sentinel"), b"replacement").unwrap();
    name(&mut app, "pinned");
    app.press("role:button[Create]").unwrap();
    app.wait_for("text:/^Created folder: pinned/").unwrap();
    closed(&mut app);
    assert!(moved.join("pinned").is_dir());
    assert!(!parent.join("pinned").exists());
    assert_eq!(
        std::fs::read(parent.join("sentinel")).unwrap(),
        b"replacement"
    );
    assert_eq!(app.screen().file_requests().len(), 1);
}

#[test]
fn mkdir_success_refresh_limit_failure_closes_dialog_and_preserves_old_view_and_selection() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("keep.txt"), b"original").unwrap();
    let mut app = open_folder(temp.path(), 1);
    app.press("role:checkbox[keep.txt]").unwrap();
    open_dialog(&mut app);
    for index in 1..peazip_native::DirectoryLimits::default().max_entries {
        std::fs::File::create(temp.path().join(format!("external-{index:05}"))).unwrap();
    }
    name(&mut app, "created-once");
    app.press("role:button[Create]").unwrap();
    app.wait_for("text:/^Created folder: created-once\\. Could not refresh the folder:/")
        .unwrap();
    closed(&mut app);
    assert!(temp.path().join("created-once").is_dir());
    keep_selected(&app);
    assert!(app.screen().shows("1 of 1 items shown"));
    assert_eq!(app.screen().count("role:checkbox").unwrap(), 1);
    assert!(
        !app.screen()
            .matches("role:checkbox[created-once/]")
            .unwrap()
    );
    assert!(
        !app.screen()
            .matches("text:/^Could not create folder:/")
            .unwrap()
    );
    assert_eq!(app.screen().file_requests().len(), 1);
    assert_eq!(
        std::fs::read(temp.path().join("keep.txt")).unwrap(),
        b"original"
    );
}
