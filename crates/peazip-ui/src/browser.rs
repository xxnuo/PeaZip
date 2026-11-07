use std::cell::RefCell;

use futures_channel::oneshot;

type Reply = Result<Option<Vec<u8>>, String>;
struct Pending {
    sender: oneshot::Sender<Reply>,
    bytes: Vec<u8>,
}

thread_local! {
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
}

#[link(wasm_import_module = "peazip")]
unsafe extern "C" {
    fn pc_create_start() -> u32;
    fn pc_download(ptr: *const u8, len: usize) -> u32;
}

pub async fn create_zip() -> Reply {
    let (sender, receiver) = oneshot::channel();
    PENDING.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_some() {
            return Err("An archive operation is already running.".to_owned());
        }
        *slot = Some(Pending {
            sender,
            bytes: Vec::new(),
        });
        Ok(())
    })?;
    // The page's host owns the picker and worker; it copies results into our bounded buffer.
    if unsafe { pc_create_start() } == 0 {
        PENDING.with(|slot| {
            slot.borrow_mut().take();
        });
        return Err("Browser archive worker is unavailable.".into());
    }
    receiver
        .await
        .map_err(|_| "Browser archive operation ended unexpectedly.".to_owned())?
}

#[unsafe(no_mangle)]
pub extern "C" fn pc_response_begin(len: usize) -> *mut u8 {
    if len as u64 > peazip_core::Limits::default().max_archive_bytes {
        return std::ptr::null_mut();
    }
    PENDING.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(pending) = slot.as_mut() else {
            return std::ptr::null_mut();
        };
        pending.bytes.resize(len, 0);
        pending.bytes.as_mut_ptr()
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn pc_response_end(code: u32) {
    let pending = PENDING.with(|slot| slot.borrow_mut().take());
    if let Some(pending) = pending {
        let result = match code {
            1 => Ok(Some(pending.bytes)),
            2 => Ok(None),
            _ => Err(String::from_utf8_lossy(&pending.bytes).into_owned()),
        };
        let _ = pending.sender.send(result);
    }
}

pub fn download(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("No archive is open.".into());
    }
    // The host copies the bytes synchronously; it never retains the pointer.
    if unsafe { pc_download(bytes.as_ptr(), bytes.len()) } == 1 {
        Ok(())
    } else {
        Err("The browser could not start a download.".into())
    }
}
