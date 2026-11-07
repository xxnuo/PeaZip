use std::{collections::BTreeSet, sync::Arc};

use nsh_base::{button, checkbox, theme, toolbar};
use nsh_core::{element::Position, prelude::*};
use peazip_core::Entry;

type LoadedArchive = (Arc<Vec<u8>>, Vec<Entry>);

#[native_module("peazip/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

#[component]
pub fn App() -> impl IntoElement {
    let snapshot = signal(Arc::new(Vec::<u8>::new()));
    let entries = signal(Vec::<Entry>::new());
    let path = signal(String::new());
    let status = signal("Open a ZIP archive to view its contents.".to_owned());
    let busy = signal(false);
    let activity = signal("Opening ZIP…".to_owned());
    let selected = signal(BTreeSet::<String>::new());
    let open = move || {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        activity.set("Opening ZIP…".to_owned());
        spawn(async move {
            match choose_archive().await {
                Ok(Some((name, (bytes, rows)))) => {
                    snapshot.set(bytes);
                    status.set(format!("{} entries", rows.len()));
                    path.set(name);
                    entries.set(rows);
                    selected.set(BTreeSet::new());
                }
                Ok(None) => {}
                Err(error) => status.set(format!("Could not open ZIP: {error}")),
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
                .child(text("Open ZIP…")),
        );
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    let create = move |folders: bool| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        activity.set("Creating ZIP…".to_owned());
        spawn(async move {
            match create_archive(folders).await {
                Ok(Some((name, (bytes, rows)))) => {
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
        });
    };
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
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
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
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
                activity.set("Extracting ZIP…".to_owned());
                spawn(async move {
                    match extract_archive(bytes, paths).await {
                        Ok(Some(count)) => status.set(format!("Extracted {count} entries")),
                        Ok(None) => {}
                        Err(error) => status.set(format!("Could not extract ZIP: {error}")),
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
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        .child(actions.child(text(move || path.get()).flex_1()))
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
                            .flex_1(),
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

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
async fn create_archive(folders: bool) -> Result<Option<(String, LoadedArchive)>, String> {
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

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
async fn choose_archive() -> Result<Option<(String, LoadedArchive)>, String> {
    use nsh_core::file_dialog::{FileFilter, OpenDialog};
    let paths = pick_paths(
        OpenDialog::files()
            .title("Open ZIP archive")
            .button_label("Open")
            .filter(FileFilter::new("ZIP archives", ["zip"])),
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
async fn choose_archive() -> Result<Option<(String, LoadedArchive)>, String> {
    use nsh_core::{file_dialog::FileFilter, picker::FilePickRequest};
    let files =
        pick_files(FilePickRequest::new(false, 1).filter(FileFilter::new("ZIP archives", ["zip"])))
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
    let entries = load_archive(bytes)?;
    Ok(Some((file.name, entries)))
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn read_archive(path: &std::path::Path) -> Result<LoadedArchive, String> {
    let bytes = peazip_native::read_archive_bytes(path, peazip_core::Limits::default())
        .map_err(|error| error.to_string())?;
    load_archive(Arc::new(bytes))
}

fn load_archive(bytes: Arc<Vec<u8>>) -> Result<LoadedArchive, String> {
    let rows = peazip_core::list_zip(&bytes, peazip_core::Limits::default())
        .map_err(|error| error.to_string())?;
    Ok((bytes, rows))
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
        peazip_native::extract_zip_to_directory(
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
