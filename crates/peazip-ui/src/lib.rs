use std::{collections::BTreeSet, sync::Arc};

use commands::{Command, CommandId, Commands};
use nsh_base::{theme, toolbar};
use nsh_core::prelude::*;
use peazip_core::{ArchiveFormat, Entry};

mod commands;
mod explorer;
mod icons;
mod menus;
mod navigation;

type LoadedArchive = (Arc<Vec<u8>>, Vec<Entry>, ArchiveFormat);

#[cfg(target_arch = "wasm32")]
mod browser;

#[cfg(target_os = "android")]
mod android;

#[native_module("peazip/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

#[component]
pub fn App() -> impl IntoElement {
    let snapshot = signal(Arc::new(Vec::<u8>::new()));
    let format = signal(ArchiveFormat::Zip);
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
                Ok(Some((name, (bytes, rows, kind)))) => {
                    format.set(kind);
                    snapshot.set(bytes);
                    status.set(format!("{} entries", rows.len()));
                    path.set(name);
                    entries.set(rows);
                    navigation.set(navigation::BrowserState::default());
                    archive_version.update(|version| *version = version.wrapping_add(1));
                    selected.set(BTreeSet::new());
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
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))]
    let create = move |folders: bool| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        #[cfg(target_arch = "wasm32")]
        cancellable.set(true);
        #[cfg(target_arch = "wasm32")]
        cancel_message.set("Creation cancelled.".into());
        activity.set("Creating ZIP…".to_owned());
        spawn(async move {
            match create_archive(folders, activity).await {
                Ok(Some((name, (bytes, rows, kind)))) => {
                    format.set(kind);
                    snapshot.set(bytes);
                    status.set(format!("Created ZIP: {} entries", rows.len()));
                    path.set(name);
                    entries.set(rows);
                    navigation.set(navigation::BrowserState::default());
                    archive_version.update(|version| *version = version.wrapping_add(1));
                    selected.set(BTreeSet::new());
                }
                Ok(None) => {}
                Err(error) => status.set(format!("Could not create ZIP: {error}")),
            }
            busy.set(false);
            #[cfg(target_arch = "wasm32")]
            cancellable.set(false);
        });
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
                move || !busy.get() && !snapshot.with(|bytes| bytes.is_empty()),
                move || {
                    snapshot.set(Arc::new(Vec::new()));
                    entries.set(Vec::new());
                    selected.set(BTreeSet::new());
                    path.set(String::new());
                    format.set(ArchiveFormat::Zip);
                    navigation.set(navigation::BrowserState::default());
                    archive_version.update(|version| *version = version.wrapping_add(1));
                    status.set("Open an archive to view its contents.".into());
                },
            ),
            "mod+w",
        ),
        Command::new(
            CommandId::Test,
            "Test archive",
            move || !busy.get() && !snapshot.with(|bytes| bytes.is_empty()),
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
            Command::new(
                CommandId::SelectAll,
                "Select all",
                move || !busy.get() && !controls.visible.with(Vec::is_empty),
                move || {
                    let paths = entries.with(|items| {
                        controls.visible.with(|rows| {
                            rows.iter()
                                .flat_map(|row| navigation::selection_paths(row, items))
                                .collect()
                        })
                    });
                    selected.set(paths);
                },
            ),
            "mod+a",
        ),
        keyboard_shortcut(
            Command::new(
                CommandId::ClearSelection,
                "Clear selection",
                move || !busy.get() && !selected.with(BTreeSet::is_empty),
                move || selected.set(BTreeSet::new()),
            ),
            "mod+shift+a",
        ),
        keyboard_shortcut(
            Command::new(
                CommandId::Back,
                "Back",
                move || !busy.get() && navigation.with(navigation::BrowserState::can_back),
                move || {
                    navigation.update(|state| {
                        state.back();
                    })
                },
            )
            .icon("back"),
            "alt+left",
        ),
        keyboard_shortcut(
            Command::new(
                CommandId::Forward,
                "Forward",
                move || !busy.get() && navigation.with(navigation::BrowserState::can_forward),
                move || {
                    navigation.update(|state| {
                        state.forward();
                    })
                },
            )
            .icon("forward"),
            "alt+right",
        ),
        keyboard_shortcut(
            Command::new(
                CommandId::Up,
                "Up",
                move || !busy.get() && !navigation.with(|state| state.current().is_empty()),
                move || {
                    navigation.update(|state| {
                        state.up();
                    })
                },
            )
            .icon("up"),
            "alt+up",
        ),
        Command::new(
            CommandId::Root,
            "Archive root",
            move || !busy.get() && !navigation.with(|state| state.current().is_empty()),
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
            move || !busy.get(),
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
    ];
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
                    "\nRead: ZIP, TAR, TAR.GZ, 7z\nCreate: ZIP\nLGPL-3.0-or-later"
                ))
                .action("close", "Close")
                .show();
            spawn(async move {
                let _ = answer.await;
                #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
                controls.content.focus();
            });
        },
    ));
    #[cfg(any(
        target_arch = "wasm32",
        target_os = "android",
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))]
    all.push(desktop_shortcut(
        Command::new(
            CommandId::CreateZip,
            "Create ZIP…",
            move || !busy.get(),
            move || create(false),
        )
        .icon("add"),
        "mod+n",
    ));
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    all.push(
        Command::new(
            CommandId::CreateFolderZip,
            "Create folder ZIP…",
            move || !busy.get(),
            move || create(true),
        )
        .icon("add"),
    );
    #[cfg(target_os = "android")]
    all.push(
        Command::new(
            CommandId::SaveArchive,
            "Save archive…",
            move || !busy.get() && !snapshot.with(|bytes| bytes.is_empty()),
            move || {
                if busy.get_untracked() {
                    return;
                }
                let bytes = snapshot.get_untracked();
                let kind = format.get_untracked();
                busy.set(true);
                activity.set("Saving archive…".into());
                spawn(async move {
                    match android::save_archive(bytes, kind).await {
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
                move || !busy.get() && !snapshot.with(|bytes| bytes.is_empty()),
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
    #[cfg(target_arch = "wasm32")]
    all.push(
        Command::new(
            CommandId::Extract,
            "Extract selected…",
            move || !busy.get() && !selected.with(BTreeSet::is_empty) && can_extract,
            move || {
                if busy.get_untracked() || !can_extract || selected.with(BTreeSet::is_empty) {
                    return;
                }
                let bytes = snapshot.get_untracked();
                let paths = selected.with(|paths| paths.iter().cloned().collect::<Vec<_>>());
                busy.set(true);
                cancellable.set(true);
                cancel_message.set("Extraction cancellation requested.".into());
                activity.set("Choose an export directory…".into());
                spawn(async move {
                    match browser::extract_archive(&bytes, &paths, move |message| {
                        activity.set(message)
                    })
                    .await
                    {
                        Ok(Some(report)) => status.set(report),
                        Ok(None) => status.set("Extraction cancelled.".into()),
                        Err(error) => status.set(format!("Could not extract archive: {error}")),
                    }
                    busy.set(false);
                    cancellable.set(false);
                });
            },
        )
        .icon("extract"),
    );
    #[cfg(any(
        target_os = "android",
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))]
    all.push(desktop_shortcut(
        Command::new(
            CommandId::Extract,
            "Extract selected…",
            move || !busy.get() && !selected.with(BTreeSet::is_empty),
            move || {
                if busy.get_untracked() || selected.with(BTreeSet::is_empty) {
                    return;
                }
                let bytes = snapshot.get_untracked();
                let paths = selected.with(|paths| paths.iter().cloned().collect::<Vec<_>>());
                busy.set(true);
                activity.set("Extracting archive…".to_owned());
                spawn(async move {
                    #[cfg(target_os = "android")]
                    match extract_android_archive(bytes, paths).await {
                        Ok(Some(report)) => status.set(report),
                        Ok(None) => status.set("Extraction cancelled.".into()),
                        Err(error) => status.set(format!("Could not extract archive: {error}")),
                    }
                    #[cfg(not(target_os = "android"))]
                    match extract_archive(bytes, paths).await {
                        Ok(Some(count)) => status.set(format!("Extracted {count} entries")),
                        Ok(None) => {}
                        Err(error) => status.set(format!("Could not extract archive: {error}")),
                    }
                    busy.set(false);
                });
            },
        )
        .icon("extract"),
        "mod+enter",
    ));
    let commands = Commands::new(all);
    for command in commands.iter() {
        if let Some(key) = command.shortcut {
            let command = command.clone();
            let when = if matches!(command.id, CommandId::Open | CommandId::FocusPath) {
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
    menus::install_native(commands.clone(), controls.content);
    let mut actions = toolbar()
        .flex_wrap()
        .items_center()
        .gap_3()
        .p_2()
        .border_b_1()
        .border_color(move || theme().border);
    for id in [
        CommandId::Open,
        CommandId::CreateZip,
        CommandId::SaveArchive,
        CommandId::Cancel,
        CommandId::CreateFolderZip,
        CommandId::Extract,
        CommandId::Test,
    ] {
        if let Some(command) = commands.get(id) {
            let label = command.label;
            actions = actions.child(action_button(&command).child(text(move || {
                if id == CommandId::SaveArchive {
                    let kind = match format.get() {
                        ArchiveFormat::Zip => "ZIP",
                        ArchiveFormat::Tar => "TAR",
                        ArchiveFormat::TarGzip => "TAR.GZ",
                        ArchiveFormat::SevenZip => "7Z",
                    };
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
    #[cfg(target_arch = "wasm32")]
    if !can_extract {
        actions = actions.child(text("Directory export is unavailable in this browser."));
    }
    let root = view()
        .flex_1()
        .p_safe_area()
        .bg(move || theme().background)
        .text_color(move || theme().foreground);
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let root = root.child(menus::menu_bar(commands.clone(), active_menu));
    root.child(actions)
        .child(explorer::explorer(explorer::Explorer {
            entries,
            selected,
            busy,
            name: path,
            navigation,
            archive_version,
            controls,
            commands,
        }))
        .child(
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

#[cfg(target_os = "android")]
async fn extract_android_archive(
    bytes: Arc<Vec<u8>>,
    selected: Vec<String>,
) -> Result<Option<String>, String> {
    let entries = blocking(move || {
        peazip_core::extract_selected_archive(&bytes, &selected, peazip_core::Limits::default())
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    android::export_entries(entries).await
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
async fn create_archive(
    folders: bool,
    _activity: Signal<String>,
) -> Result<Option<(String, LoadedArchive)>, String> {
    use nsh_core::file_dialog::{FileFilter, OpenDialog, SaveDialog};
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
    let Some(destination) = pick_save_path(
        SaveDialog::new()
            .title("Create ZIP — choose a new filename")
            .button_label("Create")
            .file_name("Archive.zip")
            .filter(FileFilter::new("ZIP archives", ["zip"])),
    )
    .await?
    else {
        return Ok(None);
    };
    let name = destination.display().to_string();
    let rows = blocking(move || {
        peazip_native::create_zip_from_paths(&paths, &destination, peazip_core::Limits::default())
            .map_err(|error| error.to_string())?;
        read_archive(&destination)
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(Some((name, rows)))
}

#[cfg(target_arch = "wasm32")]
async fn create_archive(
    _folders: bool,
    activity: Signal<String>,
) -> Result<Option<(String, LoadedArchive)>, String> {
    let Some(bytes) = browser::create_zip(move |message| activity.set(message)).await? else {
        return Ok(None);
    };
    Ok(Some(("Archive.zip".into(), load_archive(Arc::new(bytes))?)))
}

#[cfg(target_os = "android")]
async fn create_archive(
    _folders: bool,
    activity: Signal<String>,
) -> Result<Option<(String, LoadedArchive)>, String> {
    let Some(files) = android::pick_files().await? else {
        return Ok(None);
    };
    activity.set("Compressing ZIP…".into());
    let archive = blocking(move || {
        let bytes = peazip_core::create_zip(&files, peazip_core::Limits::default())
            .map_err(|error| error.to_string())?;
        load_archive(Arc::new(bytes))
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(Some(("Archive.zip".into(), archive)))
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
async fn choose_archive() -> Result<Option<(String, LoadedArchive)>, String> {
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
    let name = path.display().to_string();
    let entries = blocking(move || read_archive(&path))
        .await
        .map_err(|error| error.to_string())??;
    Ok(Some((name, entries)))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
async fn choose_archive(
    #[cfg(target_arch = "wasm32")] cancellable: Signal<bool>,
) -> Result<Option<(String, LoadedArchive)>, String> {
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
    Ok(Some((file.name, entries)))
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn read_archive(path: &std::path::Path) -> Result<LoadedArchive, String> {
    let bytes = peazip_native::read_archive_bytes(path, peazip_core::Limits::default())
        .map_err(|error| error.to_string())?;
    load_archive(Arc::new(bytes))
}

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
) -> Result<Option<usize>, String> {
    use nsh_core::file_dialog::SaveDialog;
    let Some(destination) = pick_save_path(
        SaveDialog::new()
            .title("Extract selected — choose new folder name")
            .button_label("Extract")
            .file_name("Extracted"),
    )
    .await?
    else {
        return Ok(None);
    };
    blocking(move || {
        peazip_native::extract_archive_to_directory(
            &bytes,
            &selected,
            &destination,
            peazip_core::Limits::default(),
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
