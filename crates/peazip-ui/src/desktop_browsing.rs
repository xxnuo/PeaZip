use crate::{
    ArchiveState, OpenedArchive,
    filesystem::{Files, HistoryChange, Location},
    navigation::BrowserState,
};
use nsh_core::prelude::*;
use peazip_native::{DirectoryLimits, DirectorySnapshot, EntryKind};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Copy)]
pub(crate) struct Browser {
    pub files: Files,
    pub archive: ArchiveState,
    pub busy: Signal<bool>,
    pub status: Signal<String>,
    pub activity: Signal<String>,
    pub owner: Owner,
    pub archive_content: ElementRef,
}

enum Request {
    Folder(PathBuf, HistoryChange),
    Child(Arc<DirectorySnapshot>, usize),
    Refresh(Arc<DirectorySnapshot>, HistoryChange),
    Entry(Arc<DirectorySnapshot>, usize),
    Archive(PathBuf, String, HistoryChange),
}

enum Loaded {
    Folder(DirectorySnapshot, HistoryChange),
    Archive(OpenedArchive, String, HistoryChange),
}

impl Browser {
    fn focus_contents(self) {
        if self.files.active.get_untracked() {
            self.files.content.focus();
        } else {
            self.archive_content.focus();
        }
    }

    pub fn track_navigation(self) {
        effect(move || {
            let _ = self.archive.version.get();
            let directory = self
                .archive
                .navigation
                .with(|state| state.current().to_owned());
            if self.files.context.get()
                && !self.files.active.get()
                && !self.archive.snapshot.with(|bytes| bytes.is_empty())
            {
                self.files.record_archive_directory(&directory);
            }
        });
    }

    fn seed_history(self) {
        if self.files.context.get_untracked() {
            return;
        }
        self.files.history.set(Default::default());
        if let Some(path) = self.archive.source.get_untracked() {
            let directory = self
                .archive
                .navigation
                .with(|state| state.current().to_owned());
            self.files.history.update(|history| {
                history.visit(Location::Archive { path, directory });
            });
        }
    }

    pub fn show_archive(self, opened: OpenedArchive, directory: String, change: HistoryChange) {
        let Some(path) = opened.source_path.clone() else {
            self.files.active.set(false);
            self.files.context.set(false);
            self.archive.show(opened);
            return;
        };
        let mut navigation = BrowserState::default();
        let directory = if navigation.enter(&directory, &opened.data.1).is_ok() {
            directory
        } else {
            String::new()
        };
        batch(|| {
            self.files.apply_archive(path, directory, change);
            self.archive.show(opened);
            self.archive.navigation.set(navigation);
        });
    }

    fn show_folder(self, snapshot: DirectorySnapshot, change: HistoryChange) {
        let count = snapshot.entries().len();
        batch(|| {
            self.seed_history();
            self.files.apply_directory(Arc::new(snapshot), change);
            self.archive.clear();
            self.status.set(format!("{count} items"));
        });
    }

    fn run(self, request: Request, editing: bool, closing: bool) {
        if self.busy.get_untracked() {
            return;
        }
        self.busy.set(true);
        self.activity.set("Reading folder or archive…".into());
        self.owner.run(|| {
            spawn(async move {
                let result = blocking(move || load(request))
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|value| value);
                match result {
                    Ok(Loaded::Folder(snapshot, change)) => self.show_folder(snapshot, change),
                    Ok(Loaded::Archive(opened, directory, change)) => {
                        let count = opened.data.1.len();
                        self.show_archive(opened, directory, change);
                        self.status.set(format!("{count} entries"));
                    }
                    Err(error) => {
                        if closing {
                            self.archive.clear();
                            self.files.active.set(false);
                            self.files.context.set(false);
                            self.files.current.set(None);
                            self.files.selected.set(Default::default());
                            self.status.set(format!(
                                "Archive closed. Could not return to folder: {error}"
                            ));
                        } else {
                            self.status.set(format!("Could not browse item: {error}"));
                        }
                        if editing && self.files.active.get_untracked() {
                            after_answer(move || self.files.input.focus());
                        }
                    }
                }
                self.busy.set(false);
            })
        });
    }

    pub async fn create_folder(
        self,
        parent: Arc<DirectorySnapshot>,
        name: String,
    ) -> Result<(), String> {
        let target = Arc::clone(&parent);
        let chosen_name = name.clone();
        blocking(move || target.create_folder(&chosen_name, DirectoryLimits::default()))
            .await.map_err(|error| format!("Could not confirm folder creation: {error}. Refresh the parent folder before retrying."))?
            .map_err(|error| match error {
                peazip_native::DirectoryError::Io(error) if error.kind() == std::io::ErrorKind::AlreadyExists =>
                    "An item with this name already exists. Choose another name.".to_owned(),
                peazip_native::DirectoryError::Io(error) => format!("Could not create folder: {error}"),
                error => format!("Could not create folder: {error}"),
            })?;
        let refreshed = blocking(move || parent.refresh(DirectoryLimits::default())).await;
        match refreshed {
            Ok(Ok(snapshot)) => {
                self.files.filter.set(String::new());
                self.show_folder(snapshot, HistoryChange::Refresh);
                self.status.set(format!("Created folder: {name}"));
            }
            Ok(Err(error)) => self.status.set(format!(
                "Created folder: {name}. Could not refresh the folder: {error}"
            )),
            Err(error) => self.status.set(format!(
                "Created folder: {name}. Could not refresh the folder: {error}"
            )),
        }
        Ok(())
    }

    pub fn open_folder(self) {
        if self.busy.get_untracked() {
            return;
        }
        self.busy.set(true);
        self.activity.set("Choose a folder…".into());
        self.owner.run(|| {
            spawn(async move {
                let result = pick_paths(
                    nsh_core::file_dialog::OpenDialog::folders()
                        .title("Open folder")
                        .button_label("Browse"),
                )
                .await;
                self.busy.set(false);
                match result {
                    Ok(Some(paths)) if !paths.is_empty() => self.run(
                        Request::Folder(paths[0].clone(), HistoryChange::Visit),
                        false,
                        false,
                    ),
                    Ok(_) => after_answer(move || self.focus_contents()),
                    Err(error) => self.status.set(format!("Could not open folder: {error}")),
                }
            })
        });
    }

    pub fn go(self, path: PathBuf) {
        let path = if path.is_absolute() {
            path
        } else {
            self.files
                .current
                .with(|current| current.as_ref().map(|view| view.0.path().join(&path)))
                .unwrap_or(path)
        };
        self.run(Request::Folder(path, HistoryChange::Visit), true, false);
    }

    pub fn open_entry(self, id: usize) {
        if !self.files.active.get_untracked() || self.busy.get_untracked() {
            return;
        }
        let Some(view) = self.files.current.get_untracked() else {
            return;
        };
        let Some(entry) = view.0.entries().get(id) else {
            return;
        };
        let request = if entry.kind == EntryKind::Directory {
            Request::Child(view.0, id)
        } else {
            Request::Entry(view.0, id)
        };
        self.run(request, false, false);
    }

    pub fn can_open_selected(self) -> bool {
        self.files.active.get() && self.files.selected.with(|selected| selected.len() == 1)
    }

    pub fn open_selected(self) {
        if let Some(id) = self.files.selected.with(|selected| {
            if selected.len() == 1 {
                selected.first().copied()
            } else {
                None
            }
        }) {
            self.open_entry(id);
        }
    }

    pub fn refresh(self) {
        if !self.files.active.get_untracked() {
            return;
        }
        if let Some(view) = self.files.current.get_untracked() {
            self.run(
                Request::Refresh(view.0, HistoryChange::Refresh),
                false,
                false,
            );
        }
    }

    pub fn can_back(self) -> bool {
        self.files.history.with(|history| history.can_back())
    }
    pub fn can_forward(self) -> bool {
        self.files.history.with(|history| history.can_forward())
    }
    pub fn back(self) {
        if let Some((index, location)) = self.files.history.with(|history| history.back_target()) {
            self.visit(location, HistoryChange::Move(index));
        }
    }
    pub fn forward(self) {
        if let Some((index, location)) = self.files.history.with(|history| history.forward_target())
        {
            self.visit(location, HistoryChange::Move(index));
        }
    }

    fn folder_request(self, path: PathBuf, change: HistoryChange) -> Request {
        if let Some(view) = self.files.current.get_untracked()
            && view.0.path() == path
        {
            return Request::Refresh(view.0, change);
        }
        Request::Folder(path, change)
    }

    fn visit(self, location: Location, change: HistoryChange) {
        if self.busy.get_untracked() {
            return;
        }
        match location {
            Location::Folder(path) => self.run(self.folder_request(path, change), false, false),
            Location::Archive { path, directory } => {
                if !self.files.active.get_untracked()
                    && self.archive.source.get_untracked().as_ref() == Some(&path)
                {
                    let mut next = self.archive.navigation.get_untracked();
                    match self
                        .archive
                        .entries
                        .with(|entries| next.enter(&directory, entries))
                    {
                        Ok(()) => {
                            self.files.apply_archive(path, directory, change);
                            self.archive.navigation.set(next);
                        }
                        Err(error) => self.status.set(error),
                    }
                } else {
                    self.run(Request::Archive(path, directory, change), false, false);
                }
            }
        }
    }

    pub fn can_up(self) -> bool {
        if self.files.active.get() {
            self.files.current.with(|current| {
                current
                    .as_ref()
                    .is_some_and(|view| view.0.path().parent().is_some())
            })
        } else {
            !self
                .archive
                .navigation
                .with(|state| state.current().is_empty())
                || self
                    .archive
                    .source
                    .with(|source| source.as_ref().is_some_and(|path| path.parent().is_some()))
        }
    }

    pub fn up(self) {
        if self.busy.get_untracked() {
            return;
        }
        if self.files.active.get_untracked() {
            if let Some(path) = self.files.current.with(|current| {
                current
                    .as_ref()
                    .and_then(|view| view.0.path().parent().map(Path::to_path_buf))
            }) {
                self.run(
                    self.folder_request(path, HistoryChange::Visit),
                    false,
                    false,
                );
            }
        } else if !self
            .archive
            .navigation
            .with(|state| state.current().is_empty())
        {
            self.archive.navigation.update(|state| {
                state.up();
            });
        } else if let Some(path) = self.archive.source.with(|source| {
            source
                .as_ref()
                .and_then(|path| path.parent().map(Path::to_path_buf))
        }) {
            self.run(
                self.folder_request(path, HistoryChange::Visit),
                false,
                false,
            );
        }
    }

    pub fn filesystem_root(self) {
        let folder = || {
            self.files
                .current
                .with(|current| current.as_ref().map(|view| view.0.path().to_path_buf()))
        };
        let path = if self.files.active.get_untracked() {
            folder()
        } else {
            self.archive.source.get_untracked().or_else(folder)
        }
        .or_else(|| std::env::current_dir().ok());
        if let Some(path) = path.and_then(|path| path.ancestors().last().map(Path::to_path_buf)) {
            self.run(
                self.folder_request(path, HistoryChange::Visit),
                false,
                false,
            );
        } else {
            self.status
                .set("Could not determine the filesystem root.".into());
        }
    }

    pub fn close_archive(self) {
        if self.busy.get_untracked() {
            return;
        }
        if self.files.context.get_untracked()
            && let Some(parent) = self.archive.source.with(|source| {
                source
                    .as_ref()
                    .and_then(|path| path.parent().map(Path::to_path_buf))
            })
        {
            self.run(
                self.folder_request(parent, HistoryChange::Visit),
                false,
                true,
            );
            return;
        }
        self.archive.clear();
        self.status
            .set("Open an archive to view its contents.".into());
    }

    pub fn copy_selection(self) {
        if self.busy.get_untracked() || !self.files.active.get_untracked() {
            return;
        }
        let Some(view) = self.files.current.get_untracked() else {
            return;
        };
        let ids: Vec<_> = self
            .files
            .selected
            .with(|selected| selected.iter().copied().collect());
        if ids.is_empty() {
            return;
        }
        self.busy.set(true);
        self.activity
            .set("Choose a new folder for the copy…".into());
        self.owner.run(|| spawn(async move {
            let destination = pick_save_path(nsh_core::file_dialog::SaveDialog::new()
                .title("Copy selected — choose a new folder name").button_label("Copy").file_name("Copy")).await;
            match destination {
                Ok(Some(destination)) => {
                    self.activity.set("Copying selected items…".into());
                    let label = destination.display().to_string();
                    let result = blocking(move || view.0.copy_selected_to_directory(&ids, &destination, peazip_core::Limits::default())).await;
                    match result {
                        Ok(Ok(count)) => self.status.set(format!("Copied {count} entries to {label}")),
                        Ok(Err(error)) => {
                            let detail = match error {
                                peazip_core::Error::Limit(kind) => format!("Copy limit exceeded: {kind}"),
                                peazip_core::Error::UnsupportedEntry(path) => format!("Links and special files cannot be copied: {path}"),
                                peazip_core::Error::UnsafePath(path) => format!("Invalid source name: {path}"),
                                peazip_core::Error::DuplicatePath(path) => format!("Conflicting source paths: {path}"),
                                error => error.to_string(),
                            };
                            self.status.set(format!("Could not copy selected items: {detail}"));
                        }
                        Err(error) => self.status.set(format!("Could not confirm copy completion: {error}. Check the destination before retrying.")),
                    }
                }
                Ok(None) => self.status.set("Copy cancelled.".into()),
                Err(error) => self.status.set(format!("Could not copy selected items: {error}")),
            }
            self.busy.set(false);
            after_answer(move || self.files.content.focus());
        }));
    }

    pub async fn create_selection(
        self,
        options: peazip_core::CreateOptions,
    ) -> Result<Option<OpenedArchive>, String> {
        options.validate().map_err(|error| error.to_string())?;
        let Some(view) = self.files.current.get_untracked() else {
            return Err("No folder is open.".into());
        };
        let ids = self
            .files
            .selected
            .with(|selected| selected.iter().copied().collect::<Vec<_>>());
        let Some(destination) = pick_save_path(crate::creation_save_dialog(options)).await? else {
            return Ok(None);
        };
        let source_path = destination.clone();
        let name = destination.display().to_string();
        let data = blocking(move || {
            view.0
                .create_archive(&ids, &destination, peazip_core::Limits::default(), options)
                .map_err(|error| error.to_string())?;
            crate::read_archive(&destination)
        })
        .await
        .map_err(|error| error.to_string())??;
        Ok(Some(OpenedArchive {
            name,
            data,
            source_path: Some(source_path),
        }))
    }
}

fn load(request: Request) -> Result<Loaded, String> {
    let limits = DirectoryLimits::default();
    match request {
        Request::Folder(path, change) => DirectorySnapshot::open(&path, limits)
            .map(|snapshot| Loaded::Folder(snapshot, change))
            .map_err(|error| error.to_string()),
        Request::Child(snapshot, id) => snapshot
            .open_child(id, limits)
            .map(|snapshot| Loaded::Folder(snapshot, HistoryChange::Visit))
            .map_err(|error| error.to_string()),
        Request::Refresh(snapshot, change) => snapshot
            .refresh(limits)
            .map(|snapshot| Loaded::Folder(snapshot, change))
            .map_err(|error| error.to_string()),
        Request::Entry(snapshot, id) => {
            let entry = snapshot
                .entries()
                .get(id)
                .ok_or("This item no longer exists.")?;
            let path = snapshot.path().join(&entry.name);
            let bytes = snapshot
                .read_archive(id, peazip_core::Limits::default())
                .map_err(|error| error.to_string())?;
            let data = crate::load_archive(Arc::new(bytes))?;
            Ok(Loaded::Archive(
                OpenedArchive {
                    name: path.display().to_string(),
                    data,
                    source_path: Some(path),
                },
                String::new(),
                HistoryChange::Visit,
            ))
        }
        Request::Archive(path, directory, change) => {
            let data = crate::read_archive(&path)?;
            Ok(Loaded::Archive(
                OpenedArchive {
                    name: path.display().to_string(),
                    data,
                    source_path: Some(path),
                },
                directory,
                change,
            ))
        }
    }
}
