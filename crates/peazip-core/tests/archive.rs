use peazip_core::{Error, Limits, MemoryEntry, create_zip, extract_zip, list_zip};
use std::io::{Cursor, Write};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

fn entry(path: &str, data: &[u8]) -> MemoryEntry {
    MemoryEntry {
        path: path.into(),
        is_dir: path.ends_with('/'),
        data: data.into(),
    }
}

fn raw_zip(path: &str, data: &[u8], symlink: bool) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    if symlink {
        writer.add_symlink(path, "target", opts).unwrap();
    } else {
        writer.start_file(path, opts).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

// Build duplicate central-directory records directly: ZipWriter rejects duplicates itself.
fn duplicate_zip() -> Vec<u8> {
    let mut bytes = raw_zip("same", b"hello", false);
    let end = bytes.len() - 22;
    let central = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap()) as usize;
    let record = bytes[central..end].to_vec();
    bytes.splice(end..end, record.iter().copied());
    let end = end + record.len();
    bytes[end + 8..end + 10].copy_from_slice(&2u16.to_le_bytes());
    bytes[end + 10..end + 12].copy_from_slice(&2u16.to_le_bytes());
    bytes[end + 12..end + 16].copy_from_slice(&((record.len() * 2) as u32).to_le_bytes());
    bytes
}

#[test]
fn roundtrip_files_directories_unicode_and_empty() {
    let input = vec![
        entry("folder/", b""),
        entry("folder/中文.txt", b"hello ZIP"),
        entry("empty", b""),
        entry("binary", &[0, 255, 128]),
    ];
    let bytes = create_zip(&input, Limits::default()).unwrap();
    let listed = list_zip(&bytes, Limits::default()).unwrap();
    assert_eq!(listed.len(), input.len());
    assert_eq!(listed[1].size, 9);
    assert_eq!(extract_zip(&bytes, Limits::default()).unwrap(), input);
    let empty = create_zip(&[], Limits::default()).unwrap();
    assert!(extract_zip(&empty, Limits::default()).unwrap().is_empty());
}

#[test]
fn rejects_unsafe_paths_on_read_and_write() {
    for path in [
        "../escape",
        "a/../b",
        "/absolute",
        "C:/drive",
        "C:drive",
        "\\\\server\\share",
        "a\\b",
        "./a",
        "a//b",
        "a/",
        "a\0b",
        "CON",
        "a.",
        "a ",
        "a/NUL.txt",
    ] {
        let input = MemoryEntry {
            path: path.into(),
            is_dir: false,
            data: vec![],
        };
        assert!(create_zip(&[input], Limits::default()).is_err(), "{path:?}");
        let bytes = raw_zip(path, b"", false);
        assert!(list_zip(&bytes, Limits::default()).is_err(), "{path:?}");
        assert!(extract_zip(&bytes, Limits::default()).is_err(), "{path:?}");
    }
}

#[test]
fn rejects_links_duplicates_and_file_directory_conflicts() {
    let bytes = raw_zip("link", b"", true);
    assert!(matches!(
        list_zip(&bytes, Limits::default()),
        Err(Error::UnsupportedEntry(_))
    ));
    let bytes = duplicate_zip();
    let archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    assert!(archive.has_duplicate_names());
    assert!(matches!(
        list_zip(&bytes, Limits::default()),
        Err(Error::DuplicatePath(_))
    ));
    assert!(matches!(
        extract_zip(&bytes, Limits::default()),
        Err(Error::DuplicatePath(_))
    ));
    for input in [
        vec![entry("a", b""), entry("a", b"")],
        vec![entry("a", b""), entry("A", b"")],
        vec![entry("a", b""), entry("a/b", b"")],
        vec![entry("a/b", b""), entry("a", b"")],
        vec![entry("a/", b""), entry("a", b"")],
    ] {
        assert!(matches!(
            create_zip(&input, Limits::default()),
            Err(Error::DuplicatePath(_))
        ));
    }
}

#[test]
fn enforces_each_limit() {
    let input = vec![entry("one", b"12345"), entry("two", b"67890")];
    let bytes = create_zip(&input, Limits::default()).unwrap();
    let cases = [
        Limits {
            max_archive_bytes: 1,
            ..Limits::default()
        },
        Limits {
            max_entries: 1,
            ..Limits::default()
        },
        Limits {
            max_path_bytes: 2,
            ..Limits::default()
        },
        Limits {
            max_entry_bytes: 4,
            ..Limits::default()
        },
        Limits {
            max_total_bytes: 9,
            ..Limits::default()
        },
        Limits {
            max_compression_ratio: 0,
            ..Limits::default()
        },
    ];
    for limits in cases {
        assert!(create_zip(&input, limits).is_err(), "{limits:?}");
        assert!(list_zip(&bytes, limits).is_err(), "{limits:?}");
        assert!(extract_zip(&bytes, limits).is_err(), "{limits:?}");
    }
}

#[test]
fn rejects_bombs_corruption_and_false_sizes() {
    let bytes = raw_zip("bomb", &vec![0; 1024 * 1024], false);
    assert!(matches!(
        list_zip(&bytes, Limits::default()),
        Err(Error::Limit("compression ratio"))
    ));
    assert!(extract_zip(&bytes, Limits::default()).is_err());
    for bytes in [vec![], b"not a ZIP".to_vec(), bytes[..20].to_vec()] {
        assert!(extract_zip(&bytes, Limits::default()).is_err());
    }
    let mut bytes = raw_zip("file", b"hello world", false);
    let central = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    // Wrong CRC must fail when the content is read.
    bytes[14..18].fill(0);
    bytes[central + 16..central + 20].fill(0);
    assert!(extract_zip(&bytes, Limits::default()).is_err());
    let mut bytes = raw_zip("file", &vec![b'x'; 4096], false);
    let central = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    bytes[22..26].copy_from_slice(&1u32.to_le_bytes());
    bytes[central + 24..central + 28].copy_from_slice(&1u32.to_le_bytes());
    assert!(extract_zip(&bytes, Limits::default()).is_err());
}

#[test]
fn rejects_conflicts_in_external_archives_in_both_orders() {
    for names in [
        ["a", "a/b"],
        ["a/b", "a"],
        ["a", "A"],
        ["folder/", "folder"],
    ] {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for name in names {
            if name.ends_with('/') {
                writer
                    .add_directory(name, SimpleFileOptions::default())
                    .unwrap();
            } else {
                writer
                    .start_file(name, SimpleFileOptions::default())
                    .unwrap();
            }
        }
        let bytes = writer.finish().unwrap().into_inner();
        assert!(matches!(
            list_zip(&bytes, Limits::default()),
            Err(Error::DuplicatePath(_))
        ));
        assert!(extract_zip(&bytes, Limits::default()).is_err());
    }
}

#[test]
fn rejects_entry_count_before_parsing_directory_records() {
    let mut bytes = duplicate_zip();
    let end = bytes.len() - 22;
    let central = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap()) as usize;
    // Corrupt the first record's filename encoding. A full-index reader fails here;
    // the bounded reader must reject the declared count without decoding it.
    bytes[central + 8..central + 10].copy_from_slice(&0x800u16.to_le_bytes());
    bytes[central + 46] = 0xff;
    assert!(matches!(
        list_zip(
            &bytes,
            Limits {
                max_entries: 1,
                ..Limits::default()
            }
        ),
        Err(Error::Limit("entry count"))
    ));
}

#[test]
fn directories_have_canonical_names_and_allow_children_in_either_order() {
    let noncanonical = MemoryEntry {
        path: "folder".into(),
        is_dir: true,
        data: vec![],
    };
    assert!(matches!(
        create_zip(&[noncanonical], Limits::default()),
        Err(Error::UnsafePath(_))
    ));
    for input in [
        vec![entry("folder/", b""), entry("folder/file", b"hello")],
        vec![entry("folder/file", b"hello"), entry("folder/", b"")],
    ] {
        let bytes = create_zip(&input, Limits::default()).unwrap();
        assert_eq!(extract_zip(&bytes, Limits::default()).unwrap(), input);
    }
    let input = [entry("folder/", b"")];
    let limits = Limits {
        max_path_bytes: 7,
        ..Limits::default()
    };
    assert!(create_zip(&input, limits).is_ok());
    assert!(
        create_zip(
            &input,
            Limits {
                max_path_bytes: 6,
                ..limits
            }
        )
        .is_err()
    );
}

#[test]
fn rejects_bad_crc_even_for_stored_empty_files_and_directories() {
    for (name, data) in [
        ("file", &b"hello"[..]),
        ("empty", &b""[..]),
        ("dir/", &b""[..]),
    ] {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        if name.ends_with('/') {
            writer.add_directory(name, options).unwrap();
        } else {
            writer.start_file(name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        let mut bytes = writer.finish().unwrap().into_inner();
        let central = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        bytes[14..18].copy_from_slice(&1u32.to_le_bytes());
        bytes[central + 16..central + 20].copy_from_slice(&1u32.to_le_bytes());
        assert!(extract_zip(&bytes, Limits::default()).is_err(), "{name}");
    }
}

#[test]
fn rejects_declared_sizes_above_and_below_actual_size() {
    for size in [0u32, 1000] {
        let mut bytes = raw_zip("file", b"hello world", false);
        let central = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        bytes[22..26].copy_from_slice(&size.to_le_bytes());
        bytes[central + 24..central + 28].copy_from_slice(&size.to_le_bytes());
        assert!(extract_zip(&bytes, Limits::default()).is_err(), "{size}");
    }
}
