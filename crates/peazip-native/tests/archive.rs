use peazip_native::{Limits, create_zip_from_paths};
use std::{fs, path::Path};

fn children(path: &Path) -> usize {
    fs::read_dir(path).unwrap().count()
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
    assert_eq!(listed[0].path, "中文.bin");
    let extracted =
        peazip_core::extract_zip(&fs::read(target).unwrap(), Limits::default()).unwrap();
    assert_eq!(extracted[0].data, data);
    assert!(extracted[1].data.is_empty());
    assert_eq!(fs::read(source).unwrap(), data);
    assert_eq!(children(dir.path()), 3);
}

#[test]
fn invalid_and_limits_leave_no_output_or_temporary() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    fs::write(&source, b"12345").unwrap();
    let target = dir.path().join("out.zip");
    for paths in [
        vec![],
        vec![dir.path().to_path_buf()],
        vec![dir.path().join("missing")],
    ] {
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
