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

pub fn read_archive_bytes(path: &Path, limits: Limits) -> Result<Vec<u8>, Error> {
    let mut file = open_regular(path)?;
    if file.metadata()?.len() > limits.max_archive_bytes {
        return Err(Error::Limit("archive bytes"));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limits.max_archive_bytes)
        .read_to_end(&mut bytes)?;
    // Probe separately to bound allocation even if the file grows while reading.
    if file.read(&mut [0])? != 0 {
        return Err(Error::Limit("archive bytes"));
    }
    Ok(bytes)
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

pub fn extract_zip_to_directory(
    bytes: &[u8],
    selected: &[String],
    destination: &Path,
    limits: Limits,
) -> Result<usize, Error> {
    if selected.is_empty() {
        return Err(Error::UnsupportedEntry("empty selection".into()));
    }
    let entries = peazip_core::extract_selected_zip(bytes, selected, limits)?;
    let name = destination.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "missing destination filename")
    })?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    #[cfg(windows)]
    let canonical_parent = fs::canonicalize(parent)?;
    #[cfg(windows)]
    let parent = canonical_parent.as_path();
    let destination = parent.join(name);
    match fs::symlink_metadata(&destination) {
        Ok(_) => return Err(io::Error::from(io::ErrorKind::AlreadyExists).into()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut builder = tempfile::Builder::new();
    builder.prefix(".peazip-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    let temporary = builder.tempdir_in(parent)?;
    for entry in &entries {
        let path = temporary.path().join(&entry.path);
        if entry.is_dir {
            fs::create_dir_all(path)?;
        } else {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(&entry.data)?;
            file.sync_all()?;
        }
    }
    publish_directory(temporary.path(), &destination)?;
    let _ = temporary.keep();
    Ok(entries.len())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn publish_directory(source: &Path, destination: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let source = CString::new(source.as_os_str().as_bytes())?;
    let destination = CString::new(destination.as_os_str().as_bytes())?;
    // These flags make the existence check and move one atomic operation.
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(target_os = "macos")]
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn publish_directory(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
    let wide = |path: &Path| -> io::Result<Vec<u16>> {
        let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        value.push(0);
        Ok(value)
    };
    let source = wide(source)?;
    let destination = wide(destination)?;
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0) } != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn publish_directory(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic directory publication unsupported",
    ))
}

#[cfg(test)]
mod directory_tests {
    use super::*;

    #[test]
    fn publication_race_preserves_winner_and_cleans_staging() {
        let parent = tempfile::tempdir().unwrap();
        let target = parent.path().join("target");
        for nonempty in [false, true] {
            let staging = tempfile::tempdir_in(parent.path()).unwrap();
            fs::write(staging.path().join("new"), b"new").unwrap();
            assert!(!target.exists());
            // A competitor wins after the caller's absence check.
            fs::create_dir(&target).unwrap();
            if nonempty {
                fs::write(target.join("winner"), b"winner").unwrap();
            }
            assert!(publish_directory(staging.path(), &target).is_err());
            drop(staging);
            assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 1);
            assert_eq!(
                fs::read_dir(&target).unwrap().count(),
                usize::from(nonempty)
            );
            if nonempty {
                assert_eq!(fs::read(target.join("winner")).unwrap(), b"winner");
            }
            fs::remove_dir_all(&target).unwrap();
        }
    }

    #[test]
    fn filesystem_write_failure_cleans_partial_tree() {
        let parent = tempfile::tempdir().unwrap();
        let bytes = create_zip(
            &[
                MemoryEntry {
                    path: "first".into(),
                    is_dir: false,
                    data: b"first".to_vec(),
                },
                MemoryEntry {
                    path: format!("nested/{}", "x".repeat(300)),
                    is_dir: false,
                    data: vec![],
                },
            ],
            Limits::default(),
        )
        .unwrap();
        assert!(
            extract_zip_to_directory(
                &bytes,
                &["first".into(), format!("nested/{}", "x".repeat(300))],
                &parent.path().join("out"),
                Limits::default()
            )
            .is_err()
        );
        assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
    }
}
