use peazip_core::{
    ArchiveFormat, Error, Limits, MemoryEntry, detect_format, extract_archive,
    extract_selected_archive, list_archive, list_archive_with_format,
};

fn samples() -> Vec<&'static [u8]> {
    vec![
        include_bytes!("fixtures/7z-lzma-on.7z"),
        include_bytes!("fixtures/7z-lzma-off.7z"),
        include_bytes!("fixtures/7z-lzma2-on.7z"),
        include_bytes!("fixtures/7z-lzma2-off.7z"),
        include_bytes!("fixtures/7z-copy.7z"),
        include_bytes!("fixtures/7z-copy-on.7z"),
        include_bytes!("fixtures/7z-encoded.7z"),
        include_bytes!("fixtures/7z-plain.7z"),
        include_bytes!("fixtures/7z-bcj.7z"),
        include_bytes!("fixtures/7z-delta.7z"),
        include_bytes!("fixtures/7z-bcj2.7z"),
    ]
}

fn assert_rejected(bytes: &[u8], limits: Limits) {
    assert!(list_archive(bytes, limits).is_err());
    assert!(list_archive_with_format(bytes, limits).is_err());
    assert!(extract_archive(bytes, limits).is_err());
    assert!(extract_selected_archive(bytes, &[], limits).is_err());
    assert!(extract_selected_archive(bytes, &["binary".into()], limits).is_err());
}

#[test]
fn independent_system_7z_codecs_filters_solid_and_non_solid() {
    let limits = Limits::default();
    let expected: Vec<_> = [
        ("folder/", &b""[..]),
        ("empty-dir/", b""),
        ("empty", b""),
        ("binary", &[0, 255, 128]),
        ("folder/中文.txt", "hello 7z 中文\n".as_bytes()),
    ]
    .into_iter()
    .map(|(path, data)| MemoryEntry {
        path: path.into(),
        is_dir: path.ends_with('/'),
        data: data.to_vec(),
    })
    .collect();
    for (i, bytes) in samples().into_iter().enumerate() {
        assert_eq!(detect_format(bytes).unwrap(), ArchiveFormat::SevenZip);
        let (format, entries) =
            list_archive_with_format(bytes, limits).unwrap_or_else(|e| panic!("sample {i}: {e}"));
        assert_eq!(format, ArchiveFormat::SevenZip);
        assert_eq!(entries.len(), 6);
        let output = extract_archive(bytes, limits).unwrap_or_else(|e| panic!("sample {i}: {e}"));
        assert_eq!(
            output.iter().map(|e| &e.path).collect::<Vec<_>>(),
            entries.iter().map(|e| &e.path).collect::<Vec<_>>()
        );
        for expected in &expected {
            assert!(output.contains(expected), "sample {i}: {expected:?}");
        }
        assert_eq!(
            output
                .iter()
                .find(|e| e.path == "folder/second.bin")
                .unwrap()
                .data,
            (0..=255).cycle().take(768).collect::<Vec<u8>>()
        );
        assert_eq!(
            extract_selected_archive(
                bytes,
                &["folder/".into(), "folder/中文.txt".into(), "folder/".into()],
                limits
            )
            .unwrap(),
            output
                .into_iter()
                .filter(|e| e.path.starts_with("folder/"))
                .collect::<Vec<_>>()
        );
        assert!(
            extract_selected_archive(bytes, &[], limits)
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            extract_selected_archive(bytes, &["missing".into()], limits),
            Err(Error::UnknownEntry(_))
        ));
    }
}

#[test]
fn rejects_encryption_disabled_codecs_and_links_before_selection() {
    for bytes in [
        &include_bytes!("fixtures/7z-encrypted.7z")[..],
        &include_bytes!("fixtures/7z-encrypted-header.7z")[..],
        &include_bytes!("fixtures/7z-ppmd.7z")[..],
    ] {
        let error = list_archive(bytes, Limits::default()).unwrap_err();
        assert!(matches!(error, Error::UnsupportedEntry(_)), "{error}");
        assert_rejected(bytes, Limits::default());
    }
}

fn crc(bytes: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(bytes);
    crc.sum()
}

fn refresh_start(bytes: &mut [u8]) {
    let checksum = crc(&bytes[12..32]);
    bytes[8..12].copy_from_slice(&checksum.to_le_bytes());
}

fn header_start(bytes: &[u8]) -> usize {
    32 + u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize
}

fn refresh_header(bytes: &mut [u8]) {
    let checksum = crc(&bytes[header_start(bytes)..]);
    bytes[28..32].copy_from_slice(&checksum.to_le_bytes());
    refresh_start(bytes);
}

fn number(value: u64) -> Vec<u8> {
    let mut prefix = 0u8;
    for n in 0..8 {
        if value < 1u64 << (7 + 7 * n) {
            let mut bytes = vec![prefix | (value >> (8 * n)) as u8];
            bytes.extend_from_slice(&value.to_le_bytes()[..n]);
            return bytes;
        }
        prefix |= 0x80 >> n;
    }
    [&[0xff][..], &value.to_le_bytes()].concat()
}

fn wrapped(body: &[u8], header: &[u8]) -> Vec<u8> {
    let mut bytes = b"7z\xbc\xaf\x27\x1c\0\x04".to_vec();
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&(body.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&(header.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&crc(header).to_le_bytes());
    bytes.extend_from_slice(body);
    bytes.extend_from_slice(header);
    refresh_start(&mut bytes);
    bytes
}

fn names(header: &mut Vec<u8>, paths: &[&str]) {
    let blob: Vec<_> = paths
        .iter()
        .flat_map(|p| p.encode_utf16().chain([0]).flat_map(u16::to_le_bytes))
        .collect();
    header.push(0x11);
    header.extend(number(blob.len() as u64 + 1));
    header.push(0);
    header.extend(blob);
}

fn copy_stream(body: &[u8], declared: u64) -> Vec<u8> {
    // One COPY folder, one streamed file, block CRC over the declared decoded bytes.
    let mut header = vec![1, 4, 6, 0, 1, 9];
    header.extend(number(body.len() as u64));
    header.extend([0, 7, 0x0b, 1, 0, 1, 1, 0, 0x0c]);
    header.extend(number(declared));
    header.extend([0x0a, 1]);
    header.extend(crc(&body[..body.len().min(declared as usize)]).to_le_bytes());
    header.extend([0, 0, 5, 1]);
    names(&mut header, &["x"]);
    header.extend([0, 0]);
    wrapped(body, &header)
}

fn streamless(files: &[(&str, bool)], anti: bool) -> Vec<u8> {
    let mut header = vec![1, 5];
    header.extend(number(files.len() as u64));
    for (tag, bits) in [
        (0x0e, files.iter().map(|_| true).collect::<Vec<_>>()),
        (0x0f, files.iter().map(|(_, dir)| !dir).collect()),
        (0x10, files.iter().map(|_| anti).collect()),
    ] {
        header.push(tag);
        header.extend(number(bits.len().div_ceil(8) as u64));
        for chunk in bits.chunks(8) {
            header.push(
                chunk
                    .iter()
                    .enumerate()
                    .fold(0, |b, (i, set)| b | (u8::from(*set) << (7 - i))),
            );
        }
    }
    names(
        &mut header,
        &files.iter().map(|(path, _)| *path).collect::<Vec<_>>(),
    );
    header.extend([0, 0]);
    wrapped(&[], &header)
}

fn lzma2_stream(sizes: &[u64], block_crc: Option<u32>, file_crcs: Option<&[u32]>) -> Vec<u8> {
    // A raw LZMA2 uncompressed chunk yielding "abc", followed by its end marker.
    let body = [1, 0, 2, b'a', b'b', b'c', 0];
    let mut header = vec![
        1,
        4,
        6,
        0,
        1,
        9,
        body.len() as u8,
        0,
        7,
        0x0b,
        1,
        0,
        1,
        0x21,
        0x21,
        1,
        0,
        0x0c,
    ];
    header.extend(number(sizes.iter().sum()));
    if let Some(checksum) = block_crc {
        header.extend([0x0a, 1]);
        header.extend(checksum.to_le_bytes());
    }
    header.extend([0, 8, 0x0d]);
    header.extend(number(sizes.len() as u64));
    if sizes.len() > 1 {
        header.push(9);
        for &size in &sizes[..sizes.len() - 1] {
            header.extend(number(size));
        }
    }
    if let Some(checksums) = file_crcs {
        assert_eq!(checksums.len(), sizes.len());
        assert!(sizes.len() != 1 || block_crc.is_none());
        header.extend([0x0a, 1]);
        for checksum in checksums {
            header.extend(checksum.to_le_bytes());
        }
    }
    header.extend([0, 0, 5]);
    header.extend(number(sizes.len() as u64));
    names(&mut header, &["x", "y", "z"][..sizes.len()]);
    header.extend([0, 0]);
    wrapped(&body, &header)
}

#[test]
fn lzma2_prefix_crc_cannot_hide_surplus_decoded_bytes() {
    let limits = Limits::default();
    for (block_crc, file_crc) in [
        (Some(crc(b"a")), None),
        (None, Some(crc(b"a"))),
        (None, None),
    ] {
        let checksum = file_crc.map(|value| [value]);
        let bytes = lzma2_stream(&[1], block_crc, checksum.as_ref().map(|v| &v[..]));
        assert!(list_archive(&bytes, limits).is_ok());
        assert!(matches!(
            extract_archive(&bytes, limits),
            Err(Error::InvalidSevenZip(_))
        ));
        assert!(matches!(
            extract_selected_archive(&bytes, &["x".into()], limits),
            Err(Error::InvalidSevenZip(_))
        ));
    }
    for (block_crc, file_crc) in [
        (Some(crc(b"abc")), None),
        (None, Some(crc(b"abc"))),
        (None, None),
    ] {
        let checksum = file_crc.map(|value| [value]);
        let bytes = lzma2_stream(&[3], block_crc, checksum.as_ref().map(|v| &v[..]));
        assert_eq!(extract_archive(&bytes, limits).unwrap()[0].data, b"abc");
    }
}

#[test]
fn manual_crc_checks_cover_each_file_and_the_complete_touched_block() {
    let limits = Limits::default();
    let good_files = [crc(b"a"), crc(b"bc")];
    let good_block = crc(b"abc");
    for (block_crc, file_crcs) in [
        (Some(good_block ^ 1), good_files),
        (Some(good_block), [good_files[0], good_files[1] ^ 1]),
        (None, [good_files[0] ^ 1, good_files[1]]),
    ] {
        let bytes = lzma2_stream(&[1, 2], block_crc, Some(&file_crcs));
        assert!(list_archive(&bytes, limits).is_ok());
        assert!(matches!(
            extract_archive(&bytes, limits),
            Err(Error::InvalidSevenZip(_))
        ));
        assert!(matches!(
            extract_selected_archive(&bytes, &["x".into()], limits),
            Err(Error::InvalidSevenZip(_))
        ));
    }
    let bytes = lzma2_stream(&[1, 2], Some(good_block), Some(&good_files));
    let output = extract_archive(&bytes, limits).unwrap();
    assert_eq!(output[0].data, b"a");
    assert_eq!(output[1].data, b"bc");
}

#[test]
fn empty_stream_crc_is_validated_before_any_selection() {
    let bytes = lzma2_stream(&[1, 0, 2], None, Some(&[crc(b"a"), 1, crc(b"bc")]));
    assert!(matches!(
        list_archive(&bytes, Limits::default()),
        Err(Error::InvalidSevenZip(_))
    ));
    assert_rejected(&bytes, Limits::default());
    let bytes = lzma2_stream(&[1, 0, 2], None, Some(&[crc(b"a"), 0, crc(b"bc")]));
    let output = extract_archive(&bytes, Limits::default()).unwrap();
    assert_eq!(output.len(), 3);
    assert_eq!(output[0].data, b"a");
    assert!(output[1].data.is_empty());
    assert_eq!(output[2].data, b"bc");
}

#[test]
fn metadata_paths_duplicates_anti_items_and_implicit_parents_precede_selection() {
    for path in [
        "../escape",
        "/absolute",
        "C:/drive",
        "C:drive",
        "a\\b",
        "./a",
        "a/../b",
        "a//b",
        "a/",
        "a.",
        "a ",
        "CON",
        "a/NUL.txt",
        "a?b",
        "a\nb",
        "",
    ] {
        let bytes = streamless(&[("good", false), (path, false)], false);
        assert_rejected(&bytes, Limits::default());
    }
    for files in [
        [("a", false), ("A", false)],
        [("a", false), ("a/b", false)],
        [("a/b", false), ("a", false)],
        [("a/", true), ("a", false)],
        [("folder", true), ("folder/", true)],
    ] {
        let bytes = streamless(&files, false);
        assert!(matches!(
            list_archive(&bytes, Limits::default()),
            Err(Error::DuplicatePath(_))
        ));
        assert_rejected(&bytes, Limits::default());
    }
    let bytes = streamless(&[("delete-me", false)], true);
    assert!(matches!(
        list_archive(&bytes, Limits::default()),
        Err(Error::UnsupportedEntry(_))
    ));
    assert_rejected(&bytes, Limits::default());
    let bytes = streamless(&[("a/b/file", false)], false);
    let limits = Limits {
        max_entries: 2,
        ..Limits::default()
    };
    assert!(list_archive(&bytes, limits).is_ok());
    assert!(matches!(
        extract_archive(&bytes, limits),
        Err(Error::Limit("output entry count"))
    ));
    assert!(
        extract_selected_archive(&bytes, &[], limits)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn normal_empty_archives_files_and_directories_need_no_stream_blocks() {
    let empty = wrapped(&[], &[]);
    assert!(list_archive(&empty, Limits::default()).unwrap().is_empty());
    assert!(
        extract_archive(&empty, Limits::default())
            .unwrap()
            .is_empty()
    );
    let bytes = streamless(&[("empty", false), ("directory", true)], false);
    let limits = Limits {
        max_entry_bytes: 0,
        max_total_bytes: 0,
        max_compression_ratio: 0,
        ..Limits::default()
    };
    assert_eq!(
        extract_archive(&bytes, limits).unwrap(),
        vec![
            MemoryEntry {
                path: "empty".into(),
                is_dir: false,
                data: vec![]
            },
            MemoryEntry {
                path: "directory/".into(),
                is_dir: true,
                data: vec![]
            },
        ]
    );
    assert!(matches!(
        list_archive(
            &bytes,
            Limits {
                max_path_bytes: 9,
                ..limits
            }
        ),
        Err(Error::Limit("path bytes"))
    ));
}

#[test]
fn structural_counts_and_header_sizes_are_bounded_before_allocation() {
    let mut header = vec![1, 5];
    header.extend(number(4097));
    header.resize(5000, 0);
    let bytes = wrapped(&[], &header);
    assert!(matches!(
        list_archive(&bytes, Limits::default()),
        Err(Error::Limit("entry count"))
    ));
    assert_rejected(&bytes, Limits::default());
    let limits = Limits {
        max_entries: 1,
        max_path_bytes: 1,
        ..Limits::default()
    };
    assert!(matches!(
        list_archive(&bytes, limits),
        Err(Error::Limit("7z header bytes"))
    ));

    // A tiny, valid encoded-header descriptor declares terabytes of header output.
    let mut header = vec![0x17, 6, 0, 1, 9, 1, 0, 7, 0x0b, 1, 0, 1, 1, 0, 0x0c];
    header.extend(number(1u64 << 40));
    header.extend([0, 0]);
    let bytes = wrapped(b"X", &header);
    assert!(matches!(
        list_archive(&bytes, Limits::default()),
        Err(Error::Limit(_))
    ));
    assert_rejected(&bytes, Limits::default());
}

#[test]
fn header_budget_supports_many_long_names_beyond_one_megabyte() {
    let names: Vec<_> = (0..1200)
        .map(|i| format!("{i:04}-{}", "x".repeat(900)))
        .collect();
    let files: Vec<_> = names.iter().map(|p| (p.as_str(), false)).collect();
    let bytes = streamless(&files, false);
    assert!(bytes.len() > 1024 * 1024);
    assert_eq!(
        list_archive(&bytes, Limits::default()).unwrap().len(),
        files.len()
    );
}

#[test]
fn huge_lzma2_dictionary_is_rejected_even_without_extraction() {
    let mut bytes = include_bytes!("fixtures/7z-bomb.7z").to_vec();
    let at = bytes
        .windows(4)
        .rposition(|p| p == [0x21, 0x21, 1, 16])
        .unwrap();
    bytes[at + 3] = 40; // LZMA2's 4 GiB dictionary property.
    refresh_header(&mut bytes);
    let limits = Limits {
        max_compression_ratio: 100_000,
        ..Limits::default()
    };
    assert!(matches!(
        list_archive(&bytes, limits),
        Err(Error::Limit("7z decoder memory"))
    ));
    assert_rejected(&bytes, limits);
}

#[test]
fn zero_output_stream_block_cannot_hide_packed_data() {
    let bytes = copy_stream(b"X", 0);
    assert!(matches!(
        list_archive(&bytes, Limits::default()),
        Err(Error::InvalidSevenZip(_))
    ));
    assert_rejected(&bytes, Limits::default());
}

#[test]
fn copy_actual_reads_detect_short_and_surplus_declarations() {
    for declared in [1, 4] {
        let bytes = copy_stream(b"abc", declared);
        assert!(matches!(
            list_archive(&bytes, Limits::default()),
            Err(Error::InvalidSevenZip(_))
        ));
        assert_rejected(&bytes, Limits::default());
        assert!(extract_selected_archive(&bytes, &["x".into()], Limits::default()).is_err());
    }
    assert_eq!(
        extract_archive(&copy_stream(b"abc", 3), Limits::default()).unwrap()[0].data,
        b"abc"
    );
}

#[test]
fn rejects_unix_links_special_types_reparse_and_inconsistent_directories() {
    // p7zip 17 on macOS stores symlink text with regular-file mode; inject each special
    // mode into that independently generated plain-header archive and repair envelope CRCs.
    let base = include_bytes!("fixtures/7z-symlink.7z");
    let regular = 0x81a4_8020u32.to_le_bytes();
    let at = base.windows(4).rposition(|p| p == regular).unwrap();
    for attrs in [
        0xa1ff_8020u32,
        0x21a4_8020,
        0x11a4_8020,
        0x61a4_8020,
        0xc1a4_8020,
        0x20 | 0x400,
        0x20 | 0x40,
        0x41ed_8010,
        0x10,
    ] {
        let mut bytes = base.to_vec();
        bytes[at..at + 4].copy_from_slice(&attrs.to_le_bytes());
        refresh_header(&mut bytes);
        assert!(
            matches!(
                list_archive(&bytes, Limits::default()),
                Err(Error::UnsupportedEntry(_))
            ),
            "{attrs:x}"
        );
        assert_rejected(&bytes, Limits::default());
    }
}

#[test]
fn input_limit_is_checked_before_signature_or_metadata_parsing() {
    for bytes in [
        &b"7z\xbc\xaf\x27\x1c"[..],
        &include_bytes!("fixtures/7z-encoded.7z")[..],
    ] {
        let limits = Limits {
            max_archive_bytes: 1,
            ..Limits::default()
        };
        assert!(matches!(
            list_archive_with_format(bytes, limits),
            Err(Error::Limit("archive bytes"))
        ));
        assert!(matches!(
            extract_archive(bytes, limits),
            Err(Error::Limit("archive bytes"))
        ));
        assert!(matches!(
            extract_selected_archive(bytes, &[], limits),
            Err(Error::Limit("archive bytes"))
        ));
    }
}

#[test]
fn strict_envelope_rejects_recovery_bad_crcs_ranges_trailing_split_and_sfx() {
    let limits = Limits::default();
    for base in samples() {
        for cut in 0..base.len() {
            assert_rejected(&base[..cut], limits);
        }
        for tail in [&b"\0"[..], &b"trailing garbage"[..], base] {
            assert_rejected(&[base, tail].concat(), limits);
        }
        assert_rejected(&[&b"MZ executable stub"[..], base].concat(), limits);
        for range in [8..12, 12..32, 28..32] {
            let mut bytes = base.to_vec();
            bytes[range].fill(0);
            assert_rejected(&bytes, limits);
        }
        for at in [12, 20] {
            let mut bytes = base.to_vec();
            bytes[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
            refresh_start(&mut bytes);
            assert_rejected(&bytes, limits);
        }
        let mut bytes = base.to_vec();
        let at = header_start(&bytes);
        bytes[at] ^= 1;
        assert_rejected(&bytes, limits);
        for (at, version) in [(6, 1), (7, 255)] {
            let mut bytes = base.to_vec();
            bytes[at] = version;
            assert_rejected(&bytes, limits);
        }
    }
}

#[test]
fn limits_apply_to_all_metadata_even_with_empty_selection() {
    let base = include_bytes!("fixtures/7z-encoded.7z");
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
        assert!(
            matches!(list_archive(base, limits), Err(Error::Limit(_))),
            "{limits:?}"
        );
        assert_rejected(base, limits);
    }
    let exact = Limits {
        max_archive_bytes: base.len() as u64,
        max_entries: 6,
        max_path_bytes: "folder/second.bin".len(),
        max_entry_bytes: 768,
        max_total_bytes: 787,
        ..Limits::default()
    };
    assert_eq!(extract_archive(base, exact).unwrap().len(), 6);
}

#[test]
fn compression_bomb_requires_explicit_custom_ratio() {
    let base = include_bytes!("fixtures/7z-bomb.7z");
    assert!(matches!(
        list_archive(base, Limits::default()),
        Err(Error::Limit("compression ratio"))
    ));
    assert_rejected(base, Limits::default());
    let limits = Limits {
        max_compression_ratio: 100_000,
        ..Limits::default()
    };
    assert_eq!(
        extract_archive(base, limits).unwrap()[0].data,
        vec![0; 1024 * 1024]
    );
}

#[test]
fn touched_solid_blocks_verify_unselected_files_and_return_no_partial_output() {
    let limits = Limits::default();
    let base = include_bytes!("fixtures/7z-solid-plain.7z");
    let mut bytes = base.to_vec();
    // Corrupt only the unselected final file's CRC, keeping the envelope valid.
    let checksum = crc(b"last payload").to_le_bytes();
    let at = bytes.windows(4).rposition(|p| p == checksum).unwrap();
    bytes[at] ^= 1;
    refresh_header(&mut bytes);
    assert!(list_archive(&bytes, limits).is_ok());
    assert!(extract_archive(&bytes, limits).is_err());
    assert!(extract_selected_archive(&bytes, &["a".into()], limits).is_err());
    assert!(
        extract_selected_archive(&bytes, &[], limits)
            .unwrap()
            .is_empty()
    );

    let base = include_bytes!("fixtures/7z-copy.7z");
    let mut bytes = base.to_vec();
    let listed = list_archive(base, limits).unwrap();
    let payload_size: u64 = listed.iter().map(|e| e.size).sum();
    bytes[32 + payload_size as usize - 1] ^= 1;
    let selected = listed.iter().find(|e| e.size > 0).unwrap().path.clone();
    assert!(extract_selected_archive(&bytes, &[selected], limits).is_ok());
    assert!(extract_archive(&bytes, limits).is_err());
}
