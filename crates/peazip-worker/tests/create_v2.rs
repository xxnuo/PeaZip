use peazip_core::{ArchiveFormat, Limits};
use peazip_worker::{
    MAX_CREATE_V2_REQUEST_BYTES, create_from_request, create_v1_from_request,
    create_v2_from_request, list_from_request, test_from_request,
};

fn legacy(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut packet = (entries.len() as u32).to_le_bytes().to_vec();
    for (path, bytes) in entries {
        packet.extend_from_slice(&(path.len() as u32).to_le_bytes());
        packet.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        packet.extend_from_slice(path.as_bytes());
        packet.extend_from_slice(bytes);
    }
    packet
}
fn header(format: u32, compression: u32, level: u32) -> Vec<u8> {
    [2, format, compression, level]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect()
}
fn packet(format: u32, compression: u32, level: u32, entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut bytes = header(format, compression, level);
    bytes.extend(legacy(entries));
    bytes
}

#[test]
fn default_zip_is_byte_compatible_with_legacy_and_v1_and_explicit_methods_are_preserved() {
    let entries = [("目录/你好.bin", b"hello".as_slice()), ("empty", b"")];
    let old = legacy(&entries);
    let mut v1 = [1u32, 0, 0]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>();
    v1.extend(&old);
    assert_eq!(
        create_v2_from_request(&packet(0, 0, 0, &entries)).unwrap(),
        create_from_request(&old).unwrap()
    );
    assert_eq!(
        create_v2_from_request(&packet(0, 0, 0, &entries)).unwrap(),
        create_v1_from_request(&v1).unwrap()
    );
    for (compression, level, method) in [(1, 0, 0u16), (2, 1, 8), (2, 9, 8)] {
        let zip = create_v2_from_request(&packet(0, compression, level, &entries)).unwrap();
        assert_eq!(u16::from_le_bytes(zip[8..10].try_into().unwrap()), method);
        let selected: Vec<String> = entries.iter().map(|(path, _)| (*path).into()).collect();
        let output =
            peazip_core::extract_selected_archive(&zip, &selected, Limits::default()).unwrap();
        for (actual, (name, bytes)) in output.iter().zip(entries) {
            assert_eq!(actual.path, name);
            assert_eq!(actual.data, bytes);
        }
    }
}

#[test]
fn real_tar_and_gzip_outputs_preserve_unicode_binary_empty_files_and_deterministic_headers() {
    let entries = [
        ("dir/你好.txt", "tar 你好\n".as_bytes()),
        ("dir/data.bin", &[0, 127, 255]),
        ("empty", b""),
    ];
    for (format, level) in [(1, 0)].into_iter().chain((1..=9).map(|level| (2, level))) {
        let request = packet(format, 0, level, &entries);
        let bytes = create_v2_from_request(&request).unwrap();
        assert_eq!(create_v2_from_request(&request).unwrap(), bytes);
        let (kind, metadata) =
            peazip_core::list_archive_with_format(&bytes, Limits::default()).unwrap();
        assert_eq!(
            kind,
            if format == 1 {
                ArchiveFormat::Tar
            } else {
                ArchiveFormat::TarGzip
            }
        );
        assert_eq!(metadata.len(), entries.len());
        for entry in &metadata {
            assert_eq!(
                entry.packed_size,
                if format == 1 {
                    Some(entry.size.div_ceil(512) * 512)
                } else {
                    None
                }
            );
        }
        let paths: Vec<String> = entries.iter().map(|(name, _)| (*name).into()).collect();
        let files =
            peazip_core::extract_selected_archive(&bytes, &paths, Limits::default()).unwrap();
        assert_eq!(files.len(), entries.len());
        for (actual, (name, data)) in files.iter().zip(entries) {
            assert_eq!(actual.path, name);
            assert_eq!(actual.data, data);
        }
        let report = peazip_core::test_archive(&bytes, Limits::default()).unwrap();
        assert_eq!(report.files, entries.len());
        assert_eq!(
            report.checksummed_files,
            if format == 1 { 0 } else { entries.len() }
        );
        let wire = list_from_request(&bytes).unwrap();
        assert_eq!(&wire[..4], b"PZL2");
        assert_eq!(wire[4], format as u8);
        if format == 1 {
            let checksum = std::str::from_utf8(&bytes[148..156])
                .unwrap()
                .trim_matches(['\0', ' ']);
            let expected = u64::from_str_radix(checksum, 8).unwrap();
            let actual: u64 = bytes[..512]
                .iter()
                .enumerate()
                .map(|(at, byte)| {
                    u64::from(if (148..156).contains(&at) {
                        b' '
                    } else {
                        *byte
                    })
                })
                .sum();
            assert_eq!(actual, expected);
            assert_eq!(&bytes[512..512 + entries[0].1.len()], entries[0].1);
            assert!(bytes.ends_with(&[0; 1024]));
        } else {
            assert_eq!(&bytes[..3], &[0x1f, 0x8b, 8]);
            assert_eq!(&bytes[4..8], &[0; 4]);
            let mut bad = bytes.clone();
            let crc = bad.len() - 8;
            bad[crc] ^= 1;
            assert!(list_from_request(&bad).is_err());
            assert!(test_from_request(&bad).is_err());
            assert!(
                peazip_core::extract_selected_archive(&bad, &paths, Limits::default()).is_err()
            );
        }
    }
}

#[test]
fn version_format_unused_fields_and_levels_are_rejected_before_file_packet() {
    for (format, compression, level) in [
        (0, 0, 1),
        (0, 1, 1),
        (0, 2, 0),
        (0, 2, 10),
        (0, 2, 257),
        (0, 3, 0),
        (1, 1, 0),
        (1, 0, 1),
        (2, 1, 6),
        (2, 0, 0),
        (2, 0, 10),
        (2, 0, 257),
        (3, 0, 0),
        (u32::MAX, 0, 0),
    ] {
        assert!(
            create_v2_from_request(&header(format, compression, level))
                .unwrap_err()
                .contains("options")
        );
    }
    let valid = packet(1, 0, 0, &[("file", b"data")]);
    for end in 0..valid.len() {
        assert!(
            create_v2_from_request(&valid[..end]).is_err(),
            "prefix {end}"
        );
    }
    for version in [0u32, 1, 3, u32::MAX] {
        let mut bad = valid.clone();
        bad[..4].copy_from_slice(&version.to_le_bytes());
        assert!(
            create_v2_from_request(&bad)
                .unwrap_err()
                .contains("version")
        );
    }
    let mut trailing = valid;
    trailing.push(0);
    assert!(
        create_v2_from_request(&trailing)
            .unwrap_err()
            .contains("trailing")
    );
    assert!(
        create_v2_from_request(&vec![0; MAX_CREATE_V2_REQUEST_BYTES + 1])
            .unwrap_err()
            .contains("request byte")
    );
}

#[test]
fn all_formats_reapply_file_count_path_and_payload_limits() {
    for (format, level) in [(0, 0), (1, 0), (2, 6)] {
        for count in [0u32, 4097, u32::MAX] {
            let mut request = header(format, 0, level);
            request.extend(count.to_le_bytes());
            assert!(
                create_v2_from_request(&request)
                    .unwrap_err()
                    .contains("count")
            );
        }
        for path in ["../bad", "/absolute", "dir/", "", "AUX.txt"] {
            assert!(create_v2_from_request(&packet(format, 0, level, &[(path, b"")])).is_err());
        }
        let mut request = packet(format, 0, level, &[("file", b"")]);
        request[24..28].copy_from_slice(&(16 * 1024 * 1024 + 1u32).to_le_bytes());
        assert!(
            create_v2_from_request(&request)
                .unwrap_err()
                .contains("entry byte")
        );
    }
}

#[test]
fn gzip_compression_ratio_failure_is_not_silently_changed_to_tar_or_zip() {
    let bytes = vec![0u8; 1024 * 1024];
    assert!(
        create_v2_from_request(&packet(2, 0, 9, &[("zeros", &bytes)]))
            .unwrap_err()
            .contains("ratio")
    );
    let recovered = create_v2_from_request(&packet(2, 0, 6, &[("file", b"recovered")])).unwrap();
    assert_eq!(&recovered[..3], &[0x1f, 0x8b, 8]);
    assert!(test_from_request(&recovered).is_ok());
}
