#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use nsh_core::{Checked, EventKind, EventPayload, PressEvent, Size, prelude::px};
use nsh_engine::{Clock, EngineConfig, Root};
use nsh_testing::Headless;
use peazip_core::{Limits, MemoryEntry};
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

const ARCHIVE_FILE_ERROR: &str =
    "Opening files inside archives is not available yet. Use Extract… to save the file.";

fn start() -> Headless {
    let mut config = EngineConfig::new(Root::of::<peazip_ui::App>());
    config.viewport = Size::new(px(1024.0), px(900.0));
    config.environment.idiom = nsh_engine::Idiom::Desktop;
    config.environment.os = if cfg!(target_os = "macos") {
        nsh_engine::Os::MacOs
    } else if cfg!(target_os = "windows") {
        nsh_engine::Os::Windows
    } else {
        nsh_engine::Os::Linux
    };
    config.clock = Clock::manual(SystemTime::UNIX_EPOCH);
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    app
}

fn archive(path: &Path) -> Vec<u8> {
    let entries: Vec<_> = [
        ("a-dir/", b"".as_slice()),
        ("a-dir/a.txt", b"first"),
        ("z-dir/", b""),
        ("z-dir/z.txt", b"last"),
        ("keep.txt", b"selection remains"),
        ("plain.txt", b"plain file"),
        ("nested.zip", b"not an archive to extract or launch"),
    ]
    .into_iter()
    .map(|(path, data)| MemoryEntry {
        path: path.into(),
        is_dir: path.ends_with('/'),
        data: data.into(),
    })
    .collect();
    let bytes = peazip_core::create_zip(&entries, Limits::default()).unwrap();
    fs::write(path, &bytes).unwrap();
    bytes
}

fn load(app: &mut Headless, path: &Path) {
    app.on_pick_paths(vec![path.to_path_buf()]).unwrap();
    app.press_key("mod+o").unwrap();
    app.wait_for("7 entries").unwrap();
    app.wait_for("label:Item name a-dir/").unwrap();
}

fn folder(app: &mut Headless, path: &Path, count: usize) {
    app.on_pick_paths(vec![path.to_path_buf()]).unwrap();
    app.press("role:button[Open folder…]").unwrap();
    app.wait_for(&format!("{count} of {count} items shown"))
        .unwrap();
    assert_folder(app, path);
}

fn assert_folder(app: &Headless, path: &Path) {
    let canonical = fs::canonicalize(path).unwrap();
    assert_eq!(
        app.screen().input_text("Folder path").unwrap(),
        canonical.to_str()
    );
    assert!(!app.screen().matches("Archive path").unwrap());
}

fn checked(app: &Headless, path: &str) -> bool {
    app.screen()
        .accessibility_state(&format!("role:checkbox[{path}]"))
        .unwrap()
        .checked
        == Some(Checked::True)
}

fn double_click(app: &mut Headless, name: &str) {
    let selector = format!("label:Item name {name}");
    app.press(&selector).unwrap();
    app.press(&selector).unwrap();
}

fn filter(app: &mut Headless, value: &str) {
    app.clear("Filter names").unwrap();
    app.type_text("Filter names", value).unwrap();
    app.press_key("enter").unwrap();
}

fn pointer_second(app: &mut Headless, selector: &str) {
    let target = app.screen().press_target(selector).unwrap().unwrap();
    app.engine().control().dispatch_event(
        target,
        EventKind::Press,
        EventPayload::Press(PressEvent {
            clicks: 2,
            ..Default::default()
        }),
    );
    app.next_change(Duration::from_millis(100)).unwrap();
}

#[test]
fn filesystem_double_click_opens_clicked_directory_despite_other_selected_rows() {
    let temp = tempfile::tempdir().unwrap();
    let child = temp.path().join("child");
    fs::create_dir(&child).unwrap();
    fs::write(child.join("inside.txt"), b"inside").unwrap();
    fs::write(temp.path().join("keep.txt"), b"keep").unwrap();
    let mut app = start();
    folder(&mut app, temp.path(), 2);
    app.press("role:checkbox[keep.txt]").unwrap();
    let requests = app.screen().file_requests().len();
    app.press("label:Item name child").unwrap();
    assert!(checked(&app, "child/"));
    assert!(checked(&app, "keep.txt"));
    assert_folder(&app, temp.path());
    app.press("label:Item name child").unwrap();
    app.wait_for("role:checkbox[inside.txt]").unwrap();
    assert_folder(&app, &child);
    assert!(app.screen().shows("0 selected"));
    assert_eq!(app.screen().file_requests().len(), requests);
    assert_eq!(fs::read(child.join("inside.txt")).unwrap(), b"inside");
}

#[test]
fn filtered_sorted_filesystem_archive_uses_the_clicked_snapshot_entry() {
    let temp = tempfile::tempdir().unwrap();
    archive(&temp.path().join("z-target.zip"));
    fs::write(temp.path().join("a-decoy.zip"), b"invalid archive").unwrap();
    fs::write(temp.path().join("keep.txt"), b"keep").unwrap();
    let mut app = start();
    folder(&mut app, temp.path(), 3);
    app.press("role:checkbox[keep.txt]").unwrap();
    app.press_key("mod+alt+2").unwrap();
    filter(&mut app, "target");
    app.wait_for("1 of 3 items shown").unwrap();
    let requests = app.screen().file_requests().len();
    double_click(&mut app, "z-target.zip");
    app.wait_for("7 entries").unwrap();
    assert_eq!(app.screen().input_text("Archive path").unwrap(), Some("/"));
    assert!(app.screen().matches("role:checkbox[a-dir/]").unwrap());
    assert!(!app.screen().shows("Could not browse item:"));
    assert_eq!(app.screen().file_requests().len(), requests);
    assert!(!temp.path().join("a-dir").exists());
}

#[test]
fn archive_directory_double_click_leaves_flat_view_and_targets_filtered_sorted_row() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.zip");
    archive(&source);
    let mut app = start();
    load(&mut app, &source);
    app.press("role:checkbox[keep.txt]").unwrap();
    app.press("role:button[All entries]").unwrap();
    app.press_key("mod+alt+0").unwrap();
    filter(&mut app, "z-dir");
    let requests = app.screen().file_requests().len();
    double_click(&mut app, "z-dir/");
    assert_eq!(
        app.screen().input_text("Archive path").unwrap(),
        Some("z-dir/")
    );
    assert!(app.screen().matches("role:button[All entries]").unwrap());
    assert_eq!(app.screen().input_text("Filter names").unwrap(), Some(""));
    app.wait_for("role:checkbox[z-dir/z.txt]").unwrap();
    assert!(!app.screen().matches("role:checkbox[a-dir/a.txt]").unwrap());
    assert_eq!(app.screen().file_requests().len(), requests);
    assert!(!temp.path().join("z-dir").exists());
}

#[test]
fn archive_regular_file_reports_extract_guidance_without_output_and_keeps_snapshot_selection() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.zip");
    archive(&source);
    let mut app = start();
    load(&mut app, &source);
    app.press("role:checkbox[keep.txt]").unwrap();
    fs::remove_file(&source).unwrap();
    let requests = app.screen().file_requests().len();
    double_click(&mut app, "nested.zip");
    app.wait_for(ARCHIVE_FILE_ERROR).unwrap();
    assert_eq!(app.screen().input_text("Archive path").unwrap(), Some("/"));
    assert!(checked(&app, "keep.txt"));
    assert!(checked(&app, "nested.zip"));
    assert!(app.screen().shows("2 selected"));
    assert!(app.screen().enabled("role:button[Test archive]").unwrap());
    app.press("role:button[Test archive]").unwrap();
    app.wait_for("text:/^Tested 7 entries/").unwrap();
    assert!(checked(&app, "keep.txt"));
    assert!(checked(&app, "nested.zip"));
    double_click(&mut app, "a-dir/");
    app.wait_for("role:checkbox[a-dir/a.txt]").unwrap();
    assert!(!app.screen().shows(ARCHIVE_FILE_ERROR));
    assert_eq!(app.screen().file_requests().len(), requests);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[test]
fn filesystem_unsupported_and_corrupt_files_restore_busy_and_preserve_listing_selection() {
    for name in ["plain.txt", "broken.zip"] {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join(name), b"not an archive").unwrap();
        fs::create_dir(temp.path().join("child")).unwrap();
        fs::write(temp.path().join("keep.txt"), b"keep").unwrap();
        let mut app = start();
        folder(&mut app, temp.path(), 3);
        app.press("role:checkbox[keep.txt]").unwrap();
        let requests = app.screen().file_requests().len();
        double_click(&mut app, name);
        app.wait_for("text:/^Could not browse item:/").unwrap();
        assert_folder(&app, temp.path());
        assert!(checked(&app, name));
        assert!(checked(&app, "keep.txt"));
        assert_eq!(app.screen().count("role:checkbox").unwrap(), 3);
        assert!(
            app.screen()
                .enabled("role:button[Refresh folder] >> nth=0")
                .unwrap()
        );
        double_click(&mut app, "child");
        app.wait_for("0 items").unwrap();
        assert_folder(&app, &temp.path().join("child"));
        assert_eq!(app.screen().file_requests().len(), requests);
        assert_eq!(fs::read(temp.path().join(name)).unwrap(), b"not an archive");
    }
}

#[test]
fn checkbox_pointer_and_keyboard_presses_toggle_without_opening_directories() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.zip");
    archive(&source);
    for filesystem in [false, true] {
        let mut app = start();
        let (label, path) = if filesystem {
            fs::create_dir_all(temp.path().join("child")).unwrap();
            folder(&mut app, temp.path(), 2);
            ("child/", "Folder path")
        } else {
            load(&mut app, &source);
            ("a-dir/", "Archive path")
        };
        let location = app.screen().input_text(path).unwrap().unwrap().to_owned();
        let selector = format!("role:checkbox[{label}]");
        app.press(&selector).unwrap();
        assert!(checked(&app, label));
        app.press(&selector).unwrap();
        assert!(!checked(&app, label));
        // Headless keyboard activation sends Press with no pointer/click-count payload.
        let target = app.screen().press_target(&selector).unwrap().unwrap();
        for expected in [true, false] {
            app.engine()
                .control()
                .dispatch_event(target, EventKind::Press, EventPayload::None);
            app.next_change(Duration::from_millis(100)).unwrap();
            assert_eq!(checked(&app, label), expected);
        }
        assert_eq!(
            app.screen().input_text(path).unwrap(),
            Some(location.as_str())
        );
        for input in ["Filter names", path] {
            app.wait(Duration::from_millis(501)).unwrap();
            if app
                .screen()
                .enabled("role:button[Clear selection]")
                .unwrap()
            {
                app.press("role:button[Clear selection]").unwrap();
            }
            app.clear("Filter names").unwrap();
            let value = if input == "Filter names" {
                label.trim_end_matches('/').to_owned()
            } else {
                location.clone()
            };
            app.clear(input).unwrap();
            app.type_text(input, &value).unwrap();
            // Keep the input editing: the name click must focus its original checkbox.
            let name = if filesystem { "child" } else { "a-dir/" };
            app.press(&format!("label:Item name {name}")).unwrap();
            assert!(checked(&app, label));
            app.press_key("space").unwrap();
            assert!(
                !checked(&app, label),
                "{input}: Space must toggle the clicked row"
            );
            app.press_key("mod+a").unwrap();
            assert!(
                checked(&app, label),
                "{input}: select all must affect the list"
            );
            app.press_key("mod+i").unwrap();
            assert!(
                !checked(&app, label),
                "{input}: invert must affect the list"
            );
            assert_eq!(
                app.screen().input_text(input).unwrap(),
                Some(value.as_str())
            );
        }
    }
}

#[test]
fn modified_double_clicks_open_the_clicked_directory_in_filesystem_and_archive() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.zip");
    archive(&source);
    fs::create_dir(temp.path().join("child")).unwrap();
    for filesystem in [false, true] {
        let mut app = start();
        let (name, path) = if filesystem {
            folder(&mut app, temp.path(), 2);
            ("child", "Folder path")
        } else {
            load(&mut app, &source);
            ("a-dir/", "Archive path")
        };
        let location = app.screen().input_text(path).unwrap().unwrap().to_owned();
        let requests = app.screen().file_requests().len();
        for modifier in ["shift", "ctrl", "alt", "meta"] {
            for modify_first_click in [true, false] {
                // The pinned Headless driver resets its click counter after 500 ms.
                app.wait(Duration::from_millis(501)).unwrap();
                app.hold_keys(modifier, modify_first_click).unwrap();
                app.press(&format!("label:Item name {name}")).unwrap();
                app.hold_keys(modifier, true).unwrap();
                app.press(&format!("label:Item name {name}")).unwrap();
                app.hold_keys(modifier, false).unwrap();
                if filesystem {
                    app.wait_for("0 of 0 items shown").unwrap();
                    assert_folder(&app, &temp.path().join("child"));
                } else {
                    app.wait_for("role:checkbox[a-dir/a.txt]").unwrap_or_else(|error| {
                        panic!("modifier={modifier}, modify_first_click={modify_first_click}: {error}")
                    });
                    assert_eq!(app.screen().input_text(path).unwrap(), Some("a-dir/"));
                }
                app.press_key("alt+left").unwrap();
                app.wait_for(&format!("label:Item name {name}")).unwrap();
                assert_eq!(
                    app.screen().input_text(path).unwrap(),
                    Some(location.as_str())
                );
            }
        }
        assert_eq!(app.screen().file_requests().len(), requests);
    }
}

#[test]
fn archive_reorder_between_name_clicks_does_not_activate_reused_virtual_slot() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.zip");
    archive(&source);
    let mut app = start();
    load(&mut app, &source);
    let first = app
        .screen()
        .press_target("label:Item name a-dir/")
        .unwrap()
        .unwrap();
    app.press("label:Item name a-dir/").unwrap();
    app.press_key("mod+alt+0").unwrap();
    let second = app
        .screen()
        .press_target("label:Item name z-dir/")
        .unwrap()
        .unwrap();
    assert_eq!(
        first, second,
        "the test must exercise the same virtual row slot"
    );
    app.press("label:Item name z-dir/").unwrap();
    assert_eq!(app.screen().input_text("Archive path").unwrap(), Some("/"));
    assert!(checked(&app, "a-dir/"));
    assert!(checked(&app, "z-dir/"));
}

#[test]
fn filesystem_reorder_between_name_clicks_does_not_open_a_different_directory() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["a-dir", "z-dir"] {
        fs::create_dir(temp.path().join(name)).unwrap();
    }
    let mut app = start();
    folder(&mut app, temp.path(), 2);
    let first = app
        .screen()
        .press_target("label:Item name a-dir")
        .unwrap()
        .unwrap();
    app.press("label:Item name a-dir").unwrap();
    app.press_key("mod+alt+0").unwrap();
    assert_eq!(
        app.screen().press_target("label:Item name z-dir").unwrap(),
        Some(first)
    );
    app.press("label:Item name z-dir").unwrap();
    assert_folder(&app, temp.path());
    assert!(checked(&app, "a-dir/"));
    assert!(checked(&app, "z-dir/"));
}

#[test]
fn refreshed_snapshot_between_clicks_cannot_activate_even_when_name_and_id_match() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("child")).unwrap();
    let mut app = start();
    folder(&mut app, temp.path(), 1);
    app.press("label:Item name child").unwrap();
    assert!(checked(&app, "child/"));
    app.press_key("f5").unwrap();
    app.wait_until(
        "refresh clears the old selection",
        Duration::from_secs(5),
        |screen| {
            screen.shows("0 selected")
                && screen
                    .enabled("role:button[Refresh folder] >> nth=0")
                    .unwrap_or(false)
        },
    )
    .unwrap();
    pointer_second(&mut app, "label:Item name child");
    assert_folder(&app, temp.path());
    assert!(checked(&app, "child/"));
}

#[test]
fn reloaded_archive_snapshot_does_not_pair_clicks_on_the_same_directory_path() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.zip");
    archive(&source);
    let mut app = start();
    load(&mut app, &source);
    app.press("label:Item name a-dir/").unwrap();
    assert!(checked(&app, "a-dir/"));
    app.on_pick_paths(vec![source]).unwrap();
    app.press_key("mod+o").unwrap();
    app.wait_until(
        "new archive snapshot clears selection",
        Duration::from_secs(5),
        |screen| {
            screen.shows("0 selected")
                && screen.enabled("role:button[Test archive]").unwrap_or(false)
        },
    )
    .unwrap();
    pointer_second(&mut app, "label:Item name a-dir/");
    assert_eq!(app.screen().input_text("Archive path").unwrap(), Some("/"));
    assert!(checked(&app, "a-dir/"));
    assert_eq!(app.screen().file_requests().len(), 2);
}

#[test]
fn removed_directory_failure_keeps_the_snapshot_and_allows_another_double_click() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["gone", "keep"] {
        fs::create_dir(temp.path().join(name)).unwrap();
    }
    let mut app = start();
    folder(&mut app, temp.path(), 2);
    app.press("role:checkbox[keep/]").unwrap();
    fs::remove_dir(temp.path().join("gone")).unwrap();
    double_click(&mut app, "gone");
    app.wait_for("text:/^Could not browse item:/").unwrap();
    assert_folder(&app, temp.path());
    assert!(checked(&app, "gone/"));
    assert!(checked(&app, "keep/"));
    assert_eq!(app.screen().count("role:checkbox").unwrap(), 2);
    app.press("role:button[Clear selection]").unwrap();
    double_click(&mut app, "keep");
    app.wait_for("0 items").unwrap();
    assert_folder(&app, &temp.path().join("keep"));
    assert_eq!(app.screen().file_requests().len(), 1);
}

#[test]
fn entering_then_returning_to_the_same_archive_directory_invalidates_the_first_click() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.zip");
    archive(&source);
    let mut app = start();
    load(&mut app, &source);
    let selector = "label:Item name a-dir/";
    app.press(selector).unwrap();
    let target = app.screen().press_target(selector).unwrap().unwrap();
    assert!(checked(&app, "a-dir/"));
    let requests = app.screen().file_requests().len();

    // Navigate entirely by keyboard so no intervening pointer press resets the pair.
    app.press_key("mod+l").unwrap();
    app.clear("Archive path").unwrap();
    app.type_text("Archive path", "a-dir/").unwrap();
    app.press_key("enter").unwrap();
    app.wait_for("role:checkbox[a-dir/a.txt]").unwrap();
    app.press_key("alt+left").unwrap();
    app.wait_for(selector).unwrap();
    assert_eq!(app.screen().input_text("Archive path").unwrap(), Some("/"));
    assert_eq!(app.screen().press_target(selector).unwrap(), Some(target));
    pointer_second(&mut app, selector);
    assert_eq!(app.screen().input_text("Archive path").unwrap(), Some("/"));
    assert!(!checked(&app, "a-dir/"));
    assert_eq!(app.screen().file_requests().len(), requests);
}

#[test]
fn returning_from_flat_view_invalidates_the_old_click_on_the_same_archive_row() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.zip");
    archive(&source);
    let mut app = start();
    load(&mut app, &source);
    let selector = "label:Item name a-dir/";
    app.press(selector).unwrap();
    let target = app.screen().press_target(selector).unwrap().unwrap();
    app.press("role:button[All entries]").unwrap();
    app.wait_for("role:checkbox[a-dir/a.txt]").unwrap();
    app.press("role:button[Folder view]").unwrap();
    assert_eq!(app.screen().press_target(selector).unwrap(), Some(target));
    // Supply the stale pointer count explicitly despite the intermediate toolbar presses.
    pointer_second(&mut app, selector);
    assert_eq!(app.screen().input_text("Archive path").unwrap(), Some("/"));
    assert!(!checked(&app, "a-dir/"));
    assert!(app.screen().matches("role:button[All entries]").unwrap());
}

#[test]
fn short_modal_between_name_clicks_invalidates_double_click_pair_in_both_browsers() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.zip");
    archive(&source);
    fs::create_dir(temp.path().join("child")).unwrap();
    for filesystem in [false, true] {
        let mut app = start();
        let (name, path) = if filesystem {
            folder(&mut app, temp.path(), 2);
            ("child", "Folder path")
        } else {
            load(&mut app, &source);
            ("a-dir/", "Archive path")
        };
        let location = app.screen().input_text(path).unwrap().unwrap().to_owned();
        let selector = format!("label:Item name {name}");
        app.press(&selector).unwrap();
        let target = app.screen().press_target(&selector).unwrap().unwrap();
        // Keyboard shortcuts preserve Headless's last pointer target/click counter.
        app.press_key("mod+n").unwrap();
        app.wait_for("role:button[Cancel]").unwrap();
        app.press_key("escape").unwrap();
        app.wait_for_gone("role:button[Cancel]", Duration::from_secs(2))
            .unwrap();
        assert_eq!(app.screen().press_target(&selector).unwrap(), Some(target));
        pointer_second(&mut app, &selector);
        assert_eq!(
            app.screen().input_text(path).unwrap(),
            Some(location.as_str())
        );
    }
}
