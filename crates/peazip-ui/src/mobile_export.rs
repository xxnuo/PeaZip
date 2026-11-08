use peazip_core::{Limits, MemoryEntry, validate_memory_entries};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

pub(crate) fn stage_entries(
    entries: &[MemoryEntry],
    limits: Limits,
) -> Result<(tempfile::TempDir, PathBuf), String> {
    validate_memory_entries(entries, limits).map_err(|error| error.to_string())?;
    if entries.is_empty() {
        return Err("No entries were selected for extraction.".into());
    }
    // Implicit parents must not expand a small member list into millions of directories.
    let mut nodes = BTreeSet::new();
    for entry in entries {
        let path = entry.path.trim_end_matches('/');
        for end in path
            .match_indices('/')
            .map(|(index, _)| index)
            .chain(std::iter::once(path.len()))
        {
            nodes.insert(&path[..end]);
            if nodes.len() > 16_384 {
                return Err("Extraction expands beyond the 16384 filesystem item limit.".into());
            }
        }
    }
    let directory = tempfile::Builder::new()
        .prefix("peazip-extract-")
        .tempdir()
        .map_err(|error| error.to_string())?;
    let suffix = directory
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("peazip-extract-"))
        .ok_or("Cannot determine the extraction directory name.")?;
    let root = directory.path().join(format!("PeaZip-{suffix}"));
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    for entry in entries {
        let path = root.join(&entry.path);
        if entry.is_dir {
            fs::create_dir_all(path).map_err(|error| error.to_string())?;
        } else {
            fs::create_dir_all(path.parent().ok_or("Invalid output parent.")?)
                .map_err(|error| error.to_string())?;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(path).map_err(|error| error.to_string())?;
            file.write_all(&entry.data)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok((directory, root))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file(name: &str, bytes: &[u8]) -> MemoryEntry {
        MemoryEntry {
            path: name.into(),
            is_dir: false,
            data: bytes.into(),
        }
    }

    #[test]
    fn staging_preserves_binary_unicode_implicit_parents_and_empty_directories_and_cleans_on_drop()
    {
        let entries = [
            file("docs/你好.txt", b"hello\0\xff"),
            file("zero", b""),
            MemoryEntry {
                path: "empty/".into(),
                is_dir: true,
                data: vec![],
            },
        ];
        let (owner, root) = stage_entries(&entries, Limits::default()).unwrap();
        assert_eq!(
            fs::read(root.join("docs/你好.txt")).unwrap(),
            b"hello\0\xff"
        );
        assert!(root.join("empty").is_dir());
        assert_eq!(fs::read_dir(root.join("empty")).unwrap().count(), 0);
        assert!(fs::read(root.join("zero")).unwrap().is_empty());
        drop(owner);
        assert!(!root.exists());
    }

    #[test]
    fn staging_rejects_traversal_absolute_paths_duplicates_and_parent_conflicts() {
        for name in [
            "../escape",
            "/absolute",
            "a/../../escape",
            "C:/outside",
            "a\\b",
        ] {
            assert!(stage_entries(&[file(name, b"x")], Limits::default()).is_err());
        }
        for entries in [
            vec![file("same", b"a"), file("same", b"b")],
            vec![file("a", b"a"), file("a/child", b"b")],
        ] {
            assert!(stage_entries(&entries, Limits::default()).is_err());
        }
    }

    #[test]
    fn staging_revalidates_limits_and_directory_payloads() {
        let data = [file("first", b"abcd"), file("second", b"abcd")];
        assert!(
            stage_entries(
                &data,
                Limits {
                    max_total_bytes: 7,
                    ..Limits::default()
                }
            )
            .is_err()
        );
        assert!(
            stage_entries(
                &data,
                Limits {
                    max_entries: 1,
                    ..Limits::default()
                }
            )
            .is_err()
        );
        assert!(
            stage_entries(
                &data,
                Limits {
                    max_entry_bytes: 3,
                    ..Limits::default()
                }
            )
            .is_err()
        );
        assert!(
            stage_entries(
                &[MemoryEntry {
                    path: "dir/".into(),
                    is_dir: true,
                    data: vec![1]
                }],
                Limits::default()
            )
            .is_err()
        );
        assert!(stage_entries(&[], Limits::default()).is_err());
    }

    #[test]
    fn implicit_parent_expansion_is_bounded_before_creating_the_tree() {
        let entries = (0..4096)
            .map(|index| file(&format!("{index}/a/b/c/d/e/file"), b""))
            .collect::<Vec<_>>();
        let error = stage_entries(&entries, Limits::default()).unwrap_err();
        assert!(error.contains("filesystem item limit"));
    }
}
