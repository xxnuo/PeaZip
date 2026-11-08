#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use nsh_core::{
    Checked, Command, EventKind, EventPayload, MountItem, NodeId, PayloadValue, ROOT_ID, Size,
    desktop::{DesktopCommand, MenuAction, MenuItem},
    prelude::px,
};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use peazip_core::{Limits, MemoryEntry, ZipCompression, ZipOptions};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn start(width: f32) -> Headless {
    let mut config = EngineConfig::new(Root::of::<peazip_ui::App>());
    config.viewport = Size::new(px(width), px(760.0));
    config.environment.idiom = nsh_engine::Idiom::Desktop;
    config.environment.os = if cfg!(target_os = "macos") {
        nsh_engine::Os::MacOs
    } else if cfg!(target_os = "windows") {
        nsh_engine::Os::Windows
    } else {
        nsh_engine::Os::Linux
    };
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    app.press("role:button[All entries]").unwrap();
    app
}

fn load(app: &mut Headless, source: &Path, count: usize) {
    app.on_pick_paths(vec![source.to_path_buf()]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for(&format!("{count} entries")).unwrap();
}

fn archive() -> Vec<u8> {
    let patterned: Vec<_> = (0..=255).cycle().take(4096).collect();
    let entries = [
        MemoryEntry {
            path: "folder/".into(),
            is_dir: true,
            data: vec![],
        },
        MemoryEntry {
            path: "a.bin".into(),
            is_dir: false,
            data: patterned.clone(),
        },
        MemoryEntry {
            path: "b.bin".into(),
            is_dir: false,
            data: patterned,
        },
        MemoryEntry {
            path: "c.bin".into(),
            is_dir: false,
            data: b"xyz".to_vec(),
        },
        MemoryEntry {
            path: "empty".into(),
            is_dir: false,
            data: vec![],
        },
    ];
    peazip_core::create_zip_with_options(
        &entries,
        Limits::default(),
        ZipOptions {
            compression: ZipCompression::Deflate { level: 6 },
        },
    )
    .unwrap()
}

// Read the generated ZIP's central directory independently of core metadata.
fn zip_packed(bytes: &[u8]) -> BTreeMap<String, u64> {
    let end = bytes
        .windows(4)
        .rposition(|part| part == b"PK\x05\x06")
        .unwrap();
    let count = u16::from_le_bytes(bytes[end + 10..end + 12].try_into().unwrap());
    let mut at = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap()) as usize;
    let mut sizes = BTreeMap::new();
    for _ in 0..count {
        assert_eq!(&bytes[at..at + 4], b"PK\x01\x02");
        let packed = u32::from_le_bytes(bytes[at + 20..at + 24].try_into().unwrap());
        let len = u16::from_le_bytes(bytes[at + 28..at + 30].try_into().unwrap()) as usize;
        let extra = u16::from_le_bytes(bytes[at + 30..at + 32].try_into().unwrap()) as usize;
        let comment = u16::from_le_bytes(bytes[at + 32..at + 34].try_into().unwrap()) as usize;
        let name = std::str::from_utf8(&bytes[at + 46..at + 46 + len])
            .unwrap()
            .to_owned();
        sizes.insert(name, u64::from(packed));
        at += 46 + len + extra + comment;
    }
    assert_eq!(at, end);
    sizes
}

fn action(app: &Headless, label: &str) -> MenuAction {
    for (_, command) in app.screen().commands().iter().rev() {
        if let Command::Desktop(command) = command
            && let DesktopCommand::SetMenu(Some(menu)) = &**command
        {
            let mut found = None;
            MenuItem::each_action(menu, &mut |item| {
                if item.label == label {
                    found = Some(item.clone());
                }
            });
            return found.unwrap_or_else(|| panic!("missing native menu action {label}"));
        }
    }
    panic!("application menu is missing");
}

fn choose(app: &mut Headless, label: &str) {
    let item = action(app, label);
    assert!(item.enabled, "{label} is disabled");
    app.engine().control().dispatch_event(
        ROOT_ID,
        EventKind::Menu,
        EventPayload::Map(vec![("id", PayloadValue::Number(item.id as f64))]),
    );
    app.next_change(Duration::from_secs(1)).unwrap();
}

fn wait_order(app: &mut Headless, expected: &[&str]) {
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

fn assert_cells(app: &Headless, entry: &peazip_core::Entry, packed: Option<u64>) {
    let row = format!("role:checkbox[{}]", entry.path);
    let packed_label = if entry.is_dir {
        "Packed bytes: not applicable".to_owned()
    } else {
        packed
            .map(|value| format!("Packed bytes: {value}"))
            .unwrap_or_else(|| "Packed bytes: unavailable".into())
    };
    let size_label = if entry.is_dir {
        "Size in bytes: not applicable".to_owned()
    } else {
        format!("Size in bytes: {}", entry.size)
    };
    assert!(
        app.screen()
            .matches(&format!("{row} >> label:{packed_label}"))
            .unwrap()
    );
    assert!(
        app.screen()
            .matches(&format!("{row} >> label:{size_label}"))
            .unwrap()
    );
    let value = if entry.is_dir {
        "—".to_owned()
    } else {
        packed
            .map(|value| value.to_string())
            .unwrap_or_else(|| "—".into())
    };
    assert!(
        app.screen()
            .matches(&format!("{row} >> text:{value}"))
            .unwrap()
    );
}

#[test]
fn zip_packed_cells_match_real_compressed_output_and_directories_show_unknown() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.zip");
    let bytes = archive();
    let actual = zip_packed(&bytes);
    std::fs::write(&source, &bytes).unwrap();
    let entries = peazip_core::list_archive(&bytes, Limits::default()).unwrap();
    let mut app = start(1024.0);
    load(&mut app, &source, entries.len());
    app.wait_for("role:button[Packed (bytes)]").unwrap();
    for entry in entries {
        if entry.is_dir {
            assert_cells(&app, &entry, None);
        } else {
            assert_eq!(entry.packed_size, Some(actual[&entry.path]));
            assert_cells(&app, &entry, Some(actual[&entry.path]));
        }
    }
    assert!(actual["a.bin"] < 4096);
    assert_eq!(actual["a.bin"], actual["b.bin"]);
}

#[test]
fn tar_packed_cells_use_512_byte_data_blocks_while_tgz_is_unknown() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = start(1024.0);
    for (name, bytes, packed) in [
        (
            "sample.tar",
            include_bytes!("../../../tests/fixtures/sample.tar").as_slice(),
            Some(512),
        ),
        (
            "sample.tar.gz",
            include_bytes!("../../../tests/fixtures/sample.tar.gz").as_slice(),
            None,
        ),
    ] {
        let source = temp.path().join(name);
        std::fs::write(&source, bytes).unwrap();
        load(&mut app, &source, 5);
        let entries = peazip_core::list_archive(bytes, Limits::default()).unwrap();
        for entry in entries.iter().filter(|entry| !entry.is_dir) {
            assert_eq!(entry.packed_size, packed);
            assert_cells(&app, entry, packed);
        }
    }
}

#[test]
fn solid_7z_is_unknown_and_non_solid_copy_streams_have_real_packed_sizes() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = start(1024.0);
    for (name, bytes, solid) in [
        (
            "solid.7z",
            include_bytes!("../../../crates/peazip-core/tests/fixtures/7z-lzma2-on.7z").as_slice(),
            true,
        ),
        (
            "copy.7z",
            include_bytes!("../../../crates/peazip-core/tests/fixtures/7z-copy.7z").as_slice(),
            false,
        ),
    ] {
        let source = temp.path().join(name);
        std::fs::write(&source, bytes).unwrap();
        let entries = peazip_core::list_archive(bytes, Limits::default()).unwrap();
        load(&mut app, &source, entries.len());
        for entry in entries
            .iter()
            .filter(|entry| !entry.is_dir && entry.size > 0)
        {
            let expected = if solid { None } else { Some(entry.size) };
            assert_eq!(entry.packed_size, expected);
            assert_cells(&app, entry, expected);
        }
    }
}

#[test]
fn packed_sort_keeps_folders_first_ties_by_name_and_preserves_selection() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.zip");
    let bytes = archive();
    std::fs::write(&source, &bytes).unwrap();
    let mut app = start(1024.0);
    load(&mut app, &source, 5);
    app.press("role:checkbox[b.bin]").unwrap();
    let requests = app.screen().file_requests().len();
    app.press("role:button[Packed (bytes)]").unwrap();
    wait_order(&mut app, &["folder/", "empty", "c.bin", "a.bin", "b.bin"]);
    app.press("role:button[Packed (bytes)]").unwrap();
    wait_order(&mut app, &["folder/", "a.bin", "b.bin", "c.bin", "empty"]);
    assert_eq!(
        app.screen()
            .accessibility_state("role:checkbox[b.bin]")
            .unwrap()
            .checked,
        Some(Checked::True)
    );
    assert!(app.screen().shows("1 selected"));
    assert!(!app.screen().enabled("role:button[Back]").unwrap());
    assert!(!app.screen().enabled("role:button[Forward]").unwrap());
    assert_eq!(app.screen().file_requests().len(), requests);
    assert_eq!(std::fs::read(source).unwrap(), bytes);
}

#[test]
fn unknown_sizes_stay_last_in_both_directions_and_sorting_keeps_directory_history() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("solid.7z");
    std::fs::write(
        &source,
        include_bytes!("../../../crates/peazip-core/tests/fixtures/7z-lzma2-on.7z"),
    )
    .unwrap();
    let mut app = start(1024.0);
    load(&mut app, &source, 6);
    for _ in 0..2 {
        app.press("role:button[Packed (bytes)]").unwrap();
        wait_order(
            &mut app,
            &[
                "empty-dir/",
                "folder/",
                "empty",
                "binary",
                "folder/second.bin",
                "folder/中文.txt",
            ],
        );
    }
    let tar = temp.path().join("sample.tar");
    std::fs::write(&tar, include_bytes!("../../../tests/fixtures/sample.tar")).unwrap();
    load(&mut app, &tar, 5);
    app.press("role:button[Folder view]").unwrap();
    app.press("role:button[Open folder docs/]").unwrap();
    app.press("role:checkbox[docs/你好.txt]").unwrap();
    for _ in 0..2 {
        app.press("role:button[Packed (bytes)]").unwrap();
        assert_eq!(
            app.screen().input_text("Archive path").unwrap(),
            Some("docs/")
        );
    }
    app.press("role:button[Back]").unwrap();
    app.wait_for("role:checkbox[other.txt]").unwrap();
    assert_eq!(app.screen().input_text("Archive path").unwrap(), Some("/"));
    app.press("role:button[Forward]").unwrap();
    app.wait_for("role:checkbox[docs/你好.txt]").unwrap();
    assert_eq!(
        app.screen()
            .accessibility_state("role:checkbox[docs/你好.txt]")
            .unwrap()
            .checked,
        Some(Checked::True)
    );
}

#[test]
fn entering_a_filesystem_folder_resets_packed_sort_and_disables_its_menu_action() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.zip");
    std::fs::write(&source, archive()).unwrap();
    let folder = temp.path().join("files");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("a.txt"), b"a").unwrap();
    std::fs::write(folder.join("z.txt"), b"z").unwrap();
    let mut app = start(1024.0);
    load(&mut app, &source, 5);
    choose(&mut app, "Sort by packed size");
    assert_eq!(action(&app, "Sort by packed size").checked, Some(true));
    app.on_pick_paths(vec![folder]).unwrap();
    app.press("role:button[Open folder…]").unwrap();
    app.wait_for("2 items").unwrap();
    assert!(!action(&app, "Sort by packed size").enabled);
    assert_eq!(action(&app, "Sort by name").checked, Some(true));
    assert!(!app.screen().matches("role:button[Packed (bytes)]").unwrap());
    wait_order(&mut app, &["a.txt", "z.txt"]);
}

#[test]
fn narrow_packed_sort_swaps_columns_without_squeezing_names_or_losing_selection() {
    #[derive(Default)]
    struct Seen {
        widths: BTreeMap<NodeId, f32>,
        texts: BTreeMap<NodeId, String>,
    }
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.zip");
    std::fs::write(&source, archive()).unwrap();
    let mut app = start(360.0);
    let seen = Arc::new(Mutex::new(Seen::default()));
    let captured = seen.clone();
    app.observe(move |_, batch| {
        let mut seen = captured.lock().unwrap();
        for item in &batch.items {
            match item {
                MountItem::Frame { id, frame } => {
                    seen.widths.insert(*id, frame.size.width.0);
                }
                MountItem::Text { id, text, .. } => {
                    seen.texts.insert(*id, text.text.clone());
                }
                _ => {}
            }
        }
    });
    load(&mut app, &source, 5);
    assert!(app.screen().matches("role:button[Size (bytes)]").unwrap());
    assert!(!app.screen().matches("role:button[Packed (bytes)]").unwrap());
    app.press("role:checkbox[a.bin]").unwrap();
    for restore in ["Sort by name", "Sort by size"] {
        choose(&mut app, "Sort by packed size");
        app.wait_for("role:button[Packed (bytes)]").unwrap();
        assert!(!app.screen().matches("role:button[Size (bytes)]").unwrap());
        for name in ["folder/", "a.bin", "b.bin", "c.bin", "empty"] {
            assert!(
                app.screen().shows(name),
                "the name {name} must remain readable"
            );
            let seen = seen.lock().unwrap();
            let live: Vec<_> = seen
                .texts
                .iter()
                .filter(|(id, text)| {
                    text.as_str() == name && app.screen().matches(&format!("view:{id}")).unwrap()
                })
                .map(|(id, _)| *id)
                .collect();
            assert_eq!(live.len(), 1, "missing or duplicate text node for {name}");
            assert!(
                seen.widths[&live[0]] >= 64.0,
                "packed sorting squeezed the name {name}"
            );
            let row = app
                .screen()
                .press_target(&format!("role:checkbox[{name}]"))
                .unwrap()
                .unwrap();
            assert!(
                seen.widths[&row] <= 360.0,
                "the row must fit in the viewport"
            );
        }
        assert_eq!(
            app.screen()
                .accessibility_state("role:checkbox[a.bin]")
                .unwrap()
                .checked,
            Some(Checked::True)
        );
        choose(&mut app, restore);
        app.wait_for("role:button[Size (bytes)]").unwrap();
        assert!(!app.screen().matches("role:button[Packed (bytes)]").unwrap());
    }
}
