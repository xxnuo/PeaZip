use peazip_core::{
    Error, ExtractOptions, ExtractPaths, Limits, MemoryEntry, create_zip, extract_selected_archive,
    extract_selected_archive_with_options, list_archive,
};

const FILES_ONLY: ExtractOptions = ExtractOptions {
    paths: ExtractPaths::FilesOnly,
};

fn tree() -> [&'static [u8]; 4] {
    [
        include_bytes!("fixtures/extract-tree.zip"),
        include_bytes!("fixtures/extract-tree.tar"),
        include_bytes!("fixtures/extract-tree.tar.gz"),
        include_bytes!("fixtures/extract-tree.7z"),
    ]
}

fn collisions() -> [&'static [u8]; 4] {
    [
        include_bytes!("fixtures/extract-collisions.zip"),
        include_bytes!("fixtures/extract-collisions.tar"),
        include_bytes!("fixtures/extract-collisions.tar.gz"),
        include_bytes!("fixtures/extract-collisions.7z"),
    ]
}

fn selected(bytes: &[u8]) -> Vec<String> {
    list_archive(bytes, Limits::default())
        .unwrap()
        .into_iter()
        .map(|entry| entry.path)
        .collect()
}

#[test]
fn full_default_preserves_paths_and_files_only_moves_verified_files_without_directories() {
    for bytes in tree() {
        let paths = selected(bytes);
        let full = extract_selected_archive(bytes, &paths, Limits::default()).unwrap();
        assert_eq!(
            extract_selected_archive_with_options(
                bytes,
                &paths,
                Limits::default(),
                ExtractOptions::default()
            )
            .unwrap(),
            full
        );
        let flat =
            extract_selected_archive_with_options(bytes, &paths, Limits::default(), FILES_ONLY)
                .unwrap();
        assert_eq!(flat.len(), 4);
        assert!(
            flat.iter()
                .all(|entry| !entry.is_dir && !entry.path.contains('/'))
        );
        for (name, data) in [
            ("data.bin", &b"\x00\xff\x80"[..]),
            ("中文.txt", "flatten 中文\n".as_bytes()),
            ("empty-file", b""),
            ("outside.txt", b"outside\n"),
        ] {
            assert_eq!(
                flat.iter().find(|entry| entry.path == name).unwrap().data,
                data
            );
        }
        assert_eq!(
            flat.iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            full.iter()
                .filter(|entry| !entry.is_dir)
                .map(|entry| entry.path.rsplit('/').next().unwrap())
                .collect::<Vec<_>>()
        );
        let subtree = extract_selected_archive_with_options(
            bytes,
            &["docs/".into()],
            Limits::default(),
            FILES_ONLY,
        )
        .unwrap();
        assert_eq!(subtree.len(), 2);
        assert!(
            subtree
                .iter()
                .all(|entry| matches!(entry.path.as_str(), "data.bin" | "中文.txt"))
        );
        assert_eq!(
            extract_selected_archive_with_options(
                bytes,
                &["empty-file".into()],
                Limits::default(),
                FILES_ONLY
            )
            .unwrap(),
            vec![MemoryEntry {
                path: "empty-file".into(),
                is_dir: false,
                data: vec![]
            }]
        );
    }
}

#[test]
fn duplicate_basenames_and_case_conflicts_reject_the_whole_output() {
    for bytes in collisions() {
        let all = selected(bytes);
        assert!(extract_selected_archive(bytes, &all, Limits::default()).is_ok());
        for paths in [
            vec!["a/same.txt".into(), "b/same.txt".into()],
            vec!["a/same.txt".into(), "c/SAME.TXT".into()],
            all,
        ] {
            assert!(matches!(
                extract_selected_archive_with_options(bytes, &paths, Limits::default(), FILES_ONLY),
                Err(Error::DuplicatePath(_))
            ));
        }
        let clean = extract_selected_archive_with_options(
            bytes,
            &["clean.txt".into()],
            Limits::default(),
            FILES_ONLY,
        )
        .unwrap();
        assert_eq!(clean[0].data, b"clean");
    }
}

#[test]
fn flattening_cannot_launder_unsafe_source_paths_even_when_unselected() {
    for bytes in [
        &include_bytes!("fixtures/extract-unsafe.zip")[..],
        &include_bytes!("fixtures/extract-unsafe.tar")[..],
        &include_bytes!("fixtures/extract-unsafe.tar.gz")[..],
        &include_bytes!("fixtures/extract-unsafe.7z")[..],
    ] {
        for paths in [vec![], vec!["clean.txt".into()], vec!["../file".into()]] {
            assert!(matches!(
                extract_selected_archive_with_options(bytes, &paths, Limits::default(), FILES_ONLY),
                Err(Error::UnsafePath(_))
            ));
        }
    }
}

#[test]
fn no_files_is_an_explicit_error_after_archive_and_selection_validation() {
    for bytes in [
        &include_bytes!("fixtures/extract-directories.zip")[..],
        &include_bytes!("fixtures/extract-directories.tar")[..],
        &include_bytes!("fixtures/extract-directories.tar.gz")[..],
        &include_bytes!("fixtures/extract-directories.7z")[..],
    ] {
        let all = selected(bytes);
        assert!(
            !extract_selected_archive(bytes, &all, Limits::default())
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            extract_selected_archive_with_options(bytes, &all, Limits::default(), FILES_ONLY),
            Err(Error::NoFilesSelected)
        ));
        assert!(matches!(
            extract_selected_archive_with_options(bytes, &[], Limits::default(), FILES_ONLY),
            Err(Error::NoFilesSelected)
        ));
        assert!(
            extract_selected_archive(bytes, &[], Limits::default())
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            extract_selected_archive_with_options(
                bytes,
                &["missing".into()],
                Limits::default(),
                FILES_ONLY
            ),
            Err(Error::UnknownEntry(_))
        ));
    }
    assert!(
        extract_selected_archive_with_options(b"broken", &[], Limits::default(), FILES_ONLY)
            .is_err()
    );
}

#[test]
fn selected_crc_is_verified_before_files_only_collision_or_empty_output_errors() {
    let marker = "flatten 中文\n".as_bytes();
    for source in [
        &include_bytes!("fixtures/extract-tree.zip")[..],
        &include_bytes!("fixtures/extract-tree.7z")[..],
    ] {
        let mut bytes = source.to_vec();
        let at = bytes
            .windows(marker.len())
            .position(|bytes| bytes == marker)
            .unwrap();
        bytes[at] ^= 1;
        assert!(list_archive(&bytes, Limits::default()).is_ok());
        assert!(
            extract_selected_archive_with_options(
                &bytes,
                &["docs/中文.txt".into()],
                Limits::default(),
                FILES_ONLY
            )
            .is_err()
        );
    }
    let mut bytes = include_bytes!("fixtures/extract-collisions.zip").to_vec();
    let at = bytes.windows(3).position(|bytes| bytes == b"two").unwrap();
    bytes[at] ^= 1;
    let error = extract_selected_archive_with_options(
        &bytes,
        &["a/same.txt".into(), "b/same.txt".into()],
        Limits::default(),
        FILES_ONLY,
    )
    .unwrap_err();
    assert!(!matches!(
        error,
        Error::DuplicatePath(_) | Error::NoFilesSelected
    ));
    let mut bytes = include_bytes!("fixtures/extract-directories.zip").to_vec();
    let central = bytes
        .windows(4)
        .position(|bytes| bytes == b"PK\x01\x02")
        .unwrap();
    bytes[14..18].copy_from_slice(&1u32.to_le_bytes());
    bytes[central + 16..central + 20].copy_from_slice(&1u32.to_le_bytes());
    assert!(list_archive(&bytes, Limits::default()).is_ok());
    let error = extract_selected_archive_with_options(
        &bytes,
        &["empty/".into()],
        Limits::default(),
        FILES_ONLY,
    )
    .unwrap_err();
    assert!(!matches!(error, Error::NoFilesSelected));
    let mut bytes = include_bytes!("fixtures/extract-directories.tar.gz").to_vec();
    let at = bytes.len() - 8;
    bytes[at] ^= 1;
    assert!(matches!(
        extract_selected_archive_with_options(
            &bytes,
            &["empty/".into()],
            Limits::default(),
            FILES_ONLY
        ),
        Err(Error::InvalidGzip(_))
    ));
}

#[test]
fn original_path_and_resource_limits_still_apply_before_flattening() {
    for bytes in tree() {
        for limits in [
            Limits {
                max_archive_bytes: 1,
                ..Limits::default()
            },
            Limits {
                max_entries: 1,
                ..Limits::default()
            },
            Limits {
                max_path_bytes: 10,
                ..Limits::default()
            },
            Limits {
                max_entry_bytes: 2,
                ..Limits::default()
            },
            Limits {
                max_total_bytes: 2,
                ..Limits::default()
            },
        ] {
            assert!(matches!(
                extract_selected_archive_with_options(
                    bytes,
                    &["docs/deep/data.bin".into()],
                    limits,
                    FILES_ONLY
                ),
                Err(Error::Limit(_))
            ));
        }
    }
    let entry = MemoryEntry {
        path: "a/b/file".into(),
        is_dir: false,
        data: b"data".to_vec(),
    };
    let bytes = create_zip(&[entry], Limits::default()).unwrap();
    assert!(matches!(
        extract_selected_archive_with_options(
            &bytes,
            &["a/b/file".into()],
            Limits {
                max_entries: 2,
                ..Limits::default()
            },
            FILES_ONLY
        ),
        Err(Error::Limit("output entry count"))
    ));
}
