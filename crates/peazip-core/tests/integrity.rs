use peazip_core::{
    Error, Limits, MemoryEntry, TestReport, create_zip, extract_archive, extract_selected_archive,
    list_archive, test_archive,
};
use std::io::{Cursor, Write};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

fn expected(bytes: &[u8], checksummed_files: usize) -> TestReport {
    let entries = extract_archive(bytes, Limits::default()).unwrap();
    TestReport {
        entries: entries.len(),
        files: entries.iter().filter(|entry| !entry.is_dir).count(),
        total_bytes: entries.iter().map(|entry| entry.data.len() as u64).sum(),
        checksummed_files,
    }
}

#[test]
fn reports_actual_file_bytes_and_honest_crc_coverage_across_all_fixtures() {
    for (bytes, coverage) in [
        (&include_bytes!("fixtures/ustar.tar")[..], 0),
        (&include_bytes!("fixtures/pax.tar")[..], 0),
        (&include_bytes!("fixtures/gnutar.tar")[..], 0),
        (&include_bytes!("fixtures/ustar.tar.gz")[..], 3),
        (&include_bytes!("fixtures/pax.tar.gz")[..], 4),
        (&include_bytes!("fixtures/gnutar.tar.gz")[..], 4),
        (&include_bytes!("fixtures/header.tar.gz")[..], 3),
        (&include_bytes!("fixtures/7z-lzma-on.7z")[..], 3),
        (&include_bytes!("fixtures/7z-lzma-off.7z")[..], 3),
        (&include_bytes!("fixtures/7z-lzma2-on.7z")[..], 3),
        (&include_bytes!("fixtures/7z-lzma2-off.7z")[..], 3),
        (&include_bytes!("fixtures/7z-copy.7z")[..], 3),
        (&include_bytes!("fixtures/7z-copy-on.7z")[..], 3),
        (&include_bytes!("fixtures/7z-bcj.7z")[..], 3),
        (&include_bytes!("fixtures/7z-bcj2.7z")[..], 3),
        (&include_bytes!("fixtures/7z-delta.7z")[..], 3),
    ] {
        assert_eq!(
            test_archive(bytes, Limits::default()).unwrap(),
            expected(bytes, coverage)
        );
    }
    let input = [
        MemoryEntry {
            path: "folder/".into(),
            is_dir: true,
            data: vec![],
        },
        MemoryEntry {
            path: "folder/file".into(),
            is_dir: false,
            data: b"hello".to_vec(),
        },
        MemoryEntry {
            path: "empty".into(),
            is_dir: false,
            data: vec![],
        },
    ];
    let bytes = create_zip(&input, Limits::default()).unwrap();
    assert_eq!(
        test_archive(&bytes, Limits::default()).unwrap(),
        TestReport {
            entries: 3,
            files: 2,
            total_bytes: 5,
            checksummed_files: 2,
        }
    );
}

#[test]
fn all_empty_archive_formats_report_zeroes() {
    let mut sevenzip = b"7z\xbc\xaf\x27\x1c\0\x04".to_vec();
    let mut crc = flate2::Crc::new();
    crc.update(&[0; 20]);
    sevenzip.extend(crc.sum().to_le_bytes());
    sevenzip.extend([0; 20]);
    for bytes in [
        create_zip(&[], Limits::default()).unwrap(),
        vec![0; 1024],
        include_bytes!("fixtures/empty.tar.gz").to_vec(),
        sevenzip,
    ] {
        assert_eq!(
            test_archive(&bytes, Limits::default()).unwrap(),
            TestReport {
                entries: 0,
                files: 0,
                total_bytes: 0,
                checksummed_files: 0,
            }
        );
    }
}

#[test]
fn zip_test_reads_corrupt_files_that_selected_extraction_skips() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (path, data) in [("good", b"good payload"), ("bad", b"bad! payload")] {
        writer
            .start_file(
                path,
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(data).unwrap();
    }
    let mut bytes = writer.finish().unwrap().into_inner();
    let at = bytes
        .windows(12)
        .position(|p| p == b"bad! payload")
        .unwrap();
    bytes[at] ^= 1;
    assert!(list_archive(&bytes, Limits::default()).is_ok());
    assert!(extract_selected_archive(&bytes, &["good".into()], Limits::default()).is_ok());
    assert!(test_archive(&bytes, Limits::default()).is_err());
}

#[test]
fn sevenzip_test_reads_corrupt_independent_blocks_that_selection_skips() {
    let mut bytes = include_bytes!("fixtures/7z-copy.7z").to_vec();
    let entries = list_archive(&bytes, Limits::default()).unwrap();
    let first = entries.iter().find(|e| e.size != 0).unwrap().path.clone();
    let size: u64 = entries.iter().map(|e| e.size).sum();
    bytes[32 + size as usize - 1] ^= 1;
    assert!(list_archive(&bytes, Limits::default()).is_ok());
    assert!(extract_selected_archive(&bytes, &[first], Limits::default()).is_ok());
    assert!(matches!(
        test_archive(&bytes, Limits::default()),
        Err(Error::InvalidSevenZip(_))
    ));
}

#[test]
fn tar_test_does_not_claim_a_payload_checksum_that_does_not_exist() {
    let mut bytes = include_bytes!("fixtures/ustar.tar").to_vec();
    let payload = "hello TAR 中文\n".as_bytes();
    let at = bytes
        .windows(payload.len())
        .position(|p| p == payload)
        .unwrap();
    bytes[at] ^= 1;
    let report = test_archive(&bytes, Limits::default()).unwrap();
    assert_eq!(report.files, 3);
    assert_eq!(report.checksummed_files, 0);
    let mut packed = include_bytes!("fixtures/ustar.tar.gz").to_vec();
    let at = packed.len() - 8;
    packed[at] ^= 1;
    assert!(matches!(
        test_archive(&packed, Limits::default()),
        Err(Error::InvalidGzip(_))
    ));
}

#[test]
fn integrity_testing_enforces_input_and_all_metadata_budgets() {
    for bytes in [
        &include_bytes!("fixtures/ustar.tar")[..],
        &include_bytes!("fixtures/ustar.tar.gz")[..],
    ] {
        for limits in [
            Limits {
                max_entries: 1,
                ..Limits::default()
            },
            Limits {
                max_entry_bytes: 0,
                ..Limits::default()
            },
            Limits {
                max_total_bytes: 0,
                ..Limits::default()
            },
        ] {
            assert!(matches!(test_archive(bytes, limits), Err(Error::Limit(_))));
        }
    }
    for bytes in [
        &b"not an archive"[..],
        &b"PK\x03\x04"[..],
        &include_bytes!("fixtures/7z-encoded.7z")[..],
    ] {
        assert!(matches!(
            test_archive(
                bytes,
                Limits {
                    max_archive_bytes: 1,
                    ..Limits::default()
                }
            ),
            Err(Error::Limit("archive bytes"))
        ));
    }
    let sevenzip = include_bytes!("fixtures/7z-encoded.7z");
    for limits in [
        Limits {
            max_entries: 5,
            ..Limits::default()
        },
        Limits {
            max_path_bytes: 10,
            ..Limits::default()
        },
        Limits {
            max_entry_bytes: 767,
            ..Limits::default()
        },
        Limits {
            max_total_bytes: 786,
            ..Limits::default()
        },
        Limits {
            max_compression_ratio: 0,
            ..Limits::default()
        },
    ] {
        assert!(matches!(
            test_archive(sevenzip, limits),
            Err(Error::Limit(_))
        ));
    }
    let input = [MemoryEntry {
        path: "file".into(),
        is_dir: false,
        data: b"12345".to_vec(),
    }];
    let bytes = create_zip(&input, Limits::default()).unwrap();
    for limits in [
        Limits {
            max_entries: 0,
            ..Limits::default()
        },
        Limits {
            max_path_bytes: 3,
            ..Limits::default()
        },
        Limits {
            max_entry_bytes: 4,
            ..Limits::default()
        },
        Limits {
            max_total_bytes: 4,
            ..Limits::default()
        },
        Limits {
            max_compression_ratio: 0,
            ..Limits::default()
        },
    ] {
        assert!(matches!(test_archive(&bytes, limits), Err(Error::Limit(_))));
    }
}

#[test]
fn zip_empty_files_and_directories_still_require_valid_crc() {
    for directory in [false, true] {
        let entry = MemoryEntry {
            path: if directory { "dir/" } else { "empty" }.into(),
            is_dir: directory,
            data: vec![],
        };
        let mut bytes = create_zip(&[entry], Limits::default()).unwrap();
        let at = bytes.windows(4).position(|p| p == b"PK\x01\x02").unwrap();
        bytes[14..18].copy_from_slice(&1u32.to_le_bytes());
        bytes[at + 16..at + 20].copy_from_slice(&1u32.to_le_bytes());
        assert!(list_archive(&bytes, Limits::default()).is_ok());
        assert!(test_archive(&bytes, Limits::default()).is_err());
    }
}
