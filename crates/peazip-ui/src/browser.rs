use std::cell::RefCell;

use futures_channel::mpsc;
use futures_util::StreamExt;

type Reply = Result<Option<Vec<u8>>, String>;
enum Update {
    Progress(String),
    Finished(Reply),
}
struct Pending {
    sender: mpsc::UnboundedSender<Update>,
    bytes: Vec<u8>,
}

thread_local! {
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
}

#[link(wasm_import_module = "peazip")]
unsafe extern "C" {
    fn pc_create_start() -> u32;
    fn pc_can_extract() -> u32;
    fn pc_extract_start(ptr: *const u8, len: usize) -> u32;
    fn pc_cancel() -> u32;
    fn pc_download(ptr: *const u8, len: usize, format: u32) -> u32;
}

pub async fn create_zip(progress: impl FnMut(String) + 'static) -> Reply {
    run(|| unsafe { pc_create_start() }, progress).await
}

async fn run(start: impl FnOnce() -> u32, mut progress: impl FnMut(String)) -> Reply {
    let (sender, mut receiver) = mpsc::unbounded();
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
    if start() == 0 {
        PENDING.with(|slot| {
            slot.borrow_mut().take();
        });
        return Err("Browser archive worker is unavailable.".into());
    }
    // Signal writes must run while the nsh executor has entered its reactive runtime,
    // never directly inside a JavaScript-to-Wasm progress callback.
    while let Some(update) = receiver.next().await {
        match update {
            Update::Progress(message) => progress(message),
            Update::Finished(reply) => return reply,
        }
    }
    Err("Browser archive operation ended unexpectedly.".into())
}

pub fn can_extract() -> bool {
    unsafe { pc_can_extract() == 1 }
}

pub async fn extract_archive(
    bytes: &[u8],
    selected: &[String],
    progress: impl FnMut(String),
) -> Result<Option<String>, String> {
    let limits = peazip_core::Limits::default();
    if selected.is_empty()
        || selected.len() > limits.max_entries
        || bytes.len() as u64 > limits.max_archive_bytes
    {
        return Err("Invalid extraction request size.".into());
    }
    let mut packet = Vec::new();
    packet.extend_from_slice(&(selected.len() as u32).to_le_bytes());
    for path in selected {
        if path.len() > limits.max_path_bytes {
            return Err("Path exceeds the limit.".into());
        }
        packet.extend_from_slice(&(path.len() as u32).to_le_bytes());
        packet.extend_from_slice(path.as_bytes());
    }
    packet.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    packet.extend_from_slice(bytes);
    let result = run(
        move || unsafe { pc_extract_start(packet.as_ptr(), packet.len()) },
        progress,
    )
    .await?;
    result
        .map(String::from_utf8)
        .transpose()
        .map_err(|_| "Invalid extraction report.".into())
}

pub fn cancel() -> bool {
    // The host terminates the worker before completing the pending request as cancelled.
    unsafe { pc_cancel() == 1 }
}

#[unsafe(no_mangle)]
pub extern "C" fn pc_progress(phase: u32, done: u32, total: u32) {
    if total == 0 || total as usize > peazip_core::Limits::default().max_entries || done > total {
        return;
    }
    let message = match phase {
        0 => format!("Reading files: {done}/{total}"),
        1 if done == total => "Compressing ZIP…".to_owned(),
        2 => "Validating and extracting archive…".to_owned(),
        3 => format!("Writing entries: {done}/{total}"),
        _ => return,
    };
    PENDING.with(|slot| {
        if let Some(pending) = slot.borrow().as_ref() {
            let _ = pending.sender.unbounded_send(Update::Progress(message));
        }
    });
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
        let _ = pending.sender.unbounded_send(Update::Finished(result));
    }
}

pub fn download(bytes: &[u8], format: peazip_core::ArchiveFormat) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("No archive is open.".into());
    }
    // The host copies the bytes synchronously; it never retains the pointer.
    let code = match format {
        peazip_core::ArchiveFormat::Zip => 0,
        peazip_core::ArchiveFormat::Tar => 1,
    };
    if unsafe { pc_download(bytes.as_ptr(), bytes.len(), code) } == 1 {
        Ok(())
    } else {
        Err("The browser could not start a download.".into())
    }
}
