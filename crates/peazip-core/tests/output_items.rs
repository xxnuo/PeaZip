use peazip_core::{
    CreateOptions, Error, ExtractOptions, ExtractPaths, Limits, MAX_EXTRACTED_ITEMS, MemoryEntry,
    create_archive, extract_archive, extract_selected_archive_with_options, extract_selected_zip,
    extract_zip, list_archive, test_archive, validate_extraction_entries,
};

fn file(path: &str) -> MemoryEntry {
    MemoryEntry {
        path: path.into(),
        is_dir: false,
        data: b"payload".to_vec(),
    }
}

fn expanded_limit(result: Result<Vec<MemoryEntry>, Error>) {
    assert!(matches!(
        result,
        Err(Error::Limit("expanded filesystem items"))
    ));
}

#[test]
fn output_budget_applies_after_path_mapping_without_disabling_archive_readers_or_creators() {
    let limits = Limits {
        max_output_items: 3,
        ..Limits::default()
    };
    let entries = [file("a/b/c/data")];
    for options in [
        CreateOptions::default(),
        CreateOptions::Tar,
        CreateOptions::TarGzip { level: 6 },
    ] {
        let bytes = create_archive(&entries, limits, options).unwrap();
        assert_eq!(list_archive(&bytes, limits).unwrap().len(), 1);
        assert_eq!(test_archive(&bytes, limits).unwrap().files, 1);
        expanded_limit(extract_archive(&bytes, limits));
        let selection = [entries[0].path.clone()];
        expanded_limit(extract_selected_archive_with_options(
            &bytes,
            &selection,
            limits,
            ExtractOptions::default(),
        ));
        let flat = extract_selected_archive_with_options(
            &bytes,
            &selection,
            limits,
            ExtractOptions {
                paths: ExtractPaths::FilesOnly,
            },
        )
        .unwrap();
        assert_eq!(flat, vec![file("data")]);
        assert_eq!(
            extract_archive(
                &bytes,
                Limits {
                    max_output_items: 4,
                    ..limits
                }
            )
            .unwrap(),
            entries
        );
        if options == CreateOptions::default() {
            expanded_limit(extract_zip(&bytes, limits));
            expanded_limit(extract_selected_zip(&bytes, &selection, limits));
        }
    }
}

#[test]
fn exact_default_boundary_counts_shared_and_explicit_parents_once_and_is_configurable() {
    let mut entries = (0..4096)
        .map(|index| file(&format!("{index}/a/b/file-{index}")))
        .collect::<Vec<_>>();
    let limits = Limits::default();
    assert_eq!(limits.max_output_items, MAX_EXTRACTED_ITEMS);
    validate_extraction_entries(&entries, limits).unwrap();
    entries[0].path = "0/extra/a/b/file-0".into();
    assert!(matches!(
        validate_extraction_entries(&entries, limits),
        Err(Error::Limit("expanded filesystem items"))
    ));
    validate_extraction_entries(
        &entries,
        Limits {
            max_output_items: MAX_EXTRACTED_ITEMS + 1,
            ..limits
        },
    )
    .unwrap();
    let shared = [
        MemoryEntry {
            path: "a/".into(),
            is_dir: true,
            data: vec![],
        },
        file("a/b/one"),
        file("a/b/two"),
    ];
    validate_extraction_entries(
        &shared,
        Limits {
            max_output_items: 4,
            ..limits
        },
    )
    .unwrap();
}

#[test]
fn selection_budget_does_not_count_unselected_branches_and_flattening_still_rejects_collisions() {
    let limits = Limits {
        max_output_items: 3,
        ..Limits::default()
    };
    let entries = [file("a/x/one"), file("b/x/two")];
    let bytes = create_archive(&entries, limits, CreateOptions::default()).unwrap();
    expanded_limit(extract_archive(&bytes, limits));
    assert_eq!(
        extract_selected_archive_with_options(
            &bytes,
            &[entries[0].path.clone()],
            limits,
            ExtractOptions::default()
        )
        .unwrap(),
        entries[..1]
    );
    let entries = [file("a/same"), file("b/same")];
    let bytes = create_archive(&entries, limits, CreateOptions::default()).unwrap();
    let selected = entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect::<Vec<_>>();
    assert!(matches!(
        extract_selected_archive_with_options(
            &bytes,
            &selected,
            limits,
            ExtractOptions {
                paths: ExtractPaths::FilesOnly
            }
        ),
        Err(Error::DuplicatePath(_))
    ));
}
