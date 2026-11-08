use peazip_core::{Limits, MemoryEntry};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

fn open_regular_file(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|error| error.to_string())?;
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("The selected item is no longer a regular file.".into());
    }
    Ok(file)
}

pub(crate) fn read_files(paths: &[PathBuf], limits: Limits) -> Result<Vec<MemoryEntry>, String> {
    if paths.len() > limits.max_entries {
        return Err("Selected files exceed the entry limit.".into());
    }
    let mut total = 0u64;
    let mut entries = Vec::with_capacity(paths.len());
    for path in paths {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("Selected file has no valid Unicode name.")?;
        if name.len() > limits.max_path_bytes {
            return Err("Selected filename exceeds the path limit.".into());
        }
        let metadata = path.symlink_metadata().map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err("Only regular files can be added from this picker.".into());
        }
        let file = open_regular_file(path)?;
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err("The selected item is no longer a regular file.".into());
        }
        let remaining = limits.max_total_bytes.saturating_sub(total);
        if metadata.len() > limits.max_entry_bytes || metadata.len() > remaining {
            return Err("Selected files exceed the per-file or total byte limit.".into());
        }
        let cap = limits.max_entry_bytes.min(remaining);
        let mut data = Vec::new();
        file.take(cap.saturating_add(1))
            .read_to_end(&mut data)
            .map_err(|error| error.to_string())?;
        if data.len() as u64 > cap {
            return Err("Selected files grew beyond the byte limit while reading.".into());
        }
        total += data.len() as u64;
        entries.push(MemoryEntry {
            path: name.into(),
            is_dir: false,
            data,
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_files_preserve_unicode_binary_and_empty_payloads() {
        let root = tempfile::tempdir().unwrap();
        let paths = [root.path().join("你好.bin"), root.path().join("empty")];
        std::fs::write(&paths[0], [0, 127, 255]).unwrap();
        std::fs::write(&paths[1], []).unwrap();
        let files = read_files(&paths, Limits::default()).unwrap();
        assert_eq!(files[0].path, "你好.bin");
        assert_eq!(files[0].data, [0, 127, 255]);
        assert!(files[1].data.is_empty());
    }

    #[test]
    fn limits_reject_sparse_large_files_total_overflow_and_folders() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a");
        let b = root.path().join("b");
        File::create(&a)
            .unwrap()
            .set_len(1024 * 1024 * 1024)
            .unwrap();
        assert!(read_files(std::slice::from_ref(&a), Limits::default()).is_err());
        std::fs::write(&a, b"abcd").unwrap();
        std::fs::write(&b, b"abcd").unwrap();
        let paths = [a, b];
        assert!(
            read_files(
                &paths,
                Limits {
                    max_total_bytes: 7,
                    ..Limits::default()
                }
            )
            .is_err()
        );
        assert!(
            read_files(
                &paths,
                Limits {
                    max_entries: 1,
                    ..Limits::default()
                }
            )
            .is_err()
        );
        assert!(
            read_files(
                &paths,
                Limits {
                    max_path_bytes: 0,
                    ..Limits::default()
                }
            )
            .is_err()
        );
        assert!(read_files(&[root.path().to_path_buf()], Limits::default()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn links_are_rejected_before_opening_the_target() {
        let root = tempfile::tempdir().unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(root.path().join("missing"), &link).unwrap();
        assert!(read_files(&[link], Limits::default()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn opening_rejects_links_and_does_not_block_on_replaced_fifo() {
        use std::os::unix::{ffi::OsStrExt, fs::symlink};
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("file");
        std::fs::write(&file, b"target").unwrap();
        let link = root.path().join("link");
        symlink(&file, &link).unwrap();
        assert!(open_regular_file(&link).is_err());
        let fifo = root.path().join("fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(open_regular_file(&fifo).is_err());
        });
        assert!(
            receiver
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
        );
    }
}
