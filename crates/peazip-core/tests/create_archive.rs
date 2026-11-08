use peazip_core::{
    ArchiveFormat, CreateOptions, Error, Limits, MemoryEntry, SevenZipCompression, ZipOptions,
    create_archive, create_zip, extract_archive, list_archive,
};
use std::io::Read;

fn file(path: &str, data: &[u8]) -> MemoryEntry {
    MemoryEntry {
        path: path.into(),
        is_dir: false,
        data: data.into(),
    }
}

fn directory(path: &str) -> MemoryEntry {
    MemoryEntry {
        path: path.into(),
        is_dir: true,
        data: vec![],
    }
}

fn formats() -> [CreateOptions; 5] {
    [
        CreateOptions::Zip(ZipOptions::default()),
        CreateOptions::Tar,
        CreateOptions::TarGzip { level: 6 },
        CreateOptions::SevenZip(SevenZipCompression::Stored),
        CreateOptions::SevenZip(SevenZipCompression::Lzma2 { level: 5 }),
    ]
}

#[test]
fn options_validate_format_and_default_zip_compatibility() {
    assert_eq!(
        CreateOptions::default(),
        CreateOptions::Zip(ZipOptions::default())
    );
    let entry = file("default.txt", b"legacy bytes");
    assert_eq!(
        create_archive(
            std::slice::from_ref(&entry),
            Limits::default(),
            CreateOptions::default()
        )
        .unwrap(),
        create_zip(&[entry], Limits::default()).unwrap()
    );
    for (options, format) in [
        (CreateOptions::default(), ArchiveFormat::Zip),
        (CreateOptions::Tar, ArchiveFormat::Tar),
        (CreateOptions::TarGzip { level: 1 }, ArchiveFormat::TarGzip),
    ] {
        options.validate().unwrap();
        assert_eq!(options.format(), format);
    }
    for level in [0, 10, 255] {
        let options = CreateOptions::TarGzip { level };
        assert!(matches!(
            options.validate(),
            Err(Error::InvalidCreateOptions(_))
        ));
        assert!(matches!(
            create_archive(&[file("../unsafe", b"x")], Limits::default(), options),
            Err(Error::InvalidCreateOptions(_))
        ));
    }
}

#[test]
fn tar_and_gzip_roundtrip_unicode_long_paths_empty_directories_and_deterministic_metadata() {
    let long = format!("folder/{}/{}.bin", "目录".repeat(24), "长名".repeat(24));
    let entries = [
        directory("empty/"),
        directory("folder/"),
        file("folder/中文.bin", &[0, 127, 128, 255]),
        file(&long, b"long path payload"),
        file("zero", b""),
    ];
    for options in [
        CreateOptions::Tar,
        CreateOptions::TarGzip { level: 1 },
        CreateOptions::TarGzip { level: 9 },
    ] {
        let bytes = create_archive(&entries, Limits::default(), options).unwrap();
        assert_eq!(extract_archive(&bytes, Limits::default()).unwrap(), entries);
        assert_eq!(
            create_archive(&entries, Limits::default(), options).unwrap(),
            bytes
        );
        let listed = list_archive(&bytes, Limits::default()).unwrap();
        for (entry, source) in listed.iter().zip(&entries) {
            assert_eq!(entry.path, source.path);
            assert_eq!(entry.size, source.data.len() as u64);
            let packed = if source.is_dir || options.format() == ArchiveFormat::TarGzip {
                None
            } else {
                Some(source.data.len().div_ceil(512) as u64 * 512)
            };
            assert_eq!(entry.packed_size, packed);
        }
        let tar = if options.format() == ArchiveFormat::TarGzip {
            assert_eq!(&bytes[..4], &[31, 139, 8, 0]);
            assert_eq!(&bytes[4..8], &[0; 4]);
            let mut decoded = Vec::new();
            flate2::read::GzDecoder::new(bytes.as_slice())
                .read_to_end(&mut decoded)
                .unwrap();
            decoded
        } else {
            bytes
        };
        assert_eq!(tar.len() % 512, 0);
        assert!(tar[tar.len() - 1024..].iter().all(|&byte| byte == 0));
        let mut archive = tar::Archive::new(tar.as_slice());
        let mut extensions = 0;
        for raw in archive.entries().unwrap().raw(true) {
            let raw = raw.unwrap();
            let header = raw.header();
            assert_eq!(header.uid().unwrap(), 0);
            assert_eq!(header.gid().unwrap(), 0);
            assert_eq!(header.mtime().unwrap(), 0);
            assert_eq!(
                header.mode().unwrap(),
                if header.entry_type().is_dir() {
                    0o755
                } else {
                    0o644
                }
            );
            extensions += usize::from(header.entry_type().is_gnu_longname());
        }
        assert!(extensions > 0);
    }
}

#[test]
fn requested_gzip_levels_change_real_streams_and_ratio_failures_do_not_fall_back() {
    let entries = [file(
        "payload",
        &b"The quick brown fox jumps over the lazy dog. Archive compression level test.\n"
            .repeat(2048),
    )];
    let limits = Limits {
        max_compression_ratio: 100_000,
        ..Limits::default()
    };
    let fast = create_archive(&entries, limits, CreateOptions::TarGzip { level: 1 }).unwrap();
    let best = create_archive(&entries, limits, CreateOptions::TarGzip { level: 9 }).unwrap();
    assert_eq!(fast[8], 4);
    assert_eq!(best[8], 2);
    assert!(best.len() < fast.len());
    assert_eq!(extract_archive(&fast, limits).unwrap(), entries);
    assert_eq!(extract_archive(&best, limits).unwrap(), entries);
    let limits = Limits {
        max_compression_ratio: 1,
        ..Limits::default()
    };
    for level in [1, 6, 9] {
        assert!(matches!(
            create_archive(
                &[file("bomb", &vec![0; 1024 * 1024])],
                limits,
                CreateOptions::TarGzip { level }
            ),
            Err(Error::Limit("compression ratio"))
        ));
    }
    create_archive(
        &[file("plain", &vec![0; 64 * 1024])],
        limits,
        CreateOptions::Tar,
    )
    .unwrap();
}

#[test]
fn shared_input_validation_rejects_unsafe_types_duplicates_and_resource_overruns() {
    for options in formats() {
        for path in [
            "../escape",
            "/absolute",
            "a/../b",
            "a\\b",
            "a:b",
            "CON.txt",
            "line\nbreak",
            "trailing.",
        ] {
            assert!(matches!(
                create_archive(&[file(path, b"x")], Limits::default(), options),
                Err(Error::UnsafePath(_))
            ));
        }
        assert!(matches!(
            create_archive(
                &[MemoryEntry {
                    path: "dir/".into(),
                    is_dir: true,
                    data: vec![1]
                }],
                Limits::default(),
                options
            ),
            Err(Error::UnsupportedEntry(_))
        ));
        for entries in [
            vec![file("a", b"a"), file("A", b"b")],
            vec![file("a", b"a"), file("a/child", b"b")],
            vec![file("a/child", b"b"), file("a", b"a")],
        ] {
            assert!(matches!(
                create_archive(&entries, Limits::default(), options),
                Err(Error::DuplicatePath(_))
            ));
        }
        for limits in [
            Limits {
                max_entries: 0,
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
            Limits {
                max_path_bytes: 5,
                ..Limits::default()
            },
        ] {
            assert!(matches!(
                create_archive(&[file("中文", b"123")], limits, options),
                Err(Error::Limit(_))
            ));
        }
    }
}

#[test]
fn final_archive_byte_limits_apply_to_headers_padding_and_gzip_finish() {
    for options in formats() {
        let entries = [file("file", b"payload")];
        let bytes = create_archive(&entries, Limits::default(), options).unwrap();
        let exact = Limits {
            max_archive_bytes: bytes.len() as u64,
            ..Limits::default()
        };
        assert_eq!(create_archive(&entries, exact, options).unwrap(), bytes);
        assert!(
            create_archive(
                &entries,
                Limits {
                    max_archive_bytes: exact.max_archive_bytes - 1,
                    ..exact
                },
                options
            )
            .is_err()
        );
        assert!(
            create_archive(
                &entries,
                Limits {
                    max_archive_bytes: 0,
                    ..exact
                },
                options
            )
            .is_err()
        );
    }
}

#[test]
fn gnu_long_name_metadata_consumes_reader_count_and_byte_budgets() {
    let path = "x".repeat(150);
    let entries = [file(&path, b"x")];
    for options in [CreateOptions::Tar, CreateOptions::TarGzip { level: 6 }] {
        assert!(matches!(
            create_archive(
                &entries,
                Limits {
                    max_entries: 1,
                    ..Limits::default()
                },
                options
            ),
            Err(Error::Limit("entry count"))
        ));
        assert!(matches!(
            create_archive(
                &entries,
                Limits {
                    max_entry_bytes: 150,
                    ..Limits::default()
                },
                options
            ),
            Err(Error::Limit("entry bytes"))
        ));
        assert!(matches!(
            create_archive(
                &entries,
                Limits {
                    max_total_bytes: 151,
                    ..Limits::default()
                },
                options
            ),
            Err(Error::Limit("total bytes"))
        ));
        let limits = Limits {
            max_entries: 2,
            max_path_bytes: 150,
            max_entry_bytes: 151,
            max_total_bytes: 152,
            ..Limits::default()
        };
        let bytes = create_archive(&entries, limits, options).unwrap();
        assert_eq!(extract_archive(&bytes, limits).unwrap(), entries);
    }
}

#[test]
fn empty_tar_containers_need_no_payload_or_entry_budget() {
    let limits = Limits {
        max_entries: 0,
        max_path_bytes: 0,
        max_entry_bytes: 0,
        max_total_bytes: 0,
        max_compression_ratio: 0,
        ..Limits::default()
    };
    for options in [
        CreateOptions::Tar,
        CreateOptions::TarGzip { level: 1 },
        CreateOptions::TarGzip { level: 9 },
    ] {
        let bytes = create_archive(&[], limits, options).unwrap();
        assert!(list_archive(&bytes, limits).unwrap().is_empty());
        assert!(extract_archive(&bytes, limits).unwrap().is_empty());
    }
}
