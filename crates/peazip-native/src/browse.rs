use super::{
    Entry, Error, Limits, ZipOptions, create_zip_from_sources, is_link, open_source,
    read_archive_file,
};
use cap_fs_ext::MetadataExt;
use cap_std::fs::Dir;
use std::{
    ffi::{OsStr, OsString},
    fmt, io,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    Link,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub id: usize,
    pub name: OsString,
    pub display_name: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
}

#[derive(Debug, Clone, Copy)]
pub struct DirectoryLimits {
    pub max_entries: usize,
    pub max_name_bytes: usize,
    pub max_total_name_bytes: usize,
}

impl Default for DirectoryLimits {
    fn default() -> Self {
        Self {
            max_entries: 16384,
            max_name_bytes: 1024,
            max_total_name_bytes: 16 * 1024 * 1024,
        }
    }
}

#[derive(Debug)]
pub enum DirectoryError {
    Io(io::Error),
    Limit(&'static str),
    InvalidEntry,
    InvalidName,
    InvalidPath,
    NotDirectory,
    UnsupportedEntry,
}

impl fmt::Display for DirectoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => match error.kind() {
                io::ErrorKind::PermissionDenied => {
                    f.write_str("Permission denied while reading this folder.")
                }
                io::ErrorKind::NotFound => f.write_str("This folder or item no longer exists."),
                _ => write!(f, "Could not read folder: {error}"),
            },
            Self::Limit(kind) => write!(f, "Folder limit exceeded: {kind}"),
            Self::InvalidEntry => f.write_str("This item is not in the directory snapshot."),
            Self::InvalidName => f.write_str(
                "Enter one folder name without reserved characters, device names, or a trailing dot or space.",
            ),
            Self::InvalidPath => f.write_str("Choose a valid folder path."),
            Self::NotDirectory => f.write_str("This item is not a folder."),
            Self::UnsupportedEntry => f.write_str("Links and special files cannot be opened."),
        }
    }
}

impl std::error::Error for DirectoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for DirectoryError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn directory_error(error: Error) -> DirectoryError {
    match error {
        Error::Io(error) => DirectoryError::Io(error),
        _ => DirectoryError::UnsupportedEntry,
    }
}

pub fn validate_folder_name(name: &str, limits: DirectoryLimits) -> Result<(), DirectoryError> {
    if name.len() > limits.max_name_bytes {
        return Err(DirectoryError::Limit("name bytes"));
    }
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.contains(['/', '\\', ':', '<', '>', '"', '|', '?', '*'])
        || name.chars().any(char::is_control)
        || name.ends_with(['.', ' '])
    {
        return Err(DirectoryError::InvalidName);
    }
    // Match core's path_key device-name rules, including names with extensions.
    let stem = name.split('.').next().unwrap_or_default();
    if ["CON", "PRN", "AUX", "NUL"]
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
        || (stem.len() == 4
            && stem.get(..3).is_some_and(|prefix| {
                prefix.eq_ignore_ascii_case("COM") || prefix.eq_ignore_ascii_case("LPT")
            })
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        return Err(DirectoryError::InvalidName);
    }
    Ok(())
}

fn display_name(name: &OsStr) -> String {
    match name.to_str() {
        Some(name) => name.chars().flat_map(char::escape_debug).collect(),
        None => format!("{name:?}"),
    }
}

fn open_directory(path: &Path) -> Result<Dir, DirectoryError> {
    if let Some(name) = path.file_name() {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = Dir::open_ambient_dir(parent, cap_std::ambient_authority())?;
        let file = open_source(&parent, name).map_err(directory_error)?;
        if !file.metadata()?.is_dir() {
            return Err(DirectoryError::NotDirectory);
        }
        Ok(Dir::from_std_file(file))
    } else if matches!(
        path.components().next_back(),
        Some(Component::RootDir | Component::CurDir | Component::ParentDir)
    ) {
        // Root, '.' and '..' are directories themselves; preceding components remain trusted.
        Ok(Dir::open_ambient_dir(path, cap_std::ambient_authority())?)
    } else {
        Err(DirectoryError::InvalidPath)
    }
}

fn physical_path(dir: &Dir, chosen: &Path) -> Result<PathBuf, DirectoryError> {
    let path = std::fs::canonicalize(chosen)?;
    let resolved = open_directory(&path)?;
    // Windows path metadata need not contain file IDs; compare two open handles instead.
    let opened = dir.dir_metadata()?;
    let resolved = resolved.dir_metadata()?;
    if (opened.dev(), opened.ino()) != (resolved.dev(), resolved.ino()) {
        return Err(
            io::Error::other("Folder changed while resolving its path. Open it again.").into(),
        );
    }
    Ok(path)
}

#[derive(Debug, Clone)]
pub struct DirectorySnapshot {
    dir: Arc<Dir>,
    path: PathBuf,
    entries: Arc<[DirectoryEntry]>,
}

impl DirectorySnapshot {
    pub fn open(path: &Path, limits: DirectoryLimits) -> Result<Self, DirectoryError> {
        let dir = open_directory(path)?;
        let path = physical_path(&dir, path)?;
        Self::scan(Arc::new(dir), path, limits)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entries(&self) -> &[DirectoryEntry] {
        &self.entries
    }

    pub fn open_child(&self, id: usize, limits: DirectoryLimits) -> Result<Self, DirectoryError> {
        let entry = self.entries.get(id).ok_or(DirectoryError::InvalidEntry)?;
        if entry.kind != EntryKind::Directory {
            return Err(DirectoryError::NotDirectory);
        }
        let file = open_source(&self.dir, &entry.name).map_err(directory_error)?;
        if !file.metadata()?.is_dir() {
            return Err(DirectoryError::NotDirectory);
        }
        Self::scan(
            Arc::new(Dir::from_std_file(file)),
            self.path.join(&entry.name),
            limits,
        )
    }

    pub fn refresh(&self, limits: DirectoryLimits) -> Result<Self, DirectoryError> {
        Self::scan(Arc::clone(&self.dir), self.path.clone(), limits)
    }

    pub fn create_folder(&self, name: &str, limits: DirectoryLimits) -> Result<(), DirectoryError> {
        validate_folder_name(name, limits)?;
        self.dir.create_dir(name)?;
        Ok(())
    }

    pub fn read_archive(&self, id: usize, limits: Limits) -> Result<Vec<u8>, Error> {
        let entry = self.entry(id)?;
        if entry.kind != EntryKind::File {
            return Err(Error::UnsupportedEntry(entry.display_name.clone()));
        }
        read_archive_file(open_source(&self.dir, &entry.name)?, limits)
    }

    pub fn create_zip(
        &self,
        ids: &[usize],
        destination: &Path,
        limits: Limits,
    ) -> Result<Vec<Entry>, Error> {
        self.create_zip_with_options(ids, destination, limits, ZipOptions::default())
    }

    pub fn create_zip_with_options(
        &self,
        ids: &[usize],
        destination: &Path,
        limits: Limits,
        options: ZipOptions,
    ) -> Result<Vec<Entry>, Error> {
        let sources = ids.iter().map(|&id| {
            let entry = self.entry(id)?;
            if !matches!(entry.kind, EntryKind::File | EntryKind::Directory) {
                return Err(Error::UnsupportedEntry(entry.display_name.clone()));
            }
            Ok((Arc::clone(&self.dir), entry.name.clone()))
        });
        create_zip_from_sources(ids.len(), sources, destination, options, limits)
    }

    fn entry(&self, id: usize) -> Result<&DirectoryEntry, Error> {
        self.entries
            .get(id)
            .ok_or_else(|| Error::UnknownEntry(format!("directory entry {id}")))
    }

    fn scan(dir: Arc<Dir>, path: PathBuf, limits: DirectoryLimits) -> Result<Self, DirectoryError> {
        let mut entries = Vec::new();
        let mut total_names = 0usize;
        for (attempt, child) in dir.entries()?.enumerate() {
            // Disappearing entries still consume an attempt, even if no row can be retained.
            if attempt >= limits.max_entries {
                return Err(DirectoryError::Limit("entry count"));
            }
            let child = match child {
                Ok(child) => child,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            let name = child.file_name();
            drop(child);
            let bytes = name.as_encoded_bytes().len();
            if bytes > limits.max_name_bytes {
                return Err(DirectoryError::Limit("name bytes"));
            }
            total_names = total_names
                .checked_add(bytes)
                .ok_or(DirectoryError::Limit("total name bytes"))?;
            if total_names > limits.max_total_name_bytes {
                return Err(DirectoryError::Limit("total name bytes"));
            }
            let metadata = match dir.symlink_metadata(&name) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            let kind = if is_link(&metadata) {
                EntryKind::Link
            } else if metadata.is_dir() {
                EntryKind::Directory
            } else if metadata.is_file() {
                EntryKind::File
            } else {
                EntryKind::Other
            };
            let display_name = display_name(&name);
            entries.push(DirectoryEntry {
                id: entries.len(),
                name,
                display_name,
                kind,
                size: (kind == EntryKind::File).then_some(metadata.len()),
                modified: metadata.modified().ok().map(|time| time.into_std()),
            });
        }
        Ok(Self {
            dir,
            path,
            entries: entries.into(),
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn display_escaping_distinguishes_invalid_bytes_controls_and_literal_escapes() {
        let names = [
            OsStr::from_bytes(b"raw-\xff"),
            OsStr::from_bytes(b"raw-\xfe"),
            OsStr::new("raw-�"),
            OsStr::new("line\nbreak"),
            OsStr::new("line\\nbreak"),
        ];
        let display: std::collections::BTreeSet<_> = names.into_iter().map(display_name).collect();
        assert_eq!(display.len(), names.len());
        assert!(
            display
                .iter()
                .all(|name| !name.chars().any(char::is_control))
        );
        assert!(!display_name(names[0]).contains('�'));
        assert_eq!(display_name(OsStr::new("中文.txt")), "中文.txt");
    }

    #[test]
    fn physical_path_rejects_a_directory_replaced_between_open_and_resolution() {
        let base = tempfile::tempdir().unwrap();
        let chosen = base.path().join("chosen");
        std::fs::create_dir(&chosen).unwrap();
        let dir = open_directory(&chosen).unwrap();
        std::fs::rename(&chosen, base.path().join("moved")).unwrap();
        std::fs::create_dir(&chosen).unwrap();
        assert!(physical_path(&dir, &chosen).is_err());
    }
}
