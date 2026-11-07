use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub use peazip_core::{Entry, Error, Limits};
use peazip_core::{MemoryEntry, create_zip, list_zip};

// Reject final-component symlinks (including Windows reparse points). Parent
// directories are resolved normally; callers must trust their selected parents.
fn open_regular(path: &Path) -> Result<File, Error> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(Error::UnsupportedEntry(path.display().to_string()));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(Error::UnsupportedEntry(path.display().to_string()));
        }
    }
    if !metadata.is_file() {
        return Err(Error::UnsupportedEntry(path.display().to_string()));
    }
    Ok(file)
}

pub fn create_zip_from_paths(
    paths: &[PathBuf],
    destination: &Path,
    limits: Limits,
) -> Result<Vec<Entry>, Error> {
    if paths.is_empty() {
        return Err(Error::UnsupportedEntry("empty selection".into()));
    }
    if paths.len() > limits.max_entries {
        return Err(Error::Limit("entry count"));
    }
    match fs::symlink_metadata(destination) {
        Ok(_) => return Err(io::Error::from(io::ErrorKind::AlreadyExists).into()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut names = BTreeSet::new();
    let mut entries = Vec::new();
    let mut total = 0u64;
    for path in paths {
        if path.to_str().is_none() {
            return Err(Error::UnsafePath(path.to_string_lossy().into_owned()));
        }
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::UnsafePath(path.display().to_string()))?;
        if name.len() > limits.max_path_bytes {
            return Err(Error::Limit("path bytes"));
        }
        if !names.insert(name.to_lowercase()) {
            return Err(Error::DuplicatePath(name.into()));
        }
        let mut file = open_regular(path)?;
        let remaining = limits.max_total_bytes.saturating_sub(total);
        let budget = limits.max_entry_bytes.min(remaining);
        if file.metadata()?.len() > budget {
            return Err(Error::Limit("source bytes"));
        }
        let mut data = Vec::new();
        Read::by_ref(&mut file)
            .take(budget)
            .read_to_end(&mut data)?;
        // Probe separately so even a concurrently growing file cannot allocate
        // beyond the remaining entry/total budget.
        let mut extra = [0];
        if file.read(&mut extra)? != 0 {
            return Err(Error::Limit("source bytes"));
        }
        total = total
            .checked_add(data.len() as u64)
            .ok_or(Error::Limit("total bytes"))?;
        entries.push(MemoryEntry {
            path: name.into(),
            is_dir: false,
            data,
        });
    }
    let bytes = create_zip(&entries, limits)?;
    let listed = list_zip(&bytes, limits)?;
    publish(&bytes, destination)?;
    Ok(listed)
}

fn publish(bytes: &[u8], destination: &Path) -> Result<(), Error> {
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // tempfile calls Win32 directly; canonicalization supplies the extended
    // path prefix needed when the destination exceeds MAX_PATH.
    #[cfg(windows)]
    let canonical_parent = fs::canonicalize(parent)?;
    #[cfg(windows)]
    let parent = canonical_parent.as_path();
    #[cfg(windows)]
    let destination = parent.join(destination.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "missing destination filename")
    })?);
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(destination)
        .map_err(|e| Error::Io(e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn publication_collision_cleans_temporary_and_preserves_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, b"original").unwrap();
        assert!(super::publish(b"replacement", &target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
