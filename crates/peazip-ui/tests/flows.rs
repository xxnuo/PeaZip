#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use peazip_core::{Limits, MemoryEntry};

fn start() -> Headless {
    let mut app = Headless::start(EngineConfig::new(Root::of::<peazip_ui::App>())).unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    app.press("role:button[All entries]").unwrap();
    app
}

#[test]
fn browses_implicit_directories_and_extracts_only_their_real_descendants() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("sample.tar");
    std::fs::write(
        &source,
        include_bytes!("../../../tests/fixtures/sample.tar"),
    )
    .unwrap();
    let mut app = start();
    app.press("role:button[Folder view]").unwrap();
    app.on_pick_paths(vec![source.clone()]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("5 entries").unwrap();
    assert!(
        !app.screen()
            .matches("role:checkbox[docs/你好.txt]")
            .unwrap()
    );
    app.press("role:button[Open folder docs/]").unwrap();
    app.wait_for("role:checkbox[docs/nested/]").unwrap();
    app.press("role:checkbox[docs/nested/]").unwrap();
    app.wait_for("1 selected").unwrap();
    app.press("role:button[Open folder docs/nested/]").unwrap();
    app.wait_for("role:checkbox[docs/nested/data.bin]").unwrap();
    app.press("role:button[Up]").unwrap();
    app.wait_for("role:checkbox[docs/empty/]").unwrap();
    app.press("role:button[Back]").unwrap();
    app.wait_for("role:checkbox[docs/nested/data.bin]").unwrap();
    app.press("role:button[Forward]").unwrap();
    app.wait_for("role:checkbox[docs/empty/]").unwrap();
    app.clear("Archive path").unwrap();
    app.type_text("Archive path", "../escape").unwrap();
    app.press("role:button[Go]").unwrap();
    app.wait_for("Invalid archive directory").unwrap();
    assert!(app.screen().matches("role:checkbox[docs/empty/]").unwrap());
    assert!(app.screen().shows("1 selected"));
    let output = temp.path().join("output");
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press("role:button[Extract selected…]").unwrap();
    app.wait_for("Extracted 1 entries").unwrap();
    assert_eq!(
        std::fs::read(output.join("docs/nested/data.bin")).unwrap(),
        [0, 127, 255]
    );
    assert!(!output.join("docs/你好.txt").exists());
    app.on_pick_paths(vec![source]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("5 entries").unwrap();
    assert!(!app.screen().enabled("role:button[Back]").unwrap());
    assert!(app.screen().matches("role:checkbox[other.txt]").unwrap());
    assert!(app.screen().shows("0 selected"));
    app.clear("Archive path").unwrap();
    app.type_text("Archive path", "missing").unwrap();
    app.press("role:button[Go]").unwrap();
    app.wait_for("Archive directory not found").unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("5 entries").unwrap();
    assert_eq!(app.screen().input_text("Archive path").unwrap(), Some("/"));
    assert!(!app.screen().shows("Archive directory not found"));
}

#[test]
fn unchecking_a_child_of_a_selected_directory_excludes_it_from_extraction() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("sample.tar");
    std::fs::write(
        &source,
        include_bytes!("../../../tests/fixtures/sample.tar"),
    )
    .unwrap();
    let mut app = start();
    app.press("role:button[Folder view]").unwrap();
    app.on_pick_paths(vec![source]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("5 entries").unwrap();
    app.press("role:checkbox[docs/]").unwrap();
    app.press("role:button[Open folder docs/]").unwrap();
    app.wait_for("role:checkbox[docs/你好.txt]").unwrap();
    // This press clears the effective ancestor selection for this child.
    app.press("role:checkbox[docs/你好.txt]").unwrap();
    let output = temp.path().join("output");
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press("role:button[Extract selected…]").unwrap();
    app.wait_for("Extracted 2 entries").unwrap();
    assert!(!output.join("docs/你好.txt").exists());
    assert!(output.join("docs/empty").is_dir());
    assert_eq!(
        std::fs::read(output.join("docs/nested/data.bin")).unwrap(),
        [0, 127, 255]
    );
    assert!(!output.join("other.txt").exists());
}

#[test]
fn tests_all_archive_contents_without_dialogs_or_disk_output() {
    let mut app = start();
    assert!(!app.screen().enabled("role:button[Test archive]").unwrap());
    for (bytes, entries, size, checksums) in [
        (
            include_bytes!("../../../tests/fixtures/sample.tar").as_slice(),
            5,
            27,
            0,
        ),
        (
            include_bytes!("../../../tests/fixtures/sample.tar.gz").as_slice(),
            5,
            27,
            3,
        ),
        (
            include_bytes!("../../../tests/fixtures/sample.7z").as_slice(),
            6,
            26,
            3,
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("archive.bin");
        std::fs::write(&source, bytes).unwrap();
        app.on_pick_paths(vec![source.clone()]).unwrap();
        app.press("role:button[Open archive…]").unwrap();
        app.wait_for(&format!("{entries} entries")).unwrap();
        app.press("role:checkbox[docs/]").unwrap();
        std::fs::remove_file(source).unwrap();
        let requests = app.screen().file_requests().len();
        app.press("role:button[Test archive]").unwrap();
        app.wait_for(&format!(
            "Tested {entries} entries ({size} bytes). Content checksums: {checksums}/3 files."
        ))
        .unwrap();
        assert!(app.screen().shows("1 selected"));
        assert_eq!(app.screen().file_requests().len(), requests);
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
        assert!(app.screen().enabled("role:button[Test archive]").unwrap());
    }
}

#[test]
fn archive_test_detects_unselected_zip_corruption_and_preserves_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("bad.zip");
    let mut bytes = include_bytes!("../../../web/tests/fixtures/sample.zip").to_vec();
    // The independent fixture stores notes.txt without compression.
    let offset = bytes
        .windows(b"hello\n".len())
        .position(|w| w == b"hello\n")
        .unwrap();
    bytes[offset] ^= 1;
    std::fs::write(&source, &bytes).unwrap();
    let mut app = start();
    app.on_pick_paths(vec![source.clone()]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("3 entries").unwrap();
    app.press("role:checkbox[你好.txt]").unwrap();
    let requests = app.screen().file_requests().len();
    app.press("role:button[Test archive]").unwrap();
    app.wait_for("text:/^Could not test archive:/").unwrap();
    assert!(app.screen().shows("1 selected"));
    assert!(app.screen().shows("notes.txt"));
    assert_eq!(app.screen().file_requests().len(), requests);
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    let output = temp.path().join("selected");
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press("role:button[Extract selected…]").unwrap();
    app.wait_for("Extracted 1 entries").unwrap();
    assert!(output.join("你好.txt").is_file());
}

#[test]
fn opens_tar_by_content_and_extracts_selected_directory() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("misnamed.zip");
    std::fs::write(
        &source,
        include_bytes!("../../../tests/fixtures/sample.tar"),
    )
    .unwrap();
    let mut app = start();
    app.on_pick_paths(vec![source]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("5 entries").unwrap();
    app.wait_for("docs/你好.txt").unwrap();
    app.press("role:checkbox[docs/]").unwrap();
    let destination = temp.path().join("output");
    app.on_pick_save_path(Some(destination.clone())).unwrap();
    app.press("role:button[Extract selected…]").unwrap();
    app.wait_for("Extracted 4 entries").unwrap();
    assert_eq!(
        std::fs::read(destination.join("docs/你好.txt")).unwrap(),
        "tar 你好\n".as_bytes()
    );
    assert!(destination.join("docs/empty").is_dir());
    assert!(!destination.join("other.txt").exists());
}

#[test]
fn opens_gzip_tar_and_preserves_snapshot_after_crc_failure() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("misnamed.zip");
    std::fs::write(
        &source,
        include_bytes!("../../../tests/fixtures/sample.tar.gz"),
    )
    .unwrap();
    let mut app = start();
    app.on_pick_paths(vec![source.clone()]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("5 entries").unwrap();
    app.wait_for("docs/你好.txt").unwrap();
    app.press("role:checkbox[docs/]").unwrap();
    let mut corrupt = std::fs::read(&source).unwrap();
    let footer = corrupt.len() - 8;
    corrupt[footer] ^= 1;
    std::fs::write(&source, corrupt).unwrap();
    app.on_pick_paths(vec![source]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("text:/^Could not open archive:/").unwrap();
    assert!(app.screen().shows("docs/你好.txt"));
    let destination = temp.path().join("output");
    app.on_pick_save_path(Some(destination.clone())).unwrap();
    app.press("role:button[Extract selected…]").unwrap();
    app.wait_for("Extracted 4 entries").unwrap();
    assert_eq!(
        std::fs::read(destination.join("docs/你好.txt")).unwrap(),
        "tar 你好\n".as_bytes()
    );
    assert!(destination.join("docs/empty").is_dir());
    assert!(!destination.join("other.txt").exists());
}

#[test]
fn opens_seven_zip_and_preserves_snapshot_after_header_crc_failure() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("misnamed.zip");
    std::fs::write(&source, include_bytes!("../../../tests/fixtures/sample.7z")).unwrap();
    let mut app = start();
    app.on_pick_paths(vec![source.clone()]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("6 entries").unwrap();
    app.wait_for("docs/你好.txt").unwrap();
    app.press("role:checkbox[docs/]").unwrap();
    let mut corrupt = std::fs::read(&source).unwrap();
    let footer = 8;
    corrupt[footer] ^= 1;
    std::fs::write(&source, corrupt).unwrap();
    app.on_pick_paths(vec![source]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("text:/^Could not open archive:/").unwrap();
    assert!(app.screen().shows("docs/你好.txt"));
    let destination = temp.path().join("output");
    app.on_pick_save_path(Some(destination.clone())).unwrap();
    app.press("role:button[Extract selected…]").unwrap();
    app.wait_for("Extracted 5 entries").unwrap();
    assert_eq!(
        std::fs::read(destination.join("docs/你好.txt")).unwrap(),
        "7z 你好\n".as_bytes()
    );
    assert!(destination.join("docs/empty").is_dir());
    assert!(!destination.join("other.txt").exists());
}

#[test]
fn cancel_keeps_the_initial_state_and_open_available() {
    let mut app = start();
    app.on_pick_paths(vec![]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    assert!(app.screen().enabled("role:button[Open archive…]").unwrap());
    assert_eq!(app.screen().file_requests().len(), 1);
}

#[test]
fn lists_real_zip_then_reports_invalid_zip_and_recovers() {
    let path = std::env::temp_dir().join(format!("peazip-ui-{}.zip", std::process::id()));
    let bytes = peazip_core::create_zip(
        &[MemoryEntry {
            path: "hello.txt".into(),
            is_dir: false,
            data: b"hello".to_vec(),
        }],
        Limits::default(),
    )
    .unwrap();
    std::fs::write(&path, bytes).unwrap();
    let mut app = start();
    app.on_pick_paths(vec![path.clone()]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("hello.txt").unwrap();
    app.wait_for("1 entries").unwrap();
    app.wait_for("text:5").unwrap();
    assert!(app.screen().enabled("role:button[Open archive…]").unwrap());

    std::fs::write(&path, b"not a zip").unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("text:/^Could not open archive:/").unwrap();
    assert!(app.screen().shows("hello.txt"));
    assert!(app.screen().enabled("role:button[Open archive…]").unwrap());

    std::fs::remove_file(&path).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("text:/^Could not open archive:.*os error/")
        .unwrap();
    assert!(app.screen().enabled("role:button[Open archive…]").unwrap());
}

#[test]
fn rejects_an_oversized_archive_and_allows_another_open() {
    let path = std::env::temp_dir().join(format!("peazip-ui-large-{}.zip", std::process::id()));
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(Limits::default().max_archive_bytes + 1)
        .unwrap();
    drop(file);
    let mut app = start();
    app.on_pick_paths(vec![path.clone()]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("text:/^Could not open archive: Archive limit exceeded: archive bytes/")
        .unwrap();
    assert!(app.screen().enabled("role:button[Open archive…]").unwrap());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn selects_individual_entries_all_and_none_and_resets_on_new_archive() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("selection.zip");
    let bytes = peazip_core::create_zip(
        &["first.txt", "second.txt"].map(|name| MemoryEntry {
            path: name.into(),
            is_dir: false,
            data: b"sample".to_vec(),
        }),
        Limits::default(),
    )
    .unwrap();
    std::fs::write(&path, bytes).unwrap();
    let mut app = start();
    assert!(!app.screen().enabled("role:button[Select all]").unwrap());
    assert!(
        !app.screen()
            .enabled("role:button[Clear selection]")
            .unwrap()
    );
    app.on_pick_paths(vec![path.clone()]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("first.txt").unwrap();
    app.press("role:checkbox[first.txt]").unwrap();
    app.wait_for("1 selected").unwrap();
    assert_eq!(
        app.screen()
            .accessibility_state("role:checkbox[first.txt]")
            .unwrap()
            .checked,
        Some(nsh_core::Checked::True)
    );
    app.press("role:button[Select all]").unwrap();
    app.wait_for("2 selected").unwrap();
    app.press("role:checkbox[first.txt]").unwrap();
    app.wait_for("1 selected").unwrap();
    assert_eq!(
        app.screen()
            .accessibility_state("role:checkbox[first.txt]")
            .unwrap()
            .checked,
        Some(nsh_core::Checked::False)
    );
    app.press("role:button[Clear selection]").unwrap();
    app.wait_for("0 selected").unwrap();

    app.press("role:checkbox[first.txt]").unwrap();
    app.wait_for("1 selected").unwrap();
    app.on_pick_paths(vec![]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("2 entries").unwrap();
    assert!(app.screen().shows("1 selected"));
    app.on_pick_paths(vec![temp.path().join("missing.zip")])
        .unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("text:/^Could not open archive:/").unwrap();
    assert!(app.screen().shows("1 selected"));

    app.on_pick_paths(vec![path]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("2 entries").unwrap();
    app.wait_for("0 selected").unwrap();
    assert_eq!(
        app.screen()
            .accessibility_state("role:checkbox[first.txt]")
            .unwrap()
            .checked,
        Some(nsh_core::Checked::False)
    );
}

#[test]
fn creates_zip_from_selected_files_and_refuses_to_overwrite_it() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first.txt");
    let second = temp.path().join("中文.txt");
    let output = temp.path().join("created.zip");
    std::fs::write(&first, b"first contents").unwrap();
    std::fs::write(&second, b"second contents").unwrap();
    let mut app = start();
    app.on_pick_paths(vec![first.clone(), second.clone()])
        .unwrap();
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press("role:button[Create ZIP…]").unwrap();
    app.wait_for("Created ZIP: 2 entries").unwrap();
    app.wait_for("first.txt").unwrap();
    app.wait_for("中文.txt").unwrap();
    assert!(app.screen().enabled("role:button[Open archive…]").unwrap());
    assert!(app.screen().enabled("role:button[Create ZIP…]").unwrap());
    let original = std::fs::read(&output).unwrap();
    let extracted = peazip_core::extract_zip(&original, Limits::default()).unwrap();
    assert_eq!(extracted[0].data, b"first contents");
    assert_eq!(extracted[1].data, b"second contents");
    assert_eq!(std::fs::read(&first).unwrap(), b"first contents");
    assert_eq!(std::fs::read(&second).unwrap(), b"second contents");

    app.press("role:checkbox[first.txt]").unwrap();
    app.wait_for("1 selected").unwrap();
    std::fs::write(&first, b"changed").unwrap();
    app.press("role:button[Create ZIP…]").unwrap();
    app.wait_for("text:/^Could not create ZIP:/").unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), original);
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 3);
    assert!(app.screen().shows("1 selected"));
    assert!(app.screen().enabled("role:button[Create ZIP…]").unwrap());

    let next = temp.path().join("next.zip");
    app.on_pick_save_path(Some(next.clone())).unwrap();
    app.press("role:button[Create ZIP…]").unwrap();
    app.wait_for("Created ZIP: 2 entries").unwrap();
    app.wait_for("0 selected").unwrap();
    assert!(next.is_file());
}

#[test]
fn cancelling_creation_at_either_dialog_writes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input.txt");
    std::fs::write(&input, b"unchanged").unwrap();
    let mut app = start();
    app.on_pick_paths(vec![]).unwrap();
    app.press("role:button[Create ZIP…]").unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    assert_eq!(app.screen().file_requests().len(), 1);
    assert!(app.screen().enabled("role:button[Create ZIP…]").unwrap());
    app.on_pick_paths(vec![input.clone()]).unwrap();
    app.on_pick_save_path(None).unwrap();
    app.press("role:button[Create ZIP…]").unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    assert_eq!(app.screen().file_requests().len(), 3);
    assert!(app.screen().enabled("role:button[Create ZIP…]").unwrap());
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    assert_eq!(std::fs::read(input).unwrap(), b"unchanged");
}

#[test]
fn extracts_selection_from_snapshot_and_retries_without_overwriting() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.zip");
    let bytes = peazip_core::create_zip(
        &[
            MemoryEntry {
                path: "keep.txt".into(),
                is_dir: false,
                data: b"original".to_vec(),
            },
            MemoryEntry {
                path: "skip.txt".into(),
                is_dir: false,
                data: b"skip".to_vec(),
            },
            MemoryEntry {
                path: "folder/".into(),
                is_dir: true,
                data: vec![],
            },
            MemoryEntry {
                path: "folder/child.txt".into(),
                is_dir: false,
                data: b"child".to_vec(),
            },
        ],
        Limits::default(),
    )
    .unwrap();
    std::fs::write(&source, bytes).unwrap();
    let mut app = start();
    assert!(
        !app.screen()
            .enabled("role:button[Extract selected…]")
            .unwrap()
    );
    app.on_pick_paths(vec![source.clone()]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("4 entries").unwrap();
    app.press("role:checkbox[keep.txt]").unwrap();
    app.wait_for("1 selected").unwrap();

    // A different, valid archive at the same path must never replace the loaded bytes.
    let replacement = peazip_core::create_zip(
        &[MemoryEntry {
            path: "keep.txt".into(),
            is_dir: false,
            data: b"replacement".to_vec(),
        }],
        Limits::default(),
    )
    .unwrap();
    std::fs::write(&source, replacement).unwrap();
    app.on_pick_save_path(None).unwrap();
    app.press("role:button[Extract selected…]").unwrap();
    app.wait_for("4 entries").unwrap();
    assert!(app.screen().shows("1 selected"));
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    match app.screen().file_requests().last().unwrap() {
        nsh_core::file_dialog::FileRequest::SaveDialog(dialog) => {
            assert!(dialog.filters.is_empty());
            assert!(
                dialog
                    .title
                    .as_ref()
                    .unwrap()
                    .contains("choose new folder name")
            );
        }
        other => panic!("unexpected request: {other:?}"),
    }
    let existing = temp.path().join("existing");
    std::fs::create_dir(&existing).unwrap();
    std::fs::write(existing.join("sentinel"), b"untouched").unwrap();
    app.on_pick_save_path(Some(existing.clone())).unwrap();
    app.press("role:button[Extract selected…]").unwrap();
    app.wait_for("text:/^Could not extract archive:/").unwrap();
    assert_eq!(
        std::fs::read(existing.join("sentinel")).unwrap(),
        b"untouched"
    );
    assert_eq!(std::fs::read_dir(&existing).unwrap().count(), 1);
    assert!(app.screen().shows("1 selected"));
    let output = temp.path().join("partial");
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press("role:button[Extract selected…]").unwrap();
    app.wait_for("Extracted 1 entries").unwrap();
    assert_eq!(std::fs::read(output.join("keep.txt")).unwrap(), b"original");
    assert!(!output.join("skip.txt").exists());
    assert!(!output.join("folder").exists());

    std::fs::remove_file(&source).unwrap();
    app.press("role:checkbox[keep.txt]").unwrap();
    app.press("role:checkbox[folder/]").unwrap();
    app.wait_for("1 selected").unwrap();
    let output = temp.path().join("directory");
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press("role:button[Extract selected…]").unwrap();
    app.wait_for("Extracted 2 entries").unwrap();
    assert_eq!(
        std::fs::read(output.join("folder/child.txt")).unwrap(),
        b"child"
    );
    assert!(!output.join("keep.txt").exists());
    assert!(app.screen().shows("1 selected"));
    assert!(app.screen().shows("4 entries") || app.screen().shows("skip.txt"));
}

#[test]
fn created_archive_snapshot_survives_deleted_output() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input.txt");
    let archive = temp.path().join("created.zip");
    let destination = temp.path().join("extracted");
    std::fs::write(&input, b"created snapshot").unwrap();
    let mut app = start();
    app.on_pick_paths(vec![input]).unwrap();
    app.on_pick_save_path(Some(archive.clone())).unwrap();
    app.press("role:button[Create ZIP…]").unwrap();
    app.wait_for("Created ZIP: 1 entries").unwrap();
    app.press("role:checkbox[input.txt]").unwrap();
    std::fs::remove_file(archive).unwrap();
    app.on_pick_save_path(Some(destination.clone())).unwrap();
    app.press("role:button[Extract selected…]").unwrap();
    app.wait_for("Extracted 1 entries").unwrap();
    assert_eq!(
        std::fs::read(destination.join("input.txt")).unwrap(),
        b"created snapshot"
    );
}

#[test]
fn creates_and_extracts_directory_tree_with_empty_directories() {
    let temp = tempfile::tempdir().unwrap();
    let tree = temp.path().join("tree");
    std::fs::create_dir_all(tree.join("nested")).unwrap();
    std::fs::create_dir_all(tree.join("empty")).unwrap();
    std::fs::write(tree.join("nested/中文.txt"), b"nested payload").unwrap();
    std::fs::write(tree.join("top.txt"), b"top payload").unwrap();
    let empty = temp.path().join("empty-root");
    std::fs::create_dir(&empty).unwrap();
    let archive = temp.path().join("directories.zip");
    let mut app = start();
    app.on_pick_paths(vec![tree.clone(), empty]).unwrap();
    app.on_pick_save_path(Some(archive.clone())).unwrap();
    app.press("role:button[Create folder ZIP…]").unwrap();
    app.wait_for("Created ZIP: 6 entries").unwrap();
    match &app.screen().file_requests()[0] {
        nsh_core::file_dialog::FileRequest::OpenDialog(dialog) => {
            assert_eq!(dialog.kind, nsh_core::file_dialog::OpenDialogKind::Folders);
            assert!(dialog.multiple);
        }
        other => panic!("expected folder picker, got {other:?}"),
    }
    let rows =
        peazip_core::extract_zip(&std::fs::read(&archive).unwrap(), Limits::default()).unwrap();
    assert_eq!(
        rows.iter()
            .find(|row| row.path == "tree/nested/中文.txt")
            .unwrap()
            .data,
        b"nested payload"
    );
    assert!(
        rows.iter()
            .any(|row| row.path == "tree/empty/" && row.is_dir)
    );
    assert!(
        rows.iter()
            .any(|row| row.path == "empty-root/" && row.is_dir)
    );
    assert_eq!(std::fs::read(tree.join("top.txt")).unwrap(), b"top payload");
    app.press("role:button[Select all]").unwrap();
    let output = temp.path().join("extracted");
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press("role:button[Extract selected…]").unwrap();
    app.wait_for("Extracted 6 entries").unwrap();
    assert_eq!(
        std::fs::read(output.join("tree/nested/中文.txt")).unwrap(),
        b"nested payload"
    );
    assert!(output.join("tree/empty").is_dir());
    assert!(output.join("empty-root").is_dir());
}

#[test]
fn folder_picker_cancellation_does_not_create_an_archive() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("not-created.zip");
    let mut app = start();
    app.on_pick_paths(vec![]).unwrap();
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press("role:button[Create folder ZIP…]").unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    assert_eq!(app.screen().file_requests().len(), 1);
    assert!(!output.exists());
    assert!(
        app.screen()
            .enabled("role:button[Create folder ZIP…]")
            .unwrap()
    );
}
