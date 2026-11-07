use peazip_core::{Limits, MemoryEntry};

pub const MAX_REQUEST_BYTES: usize = 64 * 1024 * 1024 + 4096 * (8 + 1024) + 4;

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
    peazip_core::create_zip(&entries, limits).map_err(|error| error.to_string())
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::{MAX_REQUEST_BYTES, create_from_request};
    use std::cell::RefCell;

    thread_local! {
        static INPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
        static OUTPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn pw_input_resize(len: usize) -> *mut u8 {
        INPUT.with_borrow_mut(|input| {
            if len > MAX_REQUEST_BYTES {
                *input = Vec::new();
                return std::ptr::null_mut();
            }
            input.resize(len, 0);
            input.as_mut_ptr()
        })
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn pw_create() -> u32 {
        OUTPUT.with_borrow_mut(|output| *output = Vec::new());
        let input = INPUT.with_borrow_mut(std::mem::take);
        let result = create_from_request(&input);
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
