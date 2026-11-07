use peazip_native::{Limits, create_zip_from_paths};
use std::{fs, path::Path};

fn children(path: &Path) -> usize {
    fs::read_dir(path).unwrap().count()
}

#[test]
fn generic_extraction_restores_tar_tree_without_overwriting() {
    let archive = include_bytes!("../../../tests/fixtures/sample.tar");
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("restored");
    let count = peazip_native::extract_archive_to_directory(
        archive,
        &["docs/".into()],
        &target,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(count, 4);
    assert_eq!(
        fs::read(target.join("docs/你好.txt")).unwrap(),
        "tar 你好\n".as_bytes()
    );
    assert_eq!(
        fs::read(target.join("docs/nested/data.bin")).unwrap(),
        [0, 127, 255]
    );
    assert!(target.join("docs/empty").is_dir());
    assert!(!target.join("other.txt").exists());
    assert!(
        peazip_native::extract_archive_to_directory(
            archive,
            &["docs/".into()],
            &target,
            Limits::default()
        )
        .is_err()
    );
    assert_eq!(
        fs::read(target.join("docs/你好.txt")).unwrap(),
        "tar 你好\n".as_bytes()
    );
    let mut corrupt = archive.to_vec();
    corrupt[0] ^= 1;
    assert!(
        peazip_native::extract_archive_to_directory(
            &corrupt,
            &["docs/".into()],
            &dir.path().join("bad"),
            Limits::default()
        )
        .is_err()
    );
    assert_eq!(children(dir.path()), 1);
}

#[test]
fn real_roundtrip_preserves_sources() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("中文.bin");
    let data = [0, 255, 1, 128];
    fs::write(&source, data).unwrap();
    let empty = dir.path().join("empty");
    fs::write(&empty, []).unwrap();
    let target = dir.path().join("output.zip");
    let listed =
        create_zip_from_paths(&[source.clone(), empty], &target, Limits::default()).unwrap();
    assert!(listed.iter().any(|entry| entry.path == "中文.bin"));
    let extracted =
        peazip_core::extract_zip(&fs::read(target).unwrap(), Limits::default()).unwrap();
    assert_eq!(
        extracted
            .iter()
            .find(|entry| entry.path == "中文.bin")
            .unwrap()
            .data,
        data
    );
    assert!(
        extracted
            .iter()
            .find(|entry| entry.path == "empty")
            .unwrap()
            .data
            .is_empty()
    );
    assert_eq!(fs::read(source).unwrap(), data);
    assert_eq!(children(dir.path()), 3);
}

#[test]
fn invalid_and_limits_leave_no_output_or_temporary() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    fs::write(&source, b"12345").unwrap();
    let target = dir.path().join("out.zip");
    for paths in [vec![], vec![dir.path().join("missing")]] {
        assert!(create_zip_from_paths(&paths, &target, Limits::default()).is_err());
        assert_eq!(children(dir.path()), 1);
    }
    for limits in [
        Limits {
            max_entry_bytes: 4,
            ..Limits::default()
        },
        Limits {
            max_total_bytes: 4,
            ..Limits::default()
        },
        Limits {
            max_archive_bytes: 1,
            ..Limits::default()
        },
        Limits {
            max_entries: 0,
            ..Limits::default()
        },
        Limits {
            max_path_bytes: 1,
            ..Limits::default()
        },
        Limits {
            max_compression_ratio: 0,
            ..Limits::default()
        },
    ] {
        assert!(create_zip_from_paths(std::slice::from_ref(&source), &target, limits).is_err());
        assert_eq!(children(dir.path()), 1);
    }
    let second = dir.path().join("second");
    fs::write(&second, b"67890").unwrap();
    assert!(
        create_zip_from_paths(
            &[source, second],
            &target,
            Limits {
                max_total_bytes: 9,
                ..Limits::default()
            }
        )
        .is_err()
    );
    assert_eq!(children(dir.path()), 2);
}

#[test]
fn rejects_duplicate_basenames() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("nested");
    fs::create_dir(&nested).unwrap();
    let a = dir.path().join("same");
    let b = nested.join("same");
    fs::write(&a, b"a").unwrap();
    fs::write(&b, b"b").unwrap();
    let target = dir.path().join("out.zip");
    assert!(matches!(
        create_zip_from_paths(&[a, b], &target, Limits::default()),
        Err(peazip_native::Error::DuplicatePath(_))
    ));
    assert_eq!(children(dir.path()), 2);
}

#[test]
fn existing_file_and_directory_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    fs::write(&source, b"input").unwrap();
    let target = dir.path().join("target");
    fs::write(&target, b"original").unwrap();
    assert!(
        create_zip_from_paths(std::slice::from_ref(&source), &target, Limits::default()).is_err()
    );
    assert_eq!(fs::read(&target).unwrap(), b"original");
    assert!(create_zip_from_paths(&[source], dir.path(), Limits::default()).is_err());
    assert_eq!(children(dir.path()), 2);
}

#[cfg(unix)]
#[test]
fn rejects_links_non_utf8_and_fifo_without_blocking() {
    use std::os::unix::{ffi::OsStringExt, fs::symlink};
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    fs::write(&source, b"original").unwrap();
    let link = dir.path().join("link");
    symlink(&source, &link).unwrap();
    let dangling = dir.path().join("dangling");
    symlink(dir.path().join("missing"), &dangling).unwrap();
    for target in [&link, &dangling] {
        assert!(
            create_zip_from_paths(std::slice::from_ref(&source), target, Limits::default())
                .is_err()
        );
        assert!(fs::symlink_metadata(target).unwrap().is_symlink());
    }
    let invalid = dir.path().join(std::ffi::OsString::from_vec(vec![0xff]));
    let fifo = dir.path().join("fifo");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(status.success());
    let target = dir.path().join("out.zip");
    for input in [link, dangling, invalid, fifo] {
        assert!(create_zip_from_paths(&[input], &target, Limits::default()).is_err());
        assert_eq!(children(dir.path()), 4);
    }
    assert_eq!(fs::read(source).unwrap(), b"original");
}

#[cfg(windows)]
#[test]
fn publishes_to_a_long_windows_path_without_overwriting() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("input.txt");
    fs::write(&source, b"long path content").unwrap();
    let mut parent = dir.path().to_path_buf();
    while parent.as_os_str().len() < 280 {
        parent.push("a-long-directory-component");
    }
    fs::create_dir_all(&parent).unwrap();
    let target = parent.join("archive.zip");
    create_zip_from_paths(std::slice::from_ref(&source), &target, Limits::default()).unwrap();
    let original = fs::read(&target).unwrap();
    assert_eq!(
        peazip_core::extract_zip(&original, Limits::default()).unwrap()[0].data,
        b"long path content"
    );
    assert!(create_zip_from_paths(&[source], &target, Limits::default()).is_err());
    assert_eq!(fs::read(&target).unwrap(), original);
    assert_eq!(children(&parent), 1);
}

fn extraction_fixture() -> Vec<u8> {
    use peazip_core::MemoryEntry;
    peazip_core::create_zip(
        &[
            MemoryEntry {
                path: "tree/".into(),
                is_dir: true,
                data: vec![],
            },
            MemoryEntry {
                path: "tree/nested/data".into(),
                is_dir: false,
                data: b"payload".to_vec(),
            },
            MemoryEntry {
                path: "tree/empty/".into(),
                is_dir: true,
                data: vec![],
            },
            MemoryEntry {
                path: "other".into(),
                is_dir: false,
                data: b"other".to_vec(),
            },
        ],
        Limits::default(),
    )
    .unwrap()
}

#[test]
fn extracts_directory_children_and_individual_files() {
    use peazip_native::extract_zip_to_directory;
    let dir = tempfile::tempdir().unwrap();
    let bytes = extraction_fixture();
    let target = dir.path().join("directory");
    assert_eq!(
        extract_zip_to_directory(&bytes, &["tree/".into()], &target, Limits::default()).unwrap(),
        3
    );
    assert_eq!(
        fs::read(target.join("tree/nested/data")).unwrap(),
        b"payload"
    );
    assert!(target.join("tree/empty").is_dir());
    assert!(!target.join("other").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    let target = dir.path().join("partial");
    assert_eq!(
        extract_zip_to_directory(
            &bytes,
            &["tree/nested/data".into(), "other".into()],
            &target,
            Limits::default()
        )
        .unwrap(),
        2
    );
    assert_eq!(fs::read(target.join("other")).unwrap(), b"other");
    assert!(!target.join("tree/empty").exists());
    assert_eq!(children(dir.path()), 2);
}

#[test]
fn extraction_rejects_existing_targets_without_changes() {
    use peazip_native::extract_zip_to_directory;
    let dir = tempfile::tempdir().unwrap();
    let bytes = extraction_fixture();
    let target = dir.path().join("target");
    fs::create_dir(&target).unwrap();
    assert!(
        extract_zip_to_directory(&bytes, &["other".into()], &target, Limits::default()).is_err()
    );
    assert_eq!(children(&target), 0);
    fs::write(target.join("original"), b"original").unwrap();
    assert!(
        extract_zip_to_directory(&bytes, &["other".into()], &target, Limits::default()).is_err()
    );
    assert_eq!(fs::read(target.join("original")).unwrap(), b"original");
    let file = dir.path().join("file");
    fs::write(&file, b"original").unwrap();
    assert!(extract_zip_to_directory(&bytes, &["other".into()], &file, Limits::default()).is_err());
    assert_eq!(fs::read(file).unwrap(), b"original");
    #[cfg(unix)]
    {
        for (name, destination) in [
            ("link", target.clone()),
            ("dangling", dir.path().join("missing")),
        ] {
            let link = dir.path().join(name);
            std::os::unix::fs::symlink(destination, &link).unwrap();
            assert!(
                extract_zip_to_directory(&bytes, &["other".into()], &link, Limits::default())
                    .is_err()
            );
            assert!(fs::symlink_metadata(link).unwrap().is_symlink());
        }
        assert_eq!(children(dir.path()), 4);
    }
}

#[test]
fn invalid_extraction_never_creates_output() {
    use peazip_native::extract_zip_to_directory;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("out");
    let bytes = extraction_fixture();
    for selection in [
        vec![],
        vec!["unknown".into()],
        vec!["other".into(), "unknown".into()],
    ] {
        assert!(extract_zip_to_directory(&bytes, &selection, &target, Limits::default()).is_err());
        assert_eq!(children(dir.path()), 0);
    }
    let malicious = bytes
        .windows(5)
        .enumerate()
        .filter_map(|(i, v)| (v == b"other").then_some(i))
        .collect::<Vec<_>>();
    let mut unsafe_bytes = bytes.clone();
    for offset in malicious {
        unsafe_bytes[offset..offset + 5].copy_from_slice(b"../xx");
    }
    assert!(
        extract_zip_to_directory(&unsafe_bytes, &["tree/".into()], &target, Limits::default())
            .is_err()
    );
    let mut corrupt = bytes.clone();
    // Truncation always invalidates the archive, regardless of compression choice.
    corrupt.truncate(corrupt.len() / 2);
    for input in [&corrupt[..], b"not a zip", &unsafe_bytes[..]] {
        assert!(
            extract_zip_to_directory(input, &["other".into()], &target, Limits::default()).is_err()
        );
        assert_eq!(children(dir.path()), 0);
    }
}

#[test]
fn selected_crc_failure_never_publishes_or_leaves_partial_files() {
    let parent = tempfile::tempdir().unwrap();
    let mut bytes = peazip_core::create_zip(
        &[peazip_core::MemoryEntry {
            path: "checked".into(),
            is_dir: false,
            data: b"payload".to_vec(),
        }],
        Limits::default(),
    )
    .unwrap();
    let central = bytes.windows(4).position(|b| b == b"PK\x01\x02").unwrap();
    bytes[14] ^= 1;
    bytes[central + 16] ^= 1;
    assert!(peazip_core::list_zip(&bytes, Limits::default()).is_ok());
    assert!(
        peazip_native::extract_zip_to_directory(
            &bytes,
            &["checked".into()],
            &parent.path().join("out"),
            Limits::default()
        )
        .is_err()
    );
    assert_eq!(children(parent.path()), 0);
}

#[cfg(windows)]
#[test]
fn extracts_to_long_windows_parent() {
    let dir = tempfile::tempdir().unwrap();
    let mut parent = dir.path().to_path_buf();
    while parent.as_os_str().len() < 280 {
        parent.push("long-extraction-parent-component");
    }
    fs::create_dir_all(&parent).unwrap();
    let target = parent.join("out");
    assert_eq!(
        peazip_native::extract_zip_to_directory(
            &extraction_fixture(),
            &["tree/".into()],
            &target,
            Limits::default()
        )
        .unwrap(),
        3
    );
    assert_eq!(
        fs::read(target.join("tree/nested/data")).unwrap(),
        b"payload"
    );
    assert_eq!(children(&parent), 1);
}

#[test]
fn corrupt_selected_payload_is_rejected_before_disk_writes() {
    let dir = tempfile::tempdir().unwrap();
    let mut bytes = peazip_core::create_zip(
        &[peazip_core::MemoryEntry {
            path: "file".into(),
            is_dir: false,
            data: b"selected payload".to_vec(),
        }],
        Limits::default(),
    )
    .unwrap();
    let name_len = u16::from_le_bytes([bytes[26], bytes[27]]) as usize;
    let extra_len = u16::from_le_bytes([bytes[28], bytes[29]]) as usize;
    bytes[30 + name_len + extra_len] ^= 0xff;
    assert!(peazip_core::list_zip(&bytes, Limits::default()).is_ok());
    assert!(
        peazip_native::extract_zip_to_directory(
            &bytes,
            &["file".into()],
            &dir.path().join("out"),
            Limits::default()
        )
        .is_err()
    );
    assert_eq!(children(dir.path()), 0);
}

#[test]
fn reads_raw_archive_bytes_with_size_limits() {
    use peazip_native::{Error, read_archive_bytes};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input");
    let data = b"raw bytes, not a ZIP\0\xff";
    fs::write(&path, data).unwrap();
    let limits = Limits {
        max_archive_bytes: data.len() as u64,
        ..Limits::default()
    };
    assert_eq!(read_archive_bytes(&path, limits).unwrap(), data);
    assert!(matches!(
        read_archive_bytes(
            &path,
            Limits {
                max_archive_bytes: data.len() as u64 - 1,
                ..limits
            }
        ),
        Err(Error::Limit("archive bytes"))
    ));
    fs::write(&path, []).unwrap();
    assert!(
        read_archive_bytes(
            &path,
            Limits {
                max_archive_bytes: 0,
                ..limits
            }
        )
        .unwrap()
        .is_empty()
    );
    assert!(read_archive_bytes(dir.path(), limits).is_err());
    assert!(read_archive_bytes(&dir.path().join("missing"), limits).is_err());
}

#[cfg(unix)]
#[test]
fn archive_reads_reject_links_and_fifo_without_blocking() {
    use peazip_native::{Error, read_archive_bytes};
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    fs::write(&source, b"original").unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&source, &link).unwrap();
    let dangling = dir.path().join("dangling");
    std::os::unix::fs::symlink(dir.path().join("missing"), &dangling).unwrap();
    let fifo = dir.path().join("fifo");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    for path in [&link, &dangling, &fifo] {
        assert!(matches!(
            read_archive_bytes(path, Limits::default()),
            Err(Error::UnsupportedEntry(_))
        ));
    }
    assert_eq!(fs::read(source).unwrap(), b"original");
    assert_eq!(children(dir.path()), 4);
}

#[test]
fn recursive_mixed_unicode_empty_and_deterministic_inside_output() {
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("目录");
    fs::create_dir_all(tree.join("nested/空目录")).unwrap();
    fs::write(tree.join("nested/中文.txt"), b"payload").unwrap();
    fs::write(tree.join("z"), []).unwrap();
    let single = dir.path().join("single");
    fs::write(&single, b"single").unwrap();
    let first = dir.path().join("first.zip");
    let paths = [tree.clone(), single.clone()];
    let listed = create_zip_from_paths(&paths, &first, Limits::default()).unwrap();
    assert_eq!(
        listed.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
        [
            "single",
            "目录/",
            "目录/nested/",
            "目录/nested/中文.txt",
            "目录/nested/空目录/",
            "目录/z"
        ]
    );
    let inside = tree.join("inside.zip");
    create_zip_from_paths(&[single, tree.clone()], &inside, Limits::default()).unwrap();
    assert_eq!(fs::read(first).unwrap(), fs::read(inside).unwrap());
    assert_eq!(fs::read(tree.join("nested/中文.txt")).unwrap(), b"payload");
    assert!(tree.join("nested/空目录").is_dir());
}

#[test]
fn recursive_quotas_and_duplicate_selections_leave_no_output() {
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("tree");
    fs::create_dir_all(tree.join("nested/empty")).unwrap();
    fs::write(tree.join("nested/data"), b"12345").unwrap();
    fs::write(tree.join("second"), b"67890").unwrap();
    let out = dir.path().join("out.zip");
    for limits in [
        Limits {
            max_entries: 3,
            ..Limits::default()
        },
        Limits {
            max_path_bytes: 10,
            ..Limits::default()
        },
        Limits {
            max_entry_bytes: 4,
            ..Limits::default()
        },
        Limits {
            max_total_bytes: 9,
            ..Limits::default()
        },
    ] {
        assert!(create_zip_from_paths(std::slice::from_ref(&tree), &out, limits).is_err());
        assert_eq!(children(dir.path()), 1);
    }
    assert!(matches!(
        create_zip_from_paths(&[tree.clone(), tree], &out, Limits::default()),
        Err(peazip_native::Error::DuplicatePath(_))
    ));
    assert_eq!(children(dir.path()), 1);
}

#[cfg(unix)]
#[test]
fn recursive_links_cycles_and_special_files_are_rejected() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("tree");
    fs::create_dir_all(tree.join("nested")).unwrap();
    let out = dir.path().join("out.zip");
    let bad = tree.join("nested/bad");
    for target in [
        tree.clone(),
        dir.path().join("missing"),
        dir.path().to_path_buf(),
    ] {
        symlink(target, &bad).unwrap();
        assert!(
            create_zip_from_paths(std::slice::from_ref(&tree), &out, Limits::default()).is_err()
        );
        assert_eq!(children(dir.path()), 1);
        fs::remove_file(&bad).unwrap();
    }
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&bad)
            .status()
            .unwrap()
            .success()
    );
    assert!(create_zip_from_paths(&[tree], &out, Limits::default()).is_err());
    assert_eq!(children(dir.path()), 1);
}

#[test]
fn wide_directory_does_not_open_all_files_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("tree");
    fs::create_dir(&tree).unwrap();
    for i in 0..512 {
        fs::write(tree.join(format!("file-{i:04}")), b"small").unwrap();
    }
    let out = dir.path().join("out.zip");
    let listed = create_zip_from_paths(&[tree], &out, Limits::default()).unwrap();
    assert_eq!(listed.len(), 513);
    assert_eq!(listed.iter().filter(|entry| entry.size == 5).count(), 512);
}

#[test]
fn many_selected_roots_use_bounded_handles() {
    #[cfg(unix)]
    if std::env::var_os("PEAZIP_ROOTS_FD_CHILD").is_none() {
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg("ulimit -n 64 && exec \"$1\" --exact many_selected_roots_use_bounded_handles --nocapture")
            .arg("peazip-fd-test")
            .arg(std::env::current_exe().unwrap())
            .env("PEAZIP_ROOTS_FD_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let paths: Vec<_> = (0..512)
        .map(|i| {
            let path = dir.path().join(format!("file-{i:04}"));
            fs::write(&path, b"small").unwrap();
            path
        })
        .collect();
    let out = dir.path().join("out.zip");
    let listed = create_zip_from_paths(&paths, &out, Limits::default()).unwrap();
    assert_eq!(listed.len(), 512);
    assert!(listed.iter().all(|entry| !entry.is_dir && entry.size == 5));
    assert!(paths.iter().all(|path| fs::read(path).unwrap() == b"small"));
}
