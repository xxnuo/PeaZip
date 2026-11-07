use super::{Error, Limits, check};
use flate2::{Crc, Decompress, FlushDecompress, Status};
use std::io;

const MAX_MEMBERS: u64 = 4096;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_TOTAL_HEADER_BYTES: usize = 1024 * 1024;
// One standard TAR record, including its end marker; never scales with payload size.
const TAR_PADDING_ALLOWANCE: u64 = 10 * 1024;

fn invalid(message: &'static str) -> Error {
    Error::InvalidGzip(io::Error::new(io::ErrorKind::InvalidData, message))
}

fn header(bytes: &[u8], budget: usize) -> Result<usize, Error> {
    // Bound even the NUL search before inspecting filename/comment; allocate neither.
    let window = &bytes[..bytes.len().min(budget)];
    let incomplete = || {
        if bytes.len() > window.len() {
            Error::Limit("gzip header bytes")
        } else {
            invalid("incomplete gzip header")
        }
    };
    let fixed = window.get(..10).ok_or_else(incomplete)?;
    if fixed[..3] != [0x1f, 0x8b, 8] || fixed[3] & 0xe0 != 0 {
        return Err(invalid("invalid gzip signature, method or flags"));
    }
    let flags = fixed[3];
    let mut offset = 10;
    if flags & 4 != 0 {
        let len = window.get(offset..offset + 2).ok_or_else(incomplete)?;
        offset += 2 + u16::from_le_bytes([len[0], len[1]]) as usize;
        window.get(..offset).ok_or_else(incomplete)?;
    }
    for flag in [8, 16] {
        if flags & flag != 0 {
            let field = window.get(offset..).ok_or_else(incomplete)?;
            offset += field.iter().position(|&b| b == 0).ok_or_else(incomplete)? + 1;
        }
    }
    if flags & 2 != 0 {
        let expected = window.get(offset..offset + 2).ok_or_else(incomplete)?;
        let mut crc = Crc::new();
        crc.update(&window[..offset]);
        if crc.sum() as u16 != u16::from_le_bytes([expected[0], expected[1]]) {
            return Err(invalid("gzip header checksum mismatch"));
        }
        offset += 2;
    }
    Ok(offset)
}

pub(super) fn decode(bytes: &[u8], limits: Limits) -> Result<Vec<u8>, Error> {
    check(
        bytes.len() as u64,
        limits.max_archive_bytes,
        "archive bytes",
    )?;
    // Each TAR header consumes max_entries; each payload needs at most 511 padding bytes.
    let absolute = limits
        .max_total_bytes
        .saturating_add((limits.max_entries as u64).saturating_mul(512 + 511))
        .saturating_add(TAR_PADDING_ALLOWANCE);
    let ratio = (bytes.len() as u64)
        .saturating_mul(limits.max_compression_ratio)
        .saturating_add(TAR_PADDING_ALLOWANCE);
    let (max_output, kind) = if absolute <= ratio {
        (absolute, "gzip decoded bytes")
    } else {
        (ratio, "compression ratio")
    };
    let mut remaining = bytes;
    let mut output = Vec::new();
    let mut members = 0;
    let mut header_bytes = 0;
    let mut compressed_bytes = 0u64;
    let mut buffer = [0; 8192];
    while !remaining.is_empty() {
        members += 1;
        check(members, MAX_MEMBERS, "gzip member count")?;
        let len = header(
            remaining,
            MAX_HEADER_BYTES.min(MAX_TOTAL_HEADER_BYTES - header_bytes),
        )?;
        header_bytes += len;
        remaining = &remaining[len..];
        let mut decoder = Decompress::new(false);
        let mut crc = Crc::new();
        loop {
            // A fixed scratch buffer detects overflow without retaining an excess byte.
            let available = max_output.saturating_sub(output.len() as u64);
            let capacity = available.saturating_add(1).min(buffer.len() as u64) as usize;
            let before_in = decoder.total_in();
            let before_out = decoder.total_out();
            let status = decoder
                .decompress(remaining, &mut buffer[..capacity], FlushDecompress::None)
                .map_err(|_| invalid("corrupt deflate stream"))?;
            let consumed = (decoder.total_in() - before_in) as usize;
            let produced = (decoder.total_out() - before_out) as usize;
            remaining = &remaining[consumed..];
            check(output.len() as u64 + produced as u64, max_output, kind)?;
            crc.update(&buffer[..produced]);
            output.extend_from_slice(&buffer[..produced]);
            if status == Status::StreamEnd {
                break;
            }
            if consumed == 0 && produced == 0 {
                return Err(invalid("incomplete deflate stream"));
            }
        }
        // Empty members are legal, but must not dilute another member's expansion ratio.
        if decoder.total_out() != 0 {
            compressed_bytes = compressed_bytes.saturating_add(decoder.total_in());
        }
        let trailer = remaining
            .get(..8)
            .ok_or_else(|| invalid("incomplete gzip trailer"))?;
        let expected_crc = u32::from_le_bytes(trailer[..4].try_into().unwrap());
        let expected_size = u32::from_le_bytes(trailer[4..].try_into().unwrap());
        if crc.sum() != expected_crc || decoder.total_out() as u32 != expected_size {
            return Err(invalid("gzip data checksum or size mismatch"));
        }
        remaining = &remaining[8..];
    }
    // Only DEFLATE bytes count toward the final ratio, never gzip header metadata.
    check(
        output.len() as u64,
        compressed_bytes
            .saturating_mul(limits.max_compression_ratio)
            .saturating_add(TAR_PADDING_ALLOWANCE),
        "compression ratio",
    )?;
    Ok(output)
}
