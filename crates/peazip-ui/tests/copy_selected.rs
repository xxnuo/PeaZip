#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use nsh_core::{Checked, Command, MountItem, Size, file_dialog::FileRequest, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

const COPY: &str = "role:button[Copy selected…]";

fn start() -> Headless {
    let mut config = EngineConfig::new(Root::of::<peazip_ui::App>());
    config.viewport = Size::new(px(1024.0), px(900.0));
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    app
}

fn open(app: &mut Headless, path: &Path, count: usize) {
    app.on_pick_paths(vec![path.to_path_buf()]).unwrap();
    app.press("role:button[Open folder…]").unwrap();
    app.wait_for(&format!("{count} items")).unwrap();
}

fn checked(app: &Headless, name: &str) -> bool {
    app.screen()
        .accessibility_state(&format!("role:checkbox[{name}]"))
        .unwrap()
        .checked
        == Some(Checked::True)
}

fn same_folder(app: &Headless, source: &Path) {
    assert_eq!(
        app.screen().input_text("Folder path").unwrap(),
        std::fs::canonicalize(source).unwrap().to_str()
    );
    assert!(!app.screen().enabled("role:button[Back]").unwrap());
    assert!(!app.screen().enabled("role:button[Forward]").unwrap());
}

fn save_only(app: &Headless, before: usize) {
    let requests = &app.screen().file_requests()[before..];
    assert_eq!(
        requests.len(),
        1,
        "copy must not request source files again"
    );
    let FileRequest::SaveDialog(dialog) = &requests[0] else {
        panic!("copy must request a destination name")
    };
    assert_eq!(
        dialog.title.as_deref(),
        Some("Copy selected — choose a new folder name")
    );
    assert_eq!(dialog.button_label.as_deref(), Some("Copy"));
    assert_eq!(dialog.file_name.as_deref(), Some("Copy"));
}

#[test]
fn copies_hidden_stable_ids_and_nested_unicode_binary_empty_directories_without_changing_view() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir_all(source.join("docs/empty")).unwrap();
    std::fs::create_dir(source.join("docs/nested")).unwrap();
    std::fs::write(source.join("docs/nested/data.bin"), [0, 127, 255]).unwrap();
    std::fs::write(source.join("docs/你好.txt"), "原始内容\n").unwrap();
    std::fs::write(source.join("loose.txt"), b"hidden selected").unwrap();
    std::fs::write(source.join("skip.txt"), b"not selected").unwrap();
    let output = temp.path().join("Copy");
    let mut app = start();
    open(&mut app, &source, 3);
    assert!(!app.screen().enabled(COPY).unwrap());
    for name in ["docs/", "loose.txt"] {
        app.press(&format!("role:checkbox[{name}]")).unwrap();
    }
    for sort in ["Name", "Size (bytes)", "Size (bytes)"] {
        app.press(&format!("role:button[{sort}]")).unwrap();
    }
    assert!(checked(&app, "docs/"));
    assert!(checked(&app, "loose.txt"));
    app.type_text("Filter names", "docs").unwrap();
    assert!(app.screen().shows("1 of 3 items shown"));
    assert!(app.screen().shows("2 selected"));
    app.press_key("mod+c").unwrap();
    assert_eq!(
        app.screen().file_requests().len(),
        1,
        "copy contents must not bind the clipboard shortcut"
    );
    app.press("role:button[New folder…]").unwrap();
    app.wait_for("Folder name").unwrap();
    assert!(
        !app.screen().enabled(COPY).unwrap(),
        "another modal must gate copy"
    );
    app.press("role:button[Cancel]").unwrap();
    app.wait_for_gone("role:button[Create]", Duration::from_secs(5))
        .unwrap();
    let target = app.screen().press_target(COPY).unwrap().unwrap();
    let disabled = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&disabled);
    app.observe(move |_, batch| {
        for item in &batch.items {
            if let MountItem::ViewProps { id, props } = item
                && *id == target
                && props.accessibility.state.disabled
            {
                observed.store(true, Ordering::SeqCst);
            }
        }
    });
    let requests = app.screen().file_requests().len();
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press(COPY).unwrap();
    app.wait_for(&format!("Copied 6 entries to {}", output.display()))
        .unwrap();
    assert!(
        disabled.load(Ordering::SeqCst),
        "copy must be disabled during its own operation"
    );
    assert!(app.screen().enabled(COPY).unwrap());
    same_folder(&app, &source);
    assert_eq!(
        app.screen().input_text("Filter names").unwrap(),
        Some("docs")
    );
    assert!(app.screen().shows("2 selected"));
    assert!(checked(&app, "docs/"));
    assert!(output.join("docs/empty").is_dir());
    assert_eq!(
        std::fs::read_dir(output.join("docs/empty"))
            .unwrap()
            .count(),
        0
    );
    for (relative, bytes) in [
        ("docs/nested/data.bin", [0, 127, 255].as_slice()),
        ("docs/你好.txt", "原始内容\n".as_bytes()),
        ("loose.txt", b"hidden selected".as_slice()),
    ] {
        assert_eq!(std::fs::read(output.join(relative)).unwrap(), bytes);
        assert_eq!(std::fs::read(source.join(relative)).unwrap(), bytes);
    }
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 2);
    assert!(!output.join("skip.txt").exists());
    assert_eq!(
        std::fs::read(source.join("skip.txt")).unwrap(),
        b"not selected"
    );
    save_only(&app, requests);
    app.press("role:button[Clear filter]").unwrap();
    assert!(checked(&app, "loose.txt"));
    assert!(!checked(&app, "skip.txt"));
}

#[test]
fn cancelling_destination_preserves_selection_filter_source_and_history() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("keep.txt"), b"original").unwrap();
    let mut app = start();
    open(&mut app, temp.path(), 1);
    app.press("role:checkbox[keep.txt]").unwrap();
    app.type_text("Filter names", "no-match").unwrap();
    assert!(
        app.screen().enabled(COPY).unwrap(),
        "hidden selections remain copyable"
    );
    let requests = app.screen().file_requests().len();
    app.on_pick_save_path(None).unwrap();
    app.press(COPY).unwrap();
    app.wait_for("Copy cancelled.").unwrap();
    same_folder(&app, temp.path());
    assert_eq!(
        app.screen().input_text("Filter names").unwrap(),
        Some("no-match")
    );
    assert!(app.screen().shows("1 selected"));
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    assert_eq!(
        std::fs::read(temp.path().join("keep.txt")).unwrap(),
        b"original"
    );
    save_only(&app, requests);
    app.press("role:button[Clear filter]").unwrap();
    assert!(checked(&app, "keep.txt"));
}

#[test]
fn existing_destination_is_not_replaced_and_failure_preserves_selection() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let output = temp.path().join("existing");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&output).unwrap();
    std::fs::write(source.join("keep.txt"), b"source").unwrap();
    std::fs::write(output.join("keep.txt"), b"destination sentinel").unwrap();
    let mut app = start();
    open(&mut app, &source, 1);
    app.press("role:checkbox[keep.txt]").unwrap();
    app.type_text("Filter names", "keep").unwrap();
    let requests = app.screen().file_requests().len();
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press(COPY).unwrap();
    app.wait_for("text:/^Could not copy selected items:/")
        .unwrap();
    same_folder(&app, &source);
    assert!(checked(&app, "keep.txt"));
    assert_eq!(
        app.screen().input_text("Filter names").unwrap(),
        Some("keep")
    );
    assert_eq!(std::fs::read(source.join("keep.txt")).unwrap(), b"source");
    assert_eq!(
        std::fs::read(output.join("keep.txt")).unwrap(),
        b"destination sentinel"
    );
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 1);
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 2);
    save_only(&app, requests);
}

#[test]
fn vanished_selected_file_is_rejected_without_publishing_output() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("gone.txt"), b"original").unwrap();
    let output = temp.path().join("rejected");
    let mut app = start();
    open(&mut app, &source, 1);
    app.press("role:checkbox[gone.txt]").unwrap();
    std::fs::remove_file(source.join("gone.txt")).unwrap();
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press(COPY).unwrap();
    app.wait_for("text:/^Could not copy selected items:/")
        .unwrap();
    assert!(!output.exists());
    same_folder(&app, &source);
    assert!(checked(&app, "gone.txt"));
    assert!(app.screen().shows("1 selected"));
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    save_only(&app, 1);
}

#[cfg(unix)]
#[test]
fn direct_nested_and_replacement_links_are_rejected_without_following_or_publishing() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(source.join("nested")).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("secret"), b"outside sentinel").unwrap();
    std::fs::write(source.join("replace.txt"), b"original").unwrap();
    symlink(outside.join("secret"), source.join("link")).unwrap();
    symlink(outside.join("secret"), source.join("nested/escape")).unwrap();
    let mut app = start();
    open(&mut app, &source, 3);
    std::fs::rename(source.join("replace.txt"), source.join("moved.txt")).unwrap();
    symlink(outside.join("secret"), source.join("replace.txt")).unwrap();
    for (index, selected) in ["link", "nested/", "replace.txt"].into_iter().enumerate() {
        if index > 0 {
            app.press("role:button[Clear selection]").unwrap();
        }
        app.press(&format!("role:checkbox[{selected}]")).unwrap();
        let output = temp.path().join(format!("rejected-{index}"));
        let requests = app.screen().file_requests().len();
        app.on_pick_save_path(Some(output.clone())).unwrap();
        app.press(COPY).unwrap();
        app.wait_for("text:/^Could not copy selected items:/")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !app.screen().enabled(COPY).unwrap() {
            assert!(Instant::now() < deadline, "copy did not finish");
            app.next_change(Duration::from_millis(25)).unwrap();
        }
        assert!(!output.exists());
        assert!(checked(&app, selected));
        same_folder(&app, &source);
        save_only(&app, requests);
    }
    assert_eq!(
        std::fs::read(outside.join("secret")).unwrap(),
        b"outside sentinel"
    );
    assert_eq!(
        std::fs::read(source.join("moved.txt")).unwrap(),
        b"original"
    );
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
fn source_capability_is_frozen_before_destination_picker_replaces_ambient_root() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let moved = temp.path().join("moved");
    let output = temp.path().join("Copy");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("keep.txt"), b"original capability").unwrap();
    let mut app = start();
    open(&mut app, &source, 1);
    app.press("role:checkbox[keep.txt]").unwrap();
    let old_path = app
        .screen()
        .input_text("Folder path")
        .unwrap()
        .unwrap()
        .to_owned();
    let replaced = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&replaced);
    let original = source.clone();
    let held = moved.clone();
    app.observe(move |_, batch| {
        for item in &batch.items {
            if let MountItem::Command {
                command: Command::FileRequest { request, .. },
                ..
            } = item
                && matches!(&**request, FileRequest::SaveDialog(_))
                && !observed.swap(true, Ordering::SeqCst)
            {
                std::fs::rename(&original, &held).unwrap();
                std::fs::create_dir(&original).unwrap();
                std::fs::write(original.join("keep.txt"), b"replacement sentinel").unwrap();
            }
        }
    });
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press(COPY).unwrap();
    app.wait_for("text:/^Copied 1 entries to /").unwrap();
    assert!(replaced.load(Ordering::SeqCst));
    assert_eq!(
        std::fs::read(output.join("keep.txt")).unwrap(),
        b"original capability"
    );
    assert_eq!(
        std::fs::read(moved.join("keep.txt")).unwrap(),
        b"original capability"
    );
    assert_eq!(
        std::fs::read(source.join("keep.txt")).unwrap(),
        b"replacement sentinel"
    );
    assert_eq!(
        app.screen().input_text("Folder path").unwrap(),
        Some(old_path.as_str())
    );
    assert!(checked(&app, "keep.txt"));
    assert!(!app.screen().enabled("role:button[Back]").unwrap());
    save_only(&app, 1);
}
