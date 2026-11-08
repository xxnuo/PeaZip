use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

use nsh_core::prelude::*;
use peazip_native::{DirectoryEntry, DirectorySnapshot, EntryKind};

use crate::{
    commands::{self, CommandId, Commands},
    navigation::Sort,
};
use nsh_base::{button, checkbox, input, theme, toolbar};
use nsh_core::element::{AutoCapitalize, Position, SubmitBehavior};

#[derive(Clone)]
pub struct DirectoryView(pub Arc<DirectorySnapshot>);

impl PartialEq for DirectoryView {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    Folder(PathBuf),
    Archive { path: PathBuf, directory: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct History {
    locations: Vec<Location>,
    position: usize,
}

impl History {
    pub fn current(&self) -> Option<&Location> {
        self.locations.get(self.position)
    }

    pub fn visit(&mut self, location: Location) -> bool {
        if self.current() == Some(&location) {
            return false;
        }
        self.locations.truncate(self.position + 1);
        self.locations.push(location);
        if self.locations.len() > 256 {
            self.locations.remove(0);
        }
        self.position = self.locations.len() - 1;
        true
    }

    pub fn can_back(&self) -> bool {
        self.position > 0
    }

    pub fn can_forward(&self) -> bool {
        self.position + 1 < self.locations.len()
    }

    pub fn back_target(&self) -> Option<(usize, Location)> {
        let index = self.position.checked_sub(1)?;
        Some((index, self.locations.get(index)?.clone()))
    }

    pub fn forward_target(&self) -> Option<(usize, Location)> {
        let index = self.position + 1;
        Some((index, self.locations.get(index)?.clone()))
    }

    pub fn move_to(&mut self, index: usize) -> bool {
        if index >= self.locations.len() || index == self.position {
            return false;
        }
        self.position = index;
        true
    }

    pub fn replace_current(&mut self, location: Location) -> bool {
        let Some(current) = self.locations.get_mut(self.position) else {
            return self.visit(location);
        };
        if *current == location {
            return false;
        }
        *current = location;
        true
    }

    fn apply(&mut self, location: Location, change: HistoryChange) -> bool {
        match change {
            HistoryChange::Visit => {
                self.visit(location);
            }
            HistoryChange::Move(index) => {
                if index >= self.locations.len() {
                    return false;
                }
                self.move_to(index);
                self.replace_current(location);
            }
            HistoryChange::Refresh => {
                self.replace_current(location);
            }
        }
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryChange {
    Visit,
    Move(usize),
    Refresh,
}

#[derive(Clone, Copy)]
pub struct Files {
    pub active: Signal<bool>,
    pub context: Signal<bool>,
    pub current: Signal<Option<DirectoryView>>,
    pub selected: Signal<BTreeSet<usize>>,
    pub history: Signal<History>,
    pub draft: Signal<String>,
    pub filter: Signal<String>,
    pub wildcards: Signal<bool>,
    pub pattern: Memo<crate::name_filter::Pattern>,
    pub input: ElementRef,
    pub content: ElementRef,
    pub version: Signal<u64>,
}

impl Files {
    pub fn new() -> Self {
        let filter = signal(String::new());
        let wildcards = signal(false);
        let pattern =
            computed(move || crate::name_filter::Pattern::new(&filter.get(), wildcards.get()));
        Self {
            active: signal(false),
            context: signal(false),
            current: signal(None),
            selected: signal(BTreeSet::new()),
            history: signal(History::default()),
            draft: signal(String::new()),
            filter,
            wildcards,
            pattern,
            input: element_ref(),
            content: element_ref(),
            version: signal(0),
        }
    }

    pub fn visible_ids(&self) -> Vec<usize> {
        let pattern = self.pattern.get();
        self.current.with(|current| {
            current
                .as_ref()
                .map(|view| {
                    view.0
                        .entries()
                        .iter()
                        .filter(|entry| pattern.matches(&entry.display_name))
                        .map(|entry| entry.id)
                        .collect()
                })
                .unwrap_or_default()
        })
    }

    pub fn apply_directory(&self, snapshot: Arc<DirectorySnapshot>, change: HistoryChange) -> bool {
        let path = snapshot.path().to_path_buf();
        let mut history = self.history.get_untracked();
        if !history.apply(Location::Folder(path.clone()), change) {
            return false;
        }
        let changed_path = self.current.with_untracked(|current| {
            current
                .as_ref()
                .is_none_or(|view| view.0.path() != path.as_path())
        });
        batch(|| {
            self.history.set(history);
            self.draft.set(path.to_string_lossy().into_owned());
            self.current.set(Some(DirectoryView(snapshot)));
            self.selected.set(BTreeSet::new());
            if changed_path {
                self.filter.set(String::new());
            }
            self.context.set(true);
            self.active.set(true);
            self.version
                .update(|version| *version = version.wrapping_add(1));
        });
        true
    }

    pub fn apply_archive(&self, path: PathBuf, inner: String, change: HistoryChange) -> bool {
        let mut history = self.history.get_untracked();
        if !history.apply(
            Location::Archive {
                path,
                directory: inner,
            },
            change,
        ) {
            return false;
        }
        batch(|| {
            self.history.set(history);
            self.selected.set(BTreeSet::new());
            self.context.set(true);
            self.active.set(false);
            self.version
                .update(|version| *version = version.wrapping_add(1));
        });
        true
    }

    pub fn record_archive_directory(&self, inner: &str) -> bool {
        if !self.context.get_untracked() || self.active.get_untracked() {
            return false;
        }
        let mut history = self.history.get_untracked();
        let Some(Location::Archive { path, .. }) = history.current().cloned() else {
            return false;
        };
        if !history.visit(Location::Archive {
            path,
            directory: inner.to_owned(),
        }) {
            return false;
        }
        self.history.set(history);
        true
    }
}

fn kind_label(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Directory => "Folder",
        EntryKind::File => "File",
        EntryKind::Link => "Link",
        EntryKind::Other => "Other",
    }
}

fn ordered_entries(entries: &[DirectoryEntry], sort: Sort) -> Vec<usize> {
    let mut order: Vec<_> = (0..entries.len()).collect();
    order.sort_by(|&a, &b| {
        let (a, b) = (&entries[a], &entries[b]);
        matches!(b.kind, EntryKind::Directory)
            .cmp(&matches!(a.kind, EntryKind::Directory))
            .then_with(|| match sort {
                Sort::NameAsc | Sort::PackedAsc | Sort::PackedDesc => {
                    a.display_name.cmp(&b.display_name)
                }
                Sort::NameDesc => b.display_name.cmp(&a.display_name),
                Sort::TypeAsc => kind_label(a.kind).cmp(kind_label(b.kind)),
                Sort::TypeDesc => kind_label(b.kind).cmp(kind_label(a.kind)),
                Sort::SizeAsc => a.size.cmp(&b.size),
                Sort::SizeDesc => b.size.cmp(&a.size),
            })
            .then_with(|| a.display_name.cmp(&b.display_name))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    order
}

struct Row {
    id: usize,
    name: String,
    directory: bool,
    kind: &'static str,
    size: Option<u64>,
}

impl Row {
    fn label(&self) -> String {
        if self.directory {
            format!("{}/", self.name)
        } else {
            self.name.clone()
        }
    }
}

pub fn browser(
    files: Files,
    busy: Signal<bool>,
    sort: Signal<Sort>,
    commands: Commands,
    open_entry: impl Fn(usize) + Copy + 'static,
    go: impl Fn(PathBuf) + Copy + 'static,
) -> impl IntoElement {
    let order = computed(move || {
        let pattern = files.pattern.get();
        files.current.with(|current| {
            current
                .as_ref()
                .map(|view| {
                    let entries = view.0.entries();
                    let mut order = ordered_entries(entries, sort.get());
                    order.retain(|&index| pattern.matches(&entries[index].display_name));
                    order
                })
                .unwrap_or_default()
        })
    });
    let wide = signal(false);
    let filter_input = element_ref();
    effect(move || {
        let _ = files.version.get();
        if files.active.get() {
            after_answer(move || {
                files.input.blur();
                filter_input.blur();
                files.content.focus();
            });
        }
    });
    let enter = move || {
        if !busy.get_untracked() {
            go(PathBuf::from(files.draft.get_untracked()));
        }
    };
    let back = commands.get(CommandId::Back).unwrap();
    let forward = commands.get(CommandId::Forward).unwrap();
    let up = commands.get(CommandId::Up).unwrap();
    let root = commands.get(CommandId::FilesystemRoot).unwrap();
    let refresh = commands.get(CommandId::RefreshFolder).unwrap();
    let select_all = commands.get(CommandId::SelectAll).unwrap();
    let clear = commands.get(CommandId::ClearSelection).unwrap();
    let invert = commands.get(CommandId::InvertSelection).unwrap();
    let sort_name = commands.get(CommandId::SortName).unwrap();
    let sort_size = commands.get(CommandId::SortSize).unwrap();
    let sort_type = commands.get(CommandId::SortType).unwrap();
    view()
        .flex_1()
        .min_h(px(0.0))
        .on_layout(move |bounds: &Bounds<Points>| wide.set(bounds.size.width.0 >= 760.0))
        .child(
            text(move || {
                files.current.with(|current| {
                    current
                        .as_ref()
                        .map(|view| view.0.path().to_string_lossy().into_owned())
                        .unwrap_or_default()
                })
            })
            .number_of_lines(1)
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(move || theme().border),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_2()
                .px_3()
                .py_1()
                .border_b_1()
                .border_color(move || theme().border)
                .child(
                    input()
                        .element_ref(&filter_input)
                        .label("Filter names")
                        .placeholder("Filter names")
                        .value(move || files.filter.get())
                        .on_change(move |value: &str| {
                            if !busy.get_untracked() {
                                files.filter.set(value.chars().take(256).collect());
                            }
                        })
                        .max_length(256)
                        .auto_capitalize(AutoCapitalize::None)
                        .auto_correct(false)
                        .submit_behavior(SubmitBehavior::Submit)
                        .on_submit(move |_: &str| {
                            filter_input.blur();
                            files.content.focus();
                        })
                        .disabled(move || busy.get())
                        .flex_1()
                        .min_w(px(120.0))
                        .border_1()
                        .border_color(move || theme().border)
                        .px_2()
                        .py_1(),
                )
                .child(checkbox().role(Some(AccessibilityRole::Switch)).accessibility_label("Use wildcards").checked(move || files.wildcards.get()).disabled(move || busy.get())
                    .on_change(move |value: &bool| {if !busy.get_untracked(){files.wildcards.set(*value);}}).flex_row().items_center().gap_2().px_2().py_1()
                    .indicator(view().size_4().border_1().border_color(move || theme().border).items_center().justify_center().child(when(move || files.wildcards.get(), || view().size_2().bg(move || theme().foreground))))
                    .child(text("Use wildcards")))
                .child(
                    button()
                        .accessibility_label("Clear filter")
                        .disabled(move || busy.get() || files.filter.with(String::is_empty))
                        .on_press(move || {
                            if !busy.get_untracked() {
                                files.filter.set(String::new());
                                filter_input.blur();
                                files.content.focus();
                            }
                        })
                        .px_2()
                        .py_1()
                        .child(text("Clear filter")),
                )
                .child(text(move || {
                    format!(
                        "{} of {} items shown",
                        order.with(Vec::len),
                        files.current.with(|current| current
                            .as_ref()
                            .map_or(0, |view| view.0.entries().len()))
                    )
                })),
        )
        .child(when(move || files.pattern.with(|pattern| pattern.error().is_some()), move || text(move || files.pattern.with(|pattern| pattern.error().unwrap_or_default().to_owned())).px_3().py_1().text_color(move || theme().destructive)))
        .child(when(
            move || !files.filter.with(String::is_empty),
            move || text(move || if files.wildcards.get() { "Name masks use *, ?, [abc], [a-z] and [!abc] (case insensitive). Hidden selections remain selected for Open selected, Copy selected and Create ZIP." } else { "Names contain the text (case insensitive). Hidden selections remain selected for Open selected, Copy selected and Create ZIP." })
                .px_3()
                .py_1()
                .text_color(move || theme().muted_foreground),
        ))
        .child(
            toolbar()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .px_2()
                .py_1()
                .border_b_1()
                .border_color(move || theme().border)
                .child(
                    commands::button(&back)
                        .px_2()
                        .py_1()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(crate::icons::icon("back"))
                        .child(text(back.label)),
                )
                .child(
                    commands::button(&forward)
                        .px_2()
                        .py_1()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(crate::icons::icon("forward"))
                        .child(text(forward.label)),
                )
                .child(
                    commands::button(&up)
                        .px_2()
                        .py_1()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(crate::icons::icon("up"))
                        .child(text(up.label)),
                )
                .child(commands::button(&root).px_2().py_1().child(text("Root")))
                .child(
                    input()
                        .element_ref(&files.input)
                        .submit_behavior(SubmitBehavior::Submit)
                        .auto_capitalize(AutoCapitalize::None)
                        .auto_correct(false)
                        .value(move || files.draft.get())
                        .on_change(move |value: &str| files.draft.set(value.to_owned()))
                        .on_submit(move |_: &str| enter())
                        .label("Folder path")
                        .placeholder("Folder path")
                        .disabled(move || busy.get())
                        .flex_1()
                        .min_w(px(110.0))
                        .border_1()
                        .border_color(move || theme().border)
                        .px_2()
                        .py_1(),
                )
                .child(
                    button()
                        .disabled(move || busy.get())
                        .on_press(enter)
                        .px_2()
                        .py_1()
                        .child(text("Go")),
                )
                .child(
                    commands::button(&refresh)
                        .px_2()
                        .py_1()
                        .child(text(refresh.label)),
                ),
        )
        .child(
            toolbar()
                .flex_row()
                .flex_wrap()
                .gap_3()
                .px_3()
                .py_1()
                .border_b_1()
                .border_color(move || theme().border)
                .child(commands::button(&select_all).child(text(select_all.label)))
                .child(commands::button(&invert).child(text(invert.label)))
                .child(commands::button(&clear).child(text(clear.label)))
                .child(text(move || {
                    format!("{} selected", files.selected.with(BTreeSet::len))
                })),
        )
        .child(
            view()
                .element_ref(&files.content)
                .focusable(true)
                .role(AccessibilityRole::Group)
                .label("Folder contents")
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .child(
                    view()
                        .flex_row()
                        .px_3()
                        .py_1()
                        .bg(move || theme().muted)
                        .border_b_1()
                        .border_color(move || theme().border)
                        .child(commands::button(&sort_name).flex_1().child(text("Name")))
                        .child(when(
                            move || wide.get(),
                            move || commands::button(&sort_type).child(text("Type")).w(px(85.0)),
                        ))
                        .child(
                            commands::button(&sort_size)
                                .w(px(110.0))
                                .child(text("Size (bytes)")),
                        )
                        .child(view().w(px(36.0))),
                )
                .child(
                    virtual_list(
                        move || order.with(Vec::len),
                        move |at: Position| {
                            let row = move || {
                                let index = order.with(|order| order.get(at.index()).copied())?;
                                files.current.with(|current| {
                                    let entry = current.as_ref()?.0.entries().get(index)?;
                                    Some(Row {
                                        id: entry.id,
                                        name: entry.display_name.clone(),
                                        directory: matches!(entry.kind, EntryKind::Directory),
                                        kind: kind_label(entry.kind),
                                        size: entry.size,
                                    })
                                })
                            };
                            let checked = move || {
                                row().is_some_and(|entry| {
                                    files.selected.with(|selected| selected.contains(&entry.id))
                                })
                            };
                            view()
                                .flex_row()
                                .items_center()
                                .border_b_1()
                                .border_color(move || theme().border)
                                .bg(move || {
                                    if checked() {
                                        theme().muted
                                    } else {
                                        theme().background
                                    }
                                })
                                .child(
                                    checkbox()
                                        .accessibility_label(move || {
                                            row().map(|entry| entry.label().into())
                                        })
                                        .checked(checked)
                                        .disabled(move || busy.get())
                                        .on_change(move |checked: &bool| {
                                            if busy.get_untracked() {
                                                return;
                                            }
                                            if let Some(entry) = row() {
                                                files.selected.update(|selected| {
                                                    if *checked {
                                                        selected.insert(entry.id);
                                                    } else {
                                                        selected.remove(&entry.id);
                                                    }
                                                });
                                            }
                                        })
                                        .flex_row()
                                        .items_center()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .gap_2()
                                        .px_3()
                                        .py_2()
                                        .indicator_class(
                                            "size-4 border border-border bg-background",
                                        )
                                        .indicator(
                                            view().size_4().items_center().justify_center().child(
                                                when(checked, || {
                                                    view()
                                                        .w(px(8.0))
                                                        .h(px(8.0))
                                                        .bg(move || theme().foreground)
                                                }),
                                            ),
                                        )
                                        .child(when(
                                            move || row().is_some_and(|entry| entry.directory),
                                            || crate::icons::icon("folder"),
                                        ))
                                        .child(when(
                                            move || row().is_some_and(|entry| !entry.directory),
                                            || crate::icons::icon("file"),
                                        ))
                                        .child(
                                            text(move || {
                                                row().map(|entry| entry.name).unwrap_or_default()
                                            })
                                            .number_of_lines(1)
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .pr_2(),
                                        )
                                        .child(when(
                                            move || wide.get(),
                                            move || {
                                                text(move || {
                                                    row()
                                                        .map(|entry| entry.kind)
                                                        .unwrap_or_default()
                                                })
                                                .number_of_lines(1)
                                                .w(px(85.0))
                                            },
                                        ))
                                        .child(
                                            text(move || {
                                                row()
                                                    .and_then(|entry| entry.size)
                                                    .map(|size| size.to_string())
                                                    .unwrap_or_else(|| "—".into())
                                            })
                                            .w(px(110.0)),
                                        ),
                                )
                                .child(
                                    view().w(px(36.0)).child(
                                        button()
                                            .accessibility_label(move || {
                                                row().map(|entry| {
                                                    format!(
                                                        "Open {} {}",
                                                        if entry.directory {
                                                            "folder"
                                                        } else {
                                                            "file"
                                                        },
                                                        entry.name
                                                    )
                                                    .into()
                                                })
                                            })
                                            .disabled(move || busy.get())
                                            .on_press(move || {
                                                if !busy.get_untracked()
                                                    && let Some(entry) = row()
                                                {
                                                    open_entry(entry.id);
                                                }
                                            })
                                            .px_2()
                                            .py_2()
                                            .child(text("›")),
                                    ),
                                )
                        },
                    )
                    .estimated_item_size(px(36.0))
                    .flex_1(),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_view_equality_tracks_snapshot_identity() {
        let directory = tempfile::tempdir().unwrap();
        let snapshot =
            DirectorySnapshot::open(directory.path(), peazip_native::DirectoryLimits::default())
                .unwrap();
        let view = DirectoryView(Arc::new(snapshot));
        assert!(view == view.clone());
        let refreshed = view
            .0
            .refresh(peazip_native::DirectoryLimits::default())
            .unwrap();
        assert!(view != DirectoryView(Arc::new(refreshed)));
    }

    fn folder(path: &str) -> Location {
        Location::Folder(PathBuf::from(path))
    }

    fn archive(path: &str, directory: &str) -> Location {
        Location::Archive {
            path: PathBuf::from(path),
            directory: directory.into(),
        }
    }

    #[test]
    fn history_unifies_folders_and_archive_directories_and_branches_after_back() {
        let mut history = History::default();
        assert!(history.visit(folder("/data")));
        assert!(history.visit(archive("/data/sample.zip", "")));
        assert!(history.visit(archive("/data/sample.zip", "docs/")));
        assert!(history.visit(folder("/other")));
        assert_eq!(
            history.back_target(),
            Some((2, archive("/data/sample.zip", "docs/")))
        );
        assert_eq!(
            history.current(),
            Some(&folder("/other")),
            "looking up a target must not move history before I/O succeeds"
        );
        assert!(history.move_to(2));
        assert_eq!(history.forward_target(), Some((3, folder("/other"))));
        assert!(!history.visit(archive("/data/sample.zip", "docs/")));
        assert!(
            history.can_forward(),
            "deduplication must keep the forward branch"
        );
        assert!(history.visit(archive("/data/sample.zip", "docs/nested/")));
        assert!(!history.can_forward());
        assert_eq!(history.locations.len(), 4);
        assert!(history.move_to(0));
        assert_eq!(
            history.forward_target(),
            Some((1, archive("/data/sample.zip", "")))
        );
        assert!(history.visit(folder("/fresh")));
        assert_eq!(history.locations, [folder("/data"), folder("/fresh")]);
        assert!(!history.can_forward());
    }

    #[test]
    fn history_bounds_retained_locations_to_256() {
        let mut history = History::default();
        for index in 0..300 {
            history.visit(folder(&format!("/dir-{index}")));
        }
        assert_eq!(history.locations.len(), 256);
        assert_eq!(history.current(), Some(&folder("/dir-299")));
        let mut back = 0;
        while let Some((index, _)) = history.back_target() {
            assert!(history.move_to(index));
            back += 1;
        }
        assert_eq!(back, 255);
        assert_eq!(history.current(), Some(&folder("/dir-44")));
        while let Some((index, _)) = history.forward_target() {
            assert!(history.move_to(index));
        }
        assert_eq!(history.current(), Some(&folder("/dir-299")));
        assert!(!history.can_forward());
    }

    #[test]
    fn empty_history_refresh_replacement_and_invalid_moves_are_safe() {
        let mut history = History::default();
        assert_eq!(history.current(), None);
        assert_eq!(history.back_target(), None);
        assert_eq!(history.forward_target(), None);
        assert!(!history.can_back());
        assert!(!history.can_forward());
        assert!(!history.move_to(0));
        assert!(!history.move_to(usize::MAX));
        let empty = history.clone();
        assert!(!history.apply(folder("/invalid"), HistoryChange::Move(0)));
        assert_eq!(history, empty);
        assert!(history.apply(folder("/data"), HistoryChange::Refresh));
        history.visit(folder("/data/child"));
        history.move_to(0);
        assert!(history.apply(folder("/canonical-data"), HistoryChange::Refresh));
        assert_eq!(history.current(), Some(&folder("/canonical-data")));
        assert_eq!(history.forward_target(), Some((1, folder("/data/child"))));
        assert!(!history.replace_current(folder("/canonical-data")));
        let previous = history.clone();
        assert!(!history.apply(folder("/invalid"), HistoryChange::Move(usize::MAX)));
        assert_eq!(history, previous);
        assert!(history.apply(folder("/data/child"), HistoryChange::Move(1)));
        assert_eq!(history.locations.len(), 2);
        assert_eq!(history.current(), Some(&folder("/data/child")));
    }

    #[cfg(unix)]
    #[test]
    fn history_preserves_distinct_raw_paths_with_identical_lossy_display_names() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let a = PathBuf::from(OsString::from_vec(b"/data/\xff.zip".to_vec()));
        let b = PathBuf::from(OsString::from_vec(b"/data/\xfe.zip".to_vec()));
        assert_eq!(a.to_string_lossy(), b.to_string_lossy());
        let mut history = History::default();
        history.visit(Location::Folder(a.clone()));
        history.visit(Location::Archive {
            path: b.clone(),
            directory: "目录/".into(),
        });
        assert_eq!(history.back_target(), Some((0, Location::Folder(a))));
        assert_eq!(
            history.current(),
            Some(&Location::Archive {
                path: b,
                directory: "目录/".into()
            })
        );
    }

    #[test]
    fn sorting_keeps_directories_first_and_preserves_entry_ids_and_unknown_sizes() {
        let entry = |id, name: &str, kind, size| DirectoryEntry {
            id,
            name: name.into(),
            display_name: name.into(),
            kind,
            size,
            modified: None,
        };
        let entries = [
            entry(40, "b", EntryKind::File, Some(8)),
            entry(99, "z", EntryKind::Directory, None),
            entry(7, "a", EntryKind::File, Some(2)),
            entry(3, "c", EntryKind::File, Some(2)),
            entry(51, "pipe", EntryKind::Other, None),
            entry(82, "link", EntryKind::Link, None),
        ];
        for (sort, expected) in [
            (Sort::NameAsc, [99, 7, 40, 3, 82, 51]),
            (Sort::NameDesc, [99, 51, 82, 3, 40, 7]),
            (Sort::SizeAsc, [99, 82, 51, 7, 3, 40]),
            (Sort::SizeDesc, [99, 40, 7, 3, 82, 51]),
        ] {
            let ids: Vec<_> = ordered_entries(&entries, sort)
                .into_iter()
                .map(|index| entries[index].id)
                .collect();
            assert_eq!(ids, expected);
        }
        assert_eq!(entries[0].id, 40);
        assert_eq!(entries[4].size, None);
    }
}
