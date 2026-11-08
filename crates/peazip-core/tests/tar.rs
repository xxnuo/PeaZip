use peazip_core::{
    ArchiveFormat, Error, Limits, MemoryEntry, create_zip, detect_format, extract_archive,
    extract_selected_archive, list_archive,
};

const DEFAULT: Limits = Limits {
    max_archive_bytes: 32 * 1024 * 1024,
    max_entries: 4096,
    max_output_items: peazip_core::MAX_EXTRACTED_ITEMS,
    max_path_bytes: 1024,
    max_entry_bytes: 16 * 1024 * 1024,
    max_total_bytes: 64 * 1024 * 1024,
    max_compression_ratio: 200,
};

fn memory(path: &str, data: &[u8]) -> MemoryEntry {
    MemoryEntry {
        path: path.into(),
        is_dir: path.ends_with('/'),
        data: data.to_vec(),
    }
}

fn record(path: &[u8], kind: u8, data: &[u8]) -> Vec<u8> {
    assert!(path.len() <= 100);
    let mut header = tar::Header::new_ustar();
    header.as_mut_bytes()[..100].fill(0);
    header.as_mut_bytes()[..path.len()].copy_from_slice(path);
    header.set_entry_type(tar::EntryType::new(kind));
    header.set_size(data.len() as u64);
    header.set_cksum();
    let mut bytes = header.as_bytes().to_vec();
    bytes.extend_from_slice(data);
    bytes.resize(bytes.len().div_ceil(512) * 512, 0);
    bytes
}

fn archive(records: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = records.concat();
    bytes.resize(bytes.len() + 1024, 0);
    bytes
}

fn pax_pair(key: &str, value: &[u8]) -> Vec<u8> {
    let mut len = key.len() + value.len() + 4;
    loop {
        let prefix = format!("{len} {key}=");
        let actual = prefix.len() + value.len() + 1;
        if len == actual {
            let mut bytes = prefix.into_bytes();
            bytes.extend_from_slice(value);
            bytes.push(b'\n');
            return bytes;
        }
        len = actual;
    }
}

fn assert_rejected(bytes: &[u8]) {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    let gzip = encoder.finish().unwrap();
    for bytes in [bytes, &gzip] {
        assert!(list_archive(bytes, DEFAULT).is_err());
        assert!(extract_archive(bytes, DEFAULT).is_err());
        assert!(extract_selected_archive(bytes, &[], DEFAULT).is_err());
    }
}

#[test]
fn pax_keywords_cannot_amplify_error_messages_or_hide_control_bytes() {
    let key = format!("GNU.sparse.{}", "x".repeat(16 * 1024));
    let bytes = archive(&[
        record(b"Pax", b'x', &pax_pair(&key, b"0")),
        record(b"file", b'0', b"data"),
    ]);
    let error = list_archive(&bytes, DEFAULT).unwrap_err();
    assert!(matches!(error, Error::Limit("PAX keyword bytes")));
    assert!(error.to_string().len() < 200);
    assert_rejected(&bytes);
    for key in ["path\0extra", "vendor.\nkey"] {
        let bytes = archive(&[
            record(b"Pax", b'x', &pax_pair(key, b"value")),
            record(b"file", b'0', b"data"),
        ]);
        assert_rejected(&bytes);
    }
}

#[test]
fn real_bsdtar_ustar_pax_and_gnu_fixtures() {
    for (bytes, long_names) in [
        (&include_bytes!("fixtures/ustar.tar")[..], false),
        (&include_bytes!("fixtures/pax.tar")[..], true),
        (&include_bytes!("fixtures/gnutar.tar")[..], true),
    ] {
        assert_eq!(detect_format(bytes).unwrap(), ArchiveFormat::Tar);
        let listed = list_archive(bytes, DEFAULT).unwrap();
        let output = extract_archive(bytes, DEFAULT).unwrap();
        assert_eq!(listed.len(), if long_names { 6 } else { 5 });
        assert!(listed.iter().all(|e| e.size == e.compressed_size));
        for expected in [
            memory("folder/", b""),
            memory("empty-dir/", b""),
            memory("empty", b""),
            memory("binary", &[0, 255, 128]),
            memory("folder/中文.txt", "hello TAR 中文\n".as_bytes()),
        ] {
            assert!(output.contains(&expected), "{expected:?}");
        }
        assert!(output.iter().all(|e| !e.path.starts_with("./")));
        if long_names {
            assert!(output.contains(&memory(
                &format!("long-{}.txt", "n".repeat(140)),
                b"long name\n"
            )));
        }
        assert_eq!(
            extract_selected_archive(bytes, &["folder/".into()], DEFAULT).unwrap(),
            output
                .into_iter()
                .filter(|e| e.path.starts_with("folder/"))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn content_detection_keeps_empty_and_sfx_zip_support() {
    for input in [vec![], vec![memory("file", b"zip payload")]] {
        let zip = create_zip(&input, DEFAULT).unwrap();
        for prefix in [&b""[..], &b"MZ executable stub\0\x01"[..]] {
            let bytes = [prefix, &zip].concat();
            assert_eq!(detect_format(&bytes).unwrap(), ArchiveFormat::Zip);
            assert_eq!(list_archive(&bytes, DEFAULT).unwrap().len(), input.len());
            assert_eq!(extract_archive(&bytes, DEFAULT).unwrap(), input);
            let selected = input.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
            assert_eq!(
                extract_selected_archive(&bytes, &selected, DEFAULT).unwrap(),
                input
            );
        }
    }
    for bytes in [
        vec![],
        b"plain text is not TAR".to_vec(),
        vec![b'a'; 2048],
        vec![0; 512],
    ] {
        assert!(matches!(
            detect_format(&bytes),
            Err(Error::UnsupportedFormat)
        ));
    }
    for size in [1024, 1536, 10240] {
        let bytes = vec![0; size];
        assert_eq!(detect_format(&bytes).unwrap(), ArchiveFormat::Tar);
        assert!(extract_archive(&bytes, DEFAULT).unwrap().is_empty());
    }
}

#[test]
fn embedded_or_trailing_zip_cannot_hide_a_tar_archive() {
    let zip = create_zip(&[], DEFAULT).unwrap();
    let bytes = archive(&[record(b"embedded.zip", b'0', &zip)]);
    assert_eq!(detect_format(&bytes).unwrap(), ArchiveFormat::Tar);
    assert_eq!(
        extract_archive(&bytes, DEFAULT).unwrap(),
        vec![memory("embedded.zip", &zip)]
    );
    assert_rejected(&[bytes, zip.clone()].concat());
    assert_rejected(&[vec![0; 1024], zip].concat());
}

#[test]
fn valid_pax_header_with_zip_magic_takes_precedence_over_zip_detection() {
    let bytes = archive(&[
        record(b"PK\x03\x04PaxHeader", b'x', &pax_pair("path", b"file.txt")),
        record(b"placeholder", b'0', b"TAR payload"),
    ]);
    assert_eq!(detect_format(&bytes).unwrap(), ArchiveFormat::Tar);
    assert_eq!(list_archive(&bytes, DEFAULT).unwrap()[0].path, "file.txt");
    assert_eq!(
        extract_archive(&bytes, DEFAULT).unwrap(),
        vec![memory("file.txt", b"TAR payload")]
    );
    assert_eq!(
        extract_selected_archive(&bytes, &["file.txt".into()], DEFAULT).unwrap(),
        vec![memory("file.txt", b"TAR payload")]
    );
    let with_eocd = [bytes, create_zip(&[], DEFAULT).unwrap()].concat();
    assert_eq!(detect_format(&with_eocd).unwrap(), ArchiveFormat::Tar);
    assert!(matches!(
        list_archive(&with_eocd, DEFAULT),
        Err(Error::InvalidTar(_))
    ));
    assert_rejected(&with_eocd);
}

#[test]
fn invalid_tar_checksum_does_not_override_valid_zip_or_mz_sfx() {
    use std::io::{Cursor, Write};
    use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

    let stored_zip = |payload: &[u8]| {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                "file",
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(payload).unwrap();
        writer.finish().unwrap().into_inner()
    };
    let template = stored_zip(&[b'x'; 1024]);
    let data_start = ZipArchive::new(Cursor::new(&template))
        .unwrap()
        .by_index(0)
        .unwrap()
        .data_start()
        .unwrap() as usize;
    for prefix in [&b""[..], &b"MZ executable stub\0\x01"[..]] {
        let mut payload = vec![b'x'; 1024];
        let at = 257 - prefix.len() - data_start;
        payload[at..at + 6].copy_from_slice(b"ustar\0");
        let bytes = [prefix, &stored_zip(&payload)].concat();
        assert_eq!(&bytes[257..263], b"ustar\0");
        assert_eq!(detect_format(&bytes).unwrap(), ArchiveFormat::Zip);
        assert_eq!(list_archive(&bytes, DEFAULT).unwrap().len(), 1);
        assert_eq!(
            extract_archive(&bytes, DEFAULT).unwrap(),
            vec![memory("file", &payload)]
        );
        assert_eq!(
            extract_selected_archive(&bytes, &["file".into()], DEFAULT).unwrap(),
            vec![memory("file", &payload)]
        );
    }
    let mut bad_tar = archive(&[record(b"file", b'0', b"payload")]);
    bad_tar[0] ^= 1;
    assert!(matches!(detect_format(&bad_tar), Err(Error::InvalidTar(_))));
}

#[test]
fn solaris_sparse_pax_is_rejected_before_returning_stored_bytes() {
    for kind in *b"xg" {
        let bytes = archive(&[
            record(b"good", b'0', b"ok"),
            record(b"PaxHeader", kind, &pax_pair("SUN.holesdata", b" 5 8")),
            record(b"sparse", b'0', b"XXXXXabc"),
        ]);
        assert!(
            matches!(list_archive(&bytes, DEFAULT), Err(Error::UnsupportedEntry(ref key)) if key == "PAX SUN.holesdata")
        );
        assert!(matches!(
            extract_archive(&bytes, DEFAULT),
            Err(Error::UnsupportedEntry(_))
        ));
        for selected in [vec![], vec!["good".into()], vec!["sparse".into()]] {
            assert!(matches!(
                extract_selected_archive(&bytes, &selected, DEFAULT),
                Err(Error::UnsupportedEntry(_))
            ));
        }
    }
}

#[test]
fn all_generic_apis_check_archive_limit_before_detection_or_parsing() {
    let limits = Limits {
        max_archive_bytes: 1,
        ..DEFAULT
    };
    for bytes in [
        &b"not an archive"[..],
        &b"PK\x03\x04"[..],
        &vec![0; 1024][..],
    ] {
        assert!(matches!(
            list_archive(bytes, limits),
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
    assert!(matches!(
        list_archive(b"PK\x03\x04", DEFAULT),
        Err(Error::InvalidArchive(_))
    ));
}

#[test]
fn canonicalizes_only_leading_dot_components_and_directory_suffixes() {
    let bytes = archive(&[
        record(b"./", b'5', b""),
        record(b"././folder", b'5', b""),
        record(b"./folder/file", b'0', b"data"),
        record(b"empty", 0, b""),
    ]);
    assert_eq!(
        extract_archive(&bytes, DEFAULT).unwrap(),
        vec![
            memory("folder/", b""),
            memory("folder/file", b"data"),
            memory("empty", b"")
        ]
    );
    assert!(
        extract_archive(&archive(&[record(b".", b'5', b"")]), DEFAULT)
            .unwrap()
            .is_empty()
    );
    for path in [
        "../x",
        "./../x",
        "/absolute",
        ".//absolute",
        "a/./b",
        "a/../b",
        "a//b",
        "C:/x",
        "C:x",
        "a\\b",
        "a.",
        "a ",
        "CON",
        "a/NUL.txt",
        "a?b",
        "a\nb",
        "",
        ".",
        "a/",
    ] {
        let bytes = archive(&[record(path.as_bytes(), b'0', b"")]);
        assert!(
            matches!(list_archive(&bytes, DEFAULT), Err(Error::UnsafePath(_))),
            "{path:?}"
        );
        assert_rejected(&bytes);
    }
    assert_rejected(&archive(&[record(b"\xff", b'0', b"")]));
}

#[test]
fn ustar_prefix_and_unterminated_name_fields_are_supported() {
    let path = format!("{}/{}", "parent".repeat(12), "leaf".repeat(20));
    let mut header = tar::Header::new_ustar();
    header.set_path(&path).unwrap();
    header.set_size(0);
    header.set_cksum();
    assert_eq!(
        extract_archive(&archive(&[header.as_bytes().to_vec()]), DEFAULT).unwrap(),
        vec![memory(&path, b"")]
    );
    let path = "a".repeat(100);
    assert_eq!(
        extract_archive(&archive(&[record(path.as_bytes(), b'0', b"")]), DEFAULT).unwrap(),
        vec![memory(&path, b"")]
    );
}

#[test]
fn rejects_duplicates_and_file_directory_conflicts_after_normalization() {
    for names in [
        [("a", b'0'), ("./a", b'0')],
        [("a", b'0'), ("A", b'0')],
        [("folder/", b'5'), ("folder", b'5')],
        [("a", b'0'), ("a/b", b'0')],
        [("a/b", b'0'), ("a", b'0')],
        [("a/", b'5'), ("a", b'0')],
        [("./", b'5'), (".", b'5')],
    ] {
        let bytes = archive(&names.map(|(name, kind)| record(name.as_bytes(), kind, b"")));
        assert!(
            matches!(list_archive(&bytes, DEFAULT), Err(Error::DuplicatePath(_))),
            "{names:?}"
        );
        assert_rejected(&bytes);
    }
}

#[test]
fn directory_and_child_are_allowed_in_either_order() {
    for records in [
        vec![record(b"a", b'5', b""), record(b"a/b", b'0', b"")],
        vec![record(b"a/b", b'0', b""), record(b"a/", b'5', b"")],
    ] {
        assert_eq!(
            extract_archive(&archive(&records), DEFAULT).unwrap().len(),
            2
        );
    }
}

#[test]
fn rejects_links_special_and_sparse_types_even_when_unselected() {
    for kind in [
        b'1', b'2', b'3', b'4', b'6', b'7', b'S', b'K', b'D', b'V', b'Z', 255,
    ] {
        let bytes = archive(&[
            record(b"good", b'0', b"ok"),
            record(b"unsupported", kind, b""),
        ]);
        assert!(
            matches!(
                extract_selected_archive(&bytes, &["good".into()], DEFAULT),
                Err(Error::UnsupportedEntry(_))
            ),
            "{kind}"
        );
        assert_rejected(&bytes);
    }
    assert_rejected(&archive(&[record(b"dir/", b'5', b"payload")]));
    for key in [
        "GNU.sparse.map",
        "GNU.sparse.major",
        "SCHILY.realsize",
        "SCHILY.filetype",
        "linkpath",
        "hdrcharset",
    ] {
        let bytes = archive(&[
            record(b"PaxHeader", b'x', &pax_pair(key, b"1")),
            record(b"file", b'0', b""),
        ]);
        assert!(
            matches!(
                list_archive(&bytes, DEFAULT),
                Err(Error::UnsupportedEntry(_))
            ),
            "{key}"
        );
    }
}

#[test]
fn checks_every_header_payload_padding_and_strict_end_marker() {
    let first = record(b"first", b'0', b"abc");
    let second = record(b"second", b'0', b"def");
    let bytes = archive(&[first.clone(), second]);
    for cut in [
        1,
        511,
        512,
        513,
        1023,
        1024,
        1537,
        2048,
        2560,
        bytes.len() - 1,
    ] {
        assert_rejected(&bytes[..cut]);
    }
    let mut bad = bytes.clone();
    bad[1024] ^= 1;
    assert!(matches!(
        list_archive(&bad, DEFAULT),
        Err(Error::InvalidTar(_))
    ));
    assert_rejected(&bad);
    let mut bad = bytes.clone();
    bad[0] ^= 1;
    assert_rejected(&bad);
    for tail in [record(b"hidden", b'0', b"secret"), vec![1], vec![0]] {
        assert_rejected(&[bytes.as_slice(), &tail].concat());
    }
    let mut single_zero_then_hidden = first;
    single_zero_then_hidden.resize(single_zero_then_hidden.len() + 512, 0);
    single_zero_then_hidden.extend(record(b"hidden", b'0', b"secret"));
    assert_rejected(&archive(&[single_zero_then_hidden]));
    assert_rejected(&[bytes.clone(), bytes].concat());
}

#[test]
fn selection_includes_subtrees_once_and_checks_unknown_names() {
    let input = [
        memory("folder/", b""),
        memory("folder/file", b"one"),
        memory("folder/deep/file", b"two"),
        memory("folder-other/file", b"three"),
        memory("other", b"four"),
    ];
    let bytes = archive(
        &input
            .iter()
            .map(|e| {
                record(
                    e.path.as_bytes(),
                    if e.is_dir { b'5' } else { b'0' },
                    &e.data,
                )
            })
            .collect::<Vec<_>>(),
    );
    assert_eq!(
        extract_selected_archive(
            &bytes,
            &["folder/".into(), "folder/file".into(), "folder/".into()],
            DEFAULT
        )
        .unwrap(),
        input[..3]
    );
    assert_eq!(
        extract_selected_archive(&bytes, &["other".into(), "folder/file".into()], DEFAULT).unwrap(),
        vec![input[1].clone(), input[4].clone()]
    );
    assert!(
        extract_selected_archive(&bytes, &[], DEFAULT)
            .unwrap()
            .is_empty()
    );
    for path in ["folder", "./folder/", "FOLDER/", "missing", "../escape"] {
        assert!(matches!(
            extract_selected_archive(&bytes, &[path.into()], DEFAULT),
            Err(Error::UnknownEntry(_))
        ));
    }
}

#[test]
fn all_limits_apply_to_unselected_entries() {
    let bytes = archive(&[
        record(b"one", b'0', b"12345"),
        record(b"two", b'0', b"67890"),
    ]);
    for limits in [
        Limits {
            max_archive_bytes: bytes.len() as u64 - 1,
            ..DEFAULT
        },
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
        assert!(
            matches!(list_archive(&bytes, limits), Err(Error::Limit(_))),
            "{limits:?}"
        );
        assert!(
            matches!(
                extract_selected_archive(&bytes, &[], limits),
                Err(Error::Limit(_))
            ),
            "{limits:?}"
        );
    }
    assert_eq!(
        extract_archive(
            &bytes,
            Limits {
                max_archive_bytes: bytes.len() as u64,
                max_entries: 2,
                max_output_items: peazip_core::MAX_EXTRACTED_ITEMS,
                max_path_bytes: 3,
                max_entry_bytes: 5,
                max_total_bytes: 10,
                max_compression_ratio: 1
            }
        )
        .unwrap()
        .len(),
        2
    );
}

#[test]
fn implicit_parent_output_budget_is_shared_with_zip() {
    let bytes = archive(&[
        record(b"a/b/file", b'0', b"deep"),
        record(b"plain", b'0', b"ok"),
    ]);
    let limits = Limits {
        max_entries: 2,
        max_output_items: 2,
        ..DEFAULT
    };
    assert!(list_archive(&bytes, limits).is_ok());
    assert!(matches!(
        extract_selected_archive(&bytes, &["a/b/file".into()], limits),
        Err(Error::Limit("expanded filesystem items"))
    ));
    assert_eq!(
        extract_selected_archive(&bytes, &["plain".into()], limits).unwrap(),
        vec![memory("plain", b"ok")]
    );
    let bytes = archive(&[record(b"a/x", b'0', b""), record(b"a/y", b'0', b"")]);
    assert_eq!(
        extract_archive(
            &bytes,
            Limits {
                max_entries: 3,
                ..DEFAULT
            }
        )
        .unwrap()
        .len(),
        2
    );
}

#[test]
fn pax_paths_sizes_and_global_informational_metadata() {
    let name = format!("folder/{}.txt", "中文".repeat(30));
    let mut metadata = pax_pair("path", name.as_bytes());
    metadata.extend(pax_pair("size", b"5"));
    metadata.extend(pax_pair("comment", b"including\nnewlines"));
    // PAX size is authoritative even when the base header declares zero bytes.
    let mut file = record(b"placeholder", b'0', b"");
    file.extend_from_slice(b"hello");
    file.resize(1024, 0);
    let bytes = archive(&[
        record(b"Global", b'g', &pax_pair("mtime", b"123.456")),
        record(b"Pax", b'x', &metadata),
        file,
        record(b"next", b'0', b"next"),
    ]);
    assert_eq!(
        extract_archive(&bytes, DEFAULT).unwrap(),
        vec![memory(&name, b"hello"), memory("next", b"next")]
    );
    for key in ["path", "size"] {
        assert_rejected(&archive(&[
            record(b"Global", b'g', &pax_pair(key, b"1")),
            record(b"file", b'0', b""),
        ]));
    }
}

#[test]
fn gnu_long_names_use_the_same_path_and_conflict_rules() {
    let name = format!("./folder/{}", "n".repeat(150));
    let long = record(b"././@LongLink", b'L', &[name.as_bytes(), &[0]].concat());
    let bytes = archive(&[long.clone(), record(b"placeholder", b'0', b"long")]);
    assert_eq!(
        extract_archive(&bytes, DEFAULT).unwrap(),
        vec![memory(&name[2..], b"long")]
    );
    assert_rejected(&archive(std::slice::from_ref(&long)));
    assert_rejected(&archive(&[
        long.clone(),
        long,
        record(b"placeholder", b'0', b""),
    ]));
    for kind in *b"Lx" {
        for name in [b"../escape".as_slice(), b"/abs", b"a\\b", b"a\0b", b"\xff"] {
            let payload = if kind == b'L' {
                [name, &[0]].concat()
            } else {
                pax_pair("path", name)
            };
            assert_rejected(&archive(&[
                record(b"metadata", kind, &payload),
                record(b"file", b'0', b""),
            ]));
        }
        let payload = if kind == b'L' {
            b"./file\0".to_vec()
        } else {
            pax_pair("path", b"./file")
        };
        assert_rejected(&archive(&[
            record(b"metadata", kind, &payload),
            record(b"placeholder", b'0', b""),
            record(b"file", b'0', b""),
        ]));
    }
}

#[test]
fn malformed_and_orphaned_pax_cannot_hide_tail_data() {
    for payload in [
        b"bad".to_vec(),
        b"0 path=x\n".to_vec(),
        b"999999999999999999999999999 path=x\n".to_vec(),
        b"10 path=x".to_vec(),
        b"10 path=x\nextra".to_vec(),
        pax_pair("size", b"-1"),
        pax_pair("size", b"18446744073709551616"),
        pax_pair("path", b""),
        [pax_pair("path", b"a"), pax_pair("path", b"b")].concat(),
        [pax_pair("size", b"0"), pax_pair("size", b"1")].concat(),
        [pax_pair("comment", b"ok"), b"\0hidden".to_vec()].concat(),
    ] {
        assert_rejected(&archive(&[
            record(b"Pax", b'x', &payload),
            record(b"file", b'0', b""),
        ]));
    }
    let pax = record(b"Pax", b'x', &pax_pair("mtime", b"1"));
    assert_rejected(&archive(std::slice::from_ref(&pax)));
    assert_rejected(&archive(&[pax.clone(), pax, record(b"file", b'0', b"")]));
}

#[test]
fn extension_bytes_paths_and_counts_are_bounded_before_interpretation() {
    for kind in *b"Lxg" {
        let mut huge = tar::Header::new_ustar();
        huge.set_entry_type(tar::EntryType::new(kind));
        huge.set_size(1024 * 1024 * 1024);
        huge.set_cksum();
        assert!(matches!(
            list_archive(&archive(&[huge.as_bytes().to_vec()]), DEFAULT),
            Err(Error::Limit("entry bytes"))
        ));
        let bytes = archive(&[
            record(b"metadata", kind, &[b'x'; 65]),
            record(b"file", b'0', b""),
        ]);
        assert!(matches!(
            list_archive(
                &bytes,
                Limits {
                    max_entry_bytes: 64,
                    ..DEFAULT
                }
            ),
            Err(Error::Limit("entry bytes"))
        ));
    }
    for (kind, data) in [
        (b'L', vec![b'x'; 20]),
        (b'x', pax_pair("path", &[b'x'; 20])),
    ] {
        let bytes = archive(&[record(b"metadata", kind, &data), record(b"file", b'0', b"")]);
        assert!(matches!(
            list_archive(
                &bytes,
                Limits {
                    max_path_bytes: 19,
                    ..DEFAULT
                }
            ),
            Err(Error::Limit("path bytes"))
        ));
        assert!(matches!(
            list_archive(
                &bytes,
                Limits {
                    max_total_bytes: data.len() as u64 - 1,
                    ..DEFAULT
                }
            ),
            Err(Error::Limit("total bytes"))
        ));
        assert!(matches!(
            list_archive(
                &bytes,
                Limits {
                    max_entries: 1,
                    ..DEFAULT
                }
            ),
            Err(Error::Limit("entry count"))
        ));
    }
    let bytes = archive(&[record(b"./", b'5', b""), record(b"file", b'0', b"")]);
    assert!(matches!(
        list_archive(
            &bytes,
            Limits {
                max_entries: 1,
                ..DEFAULT
            }
        ),
        Err(Error::Limit("entry count"))
    ));
}

#[test]
fn tar_has_no_payload_checksum_guarantee() {
    let mut bytes = archive(&[record(b"file", b'0', b"original")]);
    bytes[512] = b'X';
    assert_eq!(
        extract_archive(&bytes, DEFAULT).unwrap(),
        vec![memory("file", b"Xriginal")]
    );
}

#[test]
fn shared_error_messages_are_format_neutral() {
    for (error, prefix) in [
        (Error::Limit("entry count"), "Archive limit exceeded:"),
        (Error::UnsafePath("x".into()), "unsafe archive path:"),
        (
            Error::DuplicatePath("x".into()),
            "duplicate or conflicting archive path:",
        ),
        (
            Error::UnsupportedEntry("x".into()),
            "unsupported archive entry:",
        ),
        (Error::UnknownEntry("x".into()), "archive entry not found:"),
        (
            Error::Io(std::io::Error::other("test")),
            "archive I/O error:",
        ),
        (
            Error::InvalidTar(std::io::Error::other("test")),
            "invalid TAR:",
        ),
    ] {
        assert!(error.to_string().starts_with(prefix));
    }
    assert_eq!(
        Error::UnsupportedFormat.to_string(),
        "unsupported archive format"
    );
}
