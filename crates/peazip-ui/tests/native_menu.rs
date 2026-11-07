#![cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]

use nsh_core::{
    ApproxTextEngine, Command, EventKind, EventPayload, MountBatch, MountItem, PayloadValue,
    ROOT_ID,
    desktop::{DesktopCommand, MenuAction, MenuItem},
    file_dialog::FileRequest,
};
use nsh_engine::{Engine, EngineConfig, MountSink, Root};
use std::{
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

struct Capture(mpsc::Sender<Result<MountBatch, String>>);

impl MountSink for Capture {
    fn submit(&self, batch: MountBatch) {
        let _ = self.0.send(Ok(batch));
    }
    fn report_error(&self, message: &str) {
        let _ = self.0.send(Err(message.to_owned()));
    }
}

struct App {
    engine: Engine,
    batches: mpsc::Receiver<Result<MountBatch, String>>,
    menu: Arc<[MenuItem]>,
}

impl App {
    fn start() -> Self {
        let (sender, batches) = mpsc::channel();
        let mut config = EngineConfig::new(Root::of::<peazip_ui::App>());
        config.environment.idiom = nsh_engine::Idiom::Desktop;
        config.environment.os = if cfg!(target_os = "macos") {
            nsh_engine::Os::MacOs
        } else if cfg!(target_os = "windows") {
            nsh_engine::Os::Windows
        } else {
            nsh_engine::Os::Linux
        };
        config.environment.file_requests = true;
        let engine = Engine::start(config, || ApproxTextEngine, Capture(sender)).unwrap();
        Self {
            engine,
            batches,
            menu: Arc::from([]),
        }
    }

    fn until(&mut self, mut ready: impl FnMut(&Self, &MountBatch) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        for _ in 0..256 {
            let batch = self
                .batches
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("timed out waiting for the real App's mount batch")
                .expect("App engine error");
            for item in &batch.items {
                if let MountItem::Command {
                    id: ROOT_ID,
                    command: Command::Desktop(command),
                } = item
                    && let DesktopCommand::SetMenu(Some(menu)) = &**command
                {
                    self.menu = Arc::clone(menu);
                }
            }
            if ready(self, &batch) {
                return;
            }
        }
        panic!("App produced more than 256 batches without the expected menu state");
    }

    fn action(&self, group: &str, label: &str) -> &MenuAction {
        let items = self
            .menu
            .iter()
            .find_map(|item| match item {
                MenuItem::Submenu { label, items, .. } if label == group => Some(items),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing menu group {group}"));
        items
            .iter()
            .find_map(|item| match item {
                MenuItem::Action(action) if action.label == label => Some(action),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing menu action {group}/{label}"))
    }

    fn choose(&self, group: &str, label: &str) {
        let action = self.action(group, label);
        assert!(action.enabled, "disabled action {group}/{label}");
        assert_ne!(action.id, 0);
        self.engine.control().dispatch_event(
            ROOT_ID,
            EventKind::Menu,
            EventPayload::Map(vec![("id", PayloadValue::Number(action.id as f64))]),
        );
    }
}

#[test]
fn real_app_mounts_native_menus_and_dispatches_live_archive_and_layout_actions() {
    let sandbox = tempfile::tempdir().unwrap();
    let source = sandbox.path().join("sample.tar");
    let fixture = include_bytes!("../../../tests/fixtures/sample.tar");
    std::fs::write(&source, fixture).unwrap();
    let mut app = App::start();
    app.until(|app, _| !app.menu.is_empty());
    let groups: Vec<_> = app
        .menu
        .iter()
        .filter_map(|item| match item {
            MenuItem::Submenu { label, .. } if label != "PeaZip" => Some(label.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        groups,
        ["File", "Edit", "Browser", "Layout", "Tools", "Help"]
    );
    assert!(!app.action("File", "Close archive").enabled);
    assert!(!app.action("Tools", "Test archive").enabled);
    assert!(!app.action("Tools", "Extract selected…").enabled);
    for (group, labels) in [
        ("Edit", &["Select all", "Clear selection"][..]),
        ("Browser", &["Back", "Forward", "Up", "Archive root"][..]),
    ] {
        for label in labels {
            assert!(
                app.action(group, label).shortcut.is_none(),
                "native accelerator bypasses NotEditing: {group}/{label}"
            );
        }
    }
    MenuItem::each_action(&app.menu, &mut |action| {
        if action.role.is_none() && !matches!(action.label.as_str(), "Open archive…" | "Go to path")
        {
            assert!(
                action.shortcut.is_none(),
                "native accelerator bypasses NotEditing: {}",
                action.label
            );
        }
    });

    assert_eq!(app.action("Layout", "All entries").checked, Some(false));
    app.choose("Layout", "All entries");
    app.until(|app, _| app.action("Layout", "All entries").checked == Some(true));
    app.choose("Layout", "All entries");
    app.until(|app, _| app.action("Layout", "All entries").checked == Some(false));

    app.choose("File", "Open archive…");
    app.until(|app, batch| {
        for item in &batch.items {
            if let MountItem::Command {
                id: ROOT_ID,
                command: Command::FileRequest { id, request },
            } = item
            {
                assert!(matches!(&**request, FileRequest::OpenDialog(_)));
                app.engine.control().dispatch_event(
                    ROOT_ID,
                    EventKind::FileRequest,
                    EventPayload::file_answer(*id, vec![source.clone()]),
                );
                return true;
            }
        }
        false
    });
    app.until(|app, _| {
        app.action("File", "Close archive").enabled && app.action("Tools", "Test archive").enabled
    });
    assert!(!app.action("Tools", "Extract selected…").enabled);
    app.choose("Edit", "Select all");
    app.until(|app, _| app.action("Tools", "Extract selected…").enabled);
    app.choose("File", "Close archive");
    app.until(|app, _| {
        !app.action("File", "Close archive").enabled && !app.action("Tools", "Test archive").enabled
    });
    assert!(!app.action("Tools", "Extract selected…").enabled);
    assert_eq!(std::fs::read(&source).unwrap(), fixture);
    assert_eq!(std::fs::read_dir(sandbox.path()).unwrap().count(), 1);
}
