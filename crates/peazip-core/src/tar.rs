use super::{
    Entry, Error, Limits, MemoryEntry, check, insert_path, path_key, selected_indices, sizes,
};
use std::{collections::BTreeMap, io};

const BLOCK: usize = 512;

fn invalid(message: &'static str) -> Error {
    Error::InvalidTar(io::Error::new(io::ErrorKind::InvalidData, message))
}

fn header(bytes: &[u8]) -> Result<::tar::Header, Error> {
    let mut archive = ::tar::Archive::new(bytes);
    // Raw mode never buffers GNU/PAX extensions or expands sparse maps.
    archive
        .entries()
        .map_err(Error::InvalidTar)?
        .raw(true)
        .next()
        .ok_or_else(|| invalid("missing header"))?
        .map(|entry| entry.header().clone())
        .map_err(Error::InvalidTar)
}

pub(super) fn recognizes(bytes: &[u8]) -> Result<bool, Error> {
    if bytes.len() < BLOCK {
        return Ok(false);
    }
    if bytes[..BLOCK].iter().all(|&b| b == 0) {
        let empty = bytes.len() >= 2 * BLOCK
            && bytes.len().is_multiple_of(BLOCK)
            && bytes.iter().all(|&b| b == 0);
        if !empty
            && bytes
                .get(..2 * BLOCK)
                .is_some_and(|s| s.iter().all(|&b| b == 0))
        {
            return Err(invalid("invalid data after empty TAR end marker"));
        }
        return Ok(empty);
    }
    match header(bytes) {
        Ok(_) => Ok(true),
        Err(error) if matches!(&bytes[257..263], b"ustar\0" | b"ustar ") => Err(error),
        Err(_) => Ok(false),
    }
}

#[derive(Default)]
struct Extensions<'a> {
    path: Option<&'a [u8]>,
    size: Option<u64>,
    local: bool,
}

fn decimal<T: std::str::FromStr>(bytes: &[u8]) -> Result<T, Error> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return Err(invalid("invalid decimal extension value"));
    }
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| invalid("extension value overflow"))
}

fn pax<'a>(
    mut bytes: &'a [u8],
    pending: &mut Extensions<'a>,
    global: bool,
    limits: Limits,
) -> Result<(), Error> {
    while !bytes.is_empty() {
        let space = bytes
            .iter()
            .position(|&b| b == b' ')
            .ok_or_else(|| invalid("invalid PAX record length"))?;
        let len: usize = decimal(&bytes[..space])?;
        let record = bytes
            .get(..len)
            .filter(|record| record.len() > space + 2 && record.last() == Some(&b'\n'))
            .ok_or_else(|| invalid("incomplete PAX record"))?;
        let pair = &record[space + 1..len - 1];
        let equals = pair
            .iter()
            .position(|&b| b == b'=')
            .filter(|&i| i > 0)
            .ok_or_else(|| invalid("invalid PAX key/value"))?;
        // Keep unknown-key handling and diagnostic strings bounded too.
        check(
            equals as u64,
            limits.max_path_bytes as u64,
            "PAX keyword bytes",
        )?;
        let key = std::str::from_utf8(&pair[..equals]).map_err(|_| invalid("non-UTF8 PAX key"))?;
        if key.chars().any(char::is_control) {
            return Err(invalid("control character in PAX keyword"));
        }
        let value = &pair[equals + 1..];
        // Structural extensions we do not implement must never become ordinary files.
        if key.starts_with("GNU.sparse.")
            || matches!(
                key,
                "linkpath" | "SCHILY.realsize" | "SCHILY.filetype" | "SUN.holesdata"
            )
            || (key == "hdrcharset" && value != b"ISO-IR 10646 2000 UTF-8")
            || (global && matches!(key, "path" | "size"))
        {
            return Err(Error::UnsupportedEntry(format!("PAX {key}")));
        }
        match key {
            "path" => {
                check(
                    value.len() as u64,
                    limits.max_path_bytes as u64,
                    "path bytes",
                )?;
                if pending.path.replace(value).is_some() {
                    return Err(invalid("multiple extended paths for one entry"));
                }
            }
            "size" if pending.size.is_some() => {
                return Err(invalid("multiple extended sizes for one entry"));
            }
            "size" => pending.size = Some(decimal(value)?),
            _ => {}
        }
        bytes = &bytes[len..];
    }
    Ok(())
}

fn normalized_path(raw: &[u8], dir: bool, limits: Limits) -> Result<Option<String>, Error> {
    check(raw.len() as u64, limits.max_path_bytes as u64, "path bytes")?;
    let raw = std::str::from_utf8(raw).map_err(|_| invalid("non-UTF8 entry path"))?;
    // TAR tools routinely write './' and './name'; only leading dot components are removed.
    let mut path = raw;
    while let Some(rest) = path.strip_prefix("./") {
        path = rest;
    }
    if dir && (path == "." || path.is_empty()) && (raw == "." || raw.starts_with("./")) {
        return Ok(None);
    }
    let path = if dir && !path.ends_with('/') {
        format!("{path}/")
    } else {
        path.to_owned()
    };
    path_key(&path, dir, limits)?;
    Ok(Some(path))
}

fn scan(bytes: &[u8], limits: Limits) -> Result<(Vec<Entry>, Vec<&[u8]>), Error> {
    let mut entries = Vec::new();
    let mut payloads = Vec::new();
    let mut paths = BTreeMap::new();
    let mut pending = Extensions::default();
    let mut root_seen = false;
    let mut total = 0;
    let mut count = 0u64;
    let mut offset = 0usize;
    loop {
        let remaining = &bytes[offset..];
        if remaining.len() < BLOCK {
            return Err(invalid("missing end-of-archive blocks"));
        }
        if remaining[..BLOCK].iter().all(|&b| b == 0) {
            if remaining.len() < 2 * BLOCK
                || !remaining.len().is_multiple_of(BLOCK)
                || remaining.iter().any(|&b| b != 0)
            {
                return Err(invalid("incomplete end marker or nonzero trailing data"));
            }
            if pending.local || pending.path.is_some() {
                return Err(invalid("extension without a following entry"));
            }
            return Ok((entries, payloads));
        }
        count += 1;
        // Include extension headers and the normalized-away root in the scan budget.
        check(count, limits.max_entries as u64, "entry count")?;
        // Start a raw reader at each header so PAX size overrides determine our next offset.
        let header = header(remaining)?;
        let kind = header.entry_type().as_byte();
        let extension = matches!(kind, b'L' | b'x' | b'g');
        if !matches!(kind, 0 | b'0' | b'5' | b'L' | b'x' | b'g') {
            return Err(Error::UnsupportedEntry(format!("TAR type {kind:#x}")));
        }
        let size = if extension {
            header.entry_size().map_err(Error::InvalidTar)?
        } else {
            pending
                .size
                .unwrap_or(header.entry_size().map_err(Error::InvalidTar)?)
        };
        // Metadata payloads consume the same byte budgets as file payloads. No extension
        // allocation occurs before these checks; extension parsing borrows the input slice.
        sizes(size, size, &mut total, limits)?;
        let size = usize::try_from(size).map_err(|_| invalid("entry size overflow"))?;
        let data_padded = size
            .checked_add(BLOCK - 1)
            .map(|n| n / BLOCK * BLOCK)
            .ok_or_else(|| invalid("entry size overflow"))?;
        let padded = data_padded
            .checked_add(BLOCK)
            .ok_or_else(|| invalid("entry size overflow"))?;
        if remaining.len() < padded {
            return Err(invalid("incomplete entry payload or padding"));
        }
        let data = &remaining[BLOCK..BLOCK + size];
        offset += padded;
        match kind {
            b'L' => {
                let name = data.strip_suffix(&[0]).unwrap_or(data);
                check(
                    name.len() as u64,
                    limits.max_path_bytes as u64,
                    "path bytes",
                )?;
                if pending.path.replace(name).is_some() {
                    return Err(invalid("multiple extended paths for one entry"));
                }
                continue;
            }
            b'x' => {
                if pending.local {
                    return Err(invalid("multiple PAX headers for one entry"));
                }
                pending.local = true;
                pax(data, &mut pending, false, limits)?;
                continue;
            }
            b'g' => {
                pax(data, &mut pending, true, limits)?;
                continue;
            }
            _ => {}
        }
        let dir = kind == b'5';
        if dir && size != 0 {
            return Err(Error::UnsupportedEntry("nonempty TAR directory".into()));
        }
        let raw = header.path_bytes();
        let path = normalized_path(pending.path.unwrap_or(&raw), dir, limits)?;
        pending = Extensions::default();
        let Some(path) = path else {
            if root_seen {
                return Err(Error::DuplicatePath("./".into()));
            }
            root_seen = true;
            continue;
        };
        insert_path(&mut paths, path_key(&path, dir, limits)?, dir)?;
        entries.push(Entry {
            path,
            is_dir: dir,
            size: size as u64,
            compressed_size: size as u64,
            packed_size: (!dir).then_some(data_padded as u64),
        });
        payloads.push(data);
    }
}

pub(super) fn list(bytes: &[u8], limits: Limits) -> Result<Vec<Entry>, Error> {
    scan(bytes, limits).map(|(entries, _)| entries)
}

pub(super) fn extract(
    bytes: &[u8],
    selected: Option<&[String]>,
    limits: Limits,
) -> Result<Vec<MemoryEntry>, Error> {
    let (entries, payloads) = scan(bytes, limits)?;
    let indices = selected_indices(&entries, selected, limits)?;
    // TAR checksums cover headers only; copying a complete payload provides no CRC guarantee.
    Ok(indices
        .into_iter()
        .map(|i| MemoryEntry {
            path: entries[i].path.clone(),
            is_dir: entries[i].is_dir,
            data: payloads[i].to_vec(),
        })
        .collect())
}
