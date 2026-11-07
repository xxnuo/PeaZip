use std::{collections::BTreeSet, sync::Arc};

use nsh_base::{button, checkbox, theme, toolbar};
use nsh_core::{element::Position, prelude::*};
use peazip_core::{ArchiveFormat, Entry};

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
    let path = signal(String::new());
    let status = signal("Open an archive to view its contents.".to_owned());
    let busy = signal(false);
    #[cfg(target_arch = "wasm32")]
    let cancellable = signal(false);
    #[cfg(target_arch = "wasm32")]
    let cancel_message = signal("Creation cancelled.".to_owned());
    let activity = signal("Opening archive…".to_owned());
    let selected = signal(BTreeSet::<String>::new());
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
                    selected.set(BTreeSet::new());
                }
                Ok(None) => {}
                Err(error) => status.set(format!("Could not open archive: {error}")),
            }
            busy.set(false);
        });
    };
    let actions = toolbar()
        .flex_wrap()
        .items_center()
        .gap_3()
        .p_2()
        .border_b_1()
        .border_color(move || theme().border)
        .child(
            button()
                .disabled(move || busy.get())
                .on_press(open)
                .px_3()
                .py_2()
                .border_1()
                .border_color(move || theme().border)
                .bg(move || theme().muted)
                .active(|s| s.opacity(0.6))
                .disabled_style(|s| s.opacity(0.4))
                .child(text("Open archive…")),
        );
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
    #[cfg(any(
        target_arch = "wasm32",
        target_os = "android",
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))]
    let actions = actions.child(
        button()
            .disabled(move || busy.get())
            .on_press(move || create(false))
            .px_3()
            .py_2()
            .border_1()
            .border_color(move || theme().border)
            .bg(move || theme().muted)
            .active(|s| s.opacity(0.6))
            .disabled_style(|s| s.opacity(0.4))
            .child(text("Create ZIP…")),
    );
    #[cfg(target_os = "android")]
    let actions = actions.child(
        button()
            .disabled(move || busy.get() || snapshot.with(|bytes| bytes.is_empty()))
            .on_press(move || {
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
            })
            .px_3()
            .py_2()
            .border_1()
            .border_color(move || theme().border)
            .bg(move || theme().muted)
            .active(|s| s.opacity(0.6))
            .disabled_style(|s| s.opacity(0.4))
            .child(text(move || match format.get() {
                ArchiveFormat::Zip => "Save ZIP…",
                ArchiveFormat::Tar => "Save TAR…",
                ArchiveFormat::TarGzip => "Save TAR.GZ…",
            })),
    );
    #[cfg(target_arch = "wasm32")]
    let actions = actions.child(
        button()
            .disabled(move || busy.get() || snapshot.with(|bytes| bytes.is_empty()))
            .on_press(move || {
                if busy.get_untracked() {
                    return;
                }
                match browser::download(&snapshot.get_untracked(), format.get_untracked()) {
                    Ok(()) => status.set("Archive download requested.".into()),
                    Err(error) => status.set(format!("Could not download archive: {error}")),
                }
            })
            .px_3()
            .py_2()
            .border_1()
            .border_color(move || theme().border)
            .bg(move || theme().muted)
            .active(|s| s.opacity(0.6))
            .disabled_style(|s| s.opacity(0.4))
            .child(text(move || match format.get() {
                ArchiveFormat::Zip => "Download ZIP",
                ArchiveFormat::Tar => "Download TAR",
                ArchiveFormat::TarGzip => "Download TAR.GZ",
            })),
    );
    #[cfg(target_arch = "wasm32")]
    let actions = actions.child(
        button()
            .disabled(move || !cancellable.get())
            .on_press(move || {
                if cancellable.get_untracked() && browser::cancel() {
                    cancellable.set(false);
                    status.set(cancel_message.get_untracked());
                    activity.set("Cancelling…".into());
                }
            })
            .px_3()
            .py_2()
            .border_1()
            .border_color(move || theme().border)
            .bg(move || theme().muted)
            .active(|s| s.opacity(0.6))
            .disabled_style(|s| s.opacity(0.4))
            .child(text("Cancel")),
    );
    #[cfg(target_arch = "wasm32")]
    let can_extract = browser::can_extract();
    #[cfg(target_arch = "wasm32")]
    let actions = actions.child(
        button()
            .disabled(move || busy.get() || selected.with(BTreeSet::is_empty) || !can_extract)
            .on_press(move || {
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
            })
            .px_3()
            .py_2()
            .border_1()
            .border_color(move || theme().border)
            .bg(move || theme().muted)
            .active(|s| s.opacity(0.6))
            .disabled_style(|s| s.opacity(0.4))
            .child(text("Extract selected…")),
    );
    #[cfg(target_arch = "wasm32")]
    let actions = if can_extract {
        actions
    } else {
        actions.child(text("Directory export is unavailable in this browser."))
    };
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    let actions = actions.child(
        button()
            .disabled(move || busy.get())
            .on_press(move || create(true))
            .px_3()
            .py_2()
            .border_1()
            .border_color(move || theme().border)
            .bg(move || theme().muted)
            .active(|s| s.opacity(0.6))
            .disabled_style(|s| s.opacity(0.4))
            .child(text("Create folder ZIP…")),
    );
    #[cfg(any(
        target_os = "android",
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))]
    let actions = actions.child(
        button()
            .disabled(move || busy.get() || selected.with(BTreeSet::is_empty))
            .on_press(move || {
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
            })
            .px_3()
            .py_2()
            .border_1()
            .border_color(move || theme().border)
            .bg(move || theme().muted)
            .active(|s| s.opacity(0.6))
            .disabled_style(|s| s.opacity(0.4))
            .child(text("Extract selected…")),
    );
    view()
        .flex_1()
        .p_safe_area()
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        .child(
            actions.child(text(move || path.get()).flex_1().min_w(move || {
                px(if path.with(String::is_empty) {
                    0.0
                } else {
                    200.0
                })
            })),
        )
        .child(
            toolbar()
                .flex_row()
                .gap_3()
                .px_3()
                .py_2()
                .child(
                    button()
                        .disabled(move || busy.get() || entries.with(Vec::is_empty))
                        .on_press(move || {
                            if busy.get_untracked() {
                                return;
                            }
                            selected
                                .set(entries.with(|rows| {
                                    rows.iter().map(|row| row.path.clone()).collect()
                                }));
                        })
                        .child(text("Select all")),
                )
                .child(
                    button()
                        .disabled(move || busy.get() || selected.with(BTreeSet::is_empty))
                        .on_press(move || {
                            if !busy.get_untracked() {
                                selected.set(BTreeSet::new());
                            }
                        })
                        .child(text("Clear selection")),
                )
                .child(text(move || {
                    format!("{} selected", selected.with(BTreeSet::len))
                })),
        )
        .child(
            view()
                .flex_row()
                .px_3()
                .py_2()
                .bg(move || theme().muted)
                .child(text("Name").flex_1())
                .child(text("Size (bytes)").w(px(140.0))),
        )
        .child(
            virtual_list(
                move || entries.with(Vec::len),
                move |at: Position| {
                    checkbox()
                        .accessibility_label(move || {
                            entries.with(|rows| {
                                rows.get(at.index()).map(|row| row.path.clone().into())
                            })
                        })
                        .checked(move || {
                            entries.with(|rows| {
                                rows.get(at.index()).is_some_and(|row| {
                                    selected.with(|paths| paths.contains(&row.path))
                                })
                            })
                        })
                        .disabled(move || busy.get())
                        .on_change(move |checked: &bool| {
                            if busy.get_untracked() {
                                return;
                            }
                            if let Some(name) = entries
                                .with(|rows| rows.get(at.index()).map(|row| row.path.clone()))
                            {
                                selected.update(|paths| {
                                    if *checked {
                                        paths.insert(name);
                                    } else {
                                        paths.remove(&name);
                                    }
                                });
                            }
                        })
                        .flex_row()
                        .px_3()
                        .py_2()
                        .child(
                            text(move || {
                                let checked = entries.with(|rows| {
                                    rows.get(at.index()).is_some_and(|row| {
                                        selected.with(|paths| paths.contains(&row.path))
                                    })
                                });
                                if checked { "[x]" } else { "[ ]" }
                            })
                            .w(px(32.0)),
                        )
                        .child(
                            text(move || {
                                entries.with(|rows| {
                                    rows.get(at.index())
                                        .map(|row| row.path.clone())
                                        .unwrap_or_default()
                                })
                            })
                            .flex_1()
                            .pr_2(),
                        )
                        .child(
                            text(move || {
                                entries.with(|rows| {
                                    rows.get(at.index())
                                        .map(|row| {
                                            if row.is_dir {
                                                "—".to_owned()
                                            } else {
                                                row.size.to_string()
                                            }
                                        })
                                        .unwrap_or_default()
                                })
                            })
                            .w(px(140.0)),
                        )
                },
            )
            .estimated_item_size(px(36.0))
            .flex_1(),
        )
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
            .filter(FileFilter::new("Archives", ["zip", "tar", "gz", "tgz"])),
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
    let files = pick_files(
        FilePickRequest::new(false, 1)
            .filter(FileFilter::new("Archives", ["zip", "tar", "gz", "tgz"])),
    )
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
