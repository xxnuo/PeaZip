use peazip_native::{DirectoryError, DirectoryLimits, DirectorySnapshot, EntryKind, Error, Limits};
use std::{ffi::OsStr, fs, path::Path};

fn open(path: &Path) -> DirectorySnapshot {
    DirectorySnapshot::open(path, DirectoryLimits::default()).unwrap()
}

fn id(snapshot: &DirectorySnapshot, name: impl AsRef<OsStr>) -> usize {
    snapshot
        .entries()
        .iter()
        .find(|entry| entry.name == name.as_ref())
        .unwrap()
        .id
}

#[test]
fn parent_components_publish_a_physical_location_for_up_navigation() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("a")).unwrap();
    let snapshot = open(&root.path().join("a/.."));
    let expected = fs::canonicalize(root.path()).unwrap();
    assert_eq!(snapshot.path(), expected);
    assert_eq!(snapshot.path().parent(), expected.parent());
    assert!(snapshot.entries().iter().any(|entry| entry.name == "a"));
}

#[test]
fn lists_unicode_files_empty_directories_and_stable_snapshot_ids() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("empty")).unwrap();
    fs::write(root.path().join("中文.txt"), b"hello").unwrap();
    let snapshot = open(root.path());
    assert!(snapshot.path().is_absolute());
    assert_eq!(snapshot.entries().len(), 2);
    for (index, entry) in snapshot.entries().iter().enumerate() {
        assert_eq!(entry.id, index);
    }
    let file = &snapshot.entries()[id(&snapshot, "中文.txt")];
    assert_eq!(file.display_name, "中文.txt");
    assert_eq!(file.kind, EntryKind::File);
    assert_eq!(file.size, Some(5));
    assert!(file.modified.is_some());
    assert_eq!(
        snapshot.read_archive(file.id, Limits::default()).unwrap(),
        b"hello"
    );
    let child = snapshot
        .open_child(id(&snapshot, "empty"), DirectoryLimits::default())
        .unwrap();
    assert_eq!(child.path(), snapshot.path().join("empty"));
    assert!(child.entries().is_empty());
    let before = snapshot.entries().to_vec();
    fs::write(root.path().join("later"), b"new").unwrap();
    let refreshed = snapshot.refresh(DirectoryLimits::default()).unwrap();
    assert_eq!(refreshed.entries().len(), 3);
    assert_eq!(snapshot.entries(), before);
    assert!(
        snapshot
            .open_child(file.id, DirectoryLimits::default())
            .is_err()
    );
    assert!(matches!(
        snapshot.read_archive(usize::MAX, Limits::default()),
        Err(Error::UnknownEntry(_))
    ));
    assert!(matches!(
        snapshot.open_child(usize::MAX, DirectoryLimits::default()),
        Err(DirectoryError::InvalidEntry)
    ));
}

#[test]
fn directory_budgets_fail_instead_of_returning_partial_rows() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("abc"), b"a").unwrap();
    fs::write(root.path().join("de"), b"b").unwrap();
    let limits = DirectoryLimits {
        max_entries: 2,
        max_name_bytes: 3,
        max_total_name_bytes: 5,
    };
    let snapshot = DirectorySnapshot::open(root.path(), limits).unwrap();
    for limits in [
        DirectoryLimits {
            max_entries: 1,
            ..limits
        },
        DirectoryLimits {
            max_name_bytes: 2,
            ..limits
        },
        DirectoryLimits {
            max_total_name_bytes: 4,
            ..limits
        },
    ] {
        assert!(matches!(
            DirectorySnapshot::open(root.path(), limits),
            Err(DirectoryError::Limit(_))
        ));
        assert!(matches!(
            snapshot.refresh(limits),
            Err(DirectoryError::Limit(_))
        ));
    }
    let empty = tempfile::tempdir().unwrap();
    assert!(
        DirectorySnapshot::open(
            empty.path(),
            DirectoryLimits {
                max_entries: 0,
                max_name_bytes: 0,
                max_total_name_bytes: 0
            }
        )
        .unwrap()
        .entries()
        .is_empty()
    );
}

#[test]
fn large_files_are_listed_but_archive_reads_and_zip_creation_enforce_byte_limits() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("large");
    fs::File::create(&path)
        .unwrap()
        .set_len(Limits::default().max_archive_bytes + 1)
        .unwrap();
    let snapshot = open(root.path());
    let file = &snapshot.entries()[0];
    assert_eq!(file.size, Some(Limits::default().max_archive_bytes + 1));
    assert!(matches!(
        snapshot.read_archive(file.id, Limits::default()),
        Err(Error::Limit("archive bytes"))
    ));
    assert!(
        snapshot
            .create_zip(&[file.id], &root.path().join("out.zip"), Limits::default())
            .is_err()
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    fs::write(&path, b"12345").unwrap();
    assert_eq!(
        snapshot
            .read_archive(
                file.id,
                Limits {
                    max_archive_bytes: 5,
                    ..Limits::default()
                }
            )
            .unwrap(),
        b"12345"
    );
    assert!(matches!(
        snapshot.read_archive(
            file.id,
            Limits {
                max_archive_bytes: 4,
                ..Limits::default()
            }
        ),
        Err(Error::Limit("archive bytes"))
    ));
}

#[test]
fn creates_selected_zip_using_shared_walk_and_never_overwrites() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("tree/empty")).unwrap();
    fs::write(root.path().join("tree/中文.txt"), b"inside").unwrap();
    fs::write(root.path().join("file"), b"selected").unwrap();
    fs::write(root.path().join("unselected"), b"other").unwrap();
    let snapshot = open(root.path());
    let selection = [id(&snapshot, "tree"), id(&snapshot, "file")];
    let destination = root.path().join("selected.zip");
    let listed = snapshot
        .create_zip(&selection, &destination, Limits::default())
        .unwrap();
    let bytes = fs::read(&destination).unwrap();
    let output = peazip_core::extract_archive(&bytes, Limits::default()).unwrap();
    assert_eq!(listed.len(), 4);
    assert!(
        output
            .iter()
            .any(|entry| entry.path == "tree/empty/" && entry.is_dir)
    );
    assert_eq!(
        output
            .iter()
            .find(|entry| entry.path == "tree/中文.txt")
            .unwrap()
            .data,
        b"inside"
    );
    assert_eq!(
        output
            .iter()
            .find(|entry| entry.path == "file")
            .unwrap()
            .data,
        b"selected"
    );
    assert!(output.iter().all(|entry| entry.path != "unselected"));
    assert!(
        snapshot
            .create_zip(&selection, &destination, Limits::default())
            .is_err()
    );
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    let missing = root.path().join("missing.zip");
    for ids in [vec![], vec![usize::MAX], vec![selection[0], selection[0]]] {
        assert!(
            snapshot
                .create_zip(&ids, &missing, Limits::default())
                .is_err()
        );
        assert!(!missing.exists());
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 4);
}

#[test]
fn folder_errors_are_separate_and_friendly() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("file"), b"data").unwrap();
    assert!(matches!(
        DirectorySnapshot::open(&root.path().join("file"), DirectoryLimits::default()),
        Err(DirectoryError::NotDirectory)
    ));
    let error = DirectorySnapshot::open(&root.path().join("missing"), DirectoryLimits::default())
        .unwrap_err();
    assert!(error.to_string().contains("no longer exists"));
    let error = DirectoryError::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "private context",
    ));
    assert!(error.to_string().contains("Permission denied"));
    assert!(!error.to_string().contains("private context"));
    assert!(std::error::Error::source(&error).is_some());
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{
        ffi::{CString, OsString},
        os::unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::symlink,
        },
    };

    fn fifo(path: &Path) {
        let path = CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    }

    #[test]
    fn parent_link_then_parent_component_keeps_physical_filesystem_semantics() {
        let logical = tempfile::tempdir().unwrap();
        let physical = tempfile::tempdir().unwrap();
        fs::create_dir_all(physical.path().join("parent/child")).unwrap();
        fs::create_dir(physical.path().join("parent/selected")).unwrap();
        fs::write(physical.path().join("parent/marker"), b"physical parent").unwrap();
        symlink(
            physical.path().join("parent/child"),
            logical.path().join("link"),
        )
        .unwrap();
        let snapshot = open(&logical.path().join("link/.."));
        assert_eq!(
            snapshot.path(),
            fs::canonicalize(physical.path().join("parent")).unwrap()
        );
        assert_eq!(
            snapshot
                .read_archive(id(&snapshot, "marker"), Limits::default())
                .unwrap(),
            b"physical parent"
        );
        let selected = open(&logical.path().join("link/../selected"));
        assert_eq!(
            selected.path(),
            fs::canonicalize(physical.path().join("parent/selected")).unwrap()
        );
        assert!(
            DirectorySnapshot::open(&logical.path().join("link"), DirectoryLimits::default())
                .is_err()
        );
    }

    #[test]
    fn links_and_special_files_are_visible_but_never_opened_or_archived() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("file"), b"inside").unwrap();
        fs::create_dir(root.path().join("directory")).unwrap();
        symlink("file", root.path().join("link")).unwrap();
        symlink("directory", root.path().join("dir-link")).unwrap();
        symlink("missing", root.path().join("dangling")).unwrap();
        fifo(&root.path().join("fifo"));
        let snapshot = open(root.path());
        for name in ["link", "dir-link", "dangling", "fifo"] {
            let row = &snapshot.entries()[id(&snapshot, name)];
            assert_eq!(
                row.kind,
                if name == "fifo" {
                    EntryKind::Other
                } else {
                    EntryKind::Link
                }
            );
            assert_eq!(row.size, None);
            assert!(snapshot.read_archive(row.id, Limits::default()).is_err());
            assert!(
                snapshot
                    .open_child(row.id, DirectoryLimits::default())
                    .is_err()
            );
            assert!(
                snapshot
                    .create_zip(&[row.id], &root.path().join("out.zip"), Limits::default())
                    .is_err()
            );
        }
        assert!(
            DirectorySnapshot::open(&root.path().join("dir-link"), DirectoryLimits::default())
                .is_err()
        );
        fs::remove_file(root.path().join("file")).unwrap();
        fifo(&root.path().join("file"));
        assert!(
            snapshot
                .read_archive(id(&snapshot, "file"), Limits::default())
                .is_err()
        );
        assert!(!root.path().join("out.zip").exists());
    }

    #[test]
    fn replaced_root_keeps_original_directory_capability_for_every_row_operation() {
        let base = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = base.path().join("root");
        fs::create_dir_all(root.join("child")).unwrap();
        fs::write(root.join("file"), b"original").unwrap();
        fs::write(root.join("child/leaf"), b"original child").unwrap();
        fs::create_dir(outside.path().join("child")).unwrap();
        fs::write(outside.path().join("file"), b"outside").unwrap();
        fs::write(outside.path().join("child/leaf"), b"outside child").unwrap();
        let snapshot = open(&root);
        fs::rename(&root, base.path().join("moved")).unwrap();
        symlink(outside.path(), &root).unwrap();
        assert_eq!(
            snapshot
                .read_archive(id(&snapshot, "file"), Limits::default())
                .unwrap(),
            b"original"
        );
        fs::write(base.path().join("moved/new"), b"new original").unwrap();
        let refreshed = snapshot.refresh(DirectoryLimits::default()).unwrap();
        assert!(refreshed.entries().iter().any(|entry| entry.name == "new"));
        let child = snapshot
            .open_child(id(&snapshot, "child"), DirectoryLimits::default())
            .unwrap();
        assert_eq!(
            child
                .read_archive(id(&child, "leaf"), Limits::default())
                .unwrap(),
            b"original child"
        );
        let destination = base.path().join("selected.zip");
        snapshot
            .create_zip(
                &[id(&snapshot, "file"), id(&snapshot, "child")],
                &destination,
                Limits::default(),
            )
            .unwrap();
        let output =
            peazip_core::extract_archive(&fs::read(destination).unwrap(), Limits::default())
                .unwrap();
        assert_eq!(
            output
                .iter()
                .find(|entry| entry.path == "file")
                .unwrap()
                .data,
            b"original"
        );
        assert_eq!(
            output
                .iter()
                .find(|entry| entry.path == "child/leaf")
                .unwrap()
                .data,
            b"original child"
        );
    }

    #[test]
    fn replacing_a_child_with_a_link_is_refused_but_held_child_stays_original() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("child")).unwrap();
        fs::write(root.path().join("child/file"), b"inside").unwrap();
        fs::write(root.path().join("file"), b"inside file").unwrap();
        fs::write(outside.path().join("file"), b"outside").unwrap();
        let snapshot = open(root.path());
        let child = snapshot
            .open_child(id(&snapshot, "child"), DirectoryLimits::default())
            .unwrap();
        fs::rename(root.path().join("child"), root.path().join("moved")).unwrap();
        symlink(outside.path(), root.path().join("child")).unwrap();
        assert!(
            snapshot
                .open_child(id(&snapshot, "child"), DirectoryLimits::default())
                .is_err()
        );
        assert!(
            snapshot
                .create_zip(
                    &[id(&snapshot, "child")],
                    &root.path().join("out.zip"),
                    Limits::default()
                )
                .is_err()
        );
        let refreshed = child.refresh(DirectoryLimits::default()).unwrap();
        assert_eq!(
            refreshed
                .read_archive(id(&refreshed, "file"), Limits::default())
                .unwrap(),
            b"inside"
        );
        fs::remove_file(root.path().join("file")).unwrap();
        symlink(outside.path().join("file"), root.path().join("file")).unwrap();
        assert!(
            snapshot
                .read_archive(id(&snapshot, "file"), Limits::default())
                .is_err()
        );
    }

    #[test]
    fn raw_non_utf8_ids_and_escaped_control_names_never_reinterpret_display_strings() {
        let root = tempfile::tempdir().unwrap();
        let raw = OsString::from_vec(b"raw-\xff".to_vec());
        let raw_dir = OsString::from_vec(b"dir-\xfe".to_vec());
        fs::write(root.path().join("raw-�"), b"replacement").unwrap();
        fs::write(root.path().join("line\nbreak"), b"control").unwrap();
        fs::write(root.path().join("line\\nbreak"), b"literal").unwrap();
        let snapshot = open(root.path());
        let display: std::collections::BTreeSet<_> = snapshot
            .entries()
            .iter()
            .map(|entry| entry.display_name.as_str())
            .collect();
        assert_eq!(display.len(), snapshot.entries().len());
        assert!(
            display
                .iter()
                .all(|name| !name.chars().any(char::is_control))
        );
        // APFS refuses invalid UTF-8 names; Linux filesystems exercise the raw-ID operations.
        if let Err(error) = fs::write(root.path().join(&raw), b"raw bytes") {
            assert_eq!(error.raw_os_error(), Some(libc::EILSEQ), "{error}");
            return;
        }
        fs::create_dir(root.path().join(&raw_dir)).unwrap();
        fs::write(root.path().join(&raw_dir).join("file"), b"raw directory").unwrap();
        let snapshot = open(root.path());
        let row = &snapshot.entries()[id(&snapshot, &raw)];
        assert_eq!(row.name, raw);
        assert!(!row.display_name.contains('�'));
        assert_eq!(
            snapshot.read_archive(row.id, Limits::default()).unwrap(),
            b"raw bytes"
        );
        let displays: std::collections::BTreeSet<_> = snapshot
            .entries()
            .iter()
            .map(|entry| entry.display_name.as_str())
            .collect();
        assert_eq!(displays.len(), snapshot.entries().len());
        assert!(
            displays
                .iter()
                .all(|display| !display.chars().any(char::is_control))
        );
        let child = snapshot
            .open_child(id(&snapshot, &raw_dir), DirectoryLimits::default())
            .unwrap();
        assert_eq!(
            child
                .read_archive(id(&child, "file"), Limits::default())
                .unwrap(),
            b"raw directory"
        );
        let target = root.path().join("out.zip");
        assert!(matches!(
            snapshot.create_zip(&[row.id], &target, Limits::default()),
            Err(Error::UnsafePath(_))
        ));
        assert!(!target.exists());
    }

    #[test]
    fn unsafe_portable_zip_names_are_listable_but_cannot_be_published() {
        let root = tempfile::tempdir().unwrap();
        for name in ["bad?name", "CON", "trailing."] {
            fs::write(root.path().join(name), b"input").unwrap();
        }
        let snapshot = open(root.path());
        for entry in snapshot.entries() {
            assert!(matches!(
                snapshot.create_zip(&[entry.id], &root.path().join("out.zip"), Limits::default()),
                Err(Error::UnsafePath(_))
            ));
            assert!(!root.path().join("out.zip").exists());
        }
    }

    #[test]
    fn snapshot_listing_does_not_keep_a_descriptor_per_entry() {
        const CHILD: &str = "PEAZIP_BROWSE_FD_SOURCE";
        if let Some(path) = std::env::var_os(CHILD) {
            let mut limit = std::mem::MaybeUninit::<libc::rlimit>::uninit();
            assert_eq!(
                unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()) },
                0
            );
            let mut limit = unsafe { limit.assume_init() };
            limit.rlim_cur = limit.rlim_max.min(64);
            assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) }, 0);
            let snapshot = open(Path::new(&path));
            assert_eq!(snapshot.entries().len(), 512);
            assert_eq!(
                snapshot
                    .refresh(DirectoryLimits::default())
                    .unwrap()
                    .entries()
                    .len(),
                512
            );
            return;
        }
        let root = tempfile::tempdir().unwrap();
        for n in 0..512 {
            fs::write(root.path().join(format!("file-{n}")), b"x").unwrap();
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "unix::snapshot_listing_does_not_keep_a_descriptor_per_entry",
                "--test-threads=1",
            ])
            .env(CHILD, root.path())
            .status()
            .unwrap();
        assert!(status.success());
    }
}
