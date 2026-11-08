#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use nsh_core::{
    Accessibility, Bounds, Checked, EventKind, EventPayload, MountItem, NodeId, PayloadValue,
    Points, Size, prelude::px,
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
}

struct App {
    app: Headless,
    seen: Arc<Mutex<Seen>>,
}

impl App {
    fn open(path: &Path, count: usize) -> Self {
        let mut config = EngineConfig::new(Root::of::<peazip_ui::App>());
        config.viewport = Size::new(px(1024.0), px(900.0));
        let mut app = Headless::start(config).unwrap();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let capture = Arc::clone(&seen);
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
