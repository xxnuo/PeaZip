#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use nsh_core::{
    Checked, Command, EventKind, EventPayload, MountItem, PayloadValue, ROOT_ID, Size,
    desktop::{DesktopCommand, MenuAction, MenuItem},
    prelude::px,
};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use peazip_core::{Limits, MemoryEntry};
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const SORTS: [(&str, &str); 4] = [
    ("mod+alt+0", "Sort by name"),
    ("mod+alt+1", "Sort by type"),
    ("mod+alt+2", "Sort by size"),
    ("mod+alt+3", "Sort by packed size"),
];

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
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Open an archive to view its contents.")
        .unwrap();
    app
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
            return found.unwrap_or_else(|| panic!("missing menu action {label}"));
        }
    }
    panic!("missing application menu");
}

fn choose(app: &mut Headless, label: &str) {
    let item = action(app, label);
    assert!(item.enabled);
    app.engine().control().dispatch_event(
        ROOT_ID,
        EventKind::Menu,
        EventPayload::Map(vec![("id", PayloadValue::Number(item.id as f64))]),
    );
    app.next_change(Duration::from_secs(1)).unwrap();
}

fn order(app: &mut Headless, expected: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let actual: Vec<_> = app
            .screen()
            .texts()
            .into_iter()
            .filter(|name| expected.contains(name))
            .collect();
        if actual == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "expected {expected:?}, got {actual:?}"
        );
        app.next_change(Duration::from_millis(25)).unwrap();
    }
}

fn fixture(path: &Path) -> Vec<u8> {
    let entries: Vec<_> = [("a.txt", "123456789"), ("b.bin", "123"), ("c", "123456")]
        .into_iter()
        .map(|(path, data)| MemoryEntry {
            path: path.into(),
            is_dir: false,
            data: data.as_bytes().into(),
        })
        .collect();
    let bytes = peazip_core::create_zip(&entries, Limits::default()).unwrap();
    fs::write(path, &bytes).unwrap();
    bytes
}

fn load(app: &mut Headless, path: &Path) {
    app.on_pick_paths(vec![path.to_path_buf()]).unwrap();
    app.press("role:button[Open archive…]").unwrap();
    app.wait_for("3 entries").unwrap();
}

fn checked(app: &Headless, name: &str) -> bool {
    app.screen()
        .accessibility_state(&format!("role:checkbox[{name}]"))
        .unwrap()
        .checked
        == Some(Checked::True)
}

#[test]
fn archive_shortcuts_sort_hidden_columns_toggle_direction_and_preserve_contents() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("sort.zip");
    let original = fixture(&source);
    let mut app = start();
    load(&mut app, &source);
    app.press("role:checkbox[a.txt]").unwrap();
    app.clear("Filter names").unwrap();
    app.type_text("Filter names", "b").unwrap();
    app.wait_for("1 of 3 rows shown").unwrap();
    for label in ["Show type column", "Show size column", "Show packed column"] {
        choose(&mut app, label);
        assert_eq!(action(&app, label).checked, Some(false));
    }
    for header in ["Type", "Size (bytes)", "Packed (bytes)"] {
        assert!(
            !app.screen()
                .matches(&format!("role:button[{header}]"))
                .unwrap()
        );
    }
    let requests = app.screen().file_requests().len();
    for (key, label) in SORTS {
        app.press_key(key).unwrap();
        assert_eq!(action(&app, label).checked, Some(true));
        assert!(action(&app, label).shortcut.is_none());
        assert_eq!(app.screen().input_text("Filter names").unwrap(), Some("b"));
        assert!(app.screen().shows("1 selected"));
    }
    app.clear("Filter names").unwrap();
    app.press("role:button[Go]").unwrap();
    let cases = [
        (
            "mod+alt+1",
            ["b.bin", "c", "a.txt"],
            ["a.txt", "c", "b.bin"],
        ),
        (
            "mod+alt+2",
            ["b.bin", "c", "a.txt"],
            ["a.txt", "c", "b.bin"],
        ),
        (
            "mod+alt+3",
            ["b.bin", "c", "a.txt"],
            ["a.txt", "c", "b.bin"],
        ),
        (
            "mod+alt+0",
            ["a.txt", "b.bin", "c"],
            ["c", "b.bin", "a.txt"],
        ),
    ];
    for (key, ascending, descending) in cases {
        app.press_key(key).unwrap();
        order(&mut app, &ascending);
        app.press_key(key).unwrap();
        order(&mut app, &descending);
    }
    assert!(checked(&app, "a.txt"));
    assert!(!checked(&app, "b.bin"));
    assert!(!checked(&app, "c"));
    assert_eq!(app.screen().file_requests().len(), requests);
    assert_eq!(fs::read(source).unwrap(), original);
}

#[test]
fn filesystem_type_and_size_work_but_packed_does_not_change_sort() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("folder")).unwrap();
    fs::write(temp.path().join("a.txt"), b"longer").unwrap();
    fs::write(temp.path().join("z.bin"), b"x").unwrap();
    let mut app = start();
    app.on_pick_paths(vec![temp.path().to_path_buf()]).unwrap();
    app.press("role:button[Open folder…]").unwrap();
    app.wait_for("3 items").unwrap();
    app.press("role:checkbox[a.txt]").unwrap();
    app.press_key("mod+alt+1").unwrap();
    assert_eq!(action(&app, "Sort by type").checked, Some(true));
    order(&mut app, &["folder", "a.txt", "z.bin"]);
    app.press_key("mod+alt+2").unwrap();
    order(&mut app, &["folder", "z.bin", "a.txt"]);
    app.press_key("mod+alt+2").unwrap();
    order(&mut app, &["folder", "a.txt", "z.bin"]);
    let requests = app.screen().file_requests().len();
    assert!(!action(&app, "Sort by packed size").enabled);
    app.press_key("mod+alt+3").unwrap();
    assert_eq!(action(&app, "Sort by size").checked, Some(true));
    assert_eq!(action(&app, "Sort by packed size").checked, Some(false));
    order(&mut app, &["folder", "a.txt", "z.bin"]);
    // A further size shortcut must toggle the previous direction, not start over.
    app.press_key("mod+alt+2").unwrap();
    order(&mut app, &["folder", "z.bin", "a.txt"]);
    assert!(checked(&app, "a.txt"));
    assert_eq!(app.screen().file_requests().len(), requests);
    assert_eq!(fs::read(temp.path().join("a.txt")).unwrap(), b"longer");
    assert_eq!(fs::read(temp.path().join("z.bin")).unwrap(), b"x");
}

#[test]
fn editing_modals_and_busy_picker_block_sort_shortcuts() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("sort.zip");
    let original = fixture(&source);
    let mut app = start();
    load(&mut app, &source);
    let requests = app.screen().file_requests().len();
    for (input, value) in [("Archive path", "/"), ("Filter names", "")] {
        app.clear(input).unwrap();
        if !value.is_empty() {
            app.type_text(input, value).unwrap();
        }
        for (key, _) in SORTS {
            app.press_key(key).unwrap();
        }
        assert_eq!(app.screen().input_text(input).unwrap(), Some(value));
        assert_eq!(action(&app, "Sort by name").checked, Some(true));
        app.press_key("x").unwrap();
        assert_eq!(
            app.screen().input_text(input).unwrap(),
            Some(format!("{value}x").as_str())
        );
        app.clear(input).unwrap();
        if !value.is_empty() {
            app.type_text(input, value).unwrap();
        }
    }
    app.press("role:button[Go]").unwrap();
    for button in ["Create archive…", "Extract…"] {
        app.press(&format!("role:button[{button}]")).unwrap();
        app.wait_for("role:button[Continue]").unwrap();
        for (key, label) in SORTS {
            assert!(!action(&app, label).enabled);
            app.press_key(key).unwrap();
        }
        app.press("role:button[Cancel]").unwrap();
        app.wait_for("role:button[Name]").unwrap();
        order(&mut app, &["a.txt", "b.bin", "c"]);
    }
    assert_eq!(app.screen().file_requests().len(), requests);
    let ids: Vec<_> = SORTS
        .iter()
        .map(|(_, label)| action(&app, label).id)
        .collect();
    let attempted = Arc::new(AtomicBool::new(false));
    let probe = Arc::clone(&attempted);
    let control = app.engine().control();
    app.observe(move |_, batch| {
        for item in &batch.items {
            if let MountItem::Command {
                command: Command::Desktop(command),
                ..
            } = item
                && let DesktopCommand::SetMenu(Some(menu)) = &**command
            {
                MenuItem::each_action(menu, &mut |action| {
                    if action.label == "Sort by name"
                        && !action.enabled
                        && !probe.swap(true, Ordering::SeqCst)
                    {
                        // Queue the shared sort commands before Headless answers the busy picker.
                        for id in &ids {
                            control.dispatch_event(
                                ROOT_ID,
                                EventKind::Menu,
                                EventPayload::Map(vec![("id", PayloadValue::Number(*id as f64))]),
                            );
                        }
                    }
                });
            }
        }
    });
    app.on_pick_paths(vec![]).unwrap();
    app.press_key("mod+o").unwrap();
    app.wait_for("role:button[Name]").unwrap();
    assert!(attempted.load(Ordering::SeqCst));
    assert_eq!(action(&app, "Sort by name").checked, Some(true));
    order(&mut app, &["a.txt", "b.bin", "c"]);
    assert_eq!(app.screen().file_requests().len(), requests + 1);
    assert_eq!(fs::read(source).unwrap(), original);
}
