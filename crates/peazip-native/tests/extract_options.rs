use peazip_native::{
    Error, ExtractOptions, ExtractPaths, Limits, extract_archive_to_directory,
    extract_archive_to_directory_with_options,
};
use std::{fs, path::Path};

const FILES_ONLY: ExtractOptions = ExtractOptions {
    paths: ExtractPaths::FilesOnly,
};

fn tree() -> [&'static [u8]; 4] {
    [
        include_bytes!("../../peazip-core/tests/fixtures/extract-tree.zip"),
        include_bytes!("../../peazip-core/tests/fixtures/extract-tree.tar"),
        include_bytes!("../../peazip-core/tests/fixtures/extract-tree.tar.gz"),
        include_bytes!("../../peazip-core/tests/fixtures/extract-tree.7z"),
    ]
}

fn selected(bytes: &[u8]) -> Vec<String> {
    peazip_core::list_archive(bytes, Limits::default())
        .unwrap()
        .into_iter()
        .map(|entry| entry.path)
        .collect()
}

fn assert_flat(path: &Path) {
    assert_eq!(fs::read_dir(path).unwrap().count(), 4);
    assert!(
        fs::read_dir(path)
            .unwrap()
            .all(|entry| entry.unwrap().file_type().unwrap().is_file())
    );
    assert_eq!(fs::read(path.join("data.bin")).unwrap(), [0, 255, 128]);
    assert_eq!(
        fs::read(path.join("中文.txt")).unwrap(),
        "flatten 中文\n".as_bytes()
    );
    assert_eq!(fs::read(path.join("outside.txt")).unwrap(), b"outside\n");
    assert_eq!(fs::metadata(path.join("empty-file")).unwrap().len(), 0);
    assert!(!path.join("docs").exists());
    assert!(!path.join("empty").exists());
}

#[test]
fn files_only_publishes_real_root_files_and_full_remains_the_legacy_default() {
    for bytes in tree() {
        let parent = tempfile::tempdir().unwrap();
        let paths = selected(bytes);
        let target = parent.path().join("flat");
        assert_eq!(
            extract_archive_to_directory_with_options(
                bytes,
                &paths,
                &target,
                Limits::default(),
                FILES_ONLY
            )
            .unwrap(),
            4
        );
        assert_flat(&target);
        assert!(
            extract_archive_to_directory_with_options(
                bytes,
                &paths,
                &target,
                Limits::default(),
                FILES_ONLY
            )
            .is_err()
        );
        assert_flat(&target);
        let old = parent.path().join("old");
        let full = parent.path().join("full");
        let count = extract_archive_to_directory(bytes, &paths, &old, Limits::default()).unwrap();
        assert_eq!(
            extract_archive_to_directory_with_options(
                bytes,
                &paths,
                &full,
                Limits::default(),
                ExtractOptions::default()
            )
            .unwrap(),
            count
        );
        for root in [&old, &full] {
            assert_eq!(
                fs::read(root.join("docs/deep/data.bin")).unwrap(),
                [0, 255, 128]
            );
            assert!(root.join("docs/empty").is_dir());
            assert!(root.join("docs/deep").is_dir());
            assert!(!root.join("data.bin").exists());
        }
        let empty_file = parent.path().join("empty-file-output");
        assert_eq!(
            extract_archive_to_directory_with_options(
                bytes,
                &["empty-file".into()],
                &empty_file,
                Limits::default(),
                FILES_ONLY
            )
            .unwrap(),
            1
        );
        assert!(empty_file.join("empty-file").is_file());
        assert_eq!(
            fs::metadata(empty_file.join("empty-file")).unwrap().len(),
            0
        );
    }
}

#[test]
fn collision_failure_is_atomic_and_full_paths_retry_to_the_same_destination_succeeds() {
    for bytes in [
        &include_bytes!("../../peazip-core/tests/fixtures/extract-collisions.zip")[..],
        &include_bytes!("../../peazip-core/tests/fixtures/extract-collisions.tar")[..],
        &include_bytes!("../../peazip-core/tests/fixtures/extract-collisions.tar.gz")[..],
        &include_bytes!("../../peazip-core/tests/fixtures/extract-collisions.7z")[..],
    ] {
        let parent = tempfile::tempdir().unwrap();
        let destination = parent.path().join("output");
        let paths = selected(bytes);
        assert!(matches!(
            extract_archive_to_directory_with_options(
                bytes,
                &paths,
                &destination,
                Limits::default(),
                FILES_ONLY
            ),
            Err(Error::DuplicatePath(_))
        ));
        assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
        extract_archive_to_directory_with_options(
            bytes,
            &paths,
            &destination,
            Limits::default(),
            ExtractOptions::default(),
        )
        .unwrap();
        assert_eq!(fs::read(destination.join("a/same.txt")).unwrap(), b"one");
        assert_eq!(fs::read(destination.join("b/same.txt")).unwrap(), b"two");
        assert_eq!(fs::read(destination.join("c/SAME.TXT")).unwrap(), b"three");
        assert!(!destination.join("same.txt").exists());
    }
}

#[test]
fn directories_only_and_unsafe_sources_never_create_or_publish_any_output() {
    let parent = tempfile::tempdir().unwrap();
    let destination = parent.path().join("output");
    for bytes in [
        &include_bytes!("../../peazip-core/tests/fixtures/extract-directories.zip")[..],
        &include_bytes!("../../peazip-core/tests/fixtures/extract-directories.tar")[..],
        &include_bytes!("../../peazip-core/tests/fixtures/extract-directories.tar.gz")[..],
        &include_bytes!("../../peazip-core/tests/fixtures/extract-directories.7z")[..],
    ] {
        for paths in [selected(bytes), vec![]] {
            assert!(matches!(
                extract_archive_to_directory_with_options(
                    bytes,
                    &paths,
                    &destination,
                    Limits::default(),
                    FILES_ONLY
                ),
                Err(Error::NoFilesSelected)
            ));
            assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
        }
    }
    for bytes in [
        &include_bytes!("../../peazip-core/tests/fixtures/extract-unsafe.zip")[..],
        &include_bytes!("../../peazip-core/tests/fixtures/extract-unsafe.tar")[..],
        &include_bytes!("../../peazip-core/tests/fixtures/extract-unsafe.tar.gz")[..],
        &include_bytes!("../../peazip-core/tests/fixtures/extract-unsafe.7z")[..],
    ] {
        assert!(matches!(
            extract_archive_to_directory_with_options(
                bytes,
                &["clean.txt".into()],
                &destination,
                Limits::default(),
                FILES_ONLY
            ),
            Err(Error::UnsafePath(_))
        ));
        assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
    }
}

#[test]
fn bad_selected_payload_or_header_and_resource_failures_leave_no_partial_files() {
    let marker = "flatten 中文\n".as_bytes();
    for original in tree() {
        let mut bytes = original.to_vec();
        if bytes.starts_with(b"PK") || bytes.starts_with(b"7z") {
            let at = bytes
                .windows(marker.len())
                .position(|bytes| bytes == marker)
                .unwrap();
            bytes[at] ^= 1;
        } else if bytes.starts_with(&[31, 139]) {
            let at = bytes.len() - 8;
            bytes[at] ^= 1;
        } else {
            bytes[0] ^= 1;
        }
        let parent = tempfile::tempdir().unwrap();
        let destination = parent.path().join("output");
        assert!(
            extract_archive_to_directory_with_options(
                &bytes,
                &["docs/中文.txt".into()],
                &destination,
                Limits::default(),
                FILES_ONLY
            )
            .is_err()
        );
        assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
        assert!(
            extract_archive_to_directory_with_options(
                original,
                &selected(original),
                &destination,
                Limits {
                    max_total_bytes: 1,
                    ..Limits::default()
                },
                FILES_ONLY
            )
            .is_err()
        );
        assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
        fs::write(&destination, b"existing winner").unwrap();
        assert!(
            extract_archive_to_directory_with_options(
                original,
                &selected(original),
                &destination,
                Limits::default(),
                FILES_ONLY
            )
            .is_err()
        );
        assert_eq!(fs::read(&destination).unwrap(), b"existing winner");
        assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 1);
    }
}
