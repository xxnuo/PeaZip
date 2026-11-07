use peazip_native::{
    DirectoryLimits, DirectorySnapshot, Error, Limits, ZipCompression, ZipOptions,
    create_zip_from_paths, create_zip_from_paths_with_options,
};
use std::{fs, path::Path};

fn method(path: &Path) -> u16 {
    let bytes = fs::read(path).unwrap();
    assert_eq!(&bytes[..4], b"PK\x03\x04");
    u16::from_le_bytes(bytes[8..10].try_into().unwrap())
}

fn id(snapshot: &DirectorySnapshot) -> usize {
    snapshot
        .entries()
        .iter()
        .find(|entry| entry.name == "source")
        .unwrap()
        .id
}

#[test]
fn options_reach_path_and_snapshot_writers_with_real_methods_and_level_effects() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let data = b"The quick brown fox jumps over the lazy dog. Archive compression level test.\n"
        .repeat(2048);
    fs::write(&source, &data).unwrap();
    let snapshot = DirectorySnapshot::open(root.path(), DirectoryLimits::default()).unwrap();
    let limits = Limits {
        max_compression_ratio: 100_000,
        ..Limits::default()
    };
    let mut lengths = Vec::new();
    for (n, compression) in [
        ZipCompression::Automatic,
        ZipCompression::Stored,
        ZipCompression::Deflate { level: 1 },
        ZipCompression::Deflate { level: 9 },
    ]
    .into_iter()
    .enumerate()
    {
        let options = ZipOptions { compression };
        let paths_zip = root.path().join(format!("paths-{n}.zip"));
        let snapshot_zip = root.path().join(format!("snapshot-{n}.zip"));
        let listed = create_zip_from_paths_with_options(
            std::slice::from_ref(&source),
            &paths_zip,
            limits,
            options,
        )
        .unwrap();
        assert_eq!(
            snapshot
                .create_zip_with_options(&[id(&snapshot)], &snapshot_zip, limits, options)
                .unwrap(),
            listed
        );
        let expected = if compression == ZipCompression::Stored {
            0
        } else {
            8
        };
        assert_eq!(method(&paths_zip), expected);
        assert_eq!(method(&snapshot_zip), expected);
        let bytes = fs::read(&paths_zip).unwrap();
        assert_eq!(fs::read(&snapshot_zip).unwrap(), bytes);
        assert_eq!(
            peazip_core::extract_archive(&bytes, limits).unwrap()[0].data,
            data
        );
        assert!(
            create_zip_from_paths_with_options(
                std::slice::from_ref(&source),
                &paths_zip,
                limits,
                options
            )
            .is_err()
        );
        assert!(
            snapshot
                .create_zip_with_options(&[id(&snapshot)], &snapshot_zip, limits, options)
                .is_err()
        );
        assert_eq!(fs::read(&paths_zip).unwrap(), bytes);
        assert_eq!(fs::read(&snapshot_zip).unwrap(), bytes);
        lengths.push(bytes.len());
    }
    assert!(
        lengths[3] < lengths[2],
        "level 9: {}; level 1: {}",
        lengths[3],
        lengths[2]
    );
    let old = root.path().join("old.zip");
    create_zip_from_paths(std::slice::from_ref(&source), &old, limits).unwrap();
    assert_eq!(
        fs::read(old).unwrap(),
        fs::read(root.path().join("paths-0.zip")).unwrap()
    );
    let old_snapshot = root.path().join("old-snapshot.zip");
    snapshot
        .create_zip(&[id(&snapshot)], &old_snapshot, limits)
        .unwrap();
    assert_eq!(
        fs::read(old_snapshot).unwrap(),
        fs::read(root.path().join("snapshot-0.zip")).unwrap()
    );
    assert_eq!(fs::read(source).unwrap(), data);
}

#[test]
fn invalid_options_fail_before_reading_sources_and_never_create_output() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("source"), b"input").unwrap();
    let snapshot = DirectorySnapshot::open(root.path(), DirectoryLimits::default()).unwrap();
    let destination = root.path().join("out.zip");
    for level in [0, 10, u8::MAX] {
        let options = ZipOptions {
            compression: ZipCompression::Deflate { level },
        };
        assert!(matches!(
            create_zip_from_paths_with_options(
                &[root.path().join("missing")],
                &destination,
                Limits::default(),
                options
            ),
            Err(Error::InvalidZipOptions(_))
        ));
        assert!(matches!(
            snapshot.create_zip_with_options(
                &[usize::MAX],
                &destination,
                Limits::default(),
                options
            ),
            Err(Error::InvalidZipOptions(_))
        ));
        assert!(!destination.exists());
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn strict_deflate_ratio_failures_do_not_publish_or_leave_temporary_files() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fs::write(&source, vec![0; 1024 * 1024]).unwrap();
    let snapshot = DirectorySnapshot::open(root.path(), DirectoryLimits::default()).unwrap();
    let limits = Limits {
        max_compression_ratio: 1,
        ..Limits::default()
    };
    let destination = root.path().join("out.zip");
    for level in [1, 9] {
        let options = ZipOptions {
            compression: ZipCompression::Deflate { level },
        };
        assert!(matches!(
            create_zip_from_paths_with_options(
                std::slice::from_ref(&source),
                &destination,
                limits,
                options
            ),
            Err(Error::Limit("compression ratio"))
        ));
        assert!(matches!(
            snapshot.create_zip_with_options(&[id(&snapshot)], &destination, limits, options),
            Err(Error::Limit("compression ratio"))
        ));
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    for (n, compression) in [ZipCompression::Automatic, ZipCompression::Stored]
        .into_iter()
        .enumerate()
    {
        let destination = root.path().join(format!("stored-{n}.zip"));
        let options = ZipOptions { compression };
        snapshot
            .create_zip_with_options(&[id(&snapshot)], &destination, limits, options)
            .unwrap();
        assert_eq!(method(&destination), 0);
    }
}
