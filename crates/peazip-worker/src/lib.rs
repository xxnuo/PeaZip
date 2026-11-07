use peazip_core::{Limits, MemoryEntry, ZipCompression, ZipOptions};

pub const MAX_REQUEST_BYTES: usize = 64 * 1024 * 1024 + 4096 * (8 + 1024) + 4;
pub const MAX_CREATE_V1_REQUEST_BYTES: usize = MAX_REQUEST_BYTES + 12;
pub const MAX_EXTRACT_REQUEST_BYTES: usize = 32 * 1024 * 1024 + 4096 * (4 + 1024) + 8;
pub const MAX_EXTRACT_OUTPUT_BYTES: usize = 64 * 1024 * 1024 + 4096 * (12 + 1024) + 4;
pub const MAX_LIST_OUTPUT_BYTES: usize = 5 + 4096 * (24 + 1024);
pub const TEST_REPORT_BYTES: usize = 20;

pub fn test_from_request(archive: &[u8]) -> Result<Vec<u8>, String> {
    let limits = Limits::default();
    if archive.len() as u64 > limits.max_archive_bytes {
        return Err("archive byte limit exceeded".into());
    }
    let report = peazip_core::test_archive(archive, limits).map_err(|error| error.to_string())?;
    if report.entries > limits.max_entries
        || report.files > report.entries
        || report.checksummed_files > report.files
        || report.total_bytes > limits.max_total_bytes
        || report.total_bytes > (report.files as u64).saturating_mul(limits.max_entry_bytes)
    {
        return Err("invalid archive test report".into());
    }
    let mut output = Vec::with_capacity(TEST_REPORT_BYTES);
    output.extend_from_slice(&(report.entries as u32).to_le_bytes());
    output.extend_from_slice(&(report.files as u32).to_le_bytes());
    output.extend_from_slice(&report.total_bytes.to_le_bytes());
    output.extend_from_slice(&(report.checksummed_files as u32).to_le_bytes());
    Ok(output)
}

pub fn list_from_request(archive: &[u8]) -> Result<Vec<u8>, String> {
    let limits = Limits::default();
    if archive.len() as u64 > limits.max_archive_bytes {
        return Err("archive byte limit exceeded".into());
    }
    // The core finishes validation (including the gzip trailer) before we publish metadata.
    let (format, entries) = peazip_core::list_archive_with_format(archive, limits)
        .map_err(|error| error.to_string())?;
    let size = 5 + entries
        .iter()
        .map(|entry| 24 + entry.path.len())
        .sum::<usize>();
    if entries.len() > limits.max_entries
        || entries
            .iter()
            .any(|entry| entry.path.is_empty() || entry.path.len() > limits.max_path_bytes)
        || size > MAX_LIST_OUTPUT_BYTES
    {
        return Err("metadata byte limit exceeded".into());
    }
    let mut output = Vec::with_capacity(size);
    output.push(match format {
        peazip_core::ArchiveFormat::Zip => 0,
        peazip_core::ArchiveFormat::Tar => 1,
        peazip_core::ArchiveFormat::TarGzip => 2,
        peazip_core::ArchiveFormat::SevenZip => 3,
    });
    output.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for entry in entries {
        output.extend_from_slice(&u32::from(entry.is_dir).to_le_bytes());
        output.extend_from_slice(&(entry.path.len() as u32).to_le_bytes());
        output.extend_from_slice(entry.path.as_bytes());
        output.extend_from_slice(&entry.size.to_le_bytes());
        output.extend_from_slice(&entry.compressed_size.to_le_bytes());
    }
    Ok(output)
}

pub fn extract_from_request(packet: &[u8]) -> Result<Vec<u8>, String> {
    if packet.len() > MAX_EXTRACT_REQUEST_BYTES {
        return Err("request byte limit exceeded".into());
    }
    let limits = Limits::default();
    let mut remaining = packet;
    let count = read_u32(&mut remaining)?;
    if count == 0 || count > limits.max_entries {
        return Err("selected count limit exceeded".into());
    }
    let mut selected = Vec::with_capacity(count);
    for _ in 0..count {
        let len = read_u32(&mut remaining)?;
        if len == 0 || len > limits.max_path_bytes {
            return Err("path byte limit exceeded".into());
        }
        let path =
            std::str::from_utf8(take(&mut remaining, len)?).map_err(|_| "invalid UTF-8 path")?;
        selected.push(path.to_owned());
    }
    let len = read_u32(&mut remaining)?;
    if len as u64 > limits.max_archive_bytes {
        return Err("archive byte limit exceeded".into());
    }
    let archive = take(&mut remaining, len)?;
    if !remaining.is_empty() {
        return Err("trailing request bytes".into());
    }
    let entries = peazip_core::extract_selected_archive(archive, &selected, limits)
        .map_err(|error| error.to_string())?;
    let size = 4 + entries
        .iter()
        .map(|entry| 12 + entry.path.len() + entry.data.len())
        .sum::<usize>();
    if size > MAX_EXTRACT_OUTPUT_BYTES {
        return Err("output byte limit exceeded".into());
    }
    let mut output = Vec::with_capacity(size);
    output.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for entry in entries {
        output.extend_from_slice(&u32::from(entry.is_dir).to_le_bytes());
        output.extend_from_slice(&(entry.path.len() as u32).to_le_bytes());
        output.extend_from_slice(&(entry.data.len() as u32).to_le_bytes());
        output.extend_from_slice(entry.path.as_bytes());
        output.extend_from_slice(&entry.data);
    }
    Ok(output)
}

fn take<'a>(packet: &mut &'a [u8], len: usize) -> Result<&'a [u8], String> {
    let (value, rest) = packet.split_at_checked(len).ok_or("truncated request")?;
    *packet = rest;
    Ok(value)
}

fn read_u32(packet: &mut &[u8]) -> Result<usize, String> {
    let bytes: [u8; 4] = take(packet, 4)?.try_into().unwrap();
    Ok(u32::from_le_bytes(bytes) as usize)
}

pub fn create_from_request(packet: &[u8]) -> Result<Vec<u8>, String> {
    if packet.len() > MAX_REQUEST_BYTES {
        return Err("request byte limit exceeded".into());
    }
    create_entries(packet, ZipOptions::default())
}

pub fn create_v1_from_request(packet: &[u8]) -> Result<Vec<u8>, String> {
    if packet.len() > MAX_CREATE_V1_REQUEST_BYTES {
        return Err("request byte limit exceeded".into());
    }
    let mut remaining = packet;
    if read_u32(&mut remaining)? != 1 {
        return Err("unsupported ZIP create request version".into());
    }
    let compression = read_u32(&mut remaining)?;
    let level = read_u32(&mut remaining)?;
    let compression = match (compression, level) {
        (0, 0) => ZipCompression::Automatic,
        (1, 0) => ZipCompression::Stored,
        (2, 1..=9) => ZipCompression::Deflate { level: level as u8 },
        _ => return Err("invalid ZIP compression options".into()),
    };
    create_entries(remaining, ZipOptions { compression })
}

fn create_entries(packet: &[u8], options: ZipOptions) -> Result<Vec<u8>, String> {
    options.validate().map_err(|error| error.to_string())?;
    let limits = Limits::default();
    let mut remaining = packet;
    let count = read_u32(&mut remaining)?;
    if count == 0 || count > limits.max_entries {
        return Err("entry count limit exceeded".into());
    }
    let mut entries = Vec::with_capacity(count);
    let mut total = 0u64;
    for _ in 0..count {
        let path_len = read_u32(&mut remaining)?;
        let data_len = read_u32(&mut remaining)?;
        if path_len == 0 || path_len > limits.max_path_bytes {
            return Err("path byte limit exceeded".into());
        }
        if data_len as u64 > limits.max_entry_bytes {
            return Err("entry byte limit exceeded".into());
        }
        total += data_len as u64;
        if total > limits.max_total_bytes {
            return Err("total byte limit exceeded".into());
        }
        let path = std::str::from_utf8(take(&mut remaining, path_len)?)
            .map_err(|_| "invalid UTF-8 path")?;
        if path.ends_with('/') {
            return Err("directory entries are not supported".into());
        }
        let data = take(&mut remaining, data_len)?;
        entries.push(MemoryEntry {
            path: path.into(),
            is_dir: false,
            data: data.into(),
        });
    }
    if !remaining.is_empty() {
        return Err("trailing request bytes".into());
    }
    peazip_core::create_zip_with_options(&entries, limits, options)
        .map_err(|error| error.to_string())
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::{
        MAX_CREATE_V1_REQUEST_BYTES, create_from_request, create_v1_from_request,
        extract_from_request, list_from_request, test_from_request,
    };
    use std::cell::RefCell;

    thread_local! {
        static INPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
        static OUTPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn pw_input_resize(len: usize) -> *mut u8 {
        INPUT.with_borrow_mut(|input| {
            if len > MAX_CREATE_V1_REQUEST_BYTES {
                *input = Vec::new();
                return std::ptr::null_mut();
            }
            input.resize(len, 0);
            input.as_mut_ptr()
        })
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn pw_create() -> u32 {
        run(create_from_request)
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn pw_create_v1() -> u32 {
        run(create_v1_from_request)
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn pw_extract() -> u32 {
        run(extract_from_request)
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn pw_list() -> u32 {
        run(list_from_request)
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn pw_test() -> u32 {
        run(test_from_request)
    }

    fn run(operation: fn(&[u8]) -> Result<Vec<u8>, String>) -> u32 {
        OUTPUT.with_borrow_mut(|output| *output = Vec::new());
        let input = INPUT.with_borrow_mut(std::mem::take);
        let result = operation(&input);
        drop(input);
        let (status, bytes) = match result {
            Ok(bytes) => (1, bytes),
            Err(error) => (0, error.into_bytes()),
        };
        OUTPUT.with_borrow_mut(|output| *output = bytes);
        status
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn pw_output_ptr() -> *const u8 {
        OUTPUT.with_borrow(|output| output.as_ptr())
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn pw_output_len() -> usize {
        OUTPUT.with_borrow(Vec::len)
    }
}
