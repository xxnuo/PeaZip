use flate2::{Compression, Crc, write::GzEncoder};
use peazip_core::{
    ArchiveFormat, Error, Limits, MemoryEntry, create_zip, detect_format, extract_archive,
    extract_selected_archive, list_archive, list_archive_with_format,
};
use std::io::Write;

const DEFAULT: Limits = Limits {
    max_archive_bytes: 32 * 1024 * 1024,
    max_entries: 4096,
    max_path_bytes: 1024,
    max_entry_bytes: 16 * 1024 * 1024,
    max_total_bytes: 64 * 1024 * 1024,
    max_compression_ratio: 200,
};

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn record(path: &str, data: &[u8]) -> Vec<u8> {
    let mut header = tar::Header::new_ustar();
    header.set_path(path).unwrap();
    header.set_size(data.len() as u64);
    header.set_cksum();
    let mut bytes = header.as_bytes().to_vec();
    bytes.extend_from_slice(data);
    bytes.resize(bytes.len().div_ceil(512) * 512, 0);
    bytes
}

fn archive(records: &[Vec<u8>]) -> Vec<u8> {
    [records.concat(), vec![0; 1024]].concat()
}

fn assert_rejected(bytes: &[u8], limits: Limits) {
    assert!(list_archive(bytes, limits).is_err());
    assert!(list_archive_with_format(bytes, limits).is_err());
    assert!(extract_archive(bytes, limits).is_err());
    for selected in [vec![], vec!["file".into()], vec!["missing".into()]] {
        assert!(extract_selected_archive(bytes, &selected, limits).is_err());
    }
}

fn assert_invalid(bytes: &[u8]) {
    let error = list_archive(bytes, DEFAULT).unwrap_err();
    assert!(matches!(error, Error::InvalidGzip(_)), "{error:?}");
    assert!(error.to_string().len() < 200);
    assert!(std::error::Error::source(&error).is_some());
    for selected in [vec![], vec!["file".into()], vec!["missing".into()]] {
        assert!(matches!(
            extract_selected_archive(bytes, &selected, DEFAULT),
            Err(Error::InvalidGzip(_))
        ));
    }
    assert_rejected(bytes, DEFAULT);
    assert!(matches!(detect_format(bytes), Err(Error::InvalidGzip(_))));
}

#[test]
fn independent_system_gzip_and_bsdtar_samples() {
    // gzip -n -c over the existing bsdtar fixtures; no flate2 encoder in these samples.
    for (raw, packed) in [
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
        (
            &include_bytes!("fixtures/ustar.tar")[..],
            &include_bytes!("fixtures/header.tar.gz")[..],
        ),
    ] {
        assert_eq!(detect_format(packed).unwrap(), ArchiveFormat::TarGzip);
        assert_eq!(
            list_archive(packed, DEFAULT).unwrap(),
            list_archive(raw, DEFAULT).unwrap()
        );
        assert_eq!(
            extract_archive(packed, DEFAULT).unwrap(),
            extract_archive(raw, DEFAULT).unwrap()
        );
        assert_eq!(
            extract_selected_archive(packed, &["folder/".into()], DEFAULT).unwrap(),
            extract_selected_archive(raw, &["folder/".into()], DEFAULT).unwrap()
        );
    }
}

#[test]
fn validated_listing_returns_format_without_default_limit_redetection() {
    let raw = archive(&[record("file", b"payload")]);
    for (bytes, expected) in [
        (create_zip(&[], DEFAULT).unwrap(), ArchiveFormat::Zip),
        (raw.clone(), ArchiveFormat::Tar),
        (gzip(&raw), ArchiveFormat::TarGzip),
    ] {
        let (format, entries) = list_archive_with_format(&bytes, DEFAULT).unwrap();
        assert_eq!(format, expected);
        assert_eq!(entries, list_archive(&bytes, DEFAULT).unwrap());
    }
    let packed = gzip(&archive(&[record("file", &vec![0; 1024 * 1024])]));
    assert!(detect_format(&packed).is_err());
    let limits = Limits {
        max_compression_ratio: 100_000,
        ..DEFAULT
    };
    let (format, entries) = list_archive_with_format(&packed, limits).unwrap();
    assert_eq!(format, ArchiveFormat::TarGzip);
    assert_eq!(entries[0].size, 1024 * 1024);
}

#[test]
fn gzip_must_contain_a_complete_tar_even_for_detection() {
    for raw in [
        b"ordinary gzip data".to_vec(),
        vec![],
        vec![0; 512],
        vec![0; 1025],
        record("file", b"no end marker"),
        create_zip(&[], DEFAULT).unwrap(),
    ] {
        let packed = gzip(&raw);
        assert!(detect_format(&packed).is_err());
        assert_rejected(&packed, DEFAULT);
    }
}

#[test]
fn multiple_members_join_across_every_tar_byte_boundary() {
    let raw = archive(&[record("file", &[0xa5; 600])]);
    let expected = extract_archive(&raw, DEFAULT).unwrap();
    for split in 0..=raw.len() {
        let packed = [gzip(&raw[..split]), gzip(&raw[split..])].concat();
        assert_eq!(
            extract_archive(&packed, DEFAULT).unwrap(),
            expected,
            "split {split}"
        );
    }
    let mut packed = gzip(b"");
    // More than two members, including empty members between pieces of a TAR header.
    for chunk in raw.chunks(317) {
        packed.extend(gzip(chunk));
        packed.extend(gzip(b""));
    }
    assert_eq!(detect_format(&packed).unwrap(), ArchiveFormat::TarGzip);
    assert_eq!(
        extract_selected_archive(&packed, &["file".into()], DEFAULT).unwrap(),
        expected
    );
    assert!(
        extract_selected_archive(&packed, &[], DEFAULT)
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        extract_selected_archive(&packed, &["missing".into()], DEFAULT),
        Err(Error::UnknownEntry(_))
    ));
}

#[test]
fn standard_empty_tar_padding_is_allowed_with_zero_payload_budget() {
    let fixture = include_bytes!("fixtures/empty.tar.gz");
    assert_eq!(fixture.len(), 45);
    let limits = Limits {
        max_entries: 0,
        max_entry_bytes: 0,
        max_total_bytes: 0,
        ..DEFAULT
    };
    for packed in [fixture.to_vec(), gzip(&vec![0; 1024]), gzip(&vec![0; 1536])] {
        assert_eq!(detect_format(&packed).unwrap(), ArchiveFormat::TarGzip);
        assert!(list_archive(&packed, DEFAULT).unwrap().is_empty());
        assert!(extract_archive(&packed, limits).unwrap().is_empty());
    }
}

#[test]
fn validates_crc_isize_and_deflate_for_first_middle_and_last_members() {
    let raw = archive(&[record("file", b"payload")]);
    let members = [gzip(&raw), gzip(b""), gzip(b"")];
    for member in 0..members.len() {
        for trailer_offset in [8, 4] {
            let mut bad = members.clone();
            let at = bad[member].len() - trailer_offset;
            bad[member][at] ^= 1;
            assert_invalid(&bad.concat());
        }
        let mut bad = members.clone();
        bad[member][10] = 7; // Reserved DEFLATE block type.
        assert_invalid(&bad.concat());
    }
    let mut packed = gzip(&raw);
    let at = packed.len() - 4;
    packed[at..].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_invalid(&packed);
}

#[test]
fn rejects_all_truncations_and_outer_trailing_garbage_without_partial_results() {
    let raw = archive(&[record("file", b"payload")]);
    let member = gzip(&raw);
    for cut in 2..member.len() {
        assert_invalid(&member[..cut]);
    }
    let last = gzip(b"");
    for cut in 1..last.len() {
        assert_invalid(&[member.as_slice(), &last[..cut]].concat());
    }
    for garbage in [
        vec![0],
        vec![0; 1024],
        b"junk".to_vec(),
        vec![31, 139],
        create_zip(&[], DEFAULT).unwrap(),
    ] {
        assert_invalid(&[member.as_slice(), &garbage].concat());
    }
}

#[test]
fn all_optional_header_flag_combinations_and_fhcrc_are_checked() {
    let raw = archive(&[record("file", b"payload")]);
    let base = gzip(&raw);
    for flags in 0..32 {
        let mut header = base[..10].to_vec();
        header[3] = flags;
        if flags & 4 != 0 {
            header.extend_from_slice(&[3, 0, 255, 0, 42]);
        }
        if flags & 8 != 0 {
            header.extend_from_slice(b"../ignored\xff\0");
        }
        if flags & 16 != 0 {
            header.extend_from_slice(b"ignored comment\xff\0");
        }
        if flags & 2 != 0 {
            let mut crc = Crc::new();
            crc.update(&header);
            header.extend_from_slice(&(crc.sum() as u16).to_le_bytes());
        }
        let packed = [header.as_slice(), &base[10..]].concat();
        assert_eq!(
            extract_archive(&packed, DEFAULT).unwrap()[0].data,
            b"payload"
        );
        if flags & 2 != 0 {
            let mut bad = packed.clone();
            bad[header.len() - 1] ^= 1;
            assert_invalid(&bad);
            // FHCRC of a late member must be verified even after a complete TAR.
            assert_invalid(&[base.as_slice(), &bad].concat());
        }
    }
    for (at, value) in [(2, 0), (2, 9), (3, 32), (3, 64), (3, 128), (3, 255)] {
        let mut bad = base.clone();
        bad[at] = value;
        assert_invalid(&bad);
    }
}

#[test]
fn variable_headers_are_bounded_without_echoing_metadata() {
    let raw = archive(&[record("file", b"payload")]);
    let base = gzip(&raw);
    for flag in [8, 16] {
        let mut packed = base[..10].to_vec();
        packed[3] = flag;
        packed.extend(vec![b'x'; 128 * 1024]);
        packed.push(0);
        packed.extend_from_slice(&base[10..]);
        let error = list_archive(&packed, DEFAULT).unwrap_err();
        assert!(matches!(error, Error::Limit("gzip header bytes")));
        assert!(error.to_string().len() < 100);
        assert_rejected(&packed, DEFAULT);
        assert!(matches!(
            detect_format(&packed),
            Err(Error::Limit("gzip header bytes"))
        ));
    }
    let mut packed = base[..10].to_vec();
    packed[3] = 4;
    packed.extend_from_slice(&u16::MAX.to_le_bytes());
    packed.extend(vec![b'x'; u16::MAX as usize]);
    packed.extend_from_slice(&base[10..]);
    assert!(matches!(
        list_archive(&packed, DEFAULT),
        Err(Error::Limit("gzip header bytes"))
    ));
    assert_rejected(&packed, DEFAULT);
    // Truncated optional fields, including an unterminated string and missing FHCRC.
    for (flag, tail) in [
        (4, &b"\x03"[..]),
        (4, b"\x03\x00x"),
        (8, b"name"),
        (16, b"comment"),
        (2, b"x"),
    ] {
        let mut packed = base[..10].to_vec();
        packed[3] = flag;
        packed.extend_from_slice(tail);
        assert_invalid(&packed);
    }
}

#[test]
fn member_and_aggregate_header_budgets_are_finite() {
    let raw = archive(&[record("file", b"payload")]);
    let first = gzip(&raw);
    let empty = gzip(b"");
    let mut packed = first.clone();
    for _ in 1..4096 {
        packed.extend_from_slice(&empty);
    }
    assert!(list_archive(&packed, DEFAULT).is_ok());
    packed.extend_from_slice(&empty);
    assert!(matches!(
        list_archive(&packed, DEFAULT),
        Err(Error::Limit("gzip member count"))
    ));
    assert_rejected(&packed, DEFAULT);

    let mut large_header = empty[..10].to_vec();
    large_header[3] = 16;
    large_header.extend(vec![b'x'; 60 * 1024]);
    large_header.push(0);
    large_header.extend_from_slice(&empty[10..]);
    let mut packed = first;
    for _ in 0..17 {
        packed.extend_from_slice(&large_header);
    }
    assert!(list_archive(&packed, DEFAULT).is_ok());
    packed.extend_from_slice(&large_header);
    assert!(matches!(
        list_archive(&packed, DEFAULT),
        Err(Error::Limit("gzip header bytes"))
    ));
    assert_rejected(&packed, DEFAULT);
}

#[test]
fn input_limit_precedes_header_decode_and_selection_checks() {
    for bytes in [
        &b"\x1f\x8b"[..],
        &include_bytes!("fixtures/header.tar.gz")[..],
    ] {
        let limits = Limits {
            max_archive_bytes: 1,
            ..DEFAULT
        };
        assert!(matches!(
            list_archive_with_format(bytes, limits),
            Err(Error::Limit("archive bytes"))
        ));
        assert!(matches!(
            list_archive(bytes, limits),
            Err(Error::Limit("archive bytes"))
        ));
        assert!(matches!(
            extract_archive(bytes, limits),
            Err(Error::Limit("archive bytes"))
        ));
        assert!(matches!(
            extract_selected_archive(bytes, &["missing".into()], limits),
            Err(Error::Limit("archive bytes"))
        ));
    }
}

#[test]
fn all_tar_limits_and_implicit_parent_policy_are_reused() {
    let raw = archive(&[record("one", b"12345"), record("two", b"67890")]);
    let packed = gzip(&raw);
    for limits in [
        Limits {
            max_entries: 1,
            ..DEFAULT
        },
        Limits {
            max_path_bytes: 2,
            ..DEFAULT
        },
        Limits {
            max_entry_bytes: 4,
            ..DEFAULT
        },
        Limits {
            max_total_bytes: 9,
            ..DEFAULT
        },
        Limits {
            max_compression_ratio: 0,
            ..DEFAULT
        },
    ] {
        assert_rejected(&packed, limits);
    }
    let packed = gzip(&archive(&[
        record("a/b/file", b"deep"),
        record("plain", b"ok"),
    ]));
    let limits = Limits {
        max_entries: 2,
        ..DEFAULT
    };
    assert!(list_archive(&packed, limits).is_ok());
    assert!(matches!(
        extract_selected_archive(&packed, &["a/b/file".into()], limits),
        Err(Error::Limit("output entry count"))
    ));
    assert_eq!(
        extract_selected_archive(&packed, &["plain".into()], limits).unwrap()[0].data,
        b"ok"
    );
}

#[test]
fn empty_members_cannot_dilute_a_payload_bombs_compression_ratio() {
    let bomb = gzip(&archive(&[record("file", &vec![0; 1024 * 1024])]));
    let empty_members = gzip(b"").repeat(4095);
    for packed in [
        [bomb.as_slice(), &empty_members].concat(),
        [empty_members.as_slice(), &bomb].concat(),
    ] {
        assert!(matches!(
            list_archive(&packed, DEFAULT),
            Err(Error::Limit("compression ratio"))
        ));
        assert!(matches!(
            list_archive_with_format(&packed, DEFAULT),
            Err(Error::Limit("compression ratio"))
        ));
        assert!(matches!(
            extract_archive(&packed, DEFAULT),
            Err(Error::Limit("compression ratio"))
        ));
        for selected in [vec![], vec!["file".into()]] {
            assert!(matches!(
                extract_selected_archive(&packed, &selected, DEFAULT),
                Err(Error::Limit("compression ratio"))
            ));
        }
    }
}

#[test]
fn absolute_and_ratio_caps_stop_payload_metadata_and_padding_bombs() {
    let raw = archive(&[record("file", &vec![0; 1024 * 1024])]);
    let packed = gzip(&raw);
    assert!(matches!(
        list_archive(&packed, DEFAULT),
        Err(Error::Limit("compression ratio"))
    ));
    assert_rejected(&packed, DEFAULT);
    let expanded = Limits {
        max_compression_ratio: 100_000,
        ..DEFAULT
    };
    assert_eq!(
        extract_archive(&packed, expanded).unwrap()[0].data.len(),
        1024 * 1024
    );
    let tiny = Limits {
        max_total_bytes: 0,
        max_entries: 1,
        ..expanded
    };
    assert!(matches!(
        list_archive(&packed, tiny),
        Err(Error::Limit("gzip decoded bytes"))
    ));
    assert_rejected(&packed, tiny);
    // Declared ISIZE does not bound the actual stream or influence allocation.
    let mut false_size = packed.clone();
    let at = false_size.len() - 4;
    false_size[at..].fill(0);
    assert!(matches!(
        list_archive(&false_size, tiny),
        Err(Error::Limit("gzip decoded bytes"))
    ));

    let packed = gzip(&vec![0; 1024 * 1024]);
    assert_rejected(&packed, DEFAULT);
    assert!(matches!(
        list_archive(
            &packed,
            Limits {
                max_entries: 0,
                ..tiny
            }
        ),
        Err(Error::Limit("gzip decoded bytes"))
    ));

    // A large gzip comment cannot buy enough compression ratio for a payload bomb.
    let base = gzip(&raw);
    let mut metadata = base[..10].to_vec();
    metadata[3] = 16;
    metadata.extend(vec![b'x'; 60 * 1024]);
    metadata.push(0);
    metadata.extend_from_slice(&base[10..]);
    assert!(matches!(
        list_archive(&metadata, DEFAULT),
        Err(Error::Limit("compression ratio"))
    ));
    assert_rejected(&metadata, DEFAULT);
}

#[test]
fn gzip_magic_in_a_valid_tar_header_keeps_tar_precedence() {
    let mut pax = record("PaxHeader", b"13 path=file\n");
    pax[..2].copy_from_slice(&[31, 139]);
    pax[156] = b'x';
    let mut header = tar::Header::new_ustar();
    header.as_mut_bytes().copy_from_slice(&pax[..512]);
    header.set_cksum();
    pax[..512].copy_from_slice(header.as_bytes());
    let raw = archive(&[pax, record("placeholder", b"TAR payload")]);
    assert_eq!(detect_format(&raw).unwrap(), ArchiveFormat::Tar);
    assert_eq!(
        extract_archive(&raw, DEFAULT).unwrap(),
        vec![MemoryEntry {
            path: "file".into(),
            is_dir: false,
            data: b"TAR payload".to_vec()
        }]
    );
    let with_zip = [raw, create_zip(&[], DEFAULT).unwrap()].concat();
    assert_eq!(detect_format(&with_zip).unwrap(), ArchiveFormat::Tar);
    assert_rejected(&with_zip, DEFAULT);
}
