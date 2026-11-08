#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use nsh_core::{
    Accessibility, Bounds, Checked, Command, EventKind, EventPayload, MountItem, NodeId,
    PayloadValue, Points, ROOT_ID, Size,
    desktop::{DesktopCommand, MenuAction, MenuItem},
    prelude::px,
};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Default)]
struct Seen {
    accessibility: BTreeMap<NodeId, Accessibility>,
    parents: BTreeMap<NodeId, NodeId>,
    frames: BTreeMap<NodeId, Bounds<Points>>,
    text: BTreeMap<NodeId, String>,
    horizontal: BTreeMap<NodeId, bool>,
    probe_blocked_adjust: bool,
    blocked_adjust_attempts: usize,
    probe_blocked_visibility: bool,
    blocked_visibility_attempts: usize,
}

struct App {
    app: Headless,
    seen: Arc<Mutex<Seen>>,
}

impl App {
    fn open(path: &Path, count: usize) -> Self {
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
        let seen = Arc::new(Mutex::new(Seen::default()));
        let capture = Arc::clone(&seen);
        let control = app.engine().control();
        app.observe(move |_, batch| {
            let mut seen = capture.lock().unwrap();
            for item in &batch.items {
                match item {
                    MountItem::ViewProps { id, props } => {
                        seen.accessibility.insert(*id, props.accessibility.clone());
                    }
                    MountItem::AccessibilityLabel { id, label } => {
                        seen.accessibility.entry(*id).or_default().label = label.clone();
                    }
                    MountItem::Frame { id, frame } => {
                        seen.frames.insert(*id, *frame);
                    }
                    MountItem::Insert { parent, child, .. } => {
                        seen.parents.insert(*child, *parent);
                    }
                    MountItem::Text { id, text, .. } => {
                        seen.text.insert(*id, text.text.clone());
                    }
                    MountItem::ScrollConfig { id, horizontal, .. } => {
                        seen.horizontal.insert(*id, *horizontal);
                    }
                    MountItem::Delete { id } => {
                        seen.accessibility.remove(id);
                        seen.frames.remove(id);
                        seen.parents.remove(id);
                        seen.text.remove(id);
                        seen.horizontal.remove(id);
                    }
                    MountItem::Command {
                        command: Command::Desktop(command),
                        ..
                    } if seen.probe_blocked_adjust || seen.probe_blocked_visibility => {
                        if let DesktopCommand::SetMenu(Some(menu)) = &**command {
                            MenuItem::each_action(menu, &mut |action| {
                                if action.label == "Auto adjust columns" && !action.enabled {
                                    if !seen.probe_blocked_adjust {
                                        return;
                                    }
                                    seen.probe_blocked_adjust = false;
                                    seen.blocked_adjust_attempts += 1;
                                    // The public native menu event is queued before Headless answers the busy picker.
                                    control.dispatch_event(
                                        ROOT_ID,
                                        EventKind::Menu,
                                        EventPayload::Map(vec![(
                                            "id",
                                            PayloadValue::Number(action.id as f64),
                                        )]),
                                    );
                                }
                                if action.label == "Show size column"
                                    && !action.enabled
                                    && seen.probe_blocked_visibility
                                {
                                    seen.probe_blocked_visibility = false;
                                    seen.blocked_visibility_attempts += 1;
                                    control.dispatch_event(
                                        ROOT_ID,
                                        EventKind::Menu,
                                        EventPayload::Map(vec![(
                                            "id",
                                            PayloadValue::Number(action.id as f64),
                                        )]),
                                    );
                                }
                            });
                        }
                    }
                    _ => {}
                }
            }
        });
        app.wait_for("Open an archive to view its contents.")
            .unwrap();
        app.on_pick_paths(vec![path.to_path_buf()]).unwrap();
        app.press("role:button[Open folder…]").unwrap();
        app.wait_for(&format!("{count} of {count} items shown"))
            .unwrap();
        let mut result = Self { app, seen };
        result.until(|app| app.now("Resize Name column") > 160.0);
        result
    }

    fn until(&mut self, predicate: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !predicate(self) {
            assert!(
                Instant::now() < deadline,
                "column mount/value did not settle"
            );
            self.app.next_change(Duration::from_millis(20)).unwrap();
        }
    }

    fn id(&self, label: &str) -> NodeId {
        let seen = self.seen.lock().unwrap();
        let ids: Vec<_> = seen
            .accessibility
            .iter()
            .filter(|(id, props)| {
                props.label.as_deref() == Some(label)
                    && self.app.screen().matches(&format!("view:{id}")).unwrap()
            })
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(ids.len(), 1, "{label}: {ids:?}");
        ids[0]
    }

    fn now(&self, label: &str) -> f32 {
        let id = self.id(label);
        self.seen.lock().unwrap().accessibility[&id]
            .value
            .as_ref()
            .unwrap()
            .now
            .unwrap()
    }

    fn action(&mut self, label: &str, action: &str) {
        let id = self.id(label);
        self.send(
            id,
            EventKind::AccessibilityAction,
            EventPayload::Map(vec![("actionName", PayloadValue::Str(action.into()))]),
        );
    }

    fn send(&mut self, id: NodeId, kind: EventKind, payload: EventPayload) {
        self.app
            .engine()
            .control()
            .dispatch_event(id, kind, payload);
        self.app.next_change(Duration::from_millis(100)).unwrap();
    }

    fn checked(&self, name: &str) -> bool {
        self.app
            .screen()
            .accessibility_state(&format!("role:checkbox[{name}]"))
            .unwrap()
            .checked
            == Some(Checked::True)
    }

    fn horizontal_ancestor(&self, mut id: NodeId) -> NodeId {
        let seen = self.seen.lock().unwrap();
        for _ in 0..128 {
            if seen.horizontal.get(&id) == Some(&true) {
                return id;
            }
            id = *seen
                .parents
                .get(&id)
                .expect("no shared horizontal scroll ancestor");
        }
        panic!("cyclic mount ancestry");
    }

    fn text_id(&self, value: &str, parent: &str) -> NodeId {
        let seen = self.seen.lock().unwrap();
        let ids: Vec<_> = seen
            .text
            .iter()
            .filter(|(id, text)| {
                text.as_str() == value
                    && self
                        .app
                        .screen()
                        .matches(&format!("{parent} >> view:{id}"))
                        .unwrap()
            })
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(ids.len(), 1, "text {value} in {parent}: {ids:?}");
        ids[0]
    }

    fn x_in_table(&self, mut id: NodeId) -> f32 {
        let table = self.horizontal_ancestor(id);
        let seen = self.seen.lock().unwrap();
        let mut x = 0.0;
        while id != table {
            x += seen.frames.get(&id).map_or(0.0, |frame| frame.origin.x.0);
            id = seen.parents[&id];
        }
        x
    }

    fn assert_aligned(&self, row: &str, size: &str, type_label: &str) {
        for (label, value) in [("Type", type_label), ("Size (bytes)", size)] {
            let handle = self.id(&format!("Resize {label} column"));
            let header = self.seen.lock().unwrap().parents[&handle];
            let body = self.text_id(value, &format!("role:checkbox[{row}]"));
            assert_eq!(
                self.horizontal_ancestor(header),
                self.horizontal_ancestor(body)
            );
            let seen = self.seen.lock().unwrap();
            let header_width = seen.frames[&header].size.width.0;
            let body_width = seen.frames[&body].size.width.0;
            drop(seen);
            assert!(
                (header_width - body_width).abs() < 0.5,
                "{label} header={header_width}, cell={body_width}"
            );
            assert!(
                (self.x_in_table(header) - self.x_in_table(body)).abs() < 0.5,
                "{label} column edges differ"
            );
        }
    }

    fn menu_action(&self, group: &str, label: &str) -> MenuAction {
        for (_, command) in self.app.screen().commands().iter().rev() {
            if let Command::Desktop(command) = command
                && let DesktopCommand::SetMenu(Some(menu)) = &**command
            {
                let items = menu
                    .iter()
                    .find_map(|item| match item {
                        MenuItem::Submenu { label, items, .. } if label == group => Some(items),
                        _ => None,
                    })
                    .expect("missing native menu group");
                let mut found = None;
                MenuItem::each_action(items, &mut |action| {
                    if action.label == label {
                        found = Some(action.clone());
                    }
                });
                return found.unwrap_or_else(|| panic!("missing menu action {group}/{label}"));
            }
        }
        panic!("native application menu is missing");
    }

    fn choose(&mut self, group: &str, label: &str) {
        let action = self.menu_action(group, label);
        assert!(action.enabled, "{group}/{label} is disabled");
        self.send(
            ROOT_ID,
            EventKind::Menu,
            EventPayload::Map(vec![("id", PayloadValue::Number(action.id as f64))]),
        );
    }

    fn assert_adjusted(&mut self, packed: bool) {
        self.until(|app| {
            [
                ("Resize Name column", 240.0),
                ("Resize Type column", 72.0),
                ("Resize Size (bytes) column", 72.0),
            ]
            .into_iter()
            .all(|(label, width)| app.now(label) == width)
                && (!packed || app.now("Resize Packed (bytes) column") == 72.0)
        });
    }
}

#[test]
fn filesystem_accessibility_resize_aligns_cells_and_preserves_hidden_selection_sort_and_real_bytes()
{
    let temp = tempfile::tempdir().unwrap();
    for (name, data) in [
        ("a.txt", b"seven!!".as_slice()),
        ("hidden.bin", b"hidden bytes"),
        ("z.txt", b"z"),
    ] {
        std::fs::write(temp.path().join(name), data).unwrap();
    }
    let mut app = App::open(temp.path(), 3);
    app.app.press("role:checkbox[hidden.bin]").unwrap();
    app.app.type_text("Filter names", ".txt").unwrap();
    app.app.wait_for("2 of 3 items shown").unwrap();
    app.app.press_key("enter").unwrap();
    app.app.press("role:checkbox[a.txt]").unwrap();
    let requests = app.app.screen().file_requests().len();
    for label in [
        "Resize Type column",
        "Resize Size (bytes) column",
        "Resize Name column",
    ] {
        let before = app.now(label);
        app.action(label, "increment");
        app.until(|app| app.now(label) == before + 10.0);
        app.action(label, "decrement");
        app.until(|app| app.now(label) == before);
    }
    // Headless does not focus non-input adjustables on click; send the native focus event,
    // then use its real keymap driver. Native resize-handle drags require host smoke.
    let label = "Resize Name column";
    let handle = app.id(label);
    let before = app.now(label);
    app.send(handle, EventKind::Focus, EventPayload::None);
    app.app.press_key("right").unwrap();
    app.until(|app| app.now(label) == before + 10.0);
    app.app.press_key("shift+right").unwrap();
    app.until(|app| app.now(label) == before + 60.0);
    app.app.press_key("home").unwrap();
    app.until(|app| app.now(label) == before);
    app.send(handle, EventKind::Blur, EventPayload::None);
    app.assert_aligned("a.txt", "7", "File");
    for label in ["Name", "Type", "Size (bytes)"] {
        app.app.press(&format!("role:button[{label}]")).unwrap();
        assert!(app.checked("a.txt"));
        assert!(!app.checked("z.txt"));
        assert!(app.app.screen().shows("2 selected"));
    }
    app.app.press("role:button[Clear filter]").unwrap();
    app.app.wait_for("3 of 3 items shown").unwrap();
    assert!(app.checked("hidden.bin"));
    assert_eq!(app.app.screen().file_requests().len(), requests);
    assert_eq!(
        std::fs::read(temp.path().join("hidden.bin")).unwrap(),
        b"hidden bytes"
    );
}

#[test]
fn widths_survive_directory_refresh_and_archive_switch_and_narrow_layout_ignores_them() {
    let temp = tempfile::tempdir().unwrap();
    let child = temp.path().join("child");
    std::fs::create_dir(&child).unwrap();
    std::fs::write(child.join("inside"), b"inside").unwrap();
    std::fs::write(
        temp.path().join("sample.tar"),
        include_bytes!("../../../tests/fixtures/sample.tar"),
    )
    .unwrap();
    let mut app = App::open(temp.path(), 2);
    let original = app.now("Resize Size (bytes) column");
    app.action("Resize Size (bytes) column", "increment");
    app.until(|app| app.now("Resize Size (bytes) column") == original + 10.0);
    app.app.press("role:button[Open folder child]").unwrap();
    app.app.wait_for("role:checkbox[inside]").unwrap();
    assert_eq!(app.now("Resize Size (bytes) column"), original + 10.0);
    app.app
        .press("role:toolbar >> nth=0 >> role:button[Refresh folder]")
        .unwrap();
    app.app.wait_for("1 of 1 items shown").unwrap();
    assert_eq!(app.now("Resize Size (bytes) column"), original + 10.0);
    app.app.press("role:button[Up]").unwrap();
    app.app.wait_for("role:checkbox[sample.tar]").unwrap();
    app.app.press("role:button[Open file sample.tar]").unwrap();
    app.app.wait_for("5 entries").unwrap();
    assert_eq!(app.now("Resize Size (bytes) column"), original + 10.0);
    app.action("Resize Size (bytes) column", "increment");
    app.until(|app| app.now("Resize Size (bytes) column") == original + 20.0);
    app.app.press("role:button[Up]").unwrap();
    app.app.wait_for("role:checkbox[sample.tar]").unwrap();
    assert_eq!(app.now("Resize Size (bytes) column"), original + 20.0);
    app.app
        .set_viewport(Size::new(px(360.0), px(900.0)))
        .unwrap();
    app.app
        .wait_for_gone("Resize Size (bytes) column", Duration::from_secs(5))
        .unwrap();
    assert!(
        app.app
            .screen()
            .matches("role:button[Size (bytes)]")
            .unwrap()
    );
    assert!(!app.app.screen().matches("role:button[Type]").unwrap());
    assert!(
        app.app.scroll("Folder table", px(10.0)).is_err(),
        "narrow table must not overflow horizontally"
    );
    app.app
        .set_viewport(Size::new(px(1024.0), px(900.0)))
        .unwrap();
    app.until(|app| app.now("Resize Size (bytes) column") == original + 20.0);
    assert_eq!(app.app.screen().file_requests().len(), 1);
}

#[test]
fn large_filesystem_keeps_vertical_virtualization_and_horizontal_scrolling_together() {
    let temp = tempfile::tempdir().unwrap();
    for at in 0..256 {
        std::fs::write(temp.path().join(format!("file-{at:03}")), b"payload").unwrap();
    }
    let mut app = App::open(temp.path(), 256);
    assert!(app.app.screen().count("role:checkbox").unwrap() < 64);
    let before = app.now("Resize Name column");
    for _ in 0..6 {
        app.action("Resize Name column", "increment");
    }
    app.until(|app| app.now("Resize Name column") == before + 60.0);
    app.app.scroll("Folder table", px(40.0)).unwrap();
    let table = app.id("Folder table");
    let list = {
        let seen = app.seen.lock().unwrap();
        *seen
            .horizontal
            .iter()
            .find(|(id, horizontal)| {
                !**horizontal
                    && app
                        .app
                        .screen()
                        .matches(&format!("view:{table} >> view:{id}"))
                        .unwrap()
            })
            .unwrap()
            .0
    };
    for _ in 0..16 {
        if app.app.screen().matches("role:checkbox[file-255]").unwrap() {
            break;
        }
        app.app.scroll(&format!("view:{list}"), px(1000.0)).unwrap();
    }
    app.app.wait_for("role:checkbox[file-255]").unwrap();
    assert!(app.app.screen().count("role:checkbox").unwrap() < 64);
    assert!(
        app.app
            .screen()
            .enabled("role:button[Open file file-255]")
            .unwrap()
    );
    app.assert_aligned("file-255", "7", "File");
}

#[test]
fn modal_disables_resize_and_rejects_accessibility_events_without_changing_selection() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("keep"), b"untouched").unwrap();
    let mut app = App::open(temp.path(), 1);
    app.app.press("role:checkbox[keep]").unwrap();
    let label = "Resize Size (bytes) column";
    let before = app.now(label);
    let handle = app.id(label);
    app.app.press("role:button[Create archive…]").unwrap();
    app.app.wait_for("role:button[Continue]").unwrap();
    assert!(!app.app.screen().enabled(label).unwrap());
    app.send(
        handle,
        EventKind::AccessibilityAction,
        EventPayload::Map(vec![("actionName", PayloadValue::Str("increment".into()))]),
    );
    app.app.press("role:button[Cancel]").unwrap();
    app.app
        .wait_for_gone("role:button[Continue]", Duration::from_secs(5))
        .unwrap();
    assert_eq!(app.now(label), before);
    assert!(app.checked("keep"));
    assert!(app.app.screen().shows("1 selected"));
    assert_eq!(app.app.screen().file_requests().len(), 1);
    assert_eq!(
        std::fs::read(temp.path().join("keep")).unwrap(),
        b"untouched"
    );
}

#[test]
fn auto_adjust_layout_menu_preserves_hidden_ids_filter_sort_and_real_copy_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    for (name, data) in [
        ("shown-a.txt", b"seven!!".as_slice()),
        ("shown-z.txt", b"z"),
        ("hidden.bin", b"hidden payload"),
    ] {
        std::fs::write(source.join(name), data).unwrap();
    }
    let mut app = App::open(&source, 3);
    app.app.press("role:checkbox[hidden.bin]").unwrap();
    app.app.type_text("Filter names", ".txt").unwrap();
    app.app.press_key("enter").unwrap();
    app.app.wait_for("2 of 3 items shown").unwrap();
    app.app.press("role:checkbox[shown-a.txt]").unwrap();
    app.choose("Layout", "Sort by size");
    let sort = app.menu_action("Layout", "Sort by size").checked;
    let order_before: Vec<_> = app
        .app
        .screen()
        .texts()
        .into_iter()
        .filter(|text| matches!(*text, "shown-a.txt" | "shown-z.txt"))
        .map(str::to_owned)
        .collect();
    for label in [
        "Resize Name column",
        "Resize Type column",
        "Resize Size (bytes) column",
    ] {
        app.action(label, "increment");
    }
    let requests = app.app.screen().file_requests().len();
    app.choose("Layout", "Auto adjust columns");
    app.assert_adjusted(false);
    assert_eq!(
        app.app.screen().input_text("Filter names").unwrap(),
        Some(".txt")
    );
    assert!(app.app.screen().shows("2 of 3 items shown"));
    assert!(app.app.screen().shows("2 selected"));
    assert!(app.checked("shown-a.txt"));
    assert!(!app.checked("shown-z.txt"));
    assert_eq!(app.menu_action("Layout", "Sort by size").checked, sort);
    let order_after: Vec<_> = app
        .app
        .screen()
        .texts()
        .into_iter()
        .filter(|text| matches!(*text, "shown-a.txt" | "shown-z.txt"))
        .map(str::to_owned)
        .collect();
    assert_eq!(order_after, order_before);
    assert_eq!(app.app.screen().file_requests().len(), requests);
    app.assert_aligned("shown-a.txt", "7", "File");
    let output = temp.path().join("copied");
    app.app.on_pick_save_path(Some(output.clone())).unwrap();
    app.app.press("role:button[Copy selected…]").unwrap();
    app.app.wait_for("text:/^Copied 2 entries to /").unwrap();
    assert_eq!(
        std::fs::read(output.join("shown-a.txt")).unwrap(),
        b"seven!!"
    );
    assert_eq!(
        std::fs::read(output.join("hidden.bin")).unwrap(),
        b"hidden payload"
    );
    assert!(!output.join("shown-z.txt").exists());
    for (name, expected) in [
        ("shown-a.txt", b"seven!!".as_slice()),
        ("shown-z.txt", b"z"),
        ("hidden.bin", b"hidden payload"),
    ] {
        assert_eq!(std::fs::read(source.join(name)).unwrap(), expected);
    }
    app.app.press("role:button[Clear filter]").unwrap();
    app.app.wait_for("3 of 3 items shown").unwrap();
    assert!(app.checked("hidden.bin"));
}

#[test]
fn auto_adjust_shortcut_resets_shared_hidden_packed_width_and_home_only_resets_one_column() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let archive = source.join("sample.tar");
    let original = include_bytes!("../../../tests/fixtures/sample.tar");
    std::fs::write(&archive, original).unwrap();
    let mut app = App::open(&source, 1);
    app.app.press("role:button[Open file sample.tar]").unwrap();
    app.app.wait_for("5 entries").unwrap();
    app.choose("Layout", "All entries");
    app.app
        .wait_for("role:checkbox[docs/nested/data.bin]")
        .unwrap();
    app.app
        .press("role:checkbox[docs/nested/data.bin]")
        .unwrap();
    app.app.type_text("Filter names", "data").unwrap();
    app.app.press_key("enter").unwrap();
    app.choose("Layout", "Sort by packed size");
    for label in [
        "Resize Name column",
        "Resize Type column",
        "Resize Size (bytes) column",
        "Resize Packed (bytes) column",
    ] {
        app.action(label, "increment");
    }
    app.app.press_key("mod+alt+a").unwrap();
    app.assert_adjusted(true);
    assert_eq!(
        app.app.screen().input_text("Filter names").unwrap(),
        Some("data")
    );
    assert!(app.checked("docs/nested/data.bin"));
    assert!(app.app.screen().shows("1 selected"));
    assert_eq!(
        app.menu_action("Layout", "Sort by packed size").checked,
        Some(true)
    );
    let output = temp.path().join("selected");
    app.app.on_pick_save_path(Some(output.clone())).unwrap();
    app.app.press("role:button[Extract…]").unwrap();
    app.app.wait_for("role:button[Continue]").unwrap();
    app.app.press("role:button[Continue]").unwrap();
    app.app.wait_for("Extracted 1 entries").unwrap();
    assert_eq!(
        std::fs::read(output.join("docs/nested/data.bin")).unwrap(),
        [0, 127, 255]
    );
    assert!(!output.join("other.txt").exists());
    assert!(!output.join("docs/你好.txt").exists());
    app.action("Resize Packed (bytes) column", "increment");
    app.until(|app| app.now("Resize Packed (bytes) column") > 72.0);
    app.app.press("role:button[Up]").unwrap();
    app.app.wait_for("role:checkbox[sample.tar]").unwrap();
    assert!(
        !app.app
            .screen()
            .matches("Resize Packed (bytes) column")
            .unwrap()
    );
    app.choose("Layout", "Auto adjust columns");
    app.assert_adjusted(false);
    app.app.press("role:button[Open file sample.tar]").unwrap();
    app.app.wait_for("5 entries").unwrap();
    app.assert_adjusted(true);
    let handle = app.id("Resize Size (bytes) column");
    app.send(handle, EventKind::Focus, EventPayload::None);
    app.app.press_key("home").unwrap();
    app.until(|app| app.now("Resize Size (bytes) column") == 110.0);
    assert_eq!(app.now("Resize Name column"), 240.0);
    assert_eq!(app.now("Resize Type column"), 72.0);
    assert_eq!(app.now("Resize Packed (bytes) column"), 72.0);
    app.send(handle, EventKind::Blur, EventPayload::None);
    assert_eq!(std::fs::read(archive).unwrap(), original);
}

#[test]
fn auto_adjust_respects_editing_modal_and_busy_native_menu_gates() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("keep"), b"untouched").unwrap();
    let mut app = App::open(temp.path(), 1);
    assert!(
        app.menu_action("Layout", "Auto adjust columns")
            .shortcut
            .is_none(),
        "NotEditing commands must not install a native accelerator that bypasses editing"
    );
    app.app.press("role:checkbox[keep]").unwrap();
    app.action("Resize Size (bytes) column", "increment");
    let before = app.now("Resize Size (bytes) column");
    for (field, value) in [
        ("Filter names", "keep"),
        ("Folder path", "not-a-navigation-request"),
    ] {
        app.app.clear(field).unwrap();
        app.app.type_text(field, value).unwrap();
        app.app.press_key("mod+alt+a").unwrap();
        assert_eq!(app.now("Resize Size (bytes) column"), before);
        assert_eq!(app.app.screen().input_text(field).unwrap(), Some(value));
    }
    app.app.clear("Filter names").unwrap();
    app.app.type_text("Filter names", "keep").unwrap();
    app.app.press_key("enter").unwrap();
    let requests = app.app.screen().file_requests().len();
    app.app.press("role:button[Create archive…]").unwrap();
    app.app.wait_for("role:button[Continue]").unwrap();
    let disabled = app.menu_action("Layout", "Auto adjust columns");
    assert!(!disabled.enabled);
    app.app.press_key("mod+alt+a").unwrap();
    app.send(
        ROOT_ID,
        EventKind::Menu,
        EventPayload::Map(vec![("id", PayloadValue::Number(disabled.id as f64))]),
    );
    app.app.press("role:button[Cancel]").unwrap();
    app.app
        .wait_for_gone("role:button[Continue]", Duration::from_secs(5))
        .unwrap();
    assert_eq!(app.now("Resize Size (bytes) column"), before);
    assert_eq!(app.app.screen().file_requests().len(), requests);
    app.app.on_pick_paths(vec![]).unwrap();
    app.seen.lock().unwrap().probe_blocked_adjust = true;
    app.app.press("role:button[Open folder…]").unwrap();
    app.until(|app| app.seen.lock().unwrap().blocked_adjust_attempts == 1);
    app.until(|app| app.menu_action("Layout", "Auto adjust columns").enabled);
    assert_eq!(app.now("Resize Size (bytes) column"), before);
    assert!(app.checked("keep"));
    assert!(app.app.screen().shows("1 selected"));
    assert_eq!(
        app.app.screen().input_text("Filter names").unwrap(),
        Some("keep")
    );
    assert_eq!(
        std::fs::read(temp.path().join("keep")).unwrap(),
        b"untouched"
    );
}

#[test]
fn auto_adjust_72_point_size_column_keeps_sparse_file_on_one_line_with_full_accessibility_size() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.bin");
    let size = 16 * 1024 * 1024;
    std::fs::File::create(&path).unwrap().set_len(size).unwrap();
    let mut app = App::open(temp.path(), 1);
    app.choose("Layout", "Auto adjust columns");
    app.assert_adjusted(false);
    app.app
        .wait_for("role:checkbox[large.bin] >> label:Size in bytes: 16777216")
        .unwrap();
    let row = app.id("large.bin");
    let cell = app.id("Size in bytes: 16777216");
    let seen = app.seen.lock().unwrap();
    assert_eq!(seen.frames[&cell].size.width.0, 72.0);
    assert!(
        seen.frames[&cell].size.height.0 < 32.0,
        "size cell wrapped: {:?}",
        seen.frames[&cell]
    );
    assert!(
        seen.frames[&row].size.height.0 < 48.0,
        "row grew after reset: {:?}",
        seen.frames[&row]
    );
    drop(seen);
    assert_eq!(std::fs::metadata(path).unwrap().len(), size);
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    assert_eq!(app.app.screen().file_requests().len(), 1);
}

#[test]
fn visibility_is_shared_across_filesystem_archive_and_refresh_without_losing_hidden_widths() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("child")).unwrap();
    std::fs::write(temp.path().join("child/inside"), b"inside").unwrap();
    let archive = temp.path().join("sample.tar");
    let original = include_bytes!("../../../tests/fixtures/sample.tar");
    std::fs::write(&archive, original).unwrap();
    let mut app = App::open(temp.path(), 2);
    assert!(!app.menu_action("Layout", "Show packed column").enabled);
    assert_eq!(
        app.menu_action("Layout", "Show packed column").checked,
        Some(true)
    );
    let type_width = app.now("Resize Type column");
    let size_width = app.now("Resize Size (bytes) column");
    app.action("Resize Type column", "increment");
    app.action("Resize Size (bytes) column", "increment");
    for label in ["Show type column", "Show size column"] {
        assert!(app.menu_action("Layout", label).enabled);
        app.choose("Layout", label);
        assert_eq!(app.menu_action("Layout", label).checked, Some(false));
    }
    assert!(!app.app.screen().matches("role:button[Type]").unwrap());
    assert!(
        !app.app
            .screen()
            .matches("role:button[Size (bytes)]")
            .unwrap()
    );
    assert!(
        app.app
            .screen()
            .enabled("role:button[Open folder child]")
            .unwrap()
    );
    app.app.press("role:button[Open folder child]").unwrap();
    app.app.wait_for("role:checkbox[inside]").unwrap();
    app.app
        .press("role:toolbar >> nth=0 >> role:button[Refresh folder]")
        .unwrap();
    app.app.wait_for("1 of 1 items shown").unwrap();
    assert!(
        !app.app
            .screen()
            .matches("role:button[Size (bytes)]")
            .unwrap()
    );
    app.app.press("role:button[Up]").unwrap();
    app.app.wait_for("role:checkbox[sample.tar]").unwrap();
    app.app.press("role:button[Open file sample.tar]").unwrap();
    app.app.wait_for("5 entries").unwrap();
    assert!(!app.app.screen().matches("role:button[Type]").unwrap());
    assert!(
        !app.app
            .screen()
            .matches("role:button[Size (bytes)]")
            .unwrap()
    );
    let packed_width = app.now("Resize Packed (bytes) column");
    app.action("Resize Packed (bytes) column", "increment");
    app.choose("Layout", "Show packed column");
    app.app
        .wait_for_gone("Resize Packed (bytes) column", Duration::from_secs(5))
        .unwrap();
    app.app.press("role:button[Up]").unwrap();
    app.app.wait_for("role:checkbox[sample.tar]").unwrap();
    assert!(!app.menu_action("Layout", "Show packed column").enabled);
    assert_eq!(
        app.menu_action("Layout", "Show packed column").checked,
        Some(false)
    );
    app.app.press("role:button[Open file sample.tar]").unwrap();
    app.app.wait_for("5 entries").unwrap();
    for label in ["Show type column", "Show size column", "Show packed column"] {
        app.choose("Layout", label);
    }
    app.until(|app| {
        app.now("Resize Type column") == type_width + 10.0
            && app.now("Resize Size (bytes) column") == size_width + 10.0
            && app.now("Resize Packed (bytes) column") == packed_width + 10.0
    });
    assert_eq!(std::fs::read(archive).unwrap(), original);
    assert_eq!(app.app.screen().file_requests().len(), 1);
}

#[test]
fn hiding_sorted_columns_preserves_filter_selected_ids_copy_bytes_and_auto_adjust_visibility() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    for (name, bytes) in [
        ("a.txt", b"seven!!".as_slice()),
        ("z.txt", b"z"),
        ("hidden.bin", b"hidden payload"),
    ] {
        std::fs::write(source.join(name), bytes).unwrap();
    }
    let mut app = App::open(&source, 3);
    app.app.press("role:checkbox[hidden.bin]").unwrap();
    app.app.type_text("Filter names", ".txt").unwrap();
    app.app.press_key("enter").unwrap();
    app.app.wait_for("2 of 3 items shown").unwrap();
    app.app.press("role:checkbox[a.txt]").unwrap();
    app.choose("Layout", "Sort by size");
    let before: Vec<_> = app
        .app
        .screen()
        .texts()
        .into_iter()
        .filter(|text| matches!(*text, "a.txt" | "z.txt"))
        .map(str::to_owned)
        .collect();
    assert_eq!(before, ["z.txt", "a.txt"]);
    app.choose("Layout", "Show size column");
    app.choose("Layout", "Show type column");
    assert!(
        !app.app
            .screen()
            .matches("role:button[Size (bytes)]")
            .unwrap()
    );
    assert!(
        !app.app
            .screen()
            .matches("role:checkbox[a.txt] >> label:Size in bytes: 7")
            .unwrap()
    );
    assert!(
        !app.app
            .screen()
            .matches("role:checkbox[a.txt] >> text:File")
            .unwrap()
    );
    assert_eq!(
        app.menu_action("Layout", "Sort by size").checked,
        Some(true)
    );
    let hidden_order: Vec<_> = app
        .app
        .screen()
        .texts()
        .into_iter()
        .filter(|text| matches!(*text, "a.txt" | "z.txt"))
        .map(str::to_owned)
        .collect();
    assert_eq!(hidden_order, before);
    app.choose("Layout", "Sort by size");
    let descending: Vec<_> = app
        .app
        .screen()
        .texts()
        .into_iter()
        .filter(|text| matches!(*text, "a.txt" | "z.txt"))
        .map(str::to_owned)
        .collect();
    assert_eq!(descending, ["a.txt", "z.txt"]);
    app.choose("Layout", "Auto adjust columns");
    assert_eq!(
        app.menu_action("Layout", "Show size column").checked,
        Some(false)
    );
    assert_eq!(
        app.menu_action("Layout", "Show type column").checked,
        Some(false)
    );
    assert!(
        !app.app
            .screen()
            .matches("role:button[Size (bytes)]")
            .unwrap()
    );
    assert!(!app.app.screen().matches("role:button[Type]").unwrap());
    assert_eq!(
        app.app.screen().input_text("Filter names").unwrap(),
        Some(".txt")
    );
    assert!(app.checked("a.txt"));
    assert!(!app.checked("z.txt"));
    assert!(app.app.screen().shows("2 selected"));
    let output = temp.path().join("copied");
    app.app.on_pick_save_path(Some(output.clone())).unwrap();
    app.app.press("role:button[Copy selected…]").unwrap();
    app.app.wait_for("text:/^Copied 2 entries to /").unwrap();
    assert_eq!(std::fs::read(output.join("a.txt")).unwrap(), b"seven!!");
    assert_eq!(
        std::fs::read(output.join("hidden.bin")).unwrap(),
        b"hidden payload"
    );
    assert!(!output.join("z.txt").exists());
    app.choose("Layout", "Show type column");
    app.choose("Layout", "Show size column");
    app.assert_adjusted(false);
    app.app.press("role:button[Clear filter]").unwrap();
    app.app.wait_for("3 of 3 items shown").unwrap();
    assert!(app.checked("hidden.bin"));
    assert_eq!(
        std::fs::read(source.join("hidden.bin")).unwrap(),
        b"hidden payload"
    );
}

#[test]
fn narrow_name_only_layout_has_no_hidden_column_gaps_or_horizontal_overflow() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("keep.txt"), b"payload").unwrap();
    let mut app = App::open(temp.path(), 1);
    app.action("Resize Size (bytes) column", "increment");
    let saved_width = app.now("Resize Size (bytes) column");
    app.app.press("role:checkbox[keep.txt]").unwrap();
    app.app
        .set_viewport(Size::new(px(360.0), px(900.0)))
        .unwrap();
    app.app
        .wait_for_gone("Resize Size (bytes) column", Duration::from_secs(5))
        .unwrap();
    assert!(!app.app.screen().matches("role:button[Type]").unwrap());
    assert!(
        app.app
            .screen()
            .matches("role:button[Size (bytes)]")
            .unwrap()
    );
    app.choose("Layout", "Show size column");
    app.app
        .wait_for_gone("role:button[Size (bytes)]", Duration::from_secs(5))
        .unwrap();
    assert!(
        !app.app
            .screen()
            .matches("role:button[Packed (bytes)]")
            .unwrap()
    );
    assert!(
        !app.app
            .screen()
            .matches("role:checkbox[keep.txt] >> label:Size in bytes: 7")
            .unwrap()
    );
    let table = app.id("Folder table");
    let name = app.text_id("keep.txt", "role:checkbox[keep.txt]");
    let open = app.id("Open file keep.txt");
    let seen = app.seen.lock().unwrap();
    assert_eq!(seen.frames[&table].size.width.0, 360.0);
    assert_eq!(seen.frames[&name].size.width.0, 252.0);
    drop(seen);
    assert!((app.x_in_table(open) - 324.0).abs() < 0.5);
    assert!(app.app.scroll("Folder table", px(5.0)).is_err());
    assert!(app.checked("keep.txt"));
    assert!(
        app.app
            .screen()
            .enabled("role:button[Open file keep.txt]")
            .unwrap()
    );
    app.choose("Layout", "Show size column");
    app.app
        .wait_for("role:checkbox[keep.txt] >> label:Size in bytes: 7")
        .unwrap();
    let size = app.id("Size in bytes: 7");
    assert_eq!(app.seen.lock().unwrap().frames[&size].size.width.0, 110.0);
    app.app
        .set_viewport(Size::new(px(1024.0), px(900.0)))
        .unwrap();
    app.until(|app| app.now("Resize Size (bytes) column") == saved_width);
    assert!(app.checked("keep.txt"));
}

#[test]
fn visibility_menu_actions_recheck_modal_and_busy_gates_and_preserve_selection() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("keep"), b"unchanged").unwrap();
    let mut app = App::open(temp.path(), 1);
    app.app.press("role:checkbox[keep]").unwrap();
    let before = app.now("Resize Size (bytes) column");
    let requests = app.app.screen().file_requests().len();
    app.app.press("role:button[Create archive…]").unwrap();
    app.app.wait_for("role:button[Continue]").unwrap();
    for label in ["Show type column", "Show size column", "Show packed column"] {
        let action = app.menu_action("Layout", label);
        assert!(!action.enabled);
        app.send(
            ROOT_ID,
            EventKind::Menu,
            EventPayload::Map(vec![("id", PayloadValue::Number(action.id as f64))]),
        );
    }
    app.app.press("role:button[Cancel]").unwrap();
    app.app
        .wait_for_gone("role:button[Continue]", Duration::from_secs(5))
        .unwrap();
    for label in ["Show type column", "Show size column", "Show packed column"] {
        assert_eq!(app.menu_action("Layout", label).checked, Some(true));
    }
    app.app.on_pick_paths(vec![]).unwrap();
    app.seen.lock().unwrap().probe_blocked_visibility = true;
    app.app.press("role:button[Open folder…]").unwrap();
    app.until(|app| app.seen.lock().unwrap().blocked_visibility_attempts == 1);
    app.until(|app| app.menu_action("Layout", "Show size column").enabled);
    assert_eq!(app.now("Resize Size (bytes) column"), before);
    assert_eq!(
        app.menu_action("Layout", "Show size column").checked,
        Some(true)
    );
    assert!(app.checked("keep"));
    assert_eq!(app.app.screen().file_requests().len(), requests + 1);
    assert_eq!(
        std::fs::read(temp.path().join("keep")).unwrap(),
        b"unchanged"
    );
}
