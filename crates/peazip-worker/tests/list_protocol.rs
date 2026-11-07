#![cfg(not(target_arch = "wasm32"))]

#[path = "../../peazip-ui/src/browser.rs"]
mod browser;

use peazip_core::{Entry, Limits, MemoryEntry};
use peazip_worker::{MAX_LIST_OUTPUT_BYTES, list_from_request};

const TAR: &[u8] = include_bytes!("../../../tests/fixtures/sample.tar");
const GZIP: &[u8] = include_bytes!("../../../tests/fixtures/sample.tar.gz");

fn metadata(entries: &[(&[u8], u32, u64, u64)]) -> Vec<u8> {
    let mut bytes = vec![0];
    bytes.extend((entries.len() as u32).to_le_bytes());
    for (path, kind, size, compressed) in entries {
        bytes.extend(kind.to_le_bytes());
        bytes.extend((path.len() as u32).to_le_bytes());
        bytes.extend(*path);
        bytes.extend(size.to_le_bytes());
        bytes.extend(compressed.to_le_bytes());
    }
    bytes
}

fn decode_entries(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    browser::decode_list(bytes).map(|(_, entries)| entries)
}

#[test]
fn lists_zip_tar_and_gzip_with_browser_decoder() {
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
    ] {
        let output = list_from_request(archive).unwrap();
        assert_eq!(
            decode_entries(&output).unwrap(),
            peazip_core::list_archive(archive, Limits::default()).unwrap(),
        );
        assert_eq!(browser::decode_list(&output).unwrap().0, format);
        assert!(output.len() <= MAX_LIST_OUTPUT_BYTES);
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
}

#[test]
fn list_wire_layout_is_little_endian_and_preserves_u64_sizes() {
    let bytes = metadata(&[("目录/你好".as_bytes(), 0, 1234, 567)]);
    assert_eq!(
        decode_entries(&bytes).unwrap(),
        [Entry {
            path: "目录/你好".into(),
            is_dir: false,
            size: 1234,
            compressed_size: 567,
        }]
    );
    for size in [u32::MAX as u64 + 1, u64::MAX] {
        assert!(decode_entries(&metadata(&[(b"a", 0, size, 0)])).is_err());
        assert!(decode_entries(&metadata(&[(b"a", 0, 0, size)])).is_err());
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
        assert!(decode_entries(&[&[0], count.to_le_bytes().as_slice()].concat()).is_err());
    }
    assert_eq!(decode_entries(&[0; 5]).unwrap(), vec![]);
    for format in [3, u8::MAX] {
        let mut bytes = vec![format];
        bytes.extend(0u32.to_le_bytes());
        assert!(browser::decode_list(&bytes).unwrap_err().contains("format"));
    }
    for kind in [2, u32::MAX] {
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
    huge_path[9..13].copy_from_slice(&u32::MAX.to_le_bytes());
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
