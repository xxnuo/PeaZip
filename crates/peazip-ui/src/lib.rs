use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

use commands::{Command, CommandId, Commands};
use nsh_base::{theme, toolbar};
use nsh_core::prelude::*;
use peazip_core::{ArchiveFormat, Entry};

mod columns;
mod commands;
#[cfg(any(
    target_arch = "wasm32",
    target_os = "android",
    target_os = "ios",
    target_os = "macos",
    target_os = "linux",
    target_os = "windows"
))]
mod create_dialog;
mod explorer;
#[cfg(any(
    target_arch = "wasm32",
    target_os = "android",
    target_os = "ios",
    target_os = "macos",
    target_os = "linux",
    target_os = "windows"
))]
mod extract_dialog;
mod icons;
mod menus;
#[cfg(any(target_os = "ios", test))]
mod mobile_export;
#[cfg(any(target_os = "ios", test))]
mod mobile_files;
mod name_filter;
mod navigation;

type LoadedArchive = (Arc<Vec<u8>>, Vec<Entry>, ArchiveFormat);

struct OpenedArchive {
    name: String,
    data: LoadedArchive,
    source_path: Option<PathBuf>,
}

#[cfg(any(
    target_arch = "wasm32",
    target_os = "android",
    target_os = "ios",
    target_os = "macos",
    target_os = "linux",
    target_os = "windows"
))]
#[derive(Clone)]
struct ExtractionRequest {
    name: String,
    bytes: Arc<Vec<u8>>,
    selected: Vec<String>,
    all: Vec<String>,
    displayed: Vec<String>,
    displayed_location: String,
}

#[derive(Clone, Copy)]
struct ArchiveState {
    snapshot: Signal<Arc<Vec<u8>>>,
    source: Signal<Option<PathBuf>>,
    format: Signal<ArchiveFormat>,
    entries: Signal<Vec<Entry>>,
    navigation: Signal<navigation::BrowserState>,
    version: Signal<u64>,
    path: Signal<String>,
    selected: Signal<BTreeSet<String>>,
}

impl ArchiveState {
    fn show(self, opened: OpenedArchive) {
        let (bytes, rows, kind) = opened.data;
        batch(|| {
            self.format.set(kind);
            self.snapshot.set(bytes);
            self.path.set(opened.name);
            self.source.set(opened.source_path);
            self.entries.set(rows);
            self.navigation.set(navigation::BrowserState::default());
            self.version
                .update(|version| *version = version.wrapping_add(1));
            self.selected.set(BTreeSet::new());
        });
    }
    fn clear(self) {
        self.show(OpenedArchive {
            name: String::new(),
            data: (Arc::new(Vec::new()), Vec::new(), ArchiveFormat::Zip),
            source_path: None,
        });
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
mod desktop_browsing;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
mod filesystem;
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
mod new_folder_dialog;

#[cfg(target_arch = "wasm32")]
mod browser;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;
#[cfg(target_os = "android")]
use android as mobile_archive;
#[cfg(target_os = "ios")]
use ios as mobile_archive;

#[native_module("peazip/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

#[component]
pub fn App() -> impl IntoElement {
    let snapshot = signal(Arc::new(Vec::<u8>::new()));
    let format = signal(ArchiveFormat::Zip);
    let source = signal(None::<PathBuf>);
    let entries = signal(Vec::<Entry>::new());
    let navigation = signal(navigation::BrowserState::default());
    let archive_version = signal(0u64);
    let path = signal(String::new());
    let status = signal("Open an archive to view its contents.".to_owned());
    let busy = signal(false);
    #[cfg(target_arch = "wasm32")]
    let cancellable = signal(false);
    #[cfg(target_arch = "wasm32")]
    let cancel_message = signal("Creation cancelled.".to_owned());
    let activity = signal("Opening archive…".to_owned());
    let selected = signal(BTreeSet::<String>::new());
    let controls = explorer::Controls::new(entries, navigation);
    let filter_location = stored(None::<(u64, String)>);
    effect(move || {
        let location = (
            archive_version.get(),
            navigation.with(|state| state.current().to_owned()),
        );
        if filter_location
            .replace(Some(location.clone()))
            .is_some_and(|previous| previous != location)
        {
            controls.filter.set(String::new());
        }
    });
    let owner = Owner::current();
    let creating = signal(None::<bool>);
    let extracting = signal(false);
    let new_folder_open = signal(false);
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    let new_folder_parent = stored(None::<Arc<peazip_native::DirectorySnapshot>>);
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    let new_folder_error = signal(String::new());
    let editing_locked = signal(false);
    effect(move || {
        editing_locked.set(
            busy.get() || creating.get().is_some() || extracting.get() || new_folder_open.get(),
        )
    });
    #[cfg(any(
        target_arch = "wasm32",
        target_os = "android",
        target_os = "ios",
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))]
    let pending_extract = stored(None::<ExtractionRequest>);
    #[cfg(any(
        target_arch = "wasm32",
        target_os = "android",
        target_os = "ios",
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))]
    let extract_options = signal(peazip_core::ExtractOptions::default());
    #[cfg(any(
        target_arch = "wasm32",
        target_os = "android",
        target_os = "ios",
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))]
    let archive_options = signal(peazip_core::CreateOptions::default());
    let archive = ArchiveState {
        snapshot,
        source,
        format,
        entries,
        navigation,
        version: archive_version,
        path,
        selected,
    };
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    let files = filesystem::Files::new();
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    effect(move || {
        if files.active.get()
            && matches!(
                controls.sort.get(),
                navigation::Sort::PackedAsc | navigation::Sort::PackedDesc
            )
        {
            controls.sort.set(navigation::Sort::NameAsc);
        }
    });
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    let disk = desktop_browsing::Browser {
        files,
        archive,
        busy,
        status,
        activity,
        owner,
        archive_content: controls.content,
    };
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    disk.track_navigation();
    let file_view = move || {
        #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
        {
            files.active.get()
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            false
        }
    };
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    let focus_contents = move || {
        #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
        if files.active.get_untracked() {
            files.content.focus();
            return;
        }
        controls.content.focus();
    };
    let show_archive = move |opened: OpenedArchive| {
        #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
        if files.context.get_untracked() {
            disk.show_archive(opened, String::new(), filesystem::HistoryChange::Visit);
            return;
        }
        archive.show(opened);
    };
    let active_menu = signal(None::<&'static str>);
    let open = move || {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        activity.set("Opening archive…".to_owned());
        spawn(async move {
            #[cfg(target_arch = "wasm32")]
            let opened = {
                cancel_message.set("Opening cancelled.".into());
                choose_archive(cancellable).await
            };
            #[cfg(not(target_arch = "wasm32"))]
            let opened = choose_archive().await;
            #[cfg(target_arch = "wasm32")]
            cancellable.set(false);
            match opened {
                Ok(Some(opened)) => {
                    let count = opened.data.1.len();
                    show_archive(opened);
                    status.set(format!("{} entries", count));
                }
                Ok(None) => {}
                Err(error) => status.set(format!("Could not open archive: {error}")),
            }
            busy.set(false);
        });
    };
    #[cfg(any(
        target_arch = "wasm32",
        target_os = "android",
        target_os = "ios",
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))]
    let create = move |folders: bool, options: peazip_core::CreateOptions| {
        if busy.get_untracked() {
            return;
        }
        let label = archive_label(options.format());
        if let Err(error) = options.validate() {
            status.set(format!("Could not create {label}: {error}"));
            return;
        }
        busy.set(true);
        #[cfg(target_arch = "wasm32")]
        cancellable.set(true);
        #[cfg(target_arch = "wasm32")]
        cancel_message.set("Creation cancelled.".into());
        activity.set(format!("Creating {label}…"));
        spawn(async move {
            #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
            let result = if !folders
                && files.active.get_untracked()
                && !files.selected.with(BTreeSet::is_empty)
            {
                disk.create_selection(options).await
            } else {
                create_archive(folders, activity, options).await
            };
            #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
            let result = create_archive(folders, activity, options).await;
            match result {
                Ok(Some(opened)) => {
                    let count = opened.data.1.len();
                    show_archive(opened);
                    status.set(format!("Created {label}: {} entries", count));
                }
                Ok(None) => {}
                Err(error) => status.set(format!("Could not create {label}: {error}")),
            }
            busy.set(false);
            #[cfg(target_arch = "wasm32")]
            cancellable.set(false);
        });
    };

    let can_select = move || {
        if busy.get() {
            return false;
        }
        #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
        if files.active.get() {
            return !files.visible_ids().is_empty();
        }
        !controls.visible.with(Vec::is_empty)
    };

    let mut all = vec![
        desktop_shortcut(
            Command::new(CommandId::Open, "Open archive…", move || !busy.get(), open).icon("open"),
            "mod+o",
        ),
        desktop_shortcut(
            Command::new(
                CommandId::CloseArchive,
                "Close archive",
                move || !busy.get() && !file_view() && !snapshot.with(|bytes| bytes.is_empty()),
                move || {
                    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
                    {
                        disk.close_archive();
                    }
                    #[cfg(not(any(
                        target_os = "macos",
                        target_os = "linux",
                        target_os = "windows"
                    )))]
                    {
                        archive.clear();
                        status.set("Open an archive to view its contents.".into());
                    }
                },
            ),
            "mod+w",
        ),
        Command::new(
            CommandId::Test,
            "Test archive",
            move || !busy.get() && !file_view() && !snapshot.with(|bytes| bytes.is_empty()),
            move || {
                if busy.get_untracked() || snapshot.with(|bytes| bytes.is_empty()) {
                    return;
                }
                let bytes = snapshot.get_untracked();
                busy.set(true);
                activity.set("Testing archive…".into());
                #[cfg(target_arch = "wasm32")]
                {
                    cancellable.set(true);
                    cancel_message.set("Archive test cancelled.".into());
                }
                spawn(async move {
                    #[cfg(target_arch = "wasm32")]
                    let result = browser::test_archive(bytes).await;
                    #[cfg(not(target_arch = "wasm32"))]
                    let result = test_archive(bytes).await.map(Some);
                    match result {
                        Ok(Some(report)) => status.set(format!(
                            "Tested {} entries ({} bytes). Content checksums: {}/{} files.",
                            report.entries,
                            report.total_bytes,
                            report.checksummed_files,
                            report.files,
                        )),
                        Ok(None) => status.set("Archive test cancelled.".into()),
                        Err(error) => status.set(format!("Could not test archive: {error}")),
                    }
                    #[cfg(target_arch = "wasm32")]
                    cancellable.set(false);
                    busy.set(false);
                });
            },
        )
        .icon("test"),
        keyboard_shortcut(
            Command::new(CommandId::SelectAll, "Select all", can_select, move || {
                #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
                if files.active.get_untracked() {
                    let visible = files.visible_ids();
                    files.selected.update(|selected| selected.extend(visible));
                    return;
                }
                let paths = entries.with(|items| {
                    controls.visible.with(|rows| {
                        rows.iter()
                            .flat_map(|row| navigation::selection_paths(row, items))
                            .collect::<BTreeSet<_>>()
                    })
                });
                if controls.filter.with(String::is_empty) {
                    selected.set(paths);
                } else {
                    selected.update(|selected| selected.extend(paths));
                }
            }),
            "mod+a",
        ),
        desktop_shortcut(
            Command::new(
                CommandId::InvertSelection,
                "Invert selection",
                can_select,
                move || {
                    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
                    if files.active.get_untracked() {
                        let visible = files.visible_ids();
                        files.selected.update(|selected| {
                            for id in visible {
                                if !selected.remove(&id) {
                                    selected.insert(id);
                                }
                            }
                        });
                        return;
                    }
                    entries.with(|items| {
                        controls.visible.with(|rows| {
                            selected.update(|selected| {
                                navigation::invert_selected(rows, items, selected);
                            })
                        })
                    });
                },
            ),
            "mod+i",
        ),
        keyboard_shortcut(
            Command::new(
                CommandId::ClearSelection,
                "Clear selection",
                move || {
                    if busy.get() {
                        return false;
                    }
                    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
                    if files.active.get() {
                        return !files.selected.with(BTreeSet::is_empty);
                    }
                    !selected.with(BTreeSet::is_empty)
                },
                move || {
                    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
                    if files.active.get_untracked() {
                        files.selected.set(BTreeSet::new());
                        return;
                    }
                    selected.set(BTreeSet::new());
                },
            ),
            "mod+shift+a",
        ),
        keyboard_shortcut(
            Command::new(
                CommandId::Back,
                "Back",
                move || {
                    if busy.get() {
                        return false;
                    }
                    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
                    if files.context.get() {
                        return disk.can_back();
                    }
                    navigation.with(navigation::BrowserState::can_back)
                },
                move || {
                    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
                    if files.context.get_untracked() {
                        disk.back();
                        return;
                    }
                    navigation.update(|state| {
                        state.back();
                    });
                },
            )
            .icon("back"),
            "alt+left",
        ),
        keyboard_shortcut(
            Command::new(
                CommandId::Forward,
                "Forward",
                move || {
                    if busy.get() {
                        return false;
                    }
                    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
                    if files.context.get() {
                        return disk.can_forward();
                    }
                    navigation.with(navigation::BrowserState::can_forward)
                },
                move || {
                    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
                    if files.context.get_untracked() {
                        disk.forward();
                        return;
                    }
                    navigation.update(|state| {
                        state.forward();
                    });
                },
            )
            .icon("forward"),
            "alt+right",
        ),
        keyboard_shortcut(
            Command::new(
                CommandId::Up,
                "Up",
                move || {
                    if busy.get() {
                        return false;
                    }
                    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
                    {
                        disk.can_up()
                    }
                    #[cfg(not(any(
                        target_os = "macos",
                        target_os = "linux",
                        target_os = "windows"
                    )))]
                    {
                        !navigation.with(|state| state.current().is_empty())
                    }
                },
                move || {
                    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
                    {
                        disk.up();
                    }
                    #[cfg(not(any(
                        target_os = "macos",
                        target_os = "linux",
                        target_os = "windows"
                    )))]
                    {
                        navigation.update(|state| {
                            state.up();
                        });
                    }
                },
            )
            .icon("up"),
            "alt+up",
        ),
        Command::new(
            CommandId::Root,
            "Archive root",
            move || {
                !busy.get() && !file_view() && !navigation.with(|state| state.current().is_empty())
            },
            move || {
                navigation.update(|state| {
                    state.root();
                })
            },
        ),
        desktop_shortcut(
            Command::new(
                CommandId::FocusPath,
                "Go to path",
                move || !busy.get(),
                move || {
                    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
                    if files.active.get_untracked() {
                        files.input.focus();
                        files.input.set_selection(
                            0,
                            files.draft.with(|text| text.encode_utf16().count() as u32),
                        );
                        return;
                    }
                    controls.input.focus();
                    controls.input.set_selection(
                        0,
                        controls
                            .draft
                            .with(|text| text.encode_utf16().count() as u32),
                    );
                },
            ),
            "mod+l",
        ),
        Command::new(
            CommandId::ToggleFlat,
            "All entries",
            move || !busy.get() && !file_view(),
            move || controls.flat.update(|value| *value = !*value),
        )
        .checked(move || controls.flat.get()),
        Command::new(
            CommandId::SortName,
            "Sort by name",
            move || !busy.get(),
            move || {
                controls.sort.update(|value| {
                    *value = if *value == navigation::Sort::NameAsc {
                        navigation::Sort::NameDesc
                    } else {
                        navigation::Sort::NameAsc
                    }
                })
            },
        )
        .checked(move || {
            matches!(
                controls.sort.get(),
                navigation::Sort::NameAsc | navigation::Sort::NameDesc
            )
        }),
        Command::new(
            CommandId::SortSize,
            "Sort by size",
            move || !busy.get(),
            move || {
                controls.sort.update(|value| {
                    *value = if *value == navigation::Sort::SizeAsc {
                        navigation::Sort::SizeDesc
                    } else {
                        navigation::Sort::SizeAsc
                    }
                })
            },
        )
        .checked(move || {
            matches!(
                controls.sort.get(),
                navigation::Sort::SizeAsc | navigation::Sort::SizeDesc
            )
        }),
        Command::new(
            CommandId::SortType,
            "Sort by type",
            move || !busy.get(),
            move || {
                controls.sort.update(|sort| {
                    *sort = if *sort == navigation::Sort::TypeAsc {
                        navigation::Sort::TypeDesc
                    } else {
                        navigation::Sort::TypeAsc
                    };
                })
            },
        )
        .checked(move || {
            matches!(
                controls.sort.get(),
                navigation::Sort::TypeAsc | navigation::Sort::TypeDesc
            )
        }),
        Command::new(
            CommandId::SortPacked,
            "Sort by packed size",
            move || !busy.get() && !file_view(),
            move || {
                controls.sort.update(|sort| {
                    *sort = if *sort == navigation::Sort::PackedAsc {
                        navigation::Sort::PackedDesc
                    } else {
                        navigation::Sort::PackedAsc
                    };
                })
            },
        )
        .checked(move || {
            matches!(
                controls.sort.get(),
                navigation::Sort::PackedAsc | navigation::Sort::PackedDesc
            )
        }),
    ];
    for (id, label, column) in [
        (
            CommandId::ToggleTypeColumn,
            "Show type column",
            columns::Column::Type,
        ),
        (
            CommandId::ToggleSizeColumn,
            "Show size column",
            columns::Column::Size,
        ),
        (
            CommandId::TogglePackedColumn,
            "Show packed column",
            columns::Column::Packed,
        ),
    ] {
        all.push(
            Command::new(
                id,
                label,
                move || !busy.get() && (id != CommandId::TogglePackedColumn || !file_view()),
                move || controls.visibility.toggle(column),
            )
            .checked(move || controls.visibility.shown(column)),
        );
    }
    all.push(keyboard_shortcut(
        Command::new(
            CommandId::AdjustColumns,
            "Auto adjust columns",
            move || !busy.get(),
            move || controls.widths.adjust(),
        ),
        "mod+alt+a",
    ));
    all.push(Command::new(
        CommandId::About,
        "About PeaZip",
        move || !busy.get(),
        move || {
            let answer = nsh_base::alert_dialog()
                .title("About PeaZip")
                .description(concat!(
                    "PeaZip Rust edition ",
                    env!("CARGO_PKG_VERSION"),
                    "\nRead: ZIP, TAR, TAR.GZ, 7z\nCreate: ZIP, TAR, TAR.GZ\nLGPL-3.0-or-later"
                ))
                .action("close", "Close")
                .show();
            spawn(async move {
                let _ = answer.await;
                #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
                focus_contents();
            });
        },
    ));
    #[cfg(any(
        target_arch = "wasm32",
        target_os = "android",
        target_os = "ios",
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))]
    all.push(desktop_shortcut(
        Command::new(
            CommandId::CreateZip,
            "Create archive…",
            move || !busy.get(),
            move || creating.set(Some(false)),
        )
        .icon("add"),
        "mod+n",
    ));
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    all.push(
        Command::new(
            CommandId::CreateFolderZip,
            "Create folder archive…",
            move || !busy.get(),
            move || creating.set(Some(true)),
        )
        .icon("add"),
    );
    #[cfg(any(target_os = "android", target_os = "ios"))]
    all.push(
        Command::new(
            CommandId::SaveArchive,
            "Save archive…",
            move || !busy.get() && !file_view() && !snapshot.with(|bytes| bytes.is_empty()),
            move || {
                if busy.get_untracked() {
                    return;
                }
                let bytes = snapshot.get_untracked();
                let kind = format.get_untracked();
                busy.set(true);
                activity.set("Saving archive…".into());
                spawn(async move {
                    match mobile_archive::save_archive(bytes, kind).await {
                        Ok(Some(name)) => status.set(format!("Saved archive: {name}")),
                        Ok(None) => status.set("Save cancelled.".into()),
                        Err(error) => status.set(format!("Could not save archive: {error}")),
                    }
                    busy.set(false);
                });
            },
        )
        .icon("save"),
    );
    #[cfg(target_arch = "wasm32")]
    {
        all.push(
            Command::new(
                CommandId::SaveArchive,
                "Download archive",
                move || !busy.get() && !file_view() && !snapshot.with(|bytes| bytes.is_empty()),
                move || {
                    if busy.get_untracked() {
                        return;
                    }
                    match browser::download(&snapshot.get_untracked(), format.get_untracked()) {
                        Ok(()) => status.set("Archive download requested.".into()),
                        Err(error) => status.set(format!("Could not download archive: {error}")),
                    }
                },
            )
            .icon("save"),
        );
        all.push(Command::new(
            CommandId::Cancel,
            "Cancel",
            move || cancellable.get(),
            move || {
                if cancellable.get_untracked() && browser::cancel() {
                    cancellable.set(false);
                    status.set(cancel_message.get_untracked());
                    activity.set("Cancelling…".into());
                }
            },
        ));
    }
    #[cfg(target_arch = "wasm32")]
    let can_extract = browser::can_extract();
    #[cfg(any(
        target_os = "android",
        target_os = "ios",
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))]
    let can_extract = true;
    #[cfg(any(
        target_arch = "wasm32",
        target_os = "android",
        target_os = "ios",
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))]
    let extract = move |request: ExtractionRequest,
                        scope: extract_dialog::ExtractScope,
                        options: peazip_core::ExtractOptions| {
        if busy.get_untracked() {
            return;
        }
        let bytes = request.bytes;
        let paths = match scope {
            extract_dialog::ExtractScope::Selected => request.selected,
            extract_dialog::ExtractScope::Displayed => request.displayed,
            extract_dialog::ExtractScope::All => request.all,
        };
        busy.set(true);
        #[cfg(target_arch = "wasm32")]
        {
            cancellable.set(true);
            cancel_message.set("Extraction cancellation requested.".into());
        }
        activity.set("Extracting archive…".into());
        spawn(async move {
            #[cfg(target_arch = "wasm32")]
            match browser::extract_archive(&bytes, &paths, options, move |message| {
                activity.set(message)
            })
            .await
            {
                Ok(Some(report)) => status.set(report),
                Ok(None) => status.set("Extraction cancelled.".into()),
                Err(error) => status.set(format!("Could not extract archive: {error}")),
            }
            #[cfg(any(target_os = "android", target_os = "ios"))]
            match extract_mobile_archive(bytes, paths, options).await {
                Ok(Some(report)) => status.set(report),
                Ok(None) => status.set("Extraction cancelled.".into()),
                Err(error) => status.set(format!("Could not extract archive: {error}")),
            }
            #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
            match extract_archive(bytes, paths, options).await {
                Ok(Some(count)) => status.set(format!("Extracted {count} entries")),
                Ok(None) => status.set("Extraction cancelled.".into()),
                Err(error) => status.set(format!("Could not extract archive: {error}")),
            }
            busy.set(false);
            #[cfg(target_arch = "wasm32")]
            cancellable.set(false);
        });
    };
    #[cfg(any(
        target_arch = "wasm32",
        target_os = "android",
        target_os = "ios",
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))]
    all.push(desktop_shortcut(
        Command::new(
            CommandId::Extract,
            "Extract…",
            move || !busy.get() && !file_view() && can_extract && !entries.with(Vec::is_empty),
            move || {
                let _ = pending_extract.replace(Some(ExtractionRequest {
                    name: path.get_untracked(),
                    bytes: snapshot.get_untracked(),
                    selected: selected.with(|paths| paths.iter().cloned().collect()),
                    all: entries.with(|rows| rows.iter().map(|row| row.path.clone()).collect()),
                    displayed: entries.with(|items| {
                        controls
                            .visible
                            .with(|rows| navigation::displayed_paths(items, rows))
                    }),
                    displayed_location: {
                        let location = if controls.flat.get_untracked() {
                            "All entries view".into()
                        } else {
                            navigation.with(|state| format!("Current view: /{}", state.current()))
                        };
                        let query = controls.filter.get_untracked();
                        if query.is_empty() {
                            location
                        } else {
                            format!(
                                "{location} · {}: {query}",
                                if controls.wildcards.get_untracked() {
                                    "Name mask"
                                } else {
                                    "Names contain"
                                }
                            )
                        }
                    },
                }));
                extracting.set(true);
            },
        )
        .icon("extract"),
        "mod+enter",
    ));
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    {
        all.push(desktop_shortcut(
            Command::new(
                CommandId::OpenFolder,
                "Open folder…",
                move || !busy.get(),
                move || disk.open_folder(),
            )
            .icon("folder"),
            "mod+shift+o",
        ));
        all.push(desktop_shortcut(
            Command::new(
                CommandId::NewFolder,
                "New folder…",
                move || !busy.get() && files.active.get() && files.current.with(Option::is_some),
                move || {
                    if let Some(view) = files.current.get_untracked() {
                        let _ = new_folder_parent.replace(Some(view.0));
                        new_folder_error.set(String::new());
                        new_folder_open.set(true);
                    }
                },
            )
            .icon("folder"),
            "mod+shift+n",
        ));
        all.push(Command::new(
            CommandId::CopySelected,
            "Copy selected…",
            move || !busy.get() && files.active.get() && !files.selected.with(BTreeSet::is_empty),
            move || disk.copy_selection(),
        ));
        all.push(desktop_shortcut(
            Command::new(
                CommandId::OpenSelected,
                "Open selected",
                move || !busy.get() && disk.can_open_selected(),
                move || disk.open_selected(),
            )
            .icon("open"),
            "mod+shift+enter",
        ));
        all.push(desktop_shortcut(
            Command::new(
                CommandId::RefreshFolder,
                "Refresh folder",
                move || !busy.get() && files.active.get(),
                move || disk.refresh(),
            ),
            "f5",
        ));
        all.push(Command::new(
            CommandId::FilesystemRoot,
            "Filesystem root",
            move || !busy.get() && (!files.active.get() || disk.can_up()),
            move || disk.filesystem_root(),
        ));
    }
    let commands = Commands::new(
        all.into_iter()
            .map(|command| {
                command.in_owner(owner).unless(move || {
                    creating.get().is_some() || extracting.get() || new_folder_open.get()
                })
            })
            .collect(),
    );
    for command in commands.iter() {
        if let Some(key) = command.shortcut {
            let command = command.clone();
            let when = if matches!(
                command.id,
                CommandId::Open | CommandId::OpenFolder | CommandId::FocusPath
            ) {
                nsh_core::keymap::When::Always
            } else {
                nsh_core::keymap::When::NotEditing
            };
            on_key_with(
                key,
                KeyOptions {
                    when,
                    ..Default::default()
                },
                move |_| {
                    if active_menu.get_untracked().is_none() {
                        command.invoke();
                    }
                },
            );
        }
    }
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    menus::install_native(commands.clone(), focus_contents);
    let root = view()
        .flex_1()
        .p_safe_area()
        .bg(move || theme().background)
        .text_color(move || theme().foreground);
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let root = root.child(menus::menu_bar(commands.clone(), active_menu));
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    let root = {
        let file_commands = commands.clone();
        let archive_commands = commands.clone();
        let pane_commands = commands.clone();
        let browser_commands = commands.clone();
        root.child(when(
            move || files.active.get(),
            move || {
                render_toolbar(
                    file_commands.clone(),
                    format,
                    &[
                        CommandId::OpenFolder,
                        CommandId::NewFolder,
                        CommandId::OpenSelected,
                        CommandId::CopySelected,
                        CommandId::CreateZip,
                        CommandId::CreateFolderZip,
                        CommandId::RefreshFolder,
                    ],
                )
            },
        ))
        .child(when(
            move || !files.active.get(),
            move || {
                render_toolbar(
                    archive_commands.clone(),
                    format,
                    &[
                        CommandId::Open,
                        CommandId::OpenFolder,
                        CommandId::CreateZip,
                        CommandId::CreateFolderZip,
                        CommandId::Extract,
                        CommandId::Test,
                    ],
                )
            },
        ))
        .child(when(
            move || files.active.get(),
            move || {
                filesystem::browser(
                    files,
                    editing_locked,
                    controls.sort,
                    pane_commands.clone(),
                    (controls.widths, controls.visibility),
                    move |id| disk.open_entry(id),
                    move |path| disk.go(path),
                )
            },
        ))
        .child(when(
            move || !files.active.get(),
            move || {
                explorer::explorer(explorer::Explorer {
                    entries,
                    selected,
                    busy: editing_locked,
                    name: path,
                    navigation,
                    archive_version,
                    controls,
                    commands: browser_commands.clone(),
                })
            },
        ))
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let root = root
        .child(render_toolbar(
            commands.clone(),
            format,
            &[
                CommandId::Open,
                CommandId::CreateZip,
                CommandId::SaveArchive,
                CommandId::Cancel,
                CommandId::Extract,
                CommandId::Test,
            ],
        ))
        .child(explorer::explorer(explorer::Explorer {
            entries,
            selected,
            busy: editing_locked,
            name: path,
            navigation,
            archive_version,
            controls,
            commands: commands.clone(),
        }));
    #[cfg(target_arch = "wasm32")]
    let root = if can_extract {
        root
    } else {
        root.child(text("Directory export is unavailable in this browser."))
    };
    #[cfg(any(
        target_arch = "wasm32",
        target_os = "android",
        target_os = "ios",
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))]
    let root = root.child(when(
        move || creating.get().is_some(),
        move || {
            let folders = creating.get_untracked().unwrap_or(false);
            let input = if folders {
                "Choose folders in the next step.".to_owned()
            } else {
                "Choose files in the next step.".to_owned()
            };
            #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
            let input = if !folders
                && files.active.get_untracked()
                && !files.selected.with(BTreeSet::is_empty)
            {
                format!(
                    "{} selected items from {}",
                    files.selected.with(BTreeSet::len),
                    files.current.with(|view| view
                        .as_ref()
                        .map(|v| v.0.path().display().to_string())
                        .unwrap_or_default())
                )
            } else {
                input
            };
            create_dialog::show(
                input,
                archive_options.get_untracked(),
                move |options| {
                    owner.run(|| {
                        if creating.get_untracked().is_none() || busy.get_untracked() {
                            return;
                        }
                        archive_options.set(options);
                        creating.set(None);
                        create(folders, options);
                    })
                },
                move || {
                    owner.run(|| {
                        creating.set(None);
                        after_answer(move || {
                            #[cfg(any(
                                target_os = "macos",
                                target_os = "linux",
                                target_os = "windows"
                            ))]
                            focus_contents();
                            #[cfg(not(any(
                                target_os = "macos",
                                target_os = "linux",
                                target_os = "windows"
                            )))]
                            controls.content.focus();
                        });
                    })
                },
            )
        },
    ));
    #[cfg(any(
        target_arch = "wasm32",
        target_os = "android",
        target_os = "ios",
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))]
    let root = root.child(when(
        move || extracting.get(),
        move || {
            let request = pending_extract
                .get()
                .expect("open extraction has a snapshot");
            extract_dialog::show(
                extract_dialog::Summary {
                    name: request.name,
                    selected_count: request.selected.len(),
                    total_count: request.all.len(),
                    displayed_count: request.displayed.len(),
                    displayed_location: request.displayed_location,
                },
                extract_options.get_untracked(),
                move |scope, options| {
                    owner.run(|| {
                        if !extracting.get_untracked() || busy.get_untracked() {
                            return;
                        }
                        let request = pending_extract.replace(None);
                        extracting.set(false);
                        if let Some(request) = request {
                            extract_options.set(options);
                            extract(request, scope, options);
                        }
                    })
                },
                move || {
                    owner.run(|| {
                        let _ = pending_extract.replace(None);
                        extracting.set(false);
                        after_answer(move || controls.content.focus());
                    })
                },
            )
        },
    ));
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    let root = root.child(when(
        move || new_folder_open.get(),
        move || {
            let parent = new_folder_parent
                .get()
                .expect("new folder dialog has a directory snapshot");
            new_folder_dialog::show(
                parent.path().display().to_string(),
                new_folder_error,
                busy,
                move |name| {
                    owner.run(|| {
                        if busy.get_untracked() || !new_folder_open.get_untracked() {
                            return;
                        }
                        let Some(parent) = new_folder_parent.get() else {
                            return;
                        };
                        busy.set(true);
                        activity.set("Creating folder…".into());
                        new_folder_error.set(String::new());
                        spawn(async move {
                            match disk.create_folder(parent, name).await {
                                Ok(()) => {
                                    let _ = new_folder_parent.replace(None);
                                    new_folder_open.set(false);
                                    after_answer(move || files.content.focus());
                                }
                                Err(error) => new_folder_error.set(error),
                            }
                            busy.set(false);
                        });
                    })
                },
                move || {
                    owner.run(|| {
                        if busy.get_untracked() {
                            return;
                        }
                        let _ = new_folder_parent.replace(None);
                        new_folder_open.set(false);
                        after_answer(move || files.content.focus());
                    })
                },
            )
        },
    ));
    root.child(
        text(move || {
            if busy.get() {
                activity.get()
            } else {
                status.get()
            }
        })
        .p_2()
        .border_t_1()
        .border_color(move || theme().border),
    )
}

fn archive_label(format: ArchiveFormat) -> &'static str {
    match format {
        ArchiveFormat::Zip => "ZIP",
        ArchiveFormat::Tar => "TAR",
        ArchiveFormat::TarGzip => "TAR.GZ",
        ArchiveFormat::SevenZip => "7Z",
    }
}

#[cfg(any(
    target_arch = "wasm32",
    target_os = "android",
    target_os = "ios",
    target_os = "macos",
    target_os = "linux",
    target_os = "windows"
))]
fn archive_filename(format: ArchiveFormat) -> &'static str {
    match format {
        ArchiveFormat::Zip => "Archive.zip",
        ArchiveFormat::Tar => "Archive.tar",
        ArchiveFormat::TarGzip => "Archive.tar.gz",
        ArchiveFormat::SevenZip => "Archive.7z",
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn creation_save_dialog(options: peazip_core::CreateOptions) -> nsh_core::file_dialog::SaveDialog {
    use nsh_core::file_dialog::{FileFilter, SaveDialog};
    let filter = match options.format() {
        ArchiveFormat::Zip => FileFilter::new("ZIP archives", ["zip"]),
        ArchiveFormat::Tar => FileFilter::new("TAR archives", ["tar"]),
        ArchiveFormat::TarGzip => FileFilter::new("TAR.GZ archives", ["gz", "tgz"]),
        ArchiveFormat::SevenZip => unreachable!("7z creation is not an available option"),
    };
    SaveDialog::new()
        .title("Create archive — choose a new filename")
        .button_label("Create")
        .file_name(archive_filename(options.format()))
        .filter(filter)
}

fn render_toolbar(
    commands: Commands,
    format: Signal<ArchiveFormat>,
    ids: &'static [CommandId],
) -> impl IntoElement {
    let mut bar = toolbar()
        .flex_wrap()
        .items_center()
        .gap_3()
        .p_2()
        .border_b_1()
        .border_color(move || theme().border);
    for &id in ids {
        if let Some(command) = commands.get(id) {
            let label = command.label;
            bar = bar.child(action_button(&command).child(text(move || {
                if id == CommandId::SaveArchive {
                    let kind = archive_label(format.get());
                    if cfg!(target_arch = "wasm32") {
                        format!("Download {kind}")
                    } else {
                        format!("Save {kind}…")
                    }
                } else {
                    label.to_owned()
                }
            })));
        }
    }
    bar
}

fn action_button(command: &Command) -> nsh_base::ButtonElement {
    let button = commands::button(command)
        .px_3()
        .py_2()
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().muted)
        .active(|style| style.opacity(0.6))
        .disabled_style(|style| style.opacity(0.4))
        .flex_row()
        .items_center()
        .gap_2();
    match command.icon {
        Some(icon) => button.child(icons::icon(icon)),
        None => button,
    }
}

fn keyboard_shortcut(command: Command, key: &'static str) -> Command {
    if cfg!(any(
        target_arch = "wasm32",
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    )) {
        command.shortcut(key)
    } else {
        command
    }
}

fn desktop_shortcut(command: Command, key: &'static str) -> Command {
    if cfg!(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    )) {
        command.shortcut(key)
    } else {
        command
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
async fn extract_mobile_archive(
    bytes: Arc<Vec<u8>>,
    selected: Vec<String>,
    options: peazip_core::ExtractOptions,
) -> Result<Option<String>, String> {
    let entries = blocking(move || {
        peazip_core::extract_selected_archive_with_options(
            &bytes,
            &selected,
            peazip_core::Limits::default(),
            options,
        )
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    mobile_archive::export_entries(entries).await
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
async fn create_archive(
    folders: bool,
    _activity: Signal<String>,
    options: peazip_core::CreateOptions,
) -> Result<Option<OpenedArchive>, String> {
    use nsh_core::file_dialog::OpenDialog;
    options.validate().map_err(|error| error.to_string())?;
    let dialog = if folders {
        OpenDialog::folders()
            .title("Select folders to archive")
            .button_label("Add folders")
    } else {
        OpenDialog::files()
            .title("Select files to archive")
            .button_label("Add files")
    };
    let Some(paths) = pick_paths(dialog.multiple(true))
        .await?
        .filter(|paths| !paths.is_empty())
    else {
        return Ok(None);
    };
    let Some(destination) = pick_save_path(creation_save_dialog(options)).await? else {
        return Ok(None);
    };
    let destination = std::path::absolute(destination).map_err(|error| error.to_string())?;
    let name = destination.display().to_string();
    let source_path = Some(destination.clone());
    let rows = blocking(move || {
        peazip_native::create_archive_from_paths(
            &paths,
            &destination,
            peazip_core::Limits::default(),
            options,
        )
        .map_err(|error| error.to_string())?;
        read_archive(&destination)
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(Some(OpenedArchive {
        name,
        data: rows,
        source_path,
    }))
}

#[cfg(target_arch = "wasm32")]
async fn create_archive(
    _folders: bool,
    activity: Signal<String>,
    options: peazip_core::CreateOptions,
) -> Result<Option<OpenedArchive>, String> {
    let Some(data) =
        browser::create_and_list_archive(options, move |message| activity.set(message)).await?
    else {
        return Ok(None);
    };
    Ok(Some(OpenedArchive {
        name: archive_filename(data.2).into(),
        data,
        source_path: None,
    }))
}

#[cfg(any(target_os = "android", target_os = "ios"))]
async fn create_archive(
    _folders: bool,
    activity: Signal<String>,
    options: peazip_core::CreateOptions,
) -> Result<Option<OpenedArchive>, String> {
    options.validate().map_err(|error| error.to_string())?;
    let Some(files) = mobile_archive::pick_files().await? else {
        return Ok(None);
    };
    activity.set(format!("Creating {}…", archive_label(options.format())));
    let archive = blocking(move || {
        let bytes = peazip_core::create_archive(&files, peazip_core::Limits::default(), options)
            .map_err(|error| error.to_string())?;
        load_archive(Arc::new(bytes))
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(Some(OpenedArchive {
        name: archive_filename(archive.2).into(),
        data: archive,
        source_path: None,
    }))
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
async fn choose_archive() -> Result<Option<OpenedArchive>, String> {
    use nsh_core::file_dialog::{FileFilter, OpenDialog};
    let paths = pick_paths(
        OpenDialog::files()
            .title("Open archive")
            .button_label("Open")
            .filter(FileFilter::new(
                "Archives",
                ["zip", "tar", "gz", "tgz", "7z"],
            )),
    )
    .await?;
    let Some(path) = paths.and_then(|paths| paths.into_iter().next()) else {
        return Ok(None);
    };
    let path = std::path::absolute(path).map_err(|error| error.to_string())?;
    let name = path.display().to_string();
    let source_path = Some(path.clone());
    let entries = blocking(move || read_archive(&path))
        .await
        .map_err(|error| error.to_string())??;
    Ok(Some(OpenedArchive {
        name,
        data: entries,
        source_path,
    }))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
async fn choose_archive(
    #[cfg(target_arch = "wasm32")] cancellable: Signal<bool>,
) -> Result<Option<OpenedArchive>, String> {
    use nsh_core::{file_dialog::FileFilter, picker::FilePickRequest};
    let files = pick_files(FilePickRequest::new(false, 1).filter(FileFilter::new(
        "Archives",
        ["zip", "tar", "gz", "tgz", "7z"],
    )))
    .await;
    let Some(file) = files.into_iter().next() else {
        return Ok(None);
    };
    let limits = peazip_core::Limits::default();
    // NSH's picker reads up to 512 MiB before delivery; its API has no per-request byte limit.
    if file.data.len() as u64 > limits.max_archive_bytes {
        return Err("Archive exceeds the 32 MiB limit.".to_owned());
    }
    let bytes = Arc::new(file.data.bytes().to_vec());
    #[cfg(not(target_arch = "wasm32"))]
    let entries = blocking(move || load_archive(bytes))
        .await
        .map_err(|error| error.to_string())??;
    #[cfg(target_arch = "wasm32")]
    let entries = {
        cancellable.set(true);
        let Some((format, rows)) = browser::list_archive(Arc::clone(&bytes)).await? else {
            return Ok(None);
        };
        (bytes, rows, format)
    };
    Ok(Some(OpenedArchive {
        name: file.name,
        data: entries,
        source_path: None,
    }))
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn read_archive(path: &std::path::Path) -> Result<LoadedArchive, String> {
    let bytes = peazip_native::read_archive_bytes(path, peazip_core::Limits::default())
        .map_err(|error| error.to_string())?;
    load_archive(Arc::new(bytes))
}

#[cfg(not(target_arch = "wasm32"))]
fn load_archive(bytes: Arc<Vec<u8>>) -> Result<LoadedArchive, String> {
    let (format, rows) =
        peazip_core::list_archive_with_format(&bytes, peazip_core::Limits::default())
            .map_err(|error| error.to_string())?;
    Ok((bytes, rows, format))
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
async fn extract_archive(
    bytes: Arc<Vec<u8>>,
    selected: Vec<String>,
    options: peazip_core::ExtractOptions,
) -> Result<Option<usize>, String> {
    use nsh_core::file_dialog::SaveDialog;
    let Some(destination) = pick_save_path(
        SaveDialog::new()
            .title("Extract — choose new folder name")
            .button_label("Extract")
            .file_name("Extracted"),
    )
    .await?
    else {
        return Ok(None);
    };
    blocking(move || {
        peazip_native::extract_archive_to_directory_with_options(
            &bytes,
            &selected,
            &destination,
            peazip_core::Limits::default(),
            options,
        )
    })
    .await
    .map_err(|error| error.to_string())?
    .map(Some)
    .map_err(|error| error.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
async fn test_archive(bytes: Arc<Vec<u8>>) -> Result<peazip_core::TestReport, String> {
    blocking(move || peazip_core::test_archive(&bytes, peazip_core::Limits::default()))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}
