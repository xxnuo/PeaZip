use peazip_core::{MAX_EXTRACTED_ITEMS, MemoryEntry, create_zip, list_archive, test_archive};
use peazip_native::{
    Error, ExtractOptions, ExtractPaths, Limits, extract_archive_to_directory_with_options,
};
use std::{fs, path::Path};

fn node_count(root: &Path) -> usize {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            1 + if entry.file_type().unwrap().is_dir() {
                node_count(&entry.path())
            } else {
                0
            }
        })
        .sum()
}

#[test]
fn implicit_parent_budget_is_atomic_and_applies_after_selection_and_flattening() {
    const FILES: usize = 257;
    const PARENTS: usize = 63;
    let limits = Limits::default();
    assert_eq!(MAX_EXTRACTED_ITEMS, 16_384);
    assert!(FILES < limits.max_entries);
    const { assert!(FILES * (PARENTS + 1) > MAX_EXTRACTED_ITEMS) };
    // Each branch is independent; no directory entries are stored in the archive.
    let entries: Vec<_> = (0..FILES)
        .map(|i| MemoryEntry {
            path: format!("{i:03}/{}file-{i:03}.txt", "d/".repeat(PARENTS - 1)),
            is_dir: false,
            data: format!("payload-{i}").into_bytes(),
        })
        .collect();
    let bytes = create_zip(&entries, limits).unwrap();
    let snapshot = bytes.clone();
    let listed = list_archive(&bytes, limits).unwrap();
    assert_eq!(listed.len(), FILES);
    assert!(listed.iter().all(|entry| !entry.is_dir));
    let report = test_archive(&bytes, limits).unwrap();
    assert_eq!(report.entries, FILES);
    assert_eq!(report.files, FILES);
    assert_eq!(report.checksummed_files, FILES);
    let selected: Vec<_> = listed.into_iter().map(|entry| entry.path).collect();

    let parent = tempfile::tempdir().unwrap();
    let source = parent.path().join("source.zip");
    let sentinel = parent.path().join("sentinel");
    let target = parent.path().join("output");
    fs::write(&source, &bytes).unwrap();
    fs::write(&sentinel, b"original sentinel").unwrap();

    for destination in [&target, &sentinel] {
        let result = extract_archive_to_directory_with_options(
            &bytes,
            &selected,
            destination,
            limits,
            ExtractOptions::default(),
        );
        assert!(
            matches!(result, Err(Error::Limit("expanded filesystem items"))),
            "{result:?}"
        );
        assert!(!target.exists());
        assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 2);
        assert_eq!(fs::read(&sentinel).unwrap(), b"original sentinel");
        assert_eq!(fs::read(&source).unwrap(), snapshot);
        assert_eq!(bytes, snapshot);
    }

    assert_eq!(
        extract_archive_to_directory_with_options(
            &bytes,
            &selected,
            &target,
            limits,
            ExtractOptions {
                paths: ExtractPaths::FilesOnly,
            },
        )
        .unwrap(),
        FILES
    );
    assert_eq!(node_count(&target), FILES);
    for entry in &entries {
        let basename = entry.path.rsplit('/').next().unwrap();
        assert_eq!(fs::read(target.join(basename)).unwrap(), entry.data);
    }

    let partial = parent.path().join("partial");
    let subset = [entries[0].path.clone(), entries[FILES - 1].path.clone()];
    assert_eq!(
        extract_archive_to_directory_with_options(
            &bytes,
            &subset,
            &partial,
            limits,
            ExtractOptions::default(),
        )
        .unwrap(),
        subset.len()
    );
    assert_eq!(node_count(&partial), subset.len() * (PARENTS + 1));
    for entry in [&entries[0], &entries[FILES - 1]] {
        assert_eq!(fs::read(partial.join(&entry.path)).unwrap(), entry.data);
    }
    assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 4);
    assert_eq!(fs::read(sentinel).unwrap(), b"original sentinel");
    assert_eq!(fs::read(source).unwrap(), snapshot);
    assert_eq!(bytes, snapshot);
}
