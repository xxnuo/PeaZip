use peazip_core::{
    ArchiveFormat, Error, Limits, MemoryEntry, ZipCompression, ZipOptions, create_zip_with_options,
    extract_archive, list_archive, list_archive_with_format,
};
use sevenz_turbo::{ArchiveReader, Password};
use std::io::Cursor;
use zip::ZipArchive;

#[test]
fn zip_reports_each_real_compressed_stream_including_empty_files_but_not_directories() {
    let entries = [
        MemoryEntry {
            path: "file".into(),
            is_dir: false,
            data: b"ordinary payload".repeat(32),
        },
        MemoryEntry {
            path: "empty".into(),
            is_dir: false,
            data: vec![],
        },
        MemoryEntry {
            path: "directory/".into(),
            is_dir: true,
            data: vec![],
        },
    ];
    for compression in [ZipCompression::Stored, ZipCompression::Deflate { level: 6 }] {
        let bytes =
            create_zip_with_options(&entries, Limits::default(), ZipOptions { compression })
                .unwrap();
        let listed = list_archive(&bytes, Limits::default()).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(&bytes)).unwrap();
        for (i, entry) in listed.iter().enumerate() {
            let file = archive.by_index(i).unwrap();
            assert_eq!(entry.compressed_size, file.compressed_size());
            assert_eq!(
                entry.packed_size,
                (!entry.is_dir).then_some(file.compressed_size())
            );
        }
        assert_eq!(
            listed[1].packed_size,
            Some(if compression == ZipCompression::Stored {
                0
            } else {
                2
            })
        );
        assert_eq!(listed[2].packed_size, None);
        assert_eq!(extract_archive(&bytes, Limits::default()).unwrap(), entries);
    }
}

#[test]
fn tar_packed_size_matches_actual_aligned_data_spans_excluding_headers_and_end_markers() {
    let sizes = [0usize, 1, 511, 512, 513, 1025];
    let mut bytes = Vec::new();
    let mut spans = Vec::new();
    for (i, &size) in sizes.iter().enumerate() {
        let start = bytes.len();
        let mut header = tar::Header::new_ustar();
        header.set_path(format!("file-{i}")).unwrap();
        header.set_size(size as u64);
        header.set_cksum();
        bytes.extend_from_slice(header.as_bytes());
        bytes.extend(vec![i as u8; size]);
        bytes.resize(bytes.len().div_ceil(512) * 512, 0);
        spans.push((start, bytes.len()));
    }
    let mut directory = tar::Header::new_ustar();
    directory.set_path("directory/").unwrap();
    directory.set_entry_type(tar::EntryType::Directory);
    directory.set_size(0);
    directory.set_cksum();
    bytes.extend_from_slice(directory.as_bytes());
    bytes.resize(bytes.len() + 1024, 0);
    let entries = list_archive(&bytes, Limits::default()).unwrap();
    for (i, &(header_start, next_header)) in spans.iter().enumerate() {
        assert_eq!(entries[i].size, sizes[i] as u64);
        assert_eq!(entries[i].compressed_size, sizes[i] as u64);
        assert_eq!(
            entries[i].packed_size,
            Some((next_header - header_start - 512) as u64)
        );
        assert_eq!(
            &bytes[header_start..header_start + 6],
            b"file-"
                .iter()
                .copied()
                .chain([b'0' + i as u8])
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.packed_size)
            .collect::<Vec<_>>(),
        [
            Some(0),
            Some(512),
            Some(512),
            Some(512),
            Some(1024),
            Some(1536),
            None
        ]
    );
    let payloads = extract_archive(&bytes, Limits::default()).unwrap();
    for (i, &size) in sizes.iter().enumerate() {
        assert_eq!(payloads[i].data, vec![i as u8; size]);
    }
}

#[test]
fn tar_alignment_overflow_still_rejects_the_source_without_allocation() {
    let mut header = tar::Header::new_ustar();
    header.set_path("huge").unwrap();
    header.set_size(u64::MAX);
    header.set_cksum();
    let mut bytes = header.as_bytes().to_vec();
    bytes.resize(1536, 0);
    let limits = Limits {
        max_entry_bytes: u64::MAX,
        max_total_bytes: u64::MAX,
        ..Limits::default()
    };
    assert!(matches!(
        list_archive(&bytes, limits),
        Err(Error::InvalidTar(_))
    ));
}

#[test]
fn gzip_does_not_claim_per_file_packed_sizes_and_keeps_legacy_sizes() {
    for (tar, gzip) in [
        (
            &include_bytes!("fixtures/ustar.tar")[..],
            &include_bytes!("fixtures/ustar.tar.gz")[..],
        ),
        (
            &include_bytes!("fixtures/pax.tar")[..],
            &include_bytes!("fixtures/pax.tar.gz")[..],
        ),
        (
            &include_bytes!("fixtures/gnutar.tar")[..],
            &include_bytes!("fixtures/gnutar.tar.gz")[..],
        ),
    ] {
        let raw = list_archive(tar, Limits::default()).unwrap();
        let (format, entries) = list_archive_with_format(gzip, Limits::default()).unwrap();
        assert_eq!(format, ArchiveFormat::TarGzip);
        assert_eq!(list_archive(gzip, Limits::default()).unwrap(), entries);
        for (entry, raw) in entries.iter().zip(raw) {
            assert_eq!(entry.path, raw.path);
            assert_eq!(entry.size, raw.size);
            assert_eq!(entry.compressed_size, raw.compressed_size);
            assert_eq!(entry.packed_size, None);
            assert_eq!(
                raw.packed_size,
                (!raw.is_dir).then_some(raw.size.div_ceil(512) * 512)
            );
        }
    }
}

#[test]
fn sevenzip_solid_members_never_inherit_the_first_files_whole_block_size() {
    for bytes in [
        &include_bytes!("fixtures/7z-lzma-on.7z")[..],
        &include_bytes!("fixtures/7z-lzma2-on.7z")[..],
        &include_bytes!("fixtures/7z-bcj2.7z")[..],
    ] {
        let reader = ArchiveReader::new(Cursor::new(bytes), Password::empty()).unwrap();
        assert!(reader.archive().is_solid);
        let entries = list_archive(bytes, Limits::default()).unwrap();
        let mut first_seen = false;
        for (entry, file) in entries.iter().zip(&reader.archive().files) {
            assert_eq!(entry.compressed_size, file.compressed_size);
            if file.has_stream {
                assert_eq!(entry.packed_size, None);
                if !first_seen {
                    assert!(entry.compressed_size > 0);
                    first_seen = true;
                }
            } else {
                assert_eq!(entry.packed_size, (!entry.is_dir).then_some(0));
            }
        }
        assert!(first_seen);
    }
}

#[test]
fn sevenzip_non_solid_and_mixed_blocks_report_only_individually_known_sizes() {
    for bytes in [
        &include_bytes!("fixtures/7z-lzma-off.7z")[..],
        &include_bytes!("fixtures/7z-lzma2-off.7z")[..],
        &include_bytes!("fixtures/7z-copy.7z")[..],
    ] {
        let reader = ArchiveReader::new(Cursor::new(bytes), Password::empty()).unwrap();
        assert!(!reader.archive().is_solid);
        for (entry, file) in list_archive(bytes, Limits::default())
            .unwrap()
            .iter()
            .zip(&reader.archive().files)
        {
            assert_eq!(entry.compressed_size, file.compressed_size);
            assert_eq!(
                entry.packed_size,
                (!entry.is_dir).then_some(file.compressed_size)
            );
        }
    }
    let bytes = include_bytes!("fixtures/7z-mixed-blocks.7z");
    let reader = ArchiveReader::new(Cursor::new(bytes), Password::empty()).unwrap();
    assert!(reader.archive().is_solid);
    assert_eq!(
        (0..reader.archive().blocks.len())
            .map(|i| reader.archive().block_sub_streams(i).len())
            .collect::<Vec<_>>(),
        [2, 1]
    );
    let entries = list_archive(bytes, Limits::default()).unwrap();
    let files: Vec<_> = entries.iter().filter(|entry| entry.size > 0).collect();
    assert_eq!(
        files
            .iter()
            .filter(|entry| entry.packed_size.is_none())
            .count(),
        2
    );
    assert_eq!(
        files
            .iter()
            .filter(|entry| entry.packed_size.is_some())
            .count(),
        1
    );
    assert_eq!(
        entries
            .iter()
            .find(|entry| entry.path == "empty")
            .unwrap()
            .packed_size,
        Some(0)
    );
    assert_eq!(
        entries
            .iter()
            .find(|entry| entry.is_dir)
            .unwrap()
            .packed_size,
        None
    );
}

#[test]
fn bcj2_single_stream_file_includes_all_four_packed_streams() {
    // Reproduce both new fixtures with fixtures/generate_packed.py and system 7z.
    let bytes = include_bytes!("fixtures/7z-bcj2-single.7z");
    let reader = ArchiveReader::new(Cursor::new(bytes), Password::empty()).unwrap();
    let archive = reader.archive();
    assert_eq!(archive.blocks.len(), 1);
    assert_eq!(archive.block_sub_streams(0).len(), 1);
    let streams = archive.block_pack_streams(0);
    assert_eq!(streams.len(), 4);
    let total: u64 = streams.iter().map(|stream| stream.size).sum();
    let next_header_offset = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
    assert_eq!(bytes[32 + next_header_offset as usize], 1); // Plain header (-mhc=off).
    assert_eq!(total, next_header_offset);
    assert!(total > streams[0].size);
    let entries = list_archive(bytes, Limits::default()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].compressed_size, total);
    assert_eq!(entries[0].packed_size, Some(total));
    assert_eq!(
        extract_archive(bytes, Limits::default()).unwrap()[0]
            .data
            .len(),
        32768
    );
}

#[test]
fn unknown_packed_metadata_never_relaxes_source_validation_or_resource_limits() {
    for bytes in [
        &include_bytes!("fixtures/ustar.tar")[..],
        &include_bytes!("fixtures/ustar.tar.gz")[..],
        &include_bytes!("fixtures/7z-lzma2-on.7z")[..],
        &include_bytes!("fixtures/extract-tree.zip")[..],
    ] {
        for limits in [
            Limits {
                max_archive_bytes: 1,
                ..Limits::default()
            },
            Limits {
                max_entries: 0,
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
            Limits {
                max_compression_ratio: 0,
                ..Limits::default()
            },
        ] {
            assert!(matches!(list_archive(bytes, limits), Err(Error::Limit(_))));
        }
    }
    for bytes in [
        &include_bytes!("fixtures/extract-unsafe.zip")[..],
        &include_bytes!("fixtures/extract-unsafe.tar")[..],
        &include_bytes!("fixtures/extract-unsafe.tar.gz")[..],
        &include_bytes!("fixtures/extract-unsafe.7z")[..],
    ] {
        assert!(matches!(
            list_archive(bytes, Limits::default()),
            Err(Error::UnsafePath(_))
        ));
    }
    let mut gzip = include_bytes!("fixtures/ustar.tar.gz").to_vec();
    let at = gzip.len() - 8;
    gzip[at] ^= 1;
    assert!(matches!(
        list_archive(&gzip, Limits::default()),
        Err(Error::InvalidGzip(_))
    ));
}
