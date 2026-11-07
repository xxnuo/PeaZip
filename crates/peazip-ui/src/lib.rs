use std::collections::BTreeSet;

use nsh_base::{button, checkbox, theme, toolbar};
use nsh_core::{element::Position, prelude::*};
use peazip_core::Entry;

#[native_module("peazip/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

#[component]
pub fn App() -> impl IntoElement {
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
                Ok(Some((name, rows))) => {
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
    let actions = actions.child(
        button()
            .disabled(move || busy.get())
            .on_press(move || {
                if busy.get_untracked() {
                    return;
                }
                busy.set(true);
                activity.set("Creating ZIP…".to_owned());
                spawn(async move {
                    match create_archive().await {
                        Ok(Some((name, rows))) => {
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
            })
            .px_3()
            .py_2()
            .border_1()
            .border_color(move || theme().border)
            .bg(move || theme().muted)
            .active(|s| s.opacity(0.6))
            .disabled_style(|s| s.opacity(0.4))
            .child(text("Create ZIP…")),
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
                        .on_press(move || selected.set(BTreeSet::new()))
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
async fn create_archive() -> Result<Option<(String, Vec<Entry>)>, String> {
    use nsh_core::file_dialog::{FileFilter, OpenDialog, SaveDialog};
    let Some(paths) = pick_paths(
        OpenDialog::files()
            .multiple(true)
            .title("Select files to archive")
            .button_label("Add files"),
    )
    .await?
    .filter(|paths| !paths.is_empty()) else {
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
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    Ok(Some((name, rows)))
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
async fn choose_archive() -> Result<Option<(String, Vec<Entry>)>, String> {
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
async fn choose_archive() -> Result<Option<(String, Vec<Entry>)>, String> {
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
    #[cfg(not(target_arch = "wasm32"))]
    let entries = blocking(move || peazip_core::list_zip(file.data.bytes(), limits))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    #[cfg(target_arch = "wasm32")]
    let entries =
        peazip_core::list_zip(file.data.bytes(), limits).map_err(|error| error.to_string())?;
    Ok(Some((file.name, entries)))
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn read_archive(path: &std::path::Path) -> Result<Vec<Entry>, String> {
    use std::io::Read;
    let limits = peazip_core::Limits::default();
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|error| error.to_string())?
        .take(limits.max_archive_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    peazip_core::list_zip(&bytes, limits).map_err(|error| error.to_string())
}
