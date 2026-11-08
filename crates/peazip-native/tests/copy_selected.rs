use peazip_native::{DirectoryLimits, DirectorySnapshot, Error, Limits};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
    sync::Barrier,
    thread,
};

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

fn tree(path: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, relative: &Path, entries: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for child in fs::read_dir(root.join(relative)).unwrap() {
            let child = child.unwrap();
            let name = relative.join(child.file_name());
            let kind = child.file_type().unwrap();
            assert!(!kind.is_symlink());
            entries.insert(
                name.clone(),
                if kind.is_dir() {
                    None
                } else {
                    Some(fs::read(child.path()).unwrap())
                },
            );
            if kind.is_dir() {
                visit(root, &name, entries);
            }
        }
    }
    let mut entries = BTreeMap::new();
    visit(path, Path::new(""), &mut entries);
    entries
}

#[test]
fn copies_selected_files_unicode_binary_subtrees_and_empty_directories_without_changing_sources() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fs::create_dir_all(source.join("docs/deep")).unwrap();
    fs::create_dir(source.join("docs/empty")).unwrap();
    fs::write(source.join("docs/deep/中文.bin"), [0, 127, 128, 255]).unwrap();
    fs::write(source.join("docs/empty-file"), []).unwrap();
    fs::write(source.join("top.txt"), b"top").unwrap();
    fs::write(source.join("skip.txt"), b"not selected").unwrap();
    let before = tree(&source);
    let snapshot = open(&source);
    let output = root.path().join("output");
    assert_eq!(
        snapshot
            .copy_selected_to_directory(
                &[id(&snapshot, "docs"), id(&snapshot, "top.txt")],
                &output,
                Limits::default()
            )
            .unwrap(),
        6
    );
    let expected = before
        .iter()
        .filter(|(path, _)| !path.starts_with("skip.txt"))
        .map(|(path, data)| (path.clone(), data.clone()))
        .collect();
    assert_eq!(tree(&output), expected);
    assert_eq!(tree(&source), before);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}

#[test]
fn copy_ignores_archive_size_and_compression_ratio_but_enforces_source_budgets() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let outputs = root.path().join("outputs");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&outputs).unwrap();
    fs::write(source.join("a"), b"12345").unwrap();
    fs::write(source.join("b"), b"67890").unwrap();
    fs::create_dir_all(source.join("tree/empty")).unwrap();
    fs::write(source.join("tree/child"), b"child").unwrap();
    let snapshot = open(&source);
    let a = id(&snapshot, "a");
    let b = id(&snapshot, "b");
    let folder = id(&snapshot, "tree");
    for (ids, limits) in [
        (
            vec![a],
            Limits {
                max_entry_bytes: 4,
                ..Limits::default()
            },
        ),
        (
            vec![a, b],
            Limits {
                max_total_bytes: 9,
                ..Limits::default()
            },
        ),
        (
            vec![a, b],
            Limits {
                max_entries: 1,
                ..Limits::default()
            },
        ),
        (
            vec![folder],
            Limits {
                max_entries: 2,
                ..Limits::default()
            },
        ),
        (
            vec![folder],
            Limits {
                max_path_bytes: 5,
                ..Limits::default()
            },
        ),
        (
            vec![a],
            Limits {
                max_path_bytes: 0,
                ..Limits::default()
            },
        ),
    ] {
        assert!(matches!(
            snapshot.copy_selected_to_directory(&ids, &outputs.join("rejected"), limits),
            Err(Error::Limit(_))
        ));
        assert!(tree(&outputs).is_empty());
    }
    assert!(
        snapshot
            .copy_selected_to_directory(&[], &outputs.join("empty"), Limits::default())
            .is_err()
    );
    assert!(matches!(
        snapshot.copy_selected_to_directory(
            &[usize::MAX],
            &outputs.join("bad-id"),
            Limits::default()
        ),
        Err(Error::UnknownEntry(_))
    ));
    assert!(matches!(
        snapshot.copy_selected_to_directory(&[a, a], &outputs.join("duplicate"), Limits::default()),
        Err(Error::DuplicatePath(_))
    ));
    assert!(tree(&outputs).is_empty());
    let limits = Limits {
        max_archive_bytes: 0,
        max_compression_ratio: 0,
        ..Limits::default()
    };
    assert_eq!(
        snapshot
            .copy_selected_to_directory(&[a, b], &outputs.join("copied"), limits)
            .unwrap(),
        2
    );
    assert_eq!(fs::read(outputs.join("copied/a")).unwrap(), b"12345");
    assert_eq!(fs::read(outputs.join("copied/b")).unwrap(), b"67890");
    assert!(
        snapshot
            .create_zip(&[a], &outputs.join("not-created.zip"), limits)
            .is_err()
    );
    assert!(!outputs.join("not-created.zip").exists());
}

#[test]
fn metadata_entry_limit_rejects_oversized_source_before_any_publication() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let outputs = root.path().join("outputs");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&outputs).unwrap();
    fs::File::create(source.join("large"))
        .unwrap()
        .set_len(Limits::default().max_entry_bytes + 1)
        .unwrap();
    let snapshot = open(&source);
    assert!(matches!(
        snapshot.copy_selected_to_directory(
            &[id(&snapshot, "large")],
            &outputs.join("out"),
            Limits::default()
        ),
        Err(Error::Limit("source bytes"))
    ));
    assert!(tree(&outputs).is_empty());
}

#[test]
fn existing_targets_missing_parents_and_invalid_destination_paths_are_not_modified() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let outputs = root.path().join("outputs");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&outputs).unwrap();
    fs::write(source.join("data"), b"source").unwrap();
    fs::write(outputs.join("file"), b"sentinel").unwrap();
    fs::create_dir(outputs.join("empty")).unwrap();
    fs::create_dir(outputs.join("nonempty")).unwrap();
    fs::write(outputs.join("nonempty/sentinel"), b"retain").unwrap();
    let before = tree(&outputs);
    let snapshot = open(&source);
    let ids = [id(&snapshot, "data")];
    for name in ["file", "empty", "nonempty"] {
        assert!(
            matches!(snapshot.copy_selected_to_directory(&ids, &outputs.join(name), Limits::default()), Err(Error::Io(error)) if error.kind() == io::ErrorKind::AlreadyExists)
        );
        assert_eq!(tree(&outputs), before);
    }
    for target in [
        outputs.join("missing/child"),
        outputs.join("file/child"),
        PathBuf::new(),
    ] {
        assert!(
            snapshot
                .copy_selected_to_directory(&ids, &target, Limits::default())
                .is_err()
        );
        assert_eq!(tree(&outputs), before);
    }
    assert_eq!(fs::read(source.join("data")).unwrap(), b"source");
}

#[test]
fn destination_inside_selected_tree_is_not_recursively_copied_into_itself() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("tree/empty")).unwrap();
    fs::write(root.path().join("tree/data"), b"original").unwrap();
    let before = tree(root.path());
    let snapshot = open(root.path());
    let target = root.path().join("tree/export");
    assert_eq!(
        snapshot
            .copy_selected_to_directory(&[id(&snapshot, "tree")], &target, Limits::default())
            .unwrap(),
        3
    );
    assert_eq!(tree(&target), before);
    assert_eq!(
        fs::read(root.path().join("tree/data")).unwrap(),
        b"original"
    );
    assert!(!target.join("tree/export").exists());
    assert_eq!(fs::read_dir(root.path().join("tree")).unwrap().count(), 3);
}

#[test]
fn competing_copies_publish_only_one_complete_winner_and_clean_staging() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let outputs = root.path().join("outputs");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&outputs).unwrap();
    for at in 0..8 {
        fs::write(source.join(format!("file-{at}")), vec![at; 1024]).unwrap();
    }
    let snapshot = open(&source);
    let before = tree(&source);
    let barrier = Barrier::new(8);
    let target = outputs.join("winner");
    let results = thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|at| {
                let snapshot = &snapshot;
                let barrier = &barrier;
                let target = &target;
                scope.spawn(move || {
                    barrier.wait();
                    (
                        at,
                        snapshot.copy_selected_to_directory(
                            &[id(snapshot, format!("file-{at}"))],
                            target,
                            Limits::default(),
                        ),
                    )
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        results.iter().filter(|(_, result)| result.is_ok()).count(),
        1
    );
    let winner = results.iter().find(|(_, result)| result.is_ok()).unwrap().0;
    assert_eq!(
        tree(&target),
        BTreeMap::from([(
            PathBuf::from(format!("file-{winner}")),
            Some(vec![winner as u8; 1024])
        )])
    );
    assert_eq!(fs::read_dir(&outputs).unwrap().count(), 1);
    assert_eq!(tree(&source), before);
}

#[cfg(unix)]
#[test]
fn raw_colon_backslash_controls_and_nonportable_names_copy_literally_while_zip_rejects_them() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let outputs = root.path().join("outputs");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&outputs).unwrap();
    for name in [
        "a:b",
        "a\\b",
        "line\nbreak",
        "CON.txt",
        "trailing. ",
        "..\\outside",
    ] {
        fs::write(source.join(name), name.as_bytes()).unwrap();
    }
    fs::create_dir_all(source.join("dir:raw/empty\\child")).unwrap();
    fs::write(source.join("dir:raw/tab\tfile"), b"nested").unwrap();
    let snapshot = open(&source);
    let ids: Vec<_> = snapshot.entries().iter().map(|entry| entry.id).collect();
    let before = tree(&source);
    assert_eq!(
        snapshot
            .copy_selected_to_directory(&ids, &outputs.join("copy"), Limits::default())
            .unwrap(),
        before.len()
    );
    assert_eq!(tree(&outputs.join("copy")), before);
    for &selected in &ids {
        assert!(matches!(
            snapshot.create_zip(
                &[selected],
                &outputs.join("rejected.zip"),
                Limits::default()
            ),
            Err(Error::UnsafePath(_))
        ));
        assert!(!outputs.join("rejected.zip").exists());
    }
    assert_eq!(tree(&source), before);
    assert!(!root.path().join("outside").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn invalid_utf8_names_preserve_exact_bytes_in_files_and_directories() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fs::create_dir(&source).unwrap();
    let raw_file = OsString::from_vec(b"file-\xff".to_vec());
    let raw_dir = OsString::from_vec(b"dir-\xfe".to_vec());
    fs::write(source.join(&raw_file), b"raw file").unwrap();
    fs::create_dir(source.join(&raw_dir)).unwrap();
    fs::write(
        source.join(&raw_dir).join(OsString::from_vec(vec![0xfd])),
        [0, 255],
    )
    .unwrap();
    let snapshot = open(&source);
    let ids: Vec<_> = snapshot.entries().iter().map(|entry| entry.id).collect();
    let target = root.path().join("copy");
    assert_eq!(
        snapshot
            .copy_selected_to_directory(&ids, &target, Limits::default())
            .unwrap(),
        3
    );
    assert_eq!(tree(&target), tree(&source));
    assert!(matches!(
        snapshot.create_zip(
            &[id(&snapshot, &raw_file)],
            &root.path().join("rejected.zip"),
            Limits::default()
        ),
        Err(Error::UnsafePath(_))
    ));
    assert!(!root.path().join("rejected.zip").exists());
}

#[cfg(unix)]
#[test]
fn replaced_root_keeps_held_capability_and_source_link_swaps_are_rejected() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let chosen = root.path().join("chosen");
    let moved = root.path().join("moved");
    let outside = root.path().join("outside");
    let outputs = root.path().join("outputs");
    fs::create_dir_all(chosen.join("folder")).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::create_dir(&outputs).unwrap();
    fs::write(chosen.join("file"), b"inside").unwrap();
    fs::write(chosen.join("folder/child"), b"child").unwrap();
    fs::write(outside.join("file"), b"outside sentinel").unwrap();
    let snapshot = open(&chosen);
    let file = id(&snapshot, "file");
    let folder = id(&snapshot, "folder");
    let before = tree(&chosen);
    fs::rename(&chosen, &moved).unwrap();
    symlink(&outside, &chosen).unwrap();
    snapshot
        .copy_selected_to_directory(&[file, folder], &outputs.join("good"), Limits::default())
        .unwrap();
    assert_eq!(tree(&outputs.join("good")), before);
    assert_eq!(tree(&moved), before);
    fs::remove_file(moved.join("file")).unwrap();
    symlink(outside.join("file"), moved.join("file")).unwrap();
    assert!(matches!(
        snapshot.copy_selected_to_directory(&[file], &outputs.join("bad-file"), Limits::default()),
        Err(Error::UnsupportedEntry(_))
    ));
    fs::rename(moved.join("folder"), moved.join("old-folder")).unwrap();
    symlink(&outside, moved.join("folder")).unwrap();
    assert!(matches!(
        snapshot.copy_selected_to_directory(
            &[folder],
            &outputs.join("bad-folder"),
            Limits::default()
        ),
        Err(Error::UnsupportedEntry(_))
    ));
    assert_eq!(fs::read_dir(&outputs).unwrap().count(), 1);
    assert_eq!(fs::read(outside.join("file")).unwrap(), b"outside sentinel");
}

#[cfg(unix)]
#[test]
fn nested_links_fifo_and_destination_links_never_follow_or_publish_partial_output() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let outputs = root.path().join("outputs");
    let outside = root.path().join("outside");
    fs::create_dir_all(source.join("tree")).unwrap();
    fs::create_dir(&outputs).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(source.join("first"), b"read first").unwrap();
    fs::write(outside.join("secret"), b"outside").unwrap();
    symlink(outside.join("secret"), source.join("tree/link")).unwrap();
    let snapshot = open(&source);
    assert!(matches!(
        snapshot.copy_selected_to_directory(
            &[id(&snapshot, "first"), id(&snapshot, "tree")],
            &outputs.join("rejected"),
            Limits::default()
        ),
        Err(Error::UnsupportedEntry(_))
    ));
    assert!(tree(&outputs).is_empty());
    fs::remove_file(source.join("tree/link")).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(source.join("tree/fifo"))
            .status()
            .unwrap()
            .success()
    );
    assert!(
        snapshot
            .copy_selected_to_directory(
                &[id(&snapshot, "tree")],
                &outputs.join("fifo"),
                Limits::default()
            )
            .is_err()
    );
    assert!(tree(&outputs).is_empty());
    for (name, target) in [
        ("dir-link", outside.clone()),
        ("file-link", outside.join("secret")),
        ("dangling", outside.join("absent")),
    ] {
        let destination = outputs.join(name);
        symlink(&target, &destination).unwrap();
        assert!(
            snapshot
                .copy_selected_to_directory(
                    &[id(&snapshot, "first")],
                    &destination,
                    Limits::default()
                )
                .is_err()
        );
        assert_eq!(fs::read_link(&destination).unwrap(), target);
    }
    assert_eq!(fs::read_dir(&outputs).unwrap().count(), 3);
    assert_eq!(fs::read(outside.join("secret")).unwrap(), b"outside");
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn staging_write_failure_removes_partial_tree_and_keeps_sources() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let mut outputs = root.path().join("outputs");
    let mut nested = source.join("tree");
    let source_depth = if cfg!(target_os = "macos") { 3 } else { 10 };
    let destination_depth = if cfg!(target_os = "macos") { 5 } else { 20 };
    let component = "x".repeat(150);
    for _ in 0..source_depth {
        nested.push(&component);
    }
    for _ in 0..destination_depth {
        outputs.push(&component);
    }
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir_all(&outputs).unwrap();
    fs::write(nested.join("payload"), b"source unchanged").unwrap();
    let snapshot = open(&source);
    assert!(matches!(
        snapshot.copy_selected_to_directory(
            &[id(&snapshot, "tree")],
            &outputs.join("out"),
            Limits {
                max_path_bytes: 4096,
                ..Limits::default()
            }
        ),
        Err(Error::Io(_))
    ));
    assert_eq!(fs::read_dir(&outputs).unwrap().count(), 0);
    assert_eq!(
        fs::read(nested.join("payload")).unwrap(),
        b"source unchanged"
    );
}
