use peazip_core::{
    Error, Limits, MemoryEntry, ZipCompression, ZipOptions, create_zip, create_zip_with_options,
    extract_zip,
};
use std::io::Cursor;
use zip::{CompressionMethod, ZipArchive};

fn options(compression: ZipCompression) -> ZipOptions {
    ZipOptions { compression }
}

fn file(data: &[u8]) -> MemoryEntry {
    MemoryEntry {
        path: "file".into(),
        is_dir: false,
        data: data.to_vec(),
    }
}

fn method(bytes: &[u8]) -> CompressionMethod {
    ZipArchive::new(Cursor::new(bytes))
        .unwrap()
        .by_index(0)
        .unwrap()
        .compression()
}

#[test]
fn explicit_modes_write_real_zip_methods_and_all_nine_levels_roundtrip() {
    let entries = [
        file(b"hello ZIP options"),
        MemoryEntry {
            path: "empty/".into(),
            is_dir: true,
            data: vec![],
        },
        MemoryEntry {
            path: "empty-file".into(),
            is_dir: false,
            data: vec![],
        },
    ];
    let limits = Limits::default();
    for compression in std::iter::once(ZipCompression::Stored)
        .chain((1..=9).map(|level| ZipCompression::Deflate { level }))
    {
        let bytes = create_zip_with_options(&entries, limits, options(compression)).unwrap();
        let expected = match compression {
            ZipCompression::Stored => CompressionMethod::Stored,
            _ => CompressionMethod::Deflated,
        };
        assert_eq!(method(&bytes), expected);
        let mut archive = ZipArchive::new(Cursor::new(&bytes)).unwrap();
        assert_eq!(
            archive.by_index(1).unwrap().compression(),
            CompressionMethod::Stored
        );
        assert_eq!(archive.by_index(2).unwrap().compression(), expected);
        assert_eq!(extract_zip(&bytes, limits).unwrap(), entries);
        assert!(
            ZipArchive::new(Cursor::new(
                create_zip_with_options(&[], limits, options(compression)).unwrap()
            ))
            .unwrap()
            .is_empty()
        );
    }
}

#[test]
fn requested_deflate_level_changes_the_actual_compressed_stream() {
    let entry = file(
        &b"The quick brown fox jumps over the lazy dog. Archive compression level test.\n"
            .repeat(2048),
    );
    let limits = Limits {
        max_compression_ratio: 100_000,
        ..Limits::default()
    };
    let fast = create_zip_with_options(
        std::slice::from_ref(&entry),
        limits,
        options(ZipCompression::Deflate { level: 1 }),
    )
    .unwrap();
    let best = create_zip_with_options(
        std::slice::from_ref(&entry),
        limits,
        options(ZipCompression::Deflate { level: 9 }),
    )
    .unwrap();
    assert_eq!(method(&fast), CompressionMethod::Deflated);
    assert_eq!(method(&best), CompressionMethod::Deflated);
    let size = |bytes: &[u8]| {
        ZipArchive::new(Cursor::new(bytes))
            .unwrap()
            .by_index(0)
            .unwrap()
            .compressed_size()
    };
    assert!(
        size(&best) < size(&fast),
        "level 9: {}; level 1: {}",
        size(&best),
        size(&fast)
    );
    assert_eq!(
        extract_zip(&fast, limits).unwrap(),
        std::slice::from_ref(&entry)
    );
    assert_eq!(extract_zip(&best, limits).unwrap(), [entry]);
}

#[test]
fn only_automatic_retries_stored_when_deflate_exceeds_ratio() {
    let entry = file(&vec![0; 1024 * 1024]);
    let limits = Limits {
        max_compression_ratio: 1,
        ..Limits::default()
    };
    let automatic =
        create_zip_with_options(std::slice::from_ref(&entry), limits, ZipOptions::default())
            .unwrap();
    assert_eq!(method(&automatic), CompressionMethod::Stored);
    assert_eq!(
        method(&create_zip(std::slice::from_ref(&entry), limits).unwrap()),
        CompressionMethod::Stored
    );
    for level in [1, 6, 9] {
        assert!(matches!(
            create_zip_with_options(
                std::slice::from_ref(&entry),
                limits,
                options(ZipCompression::Deflate { level })
            ),
            Err(Error::Limit("compression ratio"))
        ));
    }
    let stored = create_zip_with_options(
        std::slice::from_ref(&entry),
        limits,
        options(ZipCompression::Stored),
    )
    .unwrap();
    assert_eq!(method(&stored), CompressionMethod::Stored);
    assert_eq!(
        extract_zip(&stored, limits).unwrap(),
        std::slice::from_ref(&entry)
    );
    for compression in [
        ZipCompression::Automatic,
        ZipCompression::Stored,
        ZipCompression::Deflate { level: 6 },
    ] {
        assert!(matches!(
            create_zip_with_options(
                std::slice::from_ref(&entry),
                Limits {
                    max_compression_ratio: 0,
                    ..limits
                },
                options(compression)
            ),
            Err(Error::Limit("compression ratio"))
        ));
    }
}

#[test]
fn invalid_levels_are_rejected_even_without_files() {
    for level in [0, 10, u8::MAX] {
        let options = options(ZipCompression::Deflate { level });
        assert!(matches!(
            options.validate(),
            Err(Error::InvalidZipOptions(_))
        ));
        let error = create_zip_with_options(&[], Limits::default(), options).unwrap_err();
        assert!(matches!(error, Error::InvalidZipOptions(_)));
        assert!(error.to_string().contains("1 and 9"));
    }
}

#[test]
fn default_options_preserve_legacy_behavior_and_explicit_modes_keep_safety_limits() {
    let entry = file(b"ordinary content");
    let limits = Limits::default();
    let old = create_zip(std::slice::from_ref(&entry), limits).unwrap();
    let new = create_zip_with_options(std::slice::from_ref(&entry), limits, ZipOptions::default())
        .unwrap();
    assert_eq!(old, new);
    for compression in [ZipCompression::Stored, ZipCompression::Deflate { level: 9 }] {
        for limits in [
            Limits {
                max_entry_bytes: 1,
                ..limits
            },
            Limits {
                max_total_bytes: 1,
                ..limits
            },
            Limits {
                max_archive_bytes: 1,
                ..limits
            },
            Limits {
                max_entries: 0,
                ..limits
            },
            Limits {
                max_path_bytes: 1,
                ..limits
            },
        ] {
            assert!(
                create_zip_with_options(std::slice::from_ref(&entry), limits, options(compression))
                    .is_err()
            );
        }
        let mut unsafe_entry = entry.clone();
        unsafe_entry.path = "../escape".into();
        assert!(matches!(
            create_zip_with_options(&[unsafe_entry], limits, options(compression)),
            Err(Error::UnsafePath(_))
        ));
    }
}
