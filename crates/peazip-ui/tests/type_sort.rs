#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use nsh_core::{Checked, Size, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use peazip_core::{Limits, MemoryEntry};
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

fn start(width: f32) -> Headless {
    let mut config = EngineConfig::new(Root::of::<peazip_ui::App>());
    config.viewport = Size::new(px(width), px(900.0));
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    app
}

fn fixture(path: &Path) -> Vec<u8> {
    let entries: Vec<_> = [
        ("a.dir/", b"".as_slice()),
        ("parent.v1/", b""),
        ("z.dir/", b""),
        ("a.bin", b"binary"),
        ("b.TxT", b"upper"),
        ("c.txt", b"selected text"),
        ("parent.v1/README", b"selected readme"),
        ("plain", b"no suffix"),
    ]
    .into_iter()
    .map(|(path, data)| MemoryEntry {
        path: path.into(),
        is_dir: path.ends_with('/'),
        data: data.into(),
    })
    .collect();
    let bytes = peazip_core::create_zip(&entries, Limits::default()).unwrap();
    std::fs::write(path, &bytes).unwrap();
    bytes
}

fn load(app: &mut Headless, path: &Path) {
    app.press("role:button[All entries]").unwrap();
    app.on_pick_paths(vec![path.to_path_buf()]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("8 entries").unwrap();
}

fn checked(app: &Headless, name: &str) -> bool {
    app.screen()
        .accessibility_state(&format!("role:checkbox[{name}]"))
        .unwrap()
        .checked
        == Some(Checked::True)
}

fn order(app: &mut Headless, expected: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let names: Vec<_> = app
            .screen()
            .texts()
            .into_iter()
            .filter(|name| expected.contains(name))
            .collect();
        if names == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "expected {expected:?}, received {names:?}"
        );
        app.next_change(Duration::from_millis(25)).unwrap();
    }
}

#[test]
fn archive_type_header_sorts_displayed_labels_and_keeps_real_selected_contents() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("types.zip");
    let original = fixture(&source);
    let mut app = start(1024.0);
    load(&mut app, &source);
    for (name, kind) in [
        ("a.dir/", "Folder"),
        ("parent.v1/", "Folder"),
        ("z.dir/", "Folder"),
        ("a.bin", "BIN"),
        ("b.TxT", "TXT"),
        ("c.txt", "TXT"),
        ("parent.v1/README", "File"),
        ("plain", "File"),
    ] {
        assert!(
            app.screen()
                .matches(&format!("role:checkbox[{name}] >> text:{kind}"))
                .unwrap()
        );
    }
    app.press("role:checkbox[c.txt]").unwrap();
    app.press("role:checkbox[parent.v1/README]").unwrap();
    let requests = app.screen().file_requests().len();
    app.press("role:button[Type]").unwrap();
    order(
        &mut app,
        &[
            "a.dir/",
            "parent.v1/",
            "z.dir/",
            "a.bin",
            "parent.v1/README",
            "plain",
            "b.TxT",
            "c.txt",
        ],
    );
    app.press("role:button[Type]").unwrap();
    order(
        &mut app,
        &[
            "a.dir/",
            "parent.v1/",
            "z.dir/",
            "b.TxT",
            "c.txt",
            "parent.v1/README",
            "plain",
            "a.bin",
        ],
    );
    assert!(checked(&app, "c.txt"));
    assert!(checked(&app, "parent.v1/README"));
    assert!(!checked(&app, "b.TxT"));
    assert!(!checked(&app, "plain"));
    assert!(app.screen().shows("2 selected"));
    assert_eq!(app.screen().file_requests().len(), requests);
    let output = temp.path().join("selected");
    app.on_pick_save_path(Some(output.clone())).unwrap();
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
    app.wait_for("Extracted 2 entries").unwrap();
    assert_eq!(
        std::fs::read(output.join("c.txt")).unwrap(),
        b"selected text"
    );
    assert_eq!(
        std::fs::read(output.join("parent.v1/README")).unwrap(),
        b"selected readme"
    );
    for name in ["a.bin", "b.TxT", "plain", "a.dir", "z.dir"] {
        assert!(!output.join(name).exists());
    }
    assert_eq!(std::fs::read(source).unwrap(), original);
}

#[test]
fn narrow_view_hides_type_header_and_keeps_names_and_selection_readable() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("types.zip");
    fixture(&source);
    let mut app = start(360.0);
    load(&mut app, &source);
    assert!(!app.screen().matches("role:button[Type]").unwrap());
    assert!(app.screen().matches("role:button[Name]").unwrap());
    assert!(app.screen().matches("role:button[Size (bytes)]").unwrap());
    for name in ["a.bin", "b.TxT", "c.txt", "plain", "parent.v1/README"] {
        assert!(app.screen().shows(name));
        assert!(
            app.screen()
                .matches(&format!("role:checkbox[{name}]"))
                .unwrap()
        );
    }
    app.press("role:checkbox[c.txt]").unwrap();
    assert!(checked(&app, "c.txt"));
    assert!(app.screen().shows("1 selected"));
}

#[test]
fn filesystem_types_sort_without_changing_selected_ids_or_zip_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir_all(source.join("folder")).unwrap();
    std::fs::write(source.join("a.txt"), b"first").unwrap();
    std::fs::write(source.join("z.bin"), b"last bytes").unwrap();
    let outside = temp.path().join("outside.txt");
    std::fs::write(&outside, b"outside sentinel").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, source.join("link")).unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(source.join("pipe"))
                .status()
                .unwrap()
                .success()
        );
    }
    let mut app = start(1024.0);
    app.on_pick_paths(vec![source]).unwrap();
    app.press("role:button[Open folder…]").unwrap();
    app.wait_for(if cfg!(unix) { "5 items" } else { "3 items" })
        .unwrap();
    for name in ["a.txt", "z.bin"] {
        assert!(
            app.screen()
                .matches(&format!("role:checkbox[{name}] >> text:File"))
                .unwrap()
        );
        app.press(&format!("role:checkbox[{name}]")).unwrap();
    }
    #[cfg(unix)]
    {
        assert!(
            app.screen()
                .matches("role:checkbox[link] >> text:Link")
                .unwrap()
        );
        assert!(
            app.screen()
                .matches("role:checkbox[pipe] >> text:Other")
                .unwrap()
        );
    }
    app.press("role:button[Type]").unwrap();
    #[cfg(unix)]
    order(&mut app, &["folder", "a.txt", "z.bin", "link", "pipe"]);
    #[cfg(not(unix))]
    order(&mut app, &["folder", "a.txt", "z.bin"]);
    app.press("role:button[Type]").unwrap();
    #[cfg(unix)]
    order(&mut app, &["folder", "pipe", "link", "a.txt", "z.bin"]);
    #[cfg(not(unix))]
    order(&mut app, &["folder", "a.txt", "z.bin"]);
    assert!(checked(&app, "a.txt"));
    assert!(checked(&app, "z.bin"));
    assert!(!checked(&app, "folder/"));
    #[cfg(unix)]
    assert!(!checked(&app, "link"));
    let requests = app.screen().file_requests().len();
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
            ("a.txt".into(), b"first".to_vec()),
            ("z.bin".into(), b"last bytes".to_vec())
        ])
    );
    assert_eq!(app.screen().file_requests().len(), requests + 1);
    assert_eq!(std::fs::read(outside).unwrap(), b"outside sentinel");
}
