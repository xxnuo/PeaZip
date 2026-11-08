#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use nsh_core::{Checked, Size, file_dialog::FileRequest, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use peazip_core::Limits;
use std::{collections::BTreeMap, path::Path};

fn start() -> Headless {
    let mut config = EngineConfig::new(Root::of::<peazip_ui::App>());
    config.viewport = Size::new(px(1024.0), px(600.0));
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    app
}

fn load(source: &Path) -> Headless {
    std::fs::write(source, include_bytes!("../../../tests/fixtures/sample.tar")).unwrap();
    let mut app = start();
    app.press("role:button[All entries]").unwrap();
    app.on_pick_paths(vec![source.to_path_buf()]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("5 entries").unwrap();
    app
}

fn checked(app: &Headless, name: &str) -> bool {
    app.screen()
        .accessibility_state(&format!("role:checkbox[{name}]"))
        .unwrap()
        .checked
        == Some(Checked::True)
}

fn extract(app: &mut Headless, destination: &Path) {
    app.on_pick_save_path(Some(destination.to_path_buf()))
        .unwrap();
    app.press("role:button[Extract…]").unwrap();
    app.wait_for("role:button[Continue]").unwrap();
    assert_eq!(
        app.screen()
            .accessibility_state("role:radio[Selected entries]")
            .unwrap()
            .checked,
        Some(Checked::True)
    );
    app.press("role:button[Continue]").unwrap();
    app.wait_for("text:/^Extracted /").unwrap();
}

fn tree(directory: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    fn walk(base: &Path, directory: &Path, output: &mut BTreeMap<String, Option<Vec<u8>>>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let name = path
                .strip_prefix(base)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            if entry.file_type().unwrap().is_dir() {
                output.insert(format!("{name}/"), None);
                walk(base, &path, output);
            } else {
                output.insert(name, Some(std::fs::read(path).unwrap()));
            }
        }
    }
    let mut output = BTreeMap::new();
    walk(directory, directory, &mut output);
    output
}

#[test]
fn tar_subdirectory_inversion_keeps_outside_selection_and_extracts_the_leaf_complement() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = load(&temp.path().join("sample.tar"));
    app.press("role:checkbox[other.txt]").unwrap();
    app.press("role:checkbox[docs/你好.txt]").unwrap();
    app.press("role:button[Folder view]").unwrap();
    app.press("role:button[Open folder docs/]").unwrap();
    app.press("role:button[Invert selection]").unwrap();
    assert!(!checked(&app, "docs/你好.txt"));
    assert!(checked(&app, "docs/empty/"));
    assert!(checked(&app, "docs/nested/"));
    let output = temp.path().join("output");
    extract(&mut app, &output);
    assert_eq!(
        std::fs::read(output.join("docs/nested/data.bin")).unwrap(),
        [0, 127, 255]
    );
    assert_eq!(
        std::fs::read(output.join("other.txt")).unwrap(),
        b"not selected\n"
    );
    assert!(output.join("docs/empty").is_dir());
    assert!(!output.join("docs/你好.txt").exists());
    app.press("role:button[Archive root]").unwrap();
    assert!(checked(&app, "other.txt"));
}

#[test]
fn partial_folder_inversion_selects_only_the_previously_unchecked_child() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = load(&temp.path().join("sample.tar"));
    app.press("role:checkbox[docs/]").unwrap();
    app.press("role:button[Folder view]").unwrap();
    app.press("role:button[Open folder docs/]").unwrap();
    app.press("role:checkbox[docs/你好.txt]").unwrap();
    app.press("role:button[Invert selection]").unwrap();
    assert!(checked(&app, "docs/你好.txt"));
    assert!(!checked(&app, "docs/empty/"));
    assert!(!checked(&app, "docs/nested/"));
    let output = temp.path().join("output");
    extract(&mut app, &output);
    assert_eq!(
        std::fs::read(output.join("docs/你好.txt")).unwrap(),
        "tar 你好\n".as_bytes()
    );
    assert!(!output.join("docs/empty").exists());
    assert!(!output.join("docs/nested").exists());
    assert!(!output.join("other.txt").exists());
}

#[test]
fn flat_inversion_does_not_reselect_parents_that_reinclude_unchecked_descendants() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = load(&temp.path().join("sample.tar"));
    app.press("role:checkbox[docs/]").unwrap();
    app.press("role:checkbox[docs/你好.txt]").unwrap();
    app.press("role:button[Invert selection]").unwrap();
    assert!(checked(&app, "docs/你好.txt"));
    assert!(checked(&app, "other.txt"));
    assert!(!checked(&app, "docs/empty/"));
    assert!(!checked(&app, "docs/nested/data.bin"));
    let output = temp.path().join("output");
    extract(&mut app, &output);
    assert_eq!(
        tree(&output),
        BTreeMap::from([
            ("docs/".into(), None),
            (
                "docs/你好.txt".into(),
                Some("tar 你好\n".as_bytes().to_vec())
            ),
            ("other.txt".into(), Some(b"not selected\n".to_vec())),
        ])
    );
}

#[test]
fn inverting_twice_restores_effective_contents_even_when_parent_paths_are_normalized() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = load(&temp.path().join("sample.tar"));
    app.press("role:checkbox[docs/]").unwrap();
    let before = temp.path().join("before");
    extract(&mut app, &before);
    app.press("role:button[Invert selection]").unwrap();
    assert!(!checked(&app, "docs/你好.txt"));
    assert!(!checked(&app, "docs/empty/"));
    assert!(!checked(&app, "docs/nested/data.bin"));
    assert!(checked(&app, "other.txt"));
    app.press("role:button[Invert selection]").unwrap();
    assert!(checked(&app, "docs/你好.txt"));
    assert!(checked(&app, "docs/empty/"));
    assert!(checked(&app, "docs/nested/data.bin"));
    assert!(!checked(&app, "other.txt"));
    let after = temp.path().join("after");
    extract(&mut app, &after);
    assert_eq!(tree(&after), tree(&before));
}

#[test]
fn invert_shortcut_leaves_path_editing_alone_and_works_after_editing_ends() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = load(&temp.path().join("sample.tar"));
    app.press("role:checkbox[docs/你好.txt]").unwrap();
    app.clear("Archive path").unwrap();
    app.type_text("Archive path", "/").unwrap();
    let requests = app.screen().file_requests().len();
    app.press_key("mod+i").unwrap();
    assert!(checked(&app, "docs/你好.txt"));
    assert!(!checked(&app, "other.txt"));
    assert!(app.screen().shows("1 selected"));
    app.press_key("x").unwrap();
    assert_eq!(app.screen().input_text("Archive path").unwrap(), Some("/x"));
    app.clear("Archive path").unwrap();
    app.type_text("Archive path", "/").unwrap();
    app.press_key("enter").unwrap();
    app.press_key("mod+i").unwrap();
    assert!(!checked(&app, "docs/你好.txt"));
    assert!(checked(&app, "docs/empty/"));
    assert!(checked(&app, "docs/nested/data.bin"));
    assert!(checked(&app, "other.txt"));
    assert_eq!(app.screen().file_requests().len(), requests);
}

#[test]
fn empty_views_and_create_extract_modals_guard_inversion() {
    let mut empty = start();
    assert!(
        !empty
            .screen()
            .enabled("role:button[Invert selection]")
            .unwrap()
    );
    empty.press_key("mod+i").unwrap();
    assert!(empty.screen().shows("0 selected"));
    assert!(empty.screen().file_requests().is_empty());
    drop(empty);
    let temp = tempfile::tempdir().unwrap();
    let mut app = load(&temp.path().join("sample.tar"));
    app.press("role:checkbox[docs/你好.txt]").unwrap();
    app.press("role:button[Folder view]").unwrap();
    app.press("role:button[Open folder docs/]").unwrap();
    app.press("role:button[Open folder docs/empty/]").unwrap();
    assert!(
        !app.screen()
            .enabled("role:button[Invert selection]")
            .unwrap()
    );
    app.press_key("mod+i").unwrap();
    assert!(app.screen().shows("1 selected"));
    app.press("role:button[Archive root]").unwrap();
    let requests = app.screen().file_requests().len();
    for action in ["Create ZIP…", "Extract…"] {
        app.press(&format!("role:button[{action}]")).unwrap();
        app.wait_for("role:button[Continue]").unwrap();
        app.press_key("mod+i").unwrap();
        assert_eq!(app.screen().file_requests().len(), requests);
        app.press("role:button[Cancel]").unwrap();
        app.wait_for("5 entries").unwrap();
        assert!(app.screen().shows("1 selected"));
    }
    app.press("role:button[Open folder docs/]").unwrap();
    assert!(checked(&app, "docs/你好.txt"));
    assert!(!checked(&app, "docs/empty/"));
    assert!(!checked(&app, "docs/nested/"));
}

#[test]
fn filesystem_inversion_uses_stable_ids_after_sort_and_zips_the_complement_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = start();
    app.on_pick_paths(vec![temp.path().to_path_buf()]).unwrap();
    app.press("role:button[Open folder…]").unwrap();
    app.wait_for("0 items").unwrap();
    assert!(
        !app.screen()
            .enabled("role:button[Invert selection]")
            .unwrap()
    );
    std::fs::create_dir(temp.path().join("child")).unwrap();
    std::fs::write(temp.path().join("child/nested.txt"), b"nested").unwrap();
    std::fs::write(temp.path().join("a.txt"), b"excluded").unwrap();
    std::fs::write(temp.path().join("b.txt"), b"b").unwrap();
    std::fs::write(temp.path().join("z.txt"), b"last payload").unwrap();
    app.press("role:toolbar >> nth=0 >> role:button[Refresh folder]")
        .unwrap();
    app.wait_for("role:checkbox[a.txt]").unwrap();
    app.press("role:checkbox[a.txt]").unwrap();
    for sort in ["Name", "Size (bytes)", "Size (bytes)"] {
        app.press(&format!("role:button[{sort}]")).unwrap();
        assert!(checked(&app, "a.txt"));
    }
    app.clear("Folder path").unwrap();
    app.type_text("Folder path", temp.path().to_str().unwrap())
        .unwrap();
    app.press_key("mod+i").unwrap();
    assert!(checked(&app, "a.txt"));
    assert!(!checked(&app, "b.txt"));
    app.press("role:button[Invert selection]").unwrap();
    assert!(!checked(&app, "a.txt"));
    for name in ["b.txt", "z.txt", "child/"] {
        assert!(checked(&app, name));
    }
    let output = temp.path().join("complement.zip");
    let requests = app.screen().file_requests().len();
    app.on_pick_save_path(Some(output.clone())).unwrap();
    app.press("role:button[Create ZIP…]").unwrap();
    app.wait_for("role:button[Continue]").unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for("Created ZIP: 4 entries").unwrap();
    let actual: BTreeMap<_, _> =
        peazip_core::extract_zip(&std::fs::read(output).unwrap(), Limits::default())
            .unwrap()
            .into_iter()
            .map(|entry| (entry.path, (entry.is_dir, entry.data)))
            .collect();
    assert_eq!(
        actual,
        BTreeMap::from([
            ("b.txt".into(), (false, b"b".to_vec())),
            ("z.txt".into(), (false, b"last payload".to_vec())),
            ("child/".into(), (true, vec![])),
            ("child/nested.txt".into(), (false, b"nested".to_vec())),
        ])
    );
    assert_eq!(app.screen().file_requests().len(), requests + 1);
    assert!(matches!(
        app.screen().file_requests().last().unwrap(),
        FileRequest::SaveDialog(_)
    ));
    assert_eq!(
        std::fs::read(temp.path().join("a.txt")).unwrap(),
        b"excluded"
    );
}
