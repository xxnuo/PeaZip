#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use std::{collections::BTreeMap, path::Path};

use nsh_core::{
    Checked, Size,
    file_dialog::{FileRequest, OpenDialogKind},
    prelude::px,
};
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

fn open_folder(app: &mut Headless, path: &Path, count: usize) {
    app.on_pick_paths(vec![path.to_path_buf()]).unwrap();
    app.press("role:button[Open folder…]").unwrap();
    app.wait_for(&format!("{count} items")).unwrap();
    assert_folder(app, path);
    match app.screen().file_requests().last().unwrap() {
        FileRequest::OpenDialog(dialog) => assert_eq!(dialog.kind, OpenDialogKind::Folders),
        other => panic!("unexpected folder request: {other:?}"),
    }
}

fn assert_folder(app: &Headless, path: &Path) {
    let canonical = std::fs::canonicalize(path).unwrap();
    assert_eq!(
        app.screen().input_text("Folder path").unwrap(),
        canonical.to_str()
    );
    assert!(!app.screen().matches("Archive path").unwrap());
}

#[test]
fn relative_parent_go_then_up_uses_the_physical_directory_and_history() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("parent");
    let child = parent.join("child");
    std::fs::create_dir_all(&child).unwrap();
    std::fs::write(parent.join("at-parent.txt"), b"parent").unwrap();
    std::fs::write(child.join("at-child.txt"), b"child").unwrap();
    let mut app = start();
    open_folder(&mut app, &child, 1);
    app.clear("Folder path").unwrap();
    app.type_text("Folder path", "..").unwrap();
    app.press_key("enter").unwrap();
    app.wait_for("role:checkbox[at-parent.txt]").unwrap();
    assert_folder(&app, &parent);
    app.press("role:button[Up]").unwrap();
    app.wait_for("role:checkbox[parent/]").unwrap();
    assert_folder(&app, temp.path());
    app.press("role:button[Back]").unwrap();
    app.wait_for("role:checkbox[at-parent.txt]").unwrap();
    assert_folder(&app, &parent);
    app.press("role:button[Back]").unwrap();
    app.wait_for("role:checkbox[at-child.txt]").unwrap();
    assert_folder(&app, &child);
    app.press("role:button[Forward]").unwrap();
    app.wait_for("role:checkbox[at-parent.txt]").unwrap();
    assert_folder(&app, &parent);
    assert_eq!(app.screen().file_requests().len(), 1);
}

fn checked(app: &Headless, name: &str) -> bool {
    app.screen()
        .accessibility_state(&format!("role:checkbox[{name}]"))
        .unwrap()
        .checked
        == Some(Checked::True)
}

fn write_tar(path: &Path) {
    std::fs::write(path, include_bytes!("../../../tests/fixtures/sample.tar")).unwrap();
}

#[test]
fn opens_real_folders_navigates_relative_paths_and_keeps_selection_ids_during_sort() {
    let temp = tempfile::tempdir().unwrap();
    let child = temp.path().join("child");
    std::fs::create_dir(&child).unwrap();
    std::fs::write(child.join("nested.txt"), b"nested").unwrap();
    std::fs::write(temp.path().join("a.txt"), b"small").unwrap();
    std::fs::write(temp.path().join("z.txt"), b"larger payload").unwrap();
    let mut app = start();
    open_folder(&mut app, temp.path(), 3);
    assert_eq!(app.screen().count("role:checkbox").unwrap(), 3);
    assert!(!app.screen().enabled("role:button[Open selected]").unwrap());
    app.press("role:checkbox[z.txt]").unwrap();
    app.wait_for("1 selected").unwrap();
    assert!(app.screen().enabled("role:button[Open selected]").unwrap());
    for sort in ["Name", "Size (bytes)", "Size (bytes)", "Name"] {
        app.press(&format!("role:button[{sort}]")).unwrap();
        assert!(checked(&app, "z.txt"));
        assert!(!checked(&app, "a.txt"));
        assert!(!checked(&app, "child/"));
    }
    app.press("role:checkbox[a.txt]").unwrap();
    app.wait_for("2 selected").unwrap();
    assert!(!app.screen().enabled("role:button[Open selected]").unwrap());
    app.press("role:button[Clear selection]").unwrap();
    app.press("role:button[Select all]").unwrap();
    app.wait_for("3 selected").unwrap();
    for name in ["a.txt", "z.txt", "child/"] {
        assert!(checked(&app, name));
    }
    assert!(!app.screen().enabled("role:button[Open selected]").unwrap());
    app.press("role:button[Clear selection]").unwrap();
    app.wait_for("0 selected").unwrap();
    let requests = app.screen().file_requests().len();
    app.press("role:button[Open folder child]").unwrap();
    app.wait_for("role:checkbox[nested.txt]").unwrap();
    assert_folder(&app, &child);
    assert!(app.screen().shows("0 selected"));
    assert!(!checked(&app, "nested.txt"));
    app.press("role:button[Up]").unwrap();
    app.wait_for("role:checkbox[child/]").unwrap();
    assert_folder(&app, temp.path());
    app.clear("Folder path").unwrap();
    app.type_text("Folder path", "child").unwrap();
    app.press_key("enter").unwrap();
    app.wait_for("role:checkbox[nested.txt]").unwrap();
    assert_folder(&app, &child);
    app.press("role:button[Back]").unwrap();
    app.wait_for("role:checkbox[a.txt]").unwrap();
    assert_folder(&app, temp.path());
    app.press("role:button[Forward]").unwrap();
    app.wait_for("role:checkbox[nested.txt]").unwrap();
    assert_folder(&app, &child);
    assert_eq!(app.screen().file_requests().len(), requests);
}

#[test]
fn creating_zip_from_filesystem_selection_requests_only_save_and_uses_real_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), b"first contents").unwrap();
    std::fs::write(source.join("中文.txt"), "中文 contents".as_bytes()).unwrap();
    std::fs::write(source.join("skip.txt"), b"not selected").unwrap();
    let output = temp.path().join("selected.zip");
    let mut app = start();
    open_folder(&mut app, &source, 3);
    app.press("role:checkbox[a.txt]").unwrap();
    app.press("role:checkbox[中文.txt]").unwrap();
    app.wait_for("2 selected").unwrap();
    let requests = app.screen().file_requests().len();
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press("role:button[Create archive…]").unwrap();
    app.wait_for("role:button[Continue]").unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("Created ZIP: 2 entries").unwrap();
    assert!(app.screen().matches("Archive path").unwrap());
    assert!(!app.screen().matches("Folder path").unwrap());
    assert!(app.screen().shows("0 selected"));
    let requested = &app.screen().file_requests()[requests..];
    assert_eq!(
        requested.len(),
        1,
        "selected sources must not open a second file picker"
    );
    assert!(matches!(requested[0], FileRequest::SaveDialog(_)));
    let actual: BTreeMap<_, _> =
        peazip_core::extract_zip(&std::fs::read(&output).unwrap(), Limits::default())
            .unwrap()
            .into_iter()
            .map(|entry| (entry.path, entry.data))
            .collect();
    assert_eq!(
        actual,
        BTreeMap::from([
            ("a.txt".into(), b"first contents".to_vec()),
            ("中文.txt".into(), "中文 contents".as_bytes().to_vec()),
        ])
    );
    assert_eq!(
        std::fs::read(source.join("a.txt")).unwrap(),
        b"first contents"
    );
    assert_eq!(
        std::fs::read(source.join("skip.txt")).unwrap(),
        b"not selected"
    );
    app.press_key("mod+w").unwrap();
    app.wait_for("2 items").unwrap();
    assert_folder(&app, temp.path());
    assert_eq!(app.screen().file_requests().len(), requests + 1);
}

#[test]
fn opens_selected_archive_and_unifies_folder_and_archive_history_without_more_pickers() {
    let temp = tempfile::tempdir().unwrap();
    let child = temp.path().join("child");
    std::fs::create_dir(&child).unwrap();
    write_tar(&child.join("sample.tar"));
    let mut app = start();
    open_folder(&mut app, temp.path(), 1);
    app.press("role:button[Open folder child]").unwrap();
    app.wait_for("role:checkbox[sample.tar]").unwrap();
    app.press("role:checkbox[sample.tar]").unwrap();
    app.press("role:button[Open selected]").unwrap();
    app.wait_for("5 entries").unwrap();
    assert_eq!(app.screen().input_text("Archive path").unwrap(), Some("/"));
    assert_eq!(app.screen().count("role:checkbox").unwrap(), 2);
    assert!(!app.screen().matches("Folder path").unwrap());
    app.press("role:button[Select all]").unwrap();
    app.wait_for("2 selected").unwrap();
    assert!(checked(&app, "docs/"));
    assert!(checked(&app, "other.txt"));
    app.press("role:button[Clear selection]").unwrap();
    app.wait_for("0 selected").unwrap();
    app.press("role:button[Open folder docs/]").unwrap();
    app.wait_for("role:checkbox[docs/nested/]").unwrap();
    app.press("role:button[Open folder docs/nested/]").unwrap();
    app.wait_for("role:checkbox[docs/nested/data.bin]").unwrap();
    app.press("role:button[Back]").unwrap();
    app.wait_for("role:checkbox[docs/你好.txt]").unwrap();
    assert_eq!(
        app.screen().input_text("Archive path").unwrap(),
        Some("docs/")
    );
    app.press("role:button[Back]").unwrap();
    app.wait_for("role:checkbox[other.txt]").unwrap();
    assert_eq!(app.screen().input_text("Archive path").unwrap(), Some("/"));
    app.press("role:button[Back]").unwrap();
    app.wait_for("role:checkbox[sample.tar]").unwrap();
    assert_folder(&app, &child);
    app.press("role:button[Back]").unwrap();
    app.wait_for("role:checkbox[child/]").unwrap();
    assert_folder(&app, temp.path());
    assert!(!app.screen().enabled("role:button[Back]").unwrap());
    app.press("role:button[Forward]").unwrap();
    app.wait_for("role:checkbox[sample.tar]").unwrap();
    app.press("role:button[Forward]").unwrap();
    app.wait_for("role:checkbox[other.txt]").unwrap();
    app.press("role:button[Forward]").unwrap();
    app.wait_for("role:checkbox[docs/你好.txt]").unwrap();
    app.press("role:button[Forward]").unwrap();
    app.wait_for("role:checkbox[docs/nested/data.bin]").unwrap();
    assert!(!app.screen().enabled("role:button[Forward]").unwrap());
    app.press("role:button[Archive root]").unwrap();
    app.wait_for("role:checkbox[other.txt]").unwrap();
    assert!(app.screen().matches("Archive path").unwrap());
    app.press("role:button[Up]").unwrap();
    app.wait_for("role:checkbox[sample.tar]").unwrap();
    assert_folder(&app, &child);
    app.press("role:button[Open file sample.tar]").unwrap();
    app.wait_for("5 entries").unwrap();
    app.press_key("mod+w").unwrap();
    app.wait_for("role:checkbox[sample.tar]").unwrap();
    assert_folder(&app, &child);
    std::fs::write(child.join("new.txt"), b"added after closing archive").unwrap();
    app.press_key("f5").unwrap();
    app.wait_for("role:checkbox[new.txt]").unwrap();
    app.press_key("mod+a").unwrap();
    app.wait_for("2 selected").unwrap();
    assert_eq!(app.screen().file_requests().len(), 1);
    assert_eq!(
        std::fs::read(child.join("sample.tar")).unwrap(),
        include_bytes!("../../../tests/fixtures/sample.tar")
    );
}

#[test]
fn refresh_observes_new_files_and_drops_stale_selection_ids() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("keep.txt"), b"keep").unwrap();
    std::fs::write(temp.path().join("old.txt"), b"old").unwrap();
    let mut app = start();
    open_folder(&mut app, temp.path(), 2);
    app.press("role:checkbox[old.txt]").unwrap();
    app.wait_for("1 selected").unwrap();
    std::fs::remove_file(temp.path().join("old.txt")).unwrap();
    std::fs::write(temp.path().join("new.txt"), b"new").unwrap();
    app.press("role:toolbar >> nth=0 >> role:button[Refresh folder]")
        .unwrap();
    app.wait_for("role:checkbox[new.txt]").unwrap();
    assert!(!app.screen().matches("role:checkbox[old.txt]").unwrap());
    assert!(app.screen().shows("0 selected"));
    assert!(!checked(&app, "keep.txt"));
    assert!(!checked(&app, "new.txt"));
    assert!(!app.screen().enabled("role:button[Open selected]").unwrap());
    assert_folder(&app, temp.path());
    app.press("role:checkbox[keep.txt]").unwrap();
    std::fs::rename(
        temp.path().join("keep.txt"),
        temp.path().join("renamed.txt"),
    )
    .unwrap();
    app.press("role:toolbar >> nth=0 >> role:button[Refresh folder]")
        .unwrap();
    app.wait_for("role:checkbox[renamed.txt]").unwrap();
    assert!(!app.screen().matches("role:checkbox[keep.txt]").unwrap());
    assert!(app.screen().shows("0 selected"));
    assert!(
        !app.screen().enabled("role:button[Back]").unwrap(),
        "refresh must not add history entries"
    );
    assert_eq!(app.screen().file_requests().len(), 1);
}

#[cfg(unix)]
#[test]
fn filesystem_root_is_distinct_from_archive_root_and_back_returns_to_the_folder() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("marker.txt"), b"untouched").unwrap();
    let root = temp.path().ancestors().last().unwrap();
    let mut app = start();
    open_folder(&mut app, temp.path(), 1);
    assert!(app.screen().enabled("role:button[Root]").unwrap());
    assert!(!app.screen().matches("role:button[Archive root]").unwrap());
    app.press("role:button[Root]").unwrap();
    app.wait_for("text:/^\\/$/").unwrap();
    assert_folder(&app, root);
    assert!(!app.screen().enabled("role:button[Root]").unwrap());
    app.press("role:button[Back]").unwrap();
    app.wait_for("role:checkbox[marker.txt]").unwrap();
    assert_folder(&app, temp.path());
    assert_eq!(app.screen().file_requests().len(), 1);
    assert_eq!(
        std::fs::read(temp.path().join("marker.txt")).unwrap(),
        b"untouched"
    );
}

#[test]
fn failed_file_and_folder_navigation_preserves_the_current_folder_and_selection() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("keep.txt"), b"original").unwrap();
    std::fs::write(temp.path().join("bad.zip"), b"not an archive").unwrap();
    let child = temp.path().join("child");
    std::fs::create_dir(&child).unwrap();
    let mut app = start();
    open_folder(&mut app, temp.path(), 3);
    app.press("role:checkbox[keep.txt]").unwrap();
    app.press("role:button[Open file bad.zip]").unwrap();
    app.wait_for("text:/^Could not browse item:/").unwrap();
    assert_folder(&app, temp.path());
    assert!(checked(&app, "keep.txt"));
    assert!(app.screen().shows("1 selected"));
    app.clear("Folder path").unwrap();
    app.type_text("Folder path", "missing-child").unwrap();
    app.press_key("enter").unwrap();
    app.wait_for("text:/^Could not browse item:.*no longer exists/")
        .unwrap();
    app.press_key("x").unwrap();
    assert_eq!(
        app.screen().input_text("Folder path").unwrap(),
        Some("missing-childx"),
        "a failed Enter navigation must preserve editing focus without refocusing the input"
    );
    assert_eq!(app.screen().count("role:checkbox").unwrap(), 3);
    assert!(checked(&app, "keep.txt"));
    assert!(app.screen().shows("1 selected"));
    assert!(!app.screen().enabled("role:button[Back]").unwrap());
    app.press("role:button[Open folder child]").unwrap();
    app.wait_for("0 items").unwrap();
    assert_folder(&app, &child);
    app.press("role:button[Back]").unwrap();
    app.wait_for("role:checkbox[keep.txt]").unwrap();
    assert_folder(&app, temp.path());
    assert!(!app.screen().enabled("role:button[Back]").unwrap());
    assert_eq!(app.screen().file_requests().len(), 1);
    assert_eq!(
        std::fs::read(temp.path().join("keep.txt")).unwrap(),
        b"original"
    );
    assert_eq!(
        std::fs::read(temp.path().join("bad.zip")).unwrap(),
        b"not an archive"
    );
}

#[test]
fn cancelling_folder_and_save_dialogs_preserves_the_filesystem_selection() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("keep.txt"), b"untouched").unwrap();
    let mut app = start();
    open_folder(&mut app, temp.path(), 1);
    app.press("role:checkbox[keep.txt]").unwrap();
    app.on_pick_paths(vec![]).unwrap();
    app.press("role:button[Open folder…]").unwrap();
    app.wait_for("1 items").unwrap();
    assert_folder(&app, temp.path());
    assert!(checked(&app, "keep.txt"));
    app.on_pick_save_path(None).unwrap();
    app.press("role:button[Create archive…]").unwrap();
    app.wait_for("role:button[Continue]").unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("1 items").unwrap();
    assert_folder(&app, temp.path());
    assert!(checked(&app, "keep.txt"));
    assert!(app.screen().shows("1 selected"));
    assert_eq!(app.screen().file_requests().len(), 3);
    assert!(matches!(
        app.screen().file_requests()[2],
        FileRequest::SaveDialog(_)
    ));
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    assert_eq!(
        std::fs::read(temp.path().join("keep.txt")).unwrap(),
        b"untouched"
    );
}

#[cfg(unix)]
#[test]
fn linked_entries_and_links_inside_selected_folders_never_follow_or_publish_output() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let outside = temp.path().join("outside");
    let ordinary = source.join("ordinary");
    std::fs::create_dir_all(&ordinary).unwrap();
    std::fs::create_dir(&outside).unwrap();
    write_tar(&outside.join("secret.tar"));
    std::fs::write(outside.join("secret.txt"), b"outside sentinel").unwrap();
    symlink(outside.join("secret.tar"), source.join("link.tar")).unwrap();
    symlink(&outside, source.join("linked-folder")).unwrap();
    symlink(outside.join("secret.txt"), ordinary.join("escape")).unwrap();
    let mut app = start();
    open_folder(&mut app, &source, 3);
    app.press("role:checkbox[link.tar]").unwrap();
    for name in ["link.tar", "linked-folder"] {
        app.press(&format!("role:button[Open file {name}]"))
            .unwrap();
        app.wait_for("text:/^Could not browse item:/").unwrap();
        assert_folder(&app, &source);
        assert!(checked(&app, "link.tar"));
        assert!(!app.screen().matches("Archive path").unwrap());
    }
    let output = temp.path().join("rejected.zip");
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press("role:button[Create archive…]").unwrap();
    app.wait_for("role:button[Continue]").unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("text:/^Could not create ZIP:/").unwrap();
    assert!(!output.exists());
    assert_folder(&app, &source);
    assert!(checked(&app, "link.tar"));
    app.press("role:button[Clear selection]").unwrap();
    app.press("role:checkbox[ordinary/]").unwrap();
    let nested = temp.path().join("nested-rejected.zip");
    app.on_pick_save_path(Some(nested.clone())).unwrap();
    app.press("role:button[Create archive…]").unwrap();
    app.wait_for("role:button[Continue]").unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("text:/^Could not create ZIP:/").unwrap();
    assert!(!nested.exists());
    assert!(checked(&app, "ordinary/"));
    assert_eq!(
        std::fs::read(outside.join("secret.txt")).unwrap(),
        b"outside sentinel"
    );
    assert_eq!(
        std::fs::read(outside.join("secret.tar")).unwrap(),
        include_bytes!("../../../tests/fixtures/sample.tar")
    );
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 2);
    assert_eq!(app.screen().file_requests().len(), 3);
}

#[cfg(unix)]
#[test]
fn replacing_listed_entries_with_links_preserves_the_snapshot_and_rejects_output() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let child = source.join("child");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&child).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(child.join("inside.txt"), b"original child").unwrap();
    write_tar(&source.join("sample.tar"));
    write_tar(&outside.join("secret.tar"));
    std::fs::write(outside.join("sentinel.txt"), b"outside remains unchanged").unwrap();
    let mut app = start();
    open_folder(&mut app, &source, 2);
    app.press("role:checkbox[sample.tar]").unwrap();
    std::fs::remove_file(source.join("sample.tar")).unwrap();
    symlink(outside.join("secret.tar"), source.join("sample.tar")).unwrap();
    app.press("role:button[Open selected]").unwrap();
    app.wait_for("text:/^Could not browse item:/").unwrap();
    assert_folder(&app, &source);
    assert!(checked(&app, "sample.tar"));
    let output = temp.path().join("replaced-file.zip");
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press("role:button[Create archive…]").unwrap();
    app.wait_for("role:button[Continue]").unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("text:/^Could not create ZIP:/").unwrap();
    assert!(!output.exists());
    assert!(checked(&app, "sample.tar"));

    let moved = source.join("moved-child");
    std::fs::rename(&child, &moved).unwrap();
    symlink(&outside, &child).unwrap();
    app.press("role:button[Open folder child]").unwrap();
    app.wait_for("text:/^Could not browse item:/").unwrap();
    assert_folder(&app, &source);
    assert_eq!(
        app.screen().count("role:checkbox").unwrap(),
        2,
        "errors must preserve the listed snapshot until refresh"
    );
    app.press("role:button[Clear selection]").unwrap();
    app.press("role:checkbox[child/]").unwrap();
    let directory_output = temp.path().join("replaced-folder.zip");
    app.on_pick_save_path(Some(directory_output.clone()))
        .unwrap();
    app.press("role:button[Create archive…]").unwrap();
    app.wait_for("role:button[Continue]").unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("text:/^Could not create ZIP:/").unwrap();
    assert!(!directory_output.exists());
    assert!(checked(&app, "child/"));
    assert_folder(&app, &source);
    assert_eq!(
        std::fs::read(moved.join("inside.txt")).unwrap(),
        b"original child"
    );
    assert_eq!(
        std::fs::read(outside.join("sentinel.txt")).unwrap(),
        b"outside remains unchanged"
    );
    assert_eq!(
        std::fs::read(outside.join("secret.tar")).unwrap(),
        include_bytes!("../../../tests/fixtures/sample.tar")
    );
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 2);
    assert_eq!(app.screen().file_requests().len(), 3);
}
