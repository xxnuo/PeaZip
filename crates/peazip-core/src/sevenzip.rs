use super::{
    Entry, Error, Limits, MemoryEntry, TestReport, check, insert_path, path_key, selected_indices,
    test_report,
};
use flate2::Crc;
use sevenz_turbo::{Archive, ArchiveLimits, ArchiveReader, EncoderMethod, Password};
use std::{
    collections::BTreeMap,
    io::{self, Cursor, Read},
};

pub(super) const SIGNATURE: &[u8] = b"7z\xbc\xaf\x27\x1c";
const MAX_HEADER_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MEMORY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_OBJECTS: u64 = 65536;
type Reader<'a> = ArchiveReader<Cursor<&'a [u8]>>;

fn invalid(message: &'static str) -> Error {
    Error::InvalidSevenZip(io::Error::new(io::ErrorKind::InvalidData, message))
}

fn backend_error(error: sevenz_turbo::Error) -> Error {
    use sevenz_turbo::{Error as E, Limit as L};
    match error {
        E::BlockDecode { cause, .. } => backend_error(*cause),
        E::MaxMemLimited { .. } | E::MemoryLimited { .. } => Error::Limit("7z decoder memory"),
        E::EndHeaderTooLarge { .. } => Error::Limit("7z header bytes"),
        E::LimitExceeded { what, .. } => Error::Limit(match what {
            L::Entries => "entry count",
            L::NameBytes | L::TotalNameBytes => "path bytes",
            L::UnpackBytes => "total bytes",
            L::UnpackRatio => "compression ratio",
            L::MemoryBytes => "7z decoder memory",
            _ => "7z metadata",
        }),
        E::PasswordRequired => Error::UnsupportedEntry("encrypted 7z archive".into()),
        E::UnsupportedCompressionMethod(_) | E::Unsupported(_) => {
            Error::UnsupportedEntry("unsupported 7z codec or structure".into())
        }
        // Do not forward backend strings containing archive-controlled metadata.
        _ => invalid("corrupt or unsupported 7z data"),
    }
}

fn policy(limits: Limits) -> ArchiveLimits {
    let entries = (limits.max_entries as u64).min(MAX_OBJECTS);
    let name_bytes = (limits.max_path_bytes as u64)
        .saturating_mul(2)
        .saturating_add(2);
    let header_bytes = entries
        .saturating_mul(name_bytes.saturating_add(256))
        .saturating_add(4096)
        .min(MAX_HEADER_BYTES);
    ArchiveLimits {
        memory_limit_bytes: MAX_MEMORY_BYTES,
        max_end_header_bytes: header_bytes,
        max_header_unpacked_bytes: header_bytes,
        max_header_depth: 2,
        // Encoded headers, filters and BCJ2 need a few objects even for one file.
        max_entries: entries.max(8),
        max_name_bytes: name_bytes.min(MAX_HEADER_BYTES),
        max_total_name_bytes: entries.saturating_mul(name_bytes).min(MAX_HEADER_BYTES),
        max_coders_per_block: 8,
        max_streams_per_coder: 4,
        max_total_coders: entries.saturating_mul(8).clamp(8, MAX_OBJECTS),
        max_unpack_bytes: limits.max_total_bytes,
        max_unpack_ratio: limits.max_compression_ratio,
        ..ArchiveLimits::default()
    }
}

fn crc(bytes: &[u8]) -> u32 {
    let mut crc = Crc::new();
    crc.update(bytes);
    crc.sum()
}

fn number(bytes: &mut &[u8]) -> Result<u64, Error> {
    let (&first, rest) = bytes
        .split_first()
        .ok_or_else(|| invalid("incomplete 7z header"))?;
    *bytes = rest;
    let mut value = 0;
    for i in 0..8 {
        let mask = 0x80 >> i;
        if first & mask == 0 {
            return Ok(value | ((first & (mask - 1)) as u64) << (8 * i));
        }
        let (&byte, rest) = bytes
            .split_first()
            .ok_or_else(|| invalid("incomplete 7z integer"))?;
        *bytes = rest;
        value |= (byte as u64) << (8 * i);
    }
    Ok(value)
}

fn envelope(bytes: &[u8], cap: u64) -> Result<u64, Error> {
    let start = bytes
        .get(..32)
        .ok_or_else(|| invalid("incomplete 7z signature header"))?;
    if !start.starts_with(SIGNATURE) || start[6] != 0 || start[7] > 4 {
        return Err(invalid("invalid 7z signature or version"));
    }
    if crc(&start[12..]) != u32::from_le_bytes(start[8..12].try_into().unwrap()) {
        return Err(invalid("7z start header checksum mismatch"));
    }
    let offset = u64::from_le_bytes(start[12..20].try_into().unwrap());
    let size = u64::from_le_bytes(start[20..28].try_into().unwrap());
    check(size, cap, "7z header bytes")?;
    let position = 32u64
        .checked_add(offset)
        .ok_or_else(|| invalid("7z header range overflow"))?;
    if position.checked_add(size) != Some(bytes.len() as u64) {
        return Err(invalid("7z next header range or trailing data"));
    }
    let next = &bytes[position as usize..];
    if crc(next) != u32::from_le_bytes(start[28..32].try_into().unwrap()) {
        return Err(invalid("7z next header checksum mismatch"));
    }
    if next.first() != Some(&0x17) {
        return Ok(position);
    }
    // An encoded header's one packed stream must sit immediately before its descriptor.
    let mut descriptor = &next[1..];
    if descriptor.first() != Some(&6) {
        return Err(invalid("missing encoded header packed stream"));
    }
    descriptor = &descriptor[1..];
    let pack_position = number(&mut descriptor)?;
    if number(&mut descriptor)? != 1 || descriptor.first() != Some(&9) {
        return Err(Error::UnsupportedEntry(
            "multi-stream 7z encoded header".into(),
        ));
    }
    descriptor = &descriptor[1..];
    let pack_size = number(&mut descriptor)?;
    let packed_start = 32u64
        .checked_add(pack_position)
        .ok_or_else(|| invalid("7z header range overflow"))?;
    if packed_start.checked_add(pack_size) != Some(position) {
        return Err(invalid("encoded header range or trailing data"));
    }
    Ok(packed_start)
}

fn supported(archive: &Archive) -> Result<(), Error> {
    for block in &archive.blocks {
        for coder in &block.coders {
            let method = coder.encoder_method_id();
            if method == EncoderMethod::ID_AES256_SHA256 {
                return Err(Error::UnsupportedEntry("encrypted 7z archive".into()));
            }
            if !matches!(
                method,
                EncoderMethod::ID_COPY
                    | EncoderMethod::ID_LZMA
                    | EncoderMethod::ID_LZMA2
                    | EncoderMethod::ID_DELTA
                    | EncoderMethod::ID_BCJ_X86
                    | EncoderMethod::ID_BCJ2
                    | EncoderMethod::ID_BCJ_PPC
                    | EncoderMethod::ID_BCJ_IA64
                    | EncoderMethod::ID_BCJ_ARM
                    | EncoderMethod::ID_BCJ_ARM64
                    | EncoderMethod::ID_BCJ_ARM_THUMB
                    | EncoderMethod::ID_BCJ_SPARC
                    | EncoderMethod::ID_BCJ_RISCV
            ) {
                return Err(Error::UnsupportedEntry("unsupported 7z codec".into()));
            }
        }
    }
    Ok(())
}

fn metadata(archive: &Archive, data_end: u64, limits: Limits) -> Result<Vec<Entry>, Error> {
    check(
        archive.files.len() as u64,
        limits.max_entries as u64,
        "entry count",
    )?;
    supported(archive)?;
    let mut packed_total = 0u64;
    for &size in archive.pack_sizes() {
        packed_total = packed_total
            .checked_add(size)
            .ok_or_else(|| invalid("7z packed size overflow"))?;
    }
    // No SFX prefixes, hidden gaps, overlapping headers, or unused trailing packed bytes.
    if archive.pack_pos() != 0 || packed_total.checked_add(32) != Some(data_end) {
        return Err(invalid("7z packed data range"));
    }
    let mut block_totals = vec![0u64; archive.blocks.len()];
    let mut paths = BTreeMap::new();
    let mut entries = Vec::new();
    let mut total = 0u64;
    for (i, file) in archive.files.iter().enumerate() {
        check(
            file.name.len() as u64,
            limits.max_path_bytes as u64,
            "path bytes",
        )?;
        let path = if file.is_directory && !file.name.ends_with('/') {
            format!("{}/", file.name)
        } else {
            file.name.clone()
        };
        insert_path(
            &mut paths,
            path_key(&path, file.is_directory, limits)?,
            file.is_directory,
        )?;
        let attrs = file.windows_attributes;
        let mode = (attrs >> 16) & 0o170000;
        if file.is_anti_item
            || file.is_symlink()
            || (file.has_windows_attributes && attrs & (0x400 | 0x40) != 0)
            || !matches!(mode, 0 | 0o100000 | 0o040000)
            || (mode == 0o040000 && !file.is_directory)
            || (mode == 0o100000 && file.is_directory)
            || (file.has_windows_attributes && (attrs & 0x10 != 0) != file.is_directory)
            || (file.is_directory && (file.has_stream || file.size != 0))
            || (!file.has_stream && file.size != 0)
        {
            return Err(Error::UnsupportedEntry(path));
        }
        check(file.size, limits.max_entry_bytes, "entry bytes")?;
        if file.size == 0 && file.has_crc && file.crc != 0 {
            return Err(invalid("empty 7z entry checksum mismatch"));
        }
        total = total
            .checked_add(file.size)
            .ok_or(Error::Limit("total bytes"))?;
        check(total, limits.max_total_bytes, "total bytes")?;
        if file.has_stream {
            let block = archive
                .stream_map
                .file_block_index
                .get(i)
                .and_then(|b| *b)
                .and_then(|b| block_totals.get_mut(b))
                .ok_or_else(|| invalid("7z stream mapping"))?;
            *block = block
                .checked_add(file.size)
                .ok_or(Error::Limit("total bytes"))?;
        }
        entries.push(Entry {
            path,
            is_dir: file.is_directory,
            size: file.size,
            compressed_size: file.compressed_size,
            packed_size: (!file.is_directory && !file.has_stream).then_some(0),
        });
    }
    let mut described_streams = 0usize;
    for (i, block) in archive.blocks.iter().enumerate() {
        // The backend hands zero-sized streams an empty reader without probing the decoder.
        if block.get_unpack_size() == 0 {
            return Err(invalid("zero-output 7z stream block"));
        }
        let sub_streams = archive.block_sub_streams(i);
        described_streams = described_streams
            .checked_add(sub_streams.len())
            .ok_or_else(|| invalid("7z sub-stream count overflow"))?;
        if sub_streams.is_empty() || block.get_unpack_size() != block_totals[i] {
            return Err(invalid("7z block size or stream mapping"));
        }
        let mut compressed = 0u64;
        for range in archive.block_pack_streams(i) {
            if range.end() > data_end {
                return Err(invalid("7z block range"));
            }
            compressed = compressed
                .checked_add(range.size)
                .ok_or_else(|| invalid("7z packed size overflow"))?;
        }
        let preserves_size = block.coders.iter().all(|coder| {
            !matches!(
                coder.encoder_method_id(),
                EncoderMethod::ID_LZMA | EncoderMethod::ID_LZMA2 | EncoderMethod::ID_BCJ2
            )
        });
        if preserves_size && compressed != block_totals[i] {
            return Err(invalid("7z COPY/filter packed and unpacked sizes differ"));
        }
        check(
            block_totals[i],
            compressed.saturating_mul(limits.max_compression_ratio),
            "compression ratio",
        )?;
        if sub_streams.len() == 1 {
            let first = archive
                .stream_map
                .block_first_file_index
                .get(i)
                .ok_or_else(|| invalid("missing 7z first-file mapping"))?;
            let entry = entries
                .get_mut(*first)
                .ok_or_else(|| invalid("7z first-file mapping out of range"))?;
            entry.packed_size = Some(compressed);
        }
    }
    // Per-block declared counts are private; equality with their public total rules out filter_map omissions.
    if described_streams != archive.num_unpack_sub_streams() {
        return Err(invalid("incomplete 7z sub-stream metadata"));
    }
    check(
        total,
        packed_total.saturating_mul(limits.max_compression_ratio),
        "compression ratio",
    )?;
    Ok(entries)
}

fn open(bytes: &[u8], limits: Limits) -> Result<(Reader<'_>, Vec<Entry>), Error> {
    check(
        bytes.len() as u64,
        limits.max_archive_bytes,
        "archive bytes",
    )?;
    let policy = policy(limits);
    let data_end = envelope(bytes, policy.max_end_header_bytes)?;
    let mut reader = ArchiveReader::with_limits(Cursor::new(bytes), Password::empty(), policy)
        .map_err(backend_error)?;
    reader.set_thread_count(1);
    reader.set_input_complete(true);
    let entries = metadata(reader.archive(), data_end, limits)?;
    Ok((reader, entries))
}

pub(super) fn list(bytes: &[u8], limits: Limits) -> Result<Vec<Entry>, Error> {
    open(bytes, limits).map(|(_, entries)| entries)
}

fn payload(
    input: &mut dyn Read,
    file: &sevenz_turbo::ArchiveEntry,
    keep: bool,
    total: &mut u64,
    block_crc: &mut Crc,
    limits: Limits,
) -> Result<Vec<u8>, Error> {
    let size = file.size;
    let mut input = input.take(size.saturating_add(1));
    let mut data = Vec::new();
    let mut file_crc = Crc::new();
    let mut actual = 0u64;
    let mut buffer = [0; 8192];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|_| invalid("7z entry checksum or data error"))?;
        if count == 0 {
            break;
        }
        actual += count as u64;
        check(actual, size, "declared entry size")?;
        check(actual, limits.max_entry_bytes, "entry bytes")?;
        *total = total
            .checked_add(count as u64)
            .ok_or(Error::Limit("total bytes"))?;
        check(*total, limits.max_total_bytes, "total bytes")?;
        file_crc.update(&buffer[..count]);
        block_crc.update(&buffer[..count]);
        if keep {
            data.extend_from_slice(&buffer[..count]);
        }
    }
    if actual != size {
        return Err(invalid("incomplete 7z entry"));
    }
    if file.has_crc && u64::from(file_crc.sum()) != file.crc {
        return Err(invalid("7z entry checksum mismatch"));
    }
    Ok(data)
}

pub(super) fn extract(
    bytes: &[u8],
    selected: Option<&[String]>,
    limits: Limits,
) -> Result<Vec<MemoryEntry>, Error> {
    let (mut reader, entries) = open(bytes, limits)?;
    let indices = selected_indices(&entries, selected)?;
    let mut output: Vec<_> = indices
        .iter()
        .map(|&i| MemoryEntry {
            path: entries[i].path.clone(),
            is_dir: entries[i].is_dir,
            data: Vec::new(),
        })
        .collect();
    let wanted: BTreeMap<_, _> = indices
        .iter()
        .enumerate()
        .map(|(out, &i)| (reader.archive().files[i].name.clone(), out))
        .collect();
    let mut blocks = std::collections::BTreeSet::new();
    for &i in &indices {
        if let Some(block) = reader.archive().stream_map.file_block_index[i] {
            blocks.insert(block);
        }
    }
    read_blocks(&mut reader, blocks, &wanted, &mut output, limits)?;
    Ok(output)
}

pub(super) fn test(bytes: &[u8], limits: Limits) -> Result<TestReport, Error> {
    let (mut reader, entries) = open(bytes, limits)?;
    let archive = reader.archive();
    let checksummed = archive
        .files
        .iter()
        .enumerate()
        .filter(|(i, file)| {
            !file.is_directory
                && (file.has_crc
                    || (file.has_stream
                        && archive.stream_map.file_block_index[*i]
                            .is_some_and(|block| archive.blocks[block].has_crc)))
        })
        .count();
    let blocks = 0..archive.blocks.len();
    let total = read_blocks(&mut reader, blocks, &BTreeMap::new(), &mut [], limits)?;
    let mut report = test_report(&entries, checksummed);
    report.total_bytes = total;
    Ok(report)
}

fn read_blocks(
    reader: &mut Reader<'_>,
    blocks: impl IntoIterator<Item = usize>,
    wanted: &BTreeMap<String, usize>,
    output: &mut [MemoryEntry],
    limits: Limits,
) -> Result<u64, Error> {
    let mut total = 0;
    for block in blocks {
        let mut callback_error = None;
        let metadata = &reader.archive().blocks[block];
        let expected_crc = metadata.has_crc.then_some(metadata.crc);
        let mut block_crc = Crc::new();
        let result = reader
            .block_decoder(block)
            .map_err(backend_error)?
            // Backend CRC readers stop at the declared size and hide surplus from its EOF probe.
            .with_verify_checksums(false)
            .for_each_entries(&mut |file, input| {
                let target = wanted.get(&file.name).copied();
                match payload(
                    input,
                    file,
                    target.is_some(),
                    &mut total,
                    &mut block_crc,
                    limits,
                ) {
                    Ok(data) => {
                        if let Some(target) = target {
                            output[target].data = data;
                        }
                        Ok(true)
                    }
                    Err(error) => {
                        callback_error = Some(error);
                        Err(sevenz_turbo::Error::Other(
                            "bounded entry read failed".into(),
                        ))
                    }
                }
            });
        if let Some(error) = callback_error {
            return Err(error);
        }
        result.map_err(backend_error)?;
        if expected_crc.is_some_and(|expected| expected != u64::from(block_crc.sum())) {
            return Err(invalid("7z block checksum mismatch"));
        }
    }
    Ok(total)
}

#[cfg(test)]
mod packed_mapping_tests {
    use super::*;

    fn single_stream_archive() -> Archive {
        ArchiveReader::new(
            Cursor::new(include_bytes!("../tests/fixtures/7z-bcj2-single.7z")),
            Password::empty(),
        )
        .unwrap()
        .archive()
        .clone()
    }

    fn data_end(archive: &Archive) -> u64 {
        32 + archive.pack_sizes().iter().sum::<u64>()
    }

    #[test]
    fn missing_sub_stream_metadata_cannot_make_a_declared_multi_stream_block_known() {
        let mut archive = single_stream_archive();
        let solid = ArchiveReader::new(
            Cursor::new(include_bytes!("../tests/fixtures/7z-solid-plain.7z")),
            Password::empty(),
        )
        .unwrap();
        archive.blocks[0] = solid.archive().blocks[0].clone();
        archive.files = solid.archive().files.clone();
        archive.stream_map.file_block_index = vec![Some(0); archive.files.len()];
        assert_eq!(archive.num_unpack_sub_streams(), 2);
        assert_eq!(archive.block_sub_streams(0).len(), 1);
        assert!(matches!(
            metadata(&archive, data_end(&archive), Limits::default()),
            Err(Error::InvalidSevenZip(_))
        ));
    }

    #[test]
    fn missing_or_out_of_range_first_file_mapping_returns_invalid_instead_of_panicking() {
        for mapping in [vec![], vec![usize::MAX]] {
            let mut archive = single_stream_archive();
            archive.stream_map.block_first_file_index = mapping;
            assert!(matches!(
                metadata(&archive, data_end(&archive), Limits::default()),
                Err(Error::InvalidSevenZip(_))
            ));
        }
    }
}
