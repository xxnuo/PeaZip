#![cfg(not(target_arch = "wasm32"))]

#[path = "../../peazip-ui/src/browser.rs"]
mod browser;

use peazip_core::{Entry, Limits, MemoryEntry};
use peazip_worker::{
    MAX_LIST_OUTPUT_BYTES, TEST_REPORT_BYTES, list_from_request, test_from_request,
};

const TAR: &[u8] = include_bytes!("../../../tests/fixtures/sample.tar");
const GZIP: &[u8] = include_bytes!("../../../tests/fixtures/sample.tar.gz");
const SEVEN_ZIP: &[u8] = include_bytes!("../../../tests/fixtures/sample.7z");

fn metadata(entries: &[(&[u8], u32, u64, u64)]) -> Vec<u8> {
    metadata_with_packed(
        &entries
            .iter()
            .map(|&(path, kind, size, compressed)| {
                (
                    path,
                    kind,
                    size,
                    compressed,
                    if kind & 2 != 0 { compressed } else { 0 },
                )
            })
            .collect::<Vec<_>>(),
    )
}

fn metadata_with_packed(entries: &[(&[u8], u32, u64, u64, u64)]) -> Vec<u8> {
    let mut bytes = b"PZL2".to_vec();
    bytes.push(0);
    bytes.extend((entries.len() as u32).to_le_bytes());
    for (path, kind, size, compressed, packed) in entries {
        bytes.extend(kind.to_le_bytes());
        bytes.extend((path.len() as u32).to_le_bytes());
        bytes.extend(*path);
        bytes.extend(size.to_le_bytes());
        bytes.extend(compressed.to_le_bytes());
        bytes.extend(packed.to_le_bytes());
    }
    bytes
}

fn decode_entries(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    browser::decode_list(bytes).map(|(_, entries)| entries)
}

#[test]
fn lists_zip_tar_gzip_and_7z_with_browser_decoder() {
    let zip = peazip_core::create_zip(
        &[
            MemoryEntry {
                path: "目录/".into(),
                is_dir: true,
                data: vec![],
            },
            MemoryEntry {
                path: "目录/你好".into(),
                is_dir: false,
                data: b"hello".to_vec(),
            },
            MemoryEntry {
                path: "empty".into(),
                is_dir: false,
                data: vec![],
            },
        ],
        Limits::default(),
    )
    .unwrap();
    for (archive, format) in [
        (zip.as_slice(), peazip_core::ArchiveFormat::Zip),
        (TAR, peazip_core::ArchiveFormat::Tar),
        (GZIP, peazip_core::ArchiveFormat::TarGzip),
        (SEVEN_ZIP, peazip_core::ArchiveFormat::SevenZip),
    ] {
        let output = list_from_request(archive).unwrap();
        assert_eq!(
            decode_entries(&output).unwrap(),
            peazip_core::list_archive(archive, Limits::default()).unwrap(),
        );
        assert_eq!(browser::decode_list(&output).unwrap().0, format);
        assert!(output.len() <= MAX_LIST_OUTPUT_BYTES);
    }
    let tar_entries = decode_entries(&list_from_request(TAR).unwrap()).unwrap();
    for entry in tar_entries {
        assert_eq!(entry.compressed_size, entry.size);
        assert_eq!(
            entry.packed_size,
            if entry.is_dir { None } else { Some(512) }
        );
    }
    let entries = decode_entries(&list_from_request(GZIP).unwrap()).unwrap();
    assert!(
        entries
            .iter()
            .any(|entry| entry.path == "docs/你好.txt" && entry.size == 11)
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.path == "docs/empty/" && entry.is_dir && entry.size == 0)
    );
    assert_eq!(
        decode_entries(&list_from_request(&[0; 1024]).unwrap()).unwrap(),
        vec![]
    );
    let output = list_from_request(SEVEN_ZIP).unwrap();
    assert_eq!(&output[..4], b"PZL2");
    assert_eq!(output[4], 3);
    let entries = decode_entries(&output).unwrap();
    assert_eq!(entries.len(), 6);
    assert!(
        entries
            .iter()
            .any(|entry| entry.path == "docs/你好.txt" && entry.size == 10)
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.path == "docs/empty/" && entry.is_dir && entry.size == 0)
    );
}

#[test]
fn list_wire_layout_is_little_endian_and_preserves_u64_sizes() {
    let bytes = metadata(&[("目录/你好".as_bytes(), 0, 1234, 567)]);
    assert_eq!(&bytes[..9], b"PZL2\x00\x01\x00\x00\x00");
    assert_eq!(&bytes[9..13], &0u32.to_le_bytes());
    assert_eq!(
        decode_entries(&bytes).unwrap(),
        [Entry {
            path: "目录/你好".into(),
            is_dir: false,
            size: 1234,
            compressed_size: 567,
            packed_size: None,
        }]
    );
    for size in [u32::MAX as u64 + 1, u64::MAX] {
        assert!(decode_entries(&metadata(&[(b"a", 0, size, 0)])).is_err());
        assert!(decode_entries(&metadata(&[(b"a", 0, 0, size)])).is_err());
    }
}

#[test]
fn stored_empty_file_serializes_known_zero_without_confusing_it_with_unknown() {
    let zip = peazip_core::create_zip_with_options(
        &[MemoryEntry {
            path: "empty".into(),
            is_dir: false,
            data: vec![],
        }],
        Limits::default(),
        peazip_core::ZipOptions {
            compression: peazip_core::ZipCompression::Stored,
        },
    )
    .unwrap();
    let packet = list_from_request(&zip).unwrap();
    assert_eq!(&packet[..9], b"PZL2\x00\x01\x00\x00\x00");
    assert_eq!(&packet[9..13], &2u32.to_le_bytes());
    assert_eq!(&packet[packet.len() - 8..], &0u64.to_le_bytes());
    let entries = decode_entries(&packet).unwrap();
    assert_eq!(entries[0].compressed_size, 0);
    assert_eq!(entries[0].packed_size, Some(0));
}

#[test]
fn packed_flags_preserve_unknown_known_zero_and_directory_semantics() {
    let entries = decode_entries(&metadata(&[
        (b"unknown", 0, 12, 7),
        (b"known", 2, 12, 7),
        (b"empty", 2, 0, 0),
        (b"directory/", 1, 0, 0),
    ]))
    .unwrap();
    assert_eq!(entries[0].compressed_size, 7);
    assert_eq!(entries[0].packed_size, None);
    assert_eq!(entries[1].compressed_size, 7);
    assert_eq!(entries[1].packed_size, Some(7));
    assert_eq!(entries[2].compressed_size, 0);
    assert_eq!(entries[2].packed_size, Some(0));
    assert!(entries[3].is_dir);
    assert_eq!(entries[3].packed_size, None);
    for kind in [3, 4, 5, 6, 7, u32::MAX] {
        assert!(decode_entries(&metadata(&[(b"directory/", kind, 0, 0)])).is_err());
    }
}

#[test]
fn packed_size_is_independent_of_legacy_compressed_size_and_has_its_own_bounds() {
    let limits = Limits::default();
    let packet = metadata_with_packed(&[
        (b"tar-file", 2, 11, 11, 512),
        (b"unknown", 0, 11, 11, 0),
        (b"known-zero", 2, 0, 7, 0),
    ]);
    let entries = decode_entries(&packet).unwrap();
    assert_eq!(entries[0].compressed_size, 11);
    assert_eq!(entries[0].packed_size, Some(512));
    assert_eq!(entries[1].compressed_size, 11);
    assert_eq!(entries[1].packed_size, None);
    assert_eq!(entries[2].compressed_size, 7);
    assert_eq!(entries[2].packed_size, Some(0));
    for (kind, legacy, packed) in [
        (0, 0, 1),
        (1, 0, 1),
        (2, 0, limits.max_archive_bytes + 1),
        (2, limits.max_archive_bytes + 1, 0),
    ] {
        assert!(
            decode_entries(&metadata_with_packed(&[(b"file", kind, 0, legacy, packed)])).is_err()
        );
    }
    let maximum = decode_entries(&metadata_with_packed(&[(
        b"file",
        2,
        0,
        0,
        limits.max_archive_bytes,
    )]))
    .unwrap();
    assert_eq!(maximum[0].packed_size, Some(limits.max_archive_bytes));
    let mut old_entry_layout = metadata(&[(b"file", 0, 1, 1)]);
    old_entry_layout.truncate(old_entry_layout.len() - 8);
    assert!(decode_entries(&old_entry_layout).is_err());
}

#[test]
fn unversioned_and_old_magic_metadata_are_rejected_instead_of_silently_reinterpreted() {
    for packet in [metadata(&[]), metadata(&[(b"file", 0, 1, 1)])] {
        assert!(
            decode_entries(&packet[4..])
                .unwrap_err()
                .contains("version")
        );
        let mut old = packet;
        old[..4].copy_from_slice(b"PZL1");
        assert!(decode_entries(&old).unwrap_err().contains("version"));
    }
}

#[test]
fn listing_rejects_bad_archives_and_checks_the_entire_gzip_trailer() {
    for bytes in [
        b"".as_slice(),
        b"not an archive",
        b"PK\x03\x04",
        &[0x1f, 0x8b],
    ] {
        assert!(list_from_request(bytes).is_err());
    }
    for end in 0..GZIP.len() {
        assert!(
            list_from_request(&GZIP[..end]).is_err(),
            "gzip prefix {end}"
        );
    }
    for offset in [GZIP.len() - 8, GZIP.len() - 4, GZIP.len() - 1] {
        let mut corrupt = GZIP.to_vec();
        corrupt[offset] ^= 1;
        assert!(
            list_from_request(&corrupt).is_err(),
            "gzip trailer byte {offset}"
        );
    }
    let mut trailing = GZIP.to_vec();
    trailing.push(1);
    assert!(list_from_request(&trailing).is_err());
    let mut concatenated = GZIP.to_vec();
    concatenated.extend(GZIP);
    assert!(list_from_request(&concatenated).is_err());
    let oversized = vec![0; Limits::default().max_archive_bytes as usize + 1];
    assert!(
        list_from_request(&oversized)
            .unwrap_err()
            .contains("archive byte")
    );
    assert!(
        list_from_request(GZIP).is_ok(),
        "a failed request must not poison the next request"
    );
}

#[test]
fn decoder_rejects_truncation_counts_kinds_paths_and_trailing_bytes() {
    let valid = metadata(&[(b"a", 0, 1, 1), ("目录/".as_bytes(), 1, 0, 0)]);
    for end in 0..valid.len() {
        assert!(
            decode_entries(&valid[..end]).is_err(),
            "metadata prefix {end}"
        );
    }
    for count in [1u32, 4097, u32::MAX] {
        assert!(
            decode_entries(&[b"PZL2".as_slice(), &[0], count.to_le_bytes().as_slice()].concat())
                .is_err()
        );
    }
    assert_eq!(decode_entries(&metadata(&[])).unwrap(), vec![]);
    assert!(decode_entries(&[0; 5]).is_err());
    for format in [4, u8::MAX] {
        let mut bytes = b"PZL2".to_vec();
        bytes.push(format);
        bytes.extend(0u32.to_le_bytes());
        assert!(browser::decode_list(&bytes).unwrap_err().contains("format"));
    }
    for kind in [3, 4, 5, 6, 7, u32::MAX] {
        assert!(decode_entries(&metadata(&[(b"a", kind, 0, 0)])).is_err());
    }
    for path in [
        b"".as_slice(),
        &[0xff],
        b"../bad",
        b"/absolute",
        b"a//b",
        b"a/./b",
        b"a\\b",
        b"a\0b",
        b"C:a",
        b"CON.txt",
        b"a.",
        b"a ",
        b"file/",
        &vec![b'a'; 1025],
    ] {
        assert!(
            decode_entries(&metadata(&[(path, 0, 0, 0)])).is_err(),
            "path {path:?}"
        );
    }
    assert!(decode_entries(&metadata(&[(b"dir", 1, 0, 0)])).is_err());
    assert!(decode_entries(&metadata(&[(b"dir/", 1, 1, 0)])).is_err());
    let mut huge_path = metadata(&[(b"a", 0, 0, 0)]);
    huge_path[13..17].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode_entries(&huge_path).is_err());
    let mut trailing = valid;
    trailing.push(0);
    assert!(decode_entries(&trailing).unwrap_err().contains("Trailing"));
    assert!(decode_entries(&vec![0; MAX_LIST_OUTPUT_BYTES + 1]).is_err());
}

#[test]
fn decoder_enforces_per_entry_compressed_and_total_size_limits() {
    let limits = Limits::default();
    for (size, compressed) in [
        (limits.max_entry_bytes + 1, 1),
        (0, limits.max_archive_bytes + 1),
    ] {
        assert!(decode_entries(&metadata(&[(b"a", 0, size, compressed)])).is_err());
    }
    let paths = [b"a".as_slice(), b"b", b"c", b"d", b"e"];
    let entries: Vec<_> = paths
        .into_iter()
        .map(|path| (path, 0, limits.max_entry_bytes, 1))
        .collect();
    assert!(decode_entries(&metadata(&entries[..4])).is_ok());
    assert!(decode_entries(&metadata(&entries)).is_err());
}

#[test]
fn maximum_metadata_stays_below_the_callback_cap() {
    let limits = Limits::default();
    let prefix = "a".repeat(limits.max_path_bytes - 4);
    let entries: Vec<_> = (0..limits.max_entries)
        .map(|i| MemoryEntry {
            path: format!("{prefix}{i:04}"),
            is_dir: false,
            data: vec![],
        })
        .collect();
    let zip = peazip_core::create_zip(&entries, limits).unwrap();
    let output = list_from_request(&zip).unwrap();
    assert_eq!(output.len(), MAX_LIST_OUTPUT_BYTES);
    assert!(output.len() as u64 <= limits.max_archive_bytes);
    assert_eq!(decode_entries(&output).unwrap().len(), limits.max_entries);
}

fn test_report_packet(entries: u32, files: u32, total: u64, checksummed: u32) -> Vec<u8> {
    let mut packet = entries.to_le_bytes().to_vec();
    packet.extend(files.to_le_bytes());
    packet.extend(total.to_le_bytes());
    packet.extend(checksummed.to_le_bytes());
    packet
}

#[test]
fn test_report_roundtrips_zip_tar_gzip_and_7z_with_exact_counts_and_coverage() {
    let zip = include_bytes!("../../../web/tests/fixtures/sample.zip");
    for (archive, entries, files, total, checksummed) in [
        (zip.as_slice(), 3, 2, 13, 2),
        (TAR, 5, 3, 27, 0),
        (GZIP, 5, 3, 27, 3),
        (SEVEN_ZIP, 6, 3, 26, 3),
    ] {
        let bytes = test_from_request(archive).unwrap();
        assert_eq!(bytes.len(), TEST_REPORT_BYTES);
        assert_eq!(
            bytes,
            test_report_packet(entries, files, total, checksummed)
        );
        let decoded = browser::decode_test_report(&bytes).unwrap();
        assert_eq!(decoded.entries, entries as usize);
        assert_eq!(decoded.files, files as usize);
        assert_eq!(decoded.total_bytes, total);
        assert_eq!(decoded.checksummed_files, checksummed as usize);
    }
    assert_eq!(
        test_from_request(&[0; 1024]).unwrap(),
        vec![0; TEST_REPORT_BYTES]
    );
}

#[test]
fn test_report_decoder_rejects_truncation_trailing_bytes_counts_coverage_and_totals() {
    let valid = test_report_packet(3, 2, 13, 2);
    for end in 0..valid.len() {
        assert!(
            browser::decode_test_report(&valid[..end]).is_err(),
            "prefix {end}"
        );
    }
    let mut trailing = valid;
    trailing.push(0);
    assert!(browser::decode_test_report(&trailing).is_err());
    for (entries, files, total, checksummed) in [
        (4097, 0, 0, 0),
        (u32::MAX, 0, 0, 0),
        (1, 2, 0, 0),
        (1, u32::MAX, 0, 0),
        (2, 1, 0, 2),
        (1, 1, 0, u32::MAX),
        (1, 0, 1, 0),
        (1, 1, 16 * 1024 * 1024 + 1, 1),
        (5, 5, 64 * 1024 * 1024 + 1, 5),
        (1, 1, 1u64 << 32, 0),
        (1, 1, u64::MAX, 0),
    ] {
        assert!(
            browser::decode_test_report(&test_report_packet(entries, files, total, checksummed))
                .is_err()
        );
    }
    for (entries, files, total, checksummed) in [
        (0, 0, 0, 0),
        (4096, 0, 0, 0),
        (4096, 4096, 0, 4096),
        (4, 4, 64 * 1024 * 1024, 0),
        (4, 4, 64 * 1024 * 1024, 4),
    ] {
        assert!(
            browser::decode_test_report(&test_report_packet(entries, files, total, checksummed))
                .is_ok()
        );
    }
}

#[test]
fn test_request_checks_unselected_zip_file_crc_and_recovers_after_invalid_archives() {
    let limits = Limits::default();
    let mut zip = peazip_core::create_zip(
        &[
            MemoryEntry {
                path: "good".into(),
                is_dir: false,
                data: b"good".to_vec(),
            },
            MemoryEntry {
                path: "hidden".into(),
                is_dir: false,
                data: b"bad!".to_vec(),
            },
        ],
        limits,
    )
    .unwrap();
    let valid = zip.clone();
    let central = zip
        .windows(4)
        .enumerate()
        .filter(|(_, bytes)| *bytes == b"PK\x01\x02")
        .nth(1)
        .unwrap()
        .0;
    let local = u32::from_le_bytes(zip[central + 42..central + 46].try_into().unwrap()) as usize;
    zip[local + 14..local + 18].fill(0);
    zip[central + 16..central + 20].fill(0);
    assert!(list_from_request(&zip).is_ok());
    assert!(peazip_core::extract_selected_archive(&zip, &["good".into()], limits).is_ok());
    assert!(
        test_from_request(&zip).is_err(),
        "testing must read hidden, unselected payloads too"
    );
    for archive in [
        b"".as_slice(),
        b"not an archive",
        &GZIP[..GZIP.len() - 1],
        &SEVEN_ZIP[..SEVEN_ZIP.len() - 1],
    ] {
        assert!(test_from_request(archive).is_err());
    }
    assert!(
        test_from_request(&vec![0; limits.max_archive_bytes as usize + 1])
            .unwrap_err()
            .contains("archive byte")
    );
    assert!(test_from_request(&valid).is_ok());
}
