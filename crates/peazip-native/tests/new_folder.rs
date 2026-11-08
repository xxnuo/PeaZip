use peazip_native::{
    DirectoryError, DirectoryLimits, DirectorySnapshot, EntryKind, validate_folder_name,
};
use std::{collections::BTreeSet, fs, io, path::Path, sync::Barrier, thread};

fn open(path: &Path) -> DirectorySnapshot {
    DirectorySnapshot::open(path, DirectoryLimits::default()).unwrap()
}

fn already_exists(result: Result<(), DirectoryError>) {
    assert!(
        matches!(result, Err(DirectoryError::Io(error)) if error.kind() == io::ErrorKind::AlreadyExists)
    );
}

#[test]
fn invalid_components_and_controls_are_rejected_before_any_output() {
    let root = tempfile::tempdir().unwrap();
    let snapshot = open(root.path());
    let limits = DirectoryLimits::default();
    for name in [
        "",
        ".",
        "..",
        "/",
        "../escape",
        "a/b",
        "a\\b",
        "C:folder",
        "a<b",
        "a>b",
        "a\"b",
        "a|b",
        "a?b",
        "a*b",
        "trailing.",
        "trailing ",
        " ",
        ".. ",
    ] {
        assert!(
            matches!(
                validate_folder_name(name, limits),
                Err(DirectoryError::InvalidName)
            ),
            "{name:?}"
        );
        assert!(
            matches!(
                snapshot.create_folder(name, limits),
                Err(DirectoryError::InvalidName)
            ),
            "{name:?}"
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
    for control in (0..=31).chain(127..=159).map(char::from_u32) {
        let name = format!("before{}after", control.unwrap());
        assert!(matches!(
            snapshot.create_folder(&name, limits),
            Err(DirectoryError::InvalidName)
        ));
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    let message = DirectoryError::InvalidName.to_string();
    assert!(message.contains("folder name") && message.len() < 160);
    assert_eq!(
        DirectoryError::Io(io::Error::from(io::ErrorKind::NotFound)).to_string(),
        "This folder or item no longer exists."
    );
}

#[test]
fn windows_device_names_with_extensions_match_core_rules_on_every_host() {
    let root = tempfile::tempdir().unwrap();
    let snapshot = open(root.path());
    let mut names = vec!["CON".to_owned(), "pRn".into(), "AuX".into(), "nul".into()];
    for number in 1..=9 {
        names.extend([format!("cOm{number}"), format!("LpT{number}")]);
    }
    for stem in names {
        for name in [
            stem.clone(),
            format!("{stem}.txt"),
            format!("{stem}.one.two"),
        ] {
            assert!(
                matches!(
                    validate_folder_name(&name, DirectoryLimits::default()),
                    Err(DirectoryError::InvalidName)
                ),
                "{name}"
            );
            assert!(
                matches!(
                    snapshot.create_folder(&name, DirectoryLimits::default()),
                    Err(DirectoryError::InvalidName)
                ),
                "{name}"
            );
        }
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn valid_unicode_and_literal_names_create_only_empty_single_directories_without_trimming() {
    let root = tempfile::tempdir().unwrap();
    let snapshot = open(root.path());
    let names = [
        "中文目录",
        "é",
        " leading",
        "new folder",
        ".hidden",
        "a..b",
        "COM0",
        "COM10",
        "LPT0",
        "CONSOLE",
        "a.con",
    ];
    for name in names {
        validate_folder_name(name, DirectoryLimits::default()).unwrap();
        snapshot
            .create_folder(name, DirectoryLimits::default())
            .unwrap();
        let path = root.path().join(name);
        assert!(path.is_dir());
        assert_eq!(fs::read_dir(path).unwrap().count(), 0);
    }
    assert!(!root.path().join("leading").exists());
    assert!(
        snapshot.entries().is_empty(),
        "mkdir must not implicitly refresh the snapshot"
    );
    let refreshed = snapshot.refresh(DirectoryLimits::default()).unwrap();
    assert!(
        refreshed
            .entries()
            .iter()
            .all(|entry| entry.kind == EntryKind::Directory)
    );
    assert_eq!(
        refreshed
            .entries()
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<BTreeSet<_>>(),
        names.into_iter().map(Into::into).collect()
    );
}

#[test]
fn name_budget_counts_utf8_bytes_and_rejects_before_mkdir() {
    let root = tempfile::tempdir().unwrap();
    let snapshot = open(root.path());
    let limits = DirectoryLimits {
        max_name_bytes: 5,
        ..DirectoryLimits::default()
    };
    assert!(matches!(
        validate_folder_name("中文", limits),
        Err(DirectoryError::Limit("name bytes"))
    ));
    assert!(matches!(
        snapshot.create_folder("中文", limits),
        Err(DirectoryError::Limit("name bytes"))
    ));
    assert!(matches!(
        snapshot.create_folder(
            "a",
            DirectoryLimits {
                max_name_bytes: 0,
                ..limits
            }
        ),
        Err(DirectoryError::Limit("name bytes"))
    ));
    assert!(matches!(
        snapshot.create_folder(&"界".repeat(10_000), limits),
        Err(DirectoryError::Limit("name bytes"))
    ));
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    snapshot
        .create_folder(
            "中文",
            DirectoryLimits {
                max_name_bytes: 6,
                ..limits
            },
        )
        .unwrap();
    assert!(root.path().join("中文").is_dir());
}

#[test]
fn existing_files_and_directories_are_never_overwritten_or_reused() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("file"), b"original file").unwrap();
    fs::create_dir(root.path().join("directory")).unwrap();
    fs::write(root.path().join("directory/sentinel"), b"original child").unwrap();
    let snapshot = open(root.path());
    already_exists(snapshot.create_folder("file", DirectoryLimits::default()));
    already_exists(snapshot.create_folder("directory", DirectoryLimits::default()));
    assert_eq!(
        fs::read(root.path().join("file")).unwrap(),
        b"original file"
    );
    assert_eq!(
        fs::read(root.path().join("directory/sentinel")).unwrap(),
        b"original child"
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}

#[test]
fn mkdir_success_is_distinct_from_refresh_limits_and_ignores_stale_snapshot_capacity() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("old"), b"sentinel").unwrap();
    let snapshot = open(root.path());
    let limits = DirectoryLimits {
        max_entries: 1,
        max_name_bytes: 3,
        max_total_name_bytes: 3,
    };
    snapshot.create_folder("new", limits).unwrap();
    assert!(root.path().join("new").is_dir());
    assert_eq!(snapshot.entries().len(), 1);
    assert!(matches!(
        snapshot.refresh(limits),
        Err(DirectoryError::Limit("entry count"))
    ));
    assert!(matches!(
        snapshot.refresh(DirectoryLimits {
            max_entries: 2,
            ..limits
        }),
        Err(DirectoryError::Limit("total name bytes"))
    ));
    already_exists(snapshot.create_folder("new", limits));
    assert_eq!(fs::read(root.path().join("old")).unwrap(), b"sentinel");
}

#[test]
fn concurrent_same_name_mkdir_has_exactly_one_success() {
    let root = tempfile::tempdir().unwrap();
    let snapshot = open(root.path());
    let barrier = Barrier::new(8);
    let results = thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    snapshot.create_folder("winner", DirectoryLimits::default())
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    for result in results.into_iter().filter(Result::is_err) {
        already_exists(result);
    }
    assert!(root.path().join("winner").is_dir());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn existing_links_including_dangling_links_are_not_followed_or_recreated() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("sentinel"), b"outside").unwrap();
    let links = [
        ("directory-link", outside.path().to_path_buf()),
        ("file-link", outside.path().join("sentinel")),
        ("dangling", outside.path().join("absent")),
    ];
    for (name, target) in &links {
        symlink(target, root.path().join(name)).unwrap();
    }
    let snapshot = open(root.path());
    for (name, target) in links {
        already_exists(snapshot.create_folder(name, DirectoryLimits::default()));
        assert!(
            fs::symlink_metadata(root.path().join(name))
                .unwrap()
                .is_symlink()
        );
        assert_eq!(fs::read_link(root.path().join(name)).unwrap(), target);
    }
    assert_eq!(
        fs::read(outside.path().join("sentinel")).unwrap(),
        b"outside"
    );
    assert!(!outside.path().join("absent").exists());
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn replaced_snapshot_root_still_creates_inside_the_original_held_directory() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let chosen = root.path().join("chosen");
    let moved = root.path().join("moved");
    let outside = root.path().join("outside");
    fs::create_dir(&chosen).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("sentinel"), b"outside").unwrap();
    let snapshot = open(&chosen);
    fs::rename(&chosen, &moved).unwrap();
    symlink(&outside, &chosen).unwrap();
    snapshot
        .create_folder("new", DirectoryLimits::default())
        .unwrap();
    assert!(moved.join("new").is_dir());
    assert!(!outside.join("new").exists());
    assert!(fs::symlink_metadata(&chosen).unwrap().is_symlink());
    assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"outside");
    let refreshed = snapshot.refresh(DirectoryLimits::default()).unwrap();
    assert_eq!(refreshed.entries().len(), 1);
    assert_eq!(refreshed.entries()[0].name, "new");
}
