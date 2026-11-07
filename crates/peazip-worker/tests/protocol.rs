use peazip_core::{Limits, extract_zip};
use peazip_worker::{MAX_REQUEST_BYTES, create_from_request};

fn packet(entries: &[(&[u8], &[u8])]) -> Vec<u8> {
    let mut bytes = (entries.len() as u32).to_le_bytes().to_vec();
    for (path, data) in entries {
        bytes.extend((path.len() as u32).to_le_bytes());
        bytes.extend((data.len() as u32).to_le_bytes());
        bytes.extend(*path);
        bytes.extend(*data);
    }
    bytes
}

#[test]
fn extraction_request_detects_tar_and_returns_real_file_bytes() {
    let archive = include_bytes!("../../../tests/fixtures/sample.tar");
    let name = b"docs/";
    let mut request = 1u32.to_le_bytes().to_vec();
    request.extend_from_slice(&(name.len() as u32).to_le_bytes());
    request.extend_from_slice(name);
    request.extend_from_slice(&(archive.len() as u32).to_le_bytes());
    request.extend_from_slice(archive);
    let output = peazip_worker::extract_from_request(&request).unwrap();
    let mut at = 0;
    fn number(bytes: &[u8], at: &mut usize) -> usize {
        let value = u32::from_le_bytes(bytes[*at..*at + 4].try_into().unwrap()) as usize;
        *at += 4;
        value
    }
    assert_eq!(number(&output, &mut at), 4);
    let mut files = std::collections::BTreeMap::new();
    for _ in 0..4 {
        let kind = number(&output, &mut at);
        let path_len = number(&output, &mut at);
        let data_len = number(&output, &mut at);
        let path = std::str::from_utf8(&output[at..at + path_len])
            .unwrap()
            .to_owned();
        at += path_len;
        let data = output[at..at + data_len].to_vec();
        at += data_len;
        files.insert(path, (kind, data));
    }
    assert_eq!(at, output.len());
    assert_eq!(
        files["docs/你好.txt"],
        (0, "tar 你好\n".as_bytes().to_vec())
    );
    assert_eq!(files["docs/nested/data.bin"], (0, vec![0, 127, 255]));
    assert_eq!(files["docs/empty/"], (1, vec![]));
    assert!(!files.contains_key("other.txt"));
}

#[test]
fn roundtrip_chinese_and_empty_files() {
    let request = packet(&[("目录/中文.txt".as_bytes(), b"hello"), (b"empty", b"")]);
    let zip = create_from_request(&request).unwrap();
    let entries = extract_zip(&zip, Limits::default()).unwrap();
    assert_eq!(entries[0].path, "目录/中文.txt");
    assert_eq!(entries[0].data, b"hello");
    assert_eq!(entries[1].path, "empty");
    assert!(entries[1].data.is_empty());
    assert!(entries.iter().all(|entry| !entry.is_dir));
}

#[test]
fn malformed_and_limits() {
    let valid = packet(&[(b"a", b"abc")]);
    for end in 0..valid.len() {
        assert!(create_from_request(&valid[..end]).is_err(), "prefix {end}");
    }
    for count in [0u32, 4097, u32::MAX] {
        assert!(create_from_request(&count.to_le_bytes()).is_err());
    }
    for path in [b"".as_slice(), &[0xff], b"dir/", b"../bad", b"/absolute"] {
        assert!(create_from_request(&packet(&[(path, b"")])).is_err());
    }
    assert!(create_from_request(&packet(&[(&vec![b'a'; 1025], b"")])).is_err());
    let mut oversized = packet(&[(b"a", b"")]);
    oversized[8..12].copy_from_slice(&(16 * 1024 * 1024 + 1u32).to_le_bytes());
    assert!(
        create_from_request(&oversized)
            .unwrap_err()
            .contains("entry byte")
    );
    assert!(
        create_from_request(&vec![0; MAX_REQUEST_BYTES + 1])
            .unwrap_err()
            .contains("request byte")
    );
    let data = vec![0; 16 * 1024 * 1024];
    let mut total = packet(&[(b"a", &data), (b"b", &data), (b"c", &data), (b"d", &data)]);
    total[..4].copy_from_slice(&5u32.to_le_bytes());
    total.extend(1u32.to_le_bytes());
    total.extend(1u32.to_le_bytes());
    total.extend(b"e!");
    assert!(
        create_from_request(&total)
            .unwrap_err()
            .contains("total byte")
    );
}

#[test]
fn trailing_and_duplicates() {
    let mut trailing = packet(&[(b"a", b"")]);
    trailing.push(0);
    assert!(
        create_from_request(&trailing)
            .unwrap_err()
            .contains("trailing")
    );
    for paths in [
        (b"a".as_slice(), b"a".as_slice()),
        (b"A", b"a"),
        (b"a", b"a/b"),
    ] {
        assert!(
            create_from_request(&packet(&[(paths.0, b""), (paths.1, b"")]))
                .unwrap_err()
                .contains("duplicate")
        );
    }
}

fn extract_packet(paths: &[&[u8]], zip: &[u8]) -> Vec<u8> {
    let mut bytes = (paths.len() as u32).to_le_bytes().to_vec();
    for path in paths {
        bytes.extend((path.len() as u32).to_le_bytes());
        bytes.extend(*path);
    }
    bytes.extend((zip.len() as u32).to_le_bytes());
    bytes.extend(zip);
    bytes
}

fn decode_extract(mut bytes: &[u8]) -> Vec<peazip_core::MemoryEntry> {
    fn number(bytes: &mut &[u8]) -> usize {
        let (head, rest) = bytes.split_at(4);
        *bytes = rest;
        u32::from_le_bytes(head.try_into().unwrap()) as usize
    }
    let count = number(&mut bytes);
    let mut entries = Vec::new();
    for _ in 0..count {
        let kind = number(&mut bytes);
        assert!(kind <= 1);
        let path_len = number(&mut bytes);
        let data_len = number(&mut bytes);
        let (path, rest) = bytes.split_at(path_len);
        let (data, rest) = rest.split_at(data_len);
        entries.push(peazip_core::MemoryEntry {
            path: std::str::from_utf8(path).unwrap().into(),
            is_dir: kind == 1,
            data: data.into(),
        });
        bytes = rest;
    }
    assert!(bytes.is_empty());
    entries
}

#[test]
fn extract_subtree_empty_directories_and_actual_bytes() {
    use peazip_core::{MemoryEntry, create_zip};
    use peazip_worker::extract_from_request;
    let entries: Vec<_> = [
        ("目录/", b"".as_slice()),
        ("目录/中文", &[0, 255, 128, 1]),
        ("目录/nested/file", b"nested"),
        ("目录/empty/", b""),
        ("empty/", b""),
        ("目录-other/file", b"excluded"),
    ]
    .into_iter()
    .map(|(path, data)| MemoryEntry {
        path: path.into(),
        is_dir: path.ends_with('/'),
        data: data.into(),
    })
    .collect();
    let zip = create_zip(&entries, Limits::default()).unwrap();
    let output = extract_from_request(&extract_packet(
        &["目录/".as_bytes(), "目录/中文".as_bytes(), b"empty/"],
        &zip,
    ))
    .unwrap();
    assert_eq!(decode_extract(&output), entries[..5]);
    assert_eq!(
        decode_extract(&extract_from_request(&extract_packet(&[b"empty/"], &zip)).unwrap()),
        entries[4..5]
    );
}

#[test]
fn extract_rejects_malformed_and_oversized_requests() {
    use peazip_worker::{MAX_EXTRACT_REQUEST_BYTES, extract_from_request};
    let zip = create_from_request(&packet(&[(b"a", b"abc")])).unwrap();
    let valid = extract_packet(&[b"a"], &zip);
    for end in 0..valid.len() {
        assert!(extract_from_request(&valid[..end]).is_err(), "prefix {end}");
    }
    for count in [0u32, 4097, u32::MAX] {
        assert!(extract_from_request(&count.to_le_bytes()).is_err());
    }
    for path in [
        b"".as_slice(),
        &[0xff],
        b"../bad",
        b"/absolute",
        b"missing",
        &vec![b'a'; 1025],
    ] {
        assert!(extract_from_request(&extract_packet(&[path], &zip)).is_err());
    }
    let mut trailing = valid.clone();
    trailing.push(0);
    assert!(
        extract_from_request(&trailing)
            .unwrap_err()
            .contains("trailing")
    );
    let mut oversized = valid;
    oversized[9..13].copy_from_slice(&(32 * 1024 * 1024 + 1u32).to_le_bytes());
    assert!(
        extract_from_request(&oversized)
            .unwrap_err()
            .contains("archive byte")
    );
    assert!(
        extract_from_request(&vec![0; MAX_EXTRACT_REQUEST_BYTES + 1])
            .unwrap_err()
            .contains("request byte")
    );
    assert!(extract_from_request(&extract_packet(&[b"a"], b"not ZIP")).is_err());
}

#[test]
fn extract_rejects_bad_archive_paths_and_crc_without_partial_results() {
    use peazip_worker::extract_from_request;
    let mut unsafe_zip = create_from_request(&packet(&[(b"safe", b"abc")])).unwrap();
    for at in (0..unsafe_zip.len() - 3).collect::<Vec<_>>() {
        if &unsafe_zip[at..at + 4] == b"safe" {
            unsafe_zip[at..at + 4].copy_from_slice(b"../x");
        }
    }
    assert!(
        extract_from_request(&extract_packet(&[b"../x"], &unsafe_zip))
            .unwrap_err()
            .contains("unsafe")
    );
    let mut zip = create_from_request(&packet(&[(b"good", b"good"), (b"bad", b"bad!")])).unwrap();
    let central = zip
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == b"PK\x01\x02")
        .map(|(i, _)| i)
        .nth(1)
        .unwrap();
    let local = u32::from_le_bytes(zip[central + 42..central + 46].try_into().unwrap()) as usize;
    zip[local + 14..local + 18].fill(0);
    zip[central + 16..central + 20].fill(0);
    assert_eq!(
        decode_extract(&extract_from_request(&extract_packet(&[b"good"], &zip)).unwrap())[0].data,
        b"good"
    );
    assert!(extract_from_request(&extract_packet(&[b"good", b"bad"], &zip)).is_err());
}

const SEVEN_ZIP: &[u8] = include_bytes!("../../../tests/fixtures/sample.7z");

#[test]
fn extracts_solid_lzma2_7z_subtree_with_encoded_header() {
    let output =
        peazip_worker::extract_from_request(&extract_packet(&[b"docs/"], SEVEN_ZIP)).unwrap();
    let entries = decode_extract(&output);
    assert_eq!(entries.len(), 5);
    let actual: std::collections::BTreeMap<_, _> = entries
        .into_iter()
        .map(|entry| (entry.path, (entry.is_dir, entry.data)))
        .collect();
    let expected: std::collections::BTreeMap<_, _> = [
        ("docs/", (true, vec![])),
        ("docs/empty/", (true, vec![])),
        ("docs/nested/", (true, vec![])),
        ("docs/nested/data.bin", (false, vec![0, 127, 255])),
        ("docs/你好.txt", (false, "7z 你好\n".as_bytes().to_vec())),
    ]
    .into_iter()
    .map(|(path, data)| (path.to_owned(), data))
    .collect();
    assert_eq!(actual, expected);
}

#[test]
fn rejects_truncated_7z_bad_header_crc_and_malformed_next_headers_without_partial_output() {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
            }
        }
        !crc
    }
    fn fix_start_crc(bytes: &mut [u8]) {
        let crc = crc32(&bytes[12..32]);
        bytes[8..12].copy_from_slice(&crc.to_le_bytes());
    }
    let next = 32 + u64::from_le_bytes(SEVEN_ZIP[12..20].try_into().unwrap()) as usize;
    assert_eq!(SEVEN_ZIP[next], 0x17, "fixture must use an encoded header");
    assert_eq!(crc32(&SEVEN_ZIP[12..32]).to_le_bytes(), SEVEN_ZIP[8..12]);
    assert_eq!(crc32(&SEVEN_ZIP[next..]).to_le_bytes(), SEVEN_ZIP[28..32]);
    let mut corruptions = Vec::new();
    let mut bad_start_crc = SEVEN_ZIP.to_vec();
    bad_start_crc[8] ^= 1;
    corruptions.push(bad_start_crc);
    let mut bad_next_crc = SEVEN_ZIP.to_vec();
    bad_next_crc[next] ^= 1;
    corruptions.push(bad_next_crc);
    let mut malformed = SEVEN_ZIP.to_vec();
    malformed[next] = 0xff;
    let crc = crc32(&malformed[next..]);
    malformed[28..32].copy_from_slice(&crc.to_le_bytes());
    fix_start_crc(&mut malformed);
    corruptions.push(malformed);
    for field in [12..20, 20..28] {
        let mut oversized = SEVEN_ZIP.to_vec();
        oversized[field].copy_from_slice(&u64::MAX.to_le_bytes());
        fix_start_crc(&mut oversized);
        corruptions.push(oversized);
    }
    for archive in (0..SEVEN_ZIP.len())
        .map(|end| &SEVEN_ZIP[..end])
        .chain(corruptions.iter().map(Vec::as_slice))
    {
        assert!(peazip_worker::list_from_request(archive).is_err());
        assert!(
            peazip_worker::extract_from_request(&extract_packet(&[b"docs/"], archive)).is_err()
        );
    }
    assert!(peazip_worker::list_from_request(SEVEN_ZIP).is_ok());
    assert!(peazip_worker::extract_from_request(&extract_packet(&[b"docs/"], SEVEN_ZIP)).is_ok());
}
