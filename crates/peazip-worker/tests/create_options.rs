use peazip_core::{Limits, extract_zip};
use peazip_worker::{MAX_CREATE_V1_REQUEST_BYTES, create_from_request, create_v1_from_request};

fn legacy(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut packet = (entries.len() as u32).to_le_bytes().to_vec();
    for (name, data) in entries {
        packet.extend_from_slice(&(name.len() as u32).to_le_bytes());
        packet.extend_from_slice(&(data.len() as u32).to_le_bytes());
        packet.extend_from_slice(name.as_bytes());
        packet.extend_from_slice(data);
    }
    packet
}

fn versioned(compression: u32, level: u32, entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut packet = 1u32.to_le_bytes().to_vec();
    packet.extend_from_slice(&compression.to_le_bytes());
    packet.extend_from_slice(&level.to_le_bytes());
    packet.extend_from_slice(&legacy(entries));
    packet
}

#[test]
fn default_protocol_is_unchanged_and_explicit_methods_roundtrip_real_bytes() {
    let data: Vec<u8> = (0..4096).map(|i| (i % 251) as u8).collect();
    let entries = [
        ("目录/你好.bin", data.as_slice()),
        ("empty", b"".as_slice()),
    ];
    let old = create_from_request(&legacy(&entries)).unwrap();
    assert_eq!(
        create_v1_from_request(&versioned(0, 0, &entries)).unwrap(),
        old
    );
    let mut sizes = Vec::new();
    for (compression, level) in [(1, 0)].into_iter().chain((1..=9).map(|level| (2, level))) {
        let archive = create_v1_from_request(&versioned(compression, level, &entries)).unwrap();
        assert_eq!(&archive[..4], b"PK\x03\x04");
        let method = u16::from_le_bytes(archive[8..10].try_into().unwrap());
        assert_eq!(method, if compression == 1 { 0 } else { 8 });
        let actual = extract_zip(&archive, Limits::default()).unwrap();
        for (actual, (name, data)) in actual.iter().zip(entries) {
            assert_eq!(actual.path, name);
            assert_eq!(actual.data, data);
        }
        assert_eq!(actual.len(), entries.len());
        if compression == 2 {
            sizes.push(archive.len());
        }
    }
    assert!(
        sizes[0] > sizes[8],
        "level 9 must compress the fixture more than level 1"
    );
}

#[test]
fn options_are_checked_before_file_packet_and_unknown_or_truncated_input_is_rejected() {
    for (compression, level) in [
        (0, 1),
        (1, 9),
        (2, 0),
        (2, 10),
        (2, 257),
        (3, 0),
        (u32::MAX, 0),
    ] {
        let packet = versioned(compression, level, &[]);
        assert!(
            create_v1_from_request(&packet[..12])
                .unwrap_err()
                .contains("options")
        );
        assert!(
            create_v1_from_request(&packet)
                .unwrap_err()
                .contains("options")
        );
    }
    let valid = versioned(2, 6, &[("file", b"bytes")]);
    for end in 0..valid.len() {
        assert!(
            create_v1_from_request(&valid[..end]).is_err(),
            "prefix {end}"
        );
    }
    let mut wrong_version = valid.clone();
    wrong_version[..4].copy_from_slice(&2u32.to_le_bytes());
    assert!(
        create_v1_from_request(&wrong_version)
            .unwrap_err()
            .contains("version")
    );
    let mut trailing = valid;
    trailing.push(0);
    assert!(
        create_v1_from_request(&trailing)
            .unwrap_err()
            .contains("trailing")
    );
    assert!(
        create_v1_from_request(&vec![0; MAX_CREATE_V1_REQUEST_BYTES + 1])
            .unwrap_err()
            .contains("request byte")
    );
    assert!(create_v1_from_request(&legacy(&[("a", b"b")])).is_err());
    assert!(create_from_request(&versioned(1, 0, &[("a", b"b")])).is_err());
}
