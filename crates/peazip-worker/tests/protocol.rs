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
