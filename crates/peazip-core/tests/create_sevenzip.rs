use peazip_core::{
    ArchiveFormat, CreateOptions, Error, Limits, MemoryEntry, SevenZipCompression, create_archive,
    extract_archive, extract_selected_archive, list_archive_with_format, test_archive,
};
use sevenz_turbo::{ArchiveReader, EncoderMethod, Password};
use std::{fs, io::Cursor, process::Command};

fn file(path: &str, data: &[u8]) -> MemoryEntry {
    MemoryEntry {
        path: path.into(),
        is_dir: false,
        data: data.into(),
    }
}

fn options() -> impl Iterator<Item = CreateOptions> {
    std::iter::once(CreateOptions::SevenZip(SevenZipCompression::Stored))
        .chain((1..=9).map(|level| CreateOptions::SevenZip(SevenZipCompression::Lzma2 { level })))
}

fn entries() -> Vec<MemoryEntry> {
    vec![
        MemoryEntry {
            path: "目录/".into(),
            is_dir: true,
            data: vec![],
        },
        file("目录/中文𠀀.bin", &[0, 255, 128, 17]),
        file("empty", b""),
        file("目录/second", &b"compressible payload\n".repeat(100)),
        MemoryEntry {
            path: "empty-dir/".into(),
            is_dir: true,
            data: vec![],
        },
    ]
}

#[test]
fn all_levels_roundtrip_real_codecs_non_solid_unicode_empty_entries_and_stable_metadata() {
    let entries = entries();
    let limits = Limits::default();
    for options in options() {
        options.validate().unwrap();
        assert_eq!(options.format(), ArchiveFormat::SevenZip);
        let bytes = create_archive(&entries, limits, options).unwrap();
        assert_eq!(&bytes[..6], b"7z\xbc\xaf\x27\x1c");
        assert_eq!(create_archive(&entries, limits, options).unwrap(), bytes);
        assert_eq!(extract_archive(&bytes, limits).unwrap(), entries);
        let (format, listed) = list_archive_with_format(&bytes, limits).unwrap();
        assert_eq!(format, ArchiveFormat::SevenZip);
        assert_eq!(listed.len(), entries.len());
        assert!(listed[0].packed_size.is_none());
        assert_eq!(listed[2].packed_size, Some(0));
        let report = test_archive(&bytes, limits).unwrap();
        assert_eq!(report.files, 3);
        assert_eq!(report.checksummed_files, 2);
        assert_eq!(report.total_bytes, 2104);
        assert_eq!(
            extract_selected_archive(&bytes, &["目录/".into()], limits).unwrap(),
            vec![entries[0].clone(), entries[1].clone(), entries[3].clone()]
        );
        let reader = ArchiveReader::new(Cursor::new(&bytes), Password::empty()).unwrap();
        let archive = reader.archive();
        assert_eq!(archive.blocks.len(), 2);
        for (index, block) in archive.blocks.iter().enumerate() {
            assert_eq!(archive.block_sub_streams(index).len(), 1);
            assert_eq!(block.coders.len(), 1);
            let coder = &block.coders[0];
            let expected = match options {
                CreateOptions::SevenZip(SevenZipCompression::Stored) => EncoderMethod::ID_COPY,
                _ => EncoderMethod::ID_LZMA2,
            };
            assert_eq!(coder.encoder_method_id(), expected);
            if expected == EncoderMethod::ID_LZMA2 {
                assert_eq!(coder.properties(), [0]); // Minimum practical 4 KiB dictionary.
            }
        }
        for entry in &archive.files {
            assert!(!entry.has_creation_date);
            assert!(!entry.has_access_date);
            assert!(!entry.has_last_modified_date);
            assert!(!entry.has_windows_attributes);
            assert!(!entry.is_anti_item);
        }
        if matches!(
            options,
            CreateOptions::SevenZip(SevenZipCompression::Stored)
        ) {
            for (entry, source) in listed.iter().zip(&entries).filter(|(_, e)| !e.is_dir) {
                assert_eq!(entry.packed_size, Some(source.data.len() as u64));
            }
            let mut corrupt = bytes;
            corrupt[32] ^= 1;
            assert!(test_archive(&corrupt, limits).is_err());
            assert!(extract_archive(&corrupt, limits).is_err());
        } else {
            assert!(listed[3].packed_size.unwrap() < listed[3].size);
        }
    }
}

#[test]
fn payload_and_final_header_overruns_report_archive_budget_errors() {
    let entries = [file("payload", &(0..=255).collect::<Vec<u8>>())];
    for compression in [
        SevenZipCompression::Stored,
        SevenZipCompression::Lzma2 { level: 3 },
    ] {
        let options = CreateOptions::SevenZip(compression);
        let bytes = create_archive(&entries, Limits::default(), options).unwrap();
        for budget in [0, 32, bytes.len() as u64 - 1] {
            assert!(
                matches!(
                    create_archive(
                        &entries,
                        Limits {
                            max_archive_bytes: budget,
                            ..Limits::default()
                        },
                        options
                    ),
                    Err(Error::Limit("archive bytes"))
                ),
                "{compression:?} cap {budget}"
            );
        }
        assert_eq!(
            create_archive(
                &entries,
                Limits {
                    max_archive_bytes: bytes.len() as u64,
                    ..Limits::default()
                },
                options
            )
            .unwrap(),
            bytes
        );
    }
}

#[test]
fn invalid_levels_fail_before_input_and_empty_archives_need_no_payload_budget() {
    for level in [0, 10, 255] {
        let options = CreateOptions::SevenZip(SevenZipCompression::Lzma2 { level });
        assert!(matches!(
            options.validate(),
            Err(Error::InvalidCreateOptions(_))
        ));
        assert!(matches!(
            create_archive(&[file("../bad", b"x")], Limits::default(), options),
            Err(Error::InvalidCreateOptions(_))
        ));
    }
    let limits = Limits {
        max_entries: 0,
        max_path_bytes: 0,
        max_entry_bytes: 0,
        max_total_bytes: 0,
        max_compression_ratio: 0,
        ..Limits::default()
    };
    for options in options() {
        let bytes = create_archive(&[], limits, options).unwrap();
        assert!(extract_archive(&bytes, limits).unwrap().is_empty());
        assert_eq!(test_archive(&bytes, limits).unwrap().entries, 0);
        let empty_limits = Limits {
            max_entries: 2,
            max_path_bytes: 20,
            ..limits
        };
        let mut empty_entries = entries();
        empty_entries.retain(|e| e.path == "empty" || e.path == "empty-dir/");
        let bytes = create_archive(&empty_entries, empty_limits, options).unwrap();
        assert_eq!(
            extract_archive(&bytes, empty_limits).unwrap(),
            empty_entries
        );
    }
}

#[test]
fn compression_ratio_rejects_each_block_without_switching_to_copy() {
    // The incompressible file must not hide the other block's excessive ratio.
    let mut state = 1u32;
    let random: Vec<_> = (0..16 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect();
    let entries = [
        file("compressible", &vec![0; 8192]),
        file("random", &random),
    ];
    let limits = Limits {
        max_compression_ratio: 2,
        ..Limits::default()
    };
    for level in 1..=9 {
        assert!(matches!(
            create_archive(
                &entries,
                limits,
                CreateOptions::SevenZip(SevenZipCompression::Lzma2 { level })
            ),
            Err(Error::Limit("compression ratio"))
        ));
    }
    let bytes = create_archive(
        &entries,
        limits,
        CreateOptions::SevenZip(SevenZipCompression::Stored),
    )
    .unwrap();
    assert_eq!(extract_archive(&bytes, limits).unwrap(), entries);
}

#[test]
fn highest_level_caps_large_file_dictionary_and_keeps_the_existing_decoder_budget() {
    let entries = [file("large", &vec![0; 16 * 1024 * 1024 + 4096])];
    let limits = Limits {
        max_entry_bytes: entries[0].data.len() as u64,
        max_compression_ratio: 1_000_000,
        ..Limits::default()
    };
    let bytes = create_archive(
        &entries,
        limits,
        CreateOptions::SevenZip(SevenZipCompression::Lzma2 { level: 9 }),
    )
    .unwrap();
    let reader = ArchiveReader::new(Cursor::new(&bytes), Password::empty()).unwrap();
    assert_eq!(reader.archive().blocks[0].coders[0].properties(), [24]); // 16 MiB.
    assert_eq!(extract_archive(&bytes, limits).unwrap(), entries);
}

#[test]
fn large_encoded_header_keeps_name_and_header_budgets_with_no_file_payload() {
    let entries: Vec<_> = (0..600)
        .map(|index| file(&format!("{index:04}-{}", "x".repeat(900)), b""))
        .collect();
    let limits = Limits {
        max_entry_bytes: 0,
        max_total_bytes: 0,
        max_compression_ratio: 0,
        ..Limits::default()
    };
    for options in [
        CreateOptions::SevenZip(SevenZipCompression::Stored),
        CreateOptions::SevenZip(SevenZipCompression::Lzma2 { level: 9 }),
    ] {
        let bytes = create_archive(&entries, limits, options).unwrap();
        let next = 32 + u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize;
        assert_eq!(bytes[next], 0x17);
        assert_eq!(extract_archive(&bytes, limits).unwrap(), entries);
        assert!(matches!(
            create_archive(
                &entries,
                Limits {
                    max_path_bytes: 904,
                    ..limits
                },
                options
            ),
            Err(Error::Limit("path bytes"))
        ));
    }
}

#[test]
fn independent_local_7z_tests_and_extracts_generated_archives() {
    let tool = ["7z", "7zz"]
        .into_iter()
        .find(|tool| Command::new(tool).output().is_ok());
    let Some(tool) = tool else {
        eprintln!("local 7z unavailable; independent extraction skipped");
        return;
    };
    let root = std::env::temp_dir().join(format!("peazip-core-create-7z-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let entries = entries();
    for options in options() {
        let bytes = create_archive(&entries, Limits::default(), options).unwrap();
        let archive = root.join("generated.7z");
        fs::write(&archive, bytes).unwrap();
        let output = Command::new(tool)
            .args(["t", "-bd"])
            .arg(&archive)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let destination = root.join("extracted");
        let output = Command::new(tool)
            .args(["x", "-bd", "-y"])
            .arg(format!("-o{}", destination.display()))
            .arg(&archive)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        for entry in &entries {
            let path = destination.join(&entry.path);
            if entry.is_dir {
                assert!(path.is_dir());
            } else {
                assert_eq!(fs::read(path).unwrap(), entry.data);
            }
        }
        fs::remove_dir_all(destination).unwrap();
    }
    let archive = root.join("empty.7z");
    fs::write(
        &archive,
        create_archive(
            &[],
            Limits::default(),
            CreateOptions::SevenZip(SevenZipCompression::Stored),
        )
        .unwrap(),
    )
    .unwrap();
    let output = Command::new(tool)
        .args(["t", "-bd"])
        .arg(&archive)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    fs::remove_dir_all(root).unwrap();
}
