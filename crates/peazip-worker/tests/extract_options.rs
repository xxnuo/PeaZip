use peazip_core::{Limits, MemoryEntry, ZipCompression, ZipOptions};
use peazip_worker::{MAX_EXTRACT_V1_REQUEST_BYTES, extract_from_request, extract_v1_from_request};
use std::collections::BTreeMap;

fn legacy(archive: &[u8], selected: &[&str]) -> Vec<u8> {
    let mut packet = (selected.len() as u32).to_le_bytes().to_vec();
    for path in selected {
        packet.extend_from_slice(&(path.len() as u32).to_le_bytes());
        packet.extend_from_slice(path.as_bytes());
    }
    packet.extend_from_slice(&(archive.len() as u32).to_le_bytes());
    packet.extend_from_slice(archive);
    packet
}

fn versioned(archive: &[u8], selected: &[&str], paths: u32) -> Vec<u8> {
    let mut packet = 1u32.to_le_bytes().to_vec();
    packet.extend_from_slice(&paths.to_le_bytes());
    packet.extend_from_slice(&legacy(archive, selected));
    packet
}

fn decode(packet: &[u8]) -> BTreeMap<String, (bool, Vec<u8>)> {
    let number = |at| u32::from_le_bytes(packet[at..at + 4].try_into().unwrap()) as usize;
    let mut output = BTreeMap::new();
    let mut at = 4;
    for _ in 0..number(0) {
        let directory = number(at) == 1;
        let path_len = number(at + 4);
        let data_len = number(at + 8);
        at += 12;
        let name = std::str::from_utf8(&packet[at..at + path_len])
            .unwrap()
            .to_owned();
        at += path_len;
        assert!(
            output
                .insert(name, (directory, packet[at..at + data_len].to_vec()))
                .is_none()
        );
        at += data_len;
    }
    assert_eq!(at, packet.len());
    output
}

fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let entries: Vec<_> = entries
        .iter()
        .map(|(path, bytes)| MemoryEntry {
            path: (*path).into(),
            is_dir: path.ends_with('/'),
            data: bytes.to_vec(),
        })
        .collect();
    peazip_core::create_zip_with_options(
        &entries,
        Limits::default(),
        ZipOptions {
            compression: ZipCompression::Stored,
        },
    )
    .unwrap()
}

#[test]
fn full_is_byte_compatible_and_files_only_flattens_in_core_for_every_format() {
    let zipped = zip(&[
        ("docs/", b""),
        ("docs/empty/", b""),
        ("docs/你好.txt", "zip 你好\n".as_bytes()),
        ("docs/nested/data.bin", &[0, 127, 255]),
        ("other.txt", b"excluded"),
    ]);
    for archive in [
        zipped.as_slice(),
        include_bytes!("../../../tests/fixtures/sample.tar").as_slice(),
        include_bytes!("../../../tests/fixtures/sample.tar.gz").as_slice(),
        include_bytes!("../../../tests/fixtures/sample.7z").as_slice(),
    ] {
        let old = extract_from_request(&legacy(archive, &["docs/"])).unwrap();
        assert_eq!(
            extract_v1_from_request(&versioned(archive, &["docs/"], 0)).unwrap(),
            old
        );
        let full = decode(&old);
        let expected: BTreeMap<_, _> = full
            .into_iter()
            .filter(|(_, (dir, _))| !dir)
            .map(|(path, value)| (path.rsplit('/').next().unwrap().to_owned(), value))
            .collect();
        let flat = decode(&extract_v1_from_request(&versioned(archive, &["docs/"], 1)).unwrap());
        assert_eq!(flat, expected);
        assert_eq!(flat.len(), 2);
        assert_eq!(flat["data.bin"], (false, vec![0, 127, 255]));
        assert!(!flat.contains_key("other.txt"));
        let all: Vec<_> = peazip_core::list_archive(archive, Limits::default())
            .unwrap()
            .into_iter()
            .map(|entry| entry.path)
            .collect();
        let selected: Vec<_> = all.iter().map(String::as_str).collect();
        assert_eq!(
            decode(&extract_v1_from_request(&versioned(archive, &selected, 1)).unwrap()).len(),
            3
        );
    }
}

#[test]
fn rejects_unknown_modes_before_archive_parsing_truncation_and_trailing_bytes() {
    for mode in [2u32, 256, u32::MAX] {
        let mut header = 1u32.to_le_bytes().to_vec();
        header.extend_from_slice(&mode.to_le_bytes());
        assert!(
            extract_v1_from_request(&header)
                .unwrap_err()
                .contains("path mode")
        );
    }
    let archive = zip(&[("file", b"data")]);
    let valid = versioned(&archive, &["file"], 1);
    for end in 0..valid.len() {
        assert!(
            extract_v1_from_request(&valid[..end]).is_err(),
            "prefix {end}"
        );
    }
    let mut bad_version = valid.clone();
    bad_version[..4].copy_from_slice(&2u32.to_le_bytes());
    assert!(
        extract_v1_from_request(&bad_version)
            .unwrap_err()
            .contains("version")
    );
    let mut trailing = valid;
    trailing.push(0);
    assert!(
        extract_v1_from_request(&trailing)
            .unwrap_err()
            .contains("trailing")
    );
    assert!(
        extract_v1_from_request(&vec![0; MAX_EXTRACT_V1_REQUEST_BYTES + 1])
            .unwrap_err()
            .contains("request byte")
    );
    assert!(extract_v1_from_request(&legacy(&archive, &["file"])).is_err());
    assert!(extract_from_request(&versioned(&archive, &["file"], 1)).is_err());
}

#[test]
fn collisions_empty_file_selection_and_bad_crc_never_publish_partial_output() {
    for other in ["name.txt", "NAME.txt"] {
        let archive = zip(&[("a/name.txt", b"first"), (&format!("b/{other}"), b"second")]);
        assert!(
            extract_v1_from_request(&versioned(
                &archive,
                &["a/name.txt", &format!("b/{other}")],
                0
            ))
            .is_ok()
        );
        assert!(
            extract_v1_from_request(&versioned(
                &archive,
                &["a/name.txt", &format!("b/{other}")],
                1
            ))
            .is_err()
        );
    }
    let archive = zip(&[("empty/", b""), ("docs/data.bin", b"payload")]);
    assert!(extract_v1_from_request(&versioned(&archive, &["empty/"], 0)).is_ok());
    assert!(extract_v1_from_request(&versioned(&archive, &["empty/"], 1)).is_err());
    let mut bad = zip(&[("docs/data.bin", b"payload")]);
    let name = u16::from_le_bytes(bad[26..28].try_into().unwrap()) as usize;
    let extra = u16::from_le_bytes(bad[28..30].try_into().unwrap()) as usize;
    bad[30 + name + extra] ^= 1;
    for mode in [0, 1] {
        assert!(extract_v1_from_request(&versioned(&bad, &["docs/data.bin"], mode)).is_err());
    }
}
