#[cfg(target_arch = "wasm32")]
pub use host::*;

#[cfg(any(target_arch = "wasm32", test))]
fn create_parameters(options: peazip_core::CreateOptions) -> (u32, u32, u32, &'static str) {
    use peazip_core::{CreateOptions, SevenZipCompression, ZipCompression};
    match options {
        CreateOptions::Zip(options) => {
            let (compression, level) = match options.compression {
                ZipCompression::Automatic => (0, 0),
                ZipCompression::Stored => (1, 0),
                ZipCompression::Deflate { level } => (2, u32::from(level)),
            };
            (0, compression, level, "Compressing ZIP…")
        }
        CreateOptions::Tar => (1, 0, 0, "Writing TAR…"),
        CreateOptions::TarGzip { level } => (2, 0, u32::from(level), "Compressing TAR.GZ…"),
        CreateOptions::SevenZip(SevenZipCompression::Stored) => (3, 0, 0, "Writing 7Z…"),
        CreateOptions::SevenZip(SevenZipCompression::Lzma2 { level }) => {
            (3, 1, u32::from(level), "Compressing 7Z…")
        }
    }
}

pub(crate) fn decode_test_report(packet: &[u8]) -> Result<peazip_core::TestReport, String> {
    if packet.len() != 20 {
        return Err("Invalid archive test report length.".into());
    }
    let report = peazip_core::TestReport {
        entries: u32::from_le_bytes(packet[..4].try_into().unwrap()) as usize,
        files: u32::from_le_bytes(packet[4..8].try_into().unwrap()) as usize,
        total_bytes: u64::from_le_bytes(packet[8..16].try_into().unwrap()),
        checksummed_files: u32::from_le_bytes(packet[16..20].try_into().unwrap()) as usize,
    };
    let limits = peazip_core::Limits::default();
    if report.entries > limits.max_entries
        || report.files > report.entries
        || report.checksummed_files > report.files
        || report.total_bytes > limits.max_total_bytes
        || report.total_bytes > (report.files as u64).saturating_mul(limits.max_entry_bytes)
    {
        return Err("Invalid archive test report counts or sizes.".into());
    }
    Ok(report)
}

pub(crate) fn decode_list(
    mut packet: &[u8],
) -> Result<(peazip_core::ArchiveFormat, Vec<peazip_core::Entry>), String> {
    fn take<'a>(packet: &mut &'a [u8], len: usize) -> Result<&'a [u8], String> {
        let (value, rest) = packet
            .split_at_checked(len)
            .ok_or("Truncated archive metadata.")?;
        *packet = rest;
        Ok(value)
    }
    fn number(packet: &mut &[u8]) -> Result<u32, String> {
        Ok(u32::from_le_bytes(take(packet, 4)?.try_into().unwrap()))
    }
    fn read_u64(packet: &mut &[u8]) -> Result<u64, String> {
        Ok(u64::from_le_bytes(take(packet, 8)?.try_into().unwrap()))
    }
    let limits = peazip_core::Limits::default();
    if packet.len() as u64 > limits.max_archive_bytes
        || packet.len() > 9 + limits.max_entries * (32 + limits.max_path_bytes)
    {
        return Err("Archive metadata exceeds the byte limit.".into());
    }
    if take(&mut packet, 4)? != b"PZL2" {
        return Err("Invalid archive metadata version.".into());
    }
    let format = match take(&mut packet, 1)?[0] {
        0 => peazip_core::ArchiveFormat::Zip,
        1 => peazip_core::ArchiveFormat::Tar,
        2 => peazip_core::ArchiveFormat::TarGzip,
        3 => peazip_core::ArchiveFormat::SevenZip,
        _ => return Err("Invalid archive metadata format.".into()),
    };
    let count = number(&mut packet)? as usize;
    if count > limits.max_entries || count > packet.len() / 33 {
        return Err("Invalid archive metadata count.".into());
    }
    let mut entries = Vec::with_capacity(count);
    let mut total = 0u64;
    for _ in 0..count {
        let kind = number(&mut packet)?;
        if kind & !3 != 0 || kind == 3 {
            return Err("Invalid archive metadata kind.".into());
        }
        let is_dir = kind & 1 != 0;
        let packed_known = kind & 2 != 0;
        let len = number(&mut packet)? as usize;
        if len == 0 || len > limits.max_path_bytes {
            return Err("Invalid archive metadata path length.".into());
        }
        let path = std::str::from_utf8(take(&mut packet, len)?)
            .map_err(|_| "Invalid UTF-8 archive metadata path.")?;
        let key = if is_dir {
            path.strip_suffix('/')
                .ok_or("Invalid archive directory path.")?
        } else {
            path
        };
        if key.is_empty()
            || path.contains(['\\', ':', '<', '>', '"', '|', '?', '*'])
            || path.chars().any(char::is_control)
            || key.split('/').any(|part| {
                let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
                part.is_empty()
                    || part == "."
                    || part == ".."
                    || part.ends_with(['.', ' '])
                    || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                    || (stem.len() == 4
                        && (stem.starts_with("COM") || stem.starts_with("LPT"))
                        && matches!(stem.as_bytes()[3], b'1'..=b'9'))
            })
        {
            return Err("Invalid archive metadata path.".into());
        }
        let size = read_u64(&mut packet)?;
        let compressed_size = read_u64(&mut packet)?;
        let packed_size = read_u64(&mut packet)?;
        if !packed_known && packed_size != 0 {
            return Err("Invalid unknown packed metadata value.".into());
        }
        total = total
            .checked_add(size)
            .ok_or("Archive metadata size overflow.")?;
        if size > limits.max_entry_bytes
            || compressed_size > limits.max_archive_bytes
            || packed_size > limits.max_archive_bytes
            || total > limits.max_total_bytes
            || (is_dir && size != 0)
        {
            return Err("Archive metadata size exceeds the limit.".into());
        }
        entries.push(peazip_core::Entry {
            path: path.to_owned(),
            is_dir,
            size,
            compressed_size,
            packed_size: packed_known.then_some(packed_size),
        });
    }
    if !packet.is_empty() {
        return Err("Trailing archive metadata bytes.".into());
    }
    Ok((format, entries))
}

#[cfg(target_arch = "wasm32")]
mod host {
    use std::{cell::RefCell, sync::Arc};

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
        create_label: Option<&'static str>,
    }

    thread_local! {
        static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
    }

    #[link(wasm_import_module = "peazip")]
    unsafe extern "C" {
        fn pc_create_archive_start(format: u32, compression: u32, level: u32) -> u32;
        fn pc_create_operation_begin() -> u32;
        fn pc_create_operation_valid(token: u32) -> u32;
        fn pc_create_operation_end(token: u32) -> u32;
        fn pc_list_start(ptr: *const u8, len: usize) -> u32;
        fn pc_test_start(ptr: *const u8, len: usize) -> u32;
        fn pc_can_extract() -> u32;
        fn pc_extract_with_options_start(ptr: *const u8, len: usize, paths: u32) -> u32;
        fn pc_cancel() -> u32;
        fn pc_download(ptr: *const u8, len: usize, format: u32) -> u32;
    }

    struct CreateOperation(u32);
    impl CreateOperation {
        fn begin() -> Result<Self, String> {
            let token = unsafe { pc_create_operation_begin() };
            if token == 0 {
                Err("Browser creation operation is unavailable or already running.".into())
            } else {
                Ok(Self(token))
            }
        }
        fn valid(&self) -> bool {
            unsafe { pc_create_operation_valid(self.0) == 1 }
        }
    }
    impl Drop for CreateOperation {
        fn drop(&mut self) {
            unsafe {
                pc_create_operation_end(self.0);
            }
        }
    }

    pub async fn create_and_list_archive(
        options: peazip_core::CreateOptions,
        mut progress: impl FnMut(String),
    ) -> Result<Option<crate::LoadedArchive>, String> {
        options.validate().map_err(|error| error.to_string())?;
        let operation = CreateOperation::begin()?;
        let created = create_archive(options, &mut progress).await;
        if !operation.valid() {
            return Ok(None);
        }
        let Some(bytes) = created? else {
            return Ok(None);
        };
        let bytes = Arc::new(bytes);
        progress("Listing created archive…".to_owned());
        if !operation.valid() {
            return Ok(None);
        }
        let listed = list_archive(bytes.clone()).await;
        if !operation.valid() {
            return Ok(None);
        }
        let Some((format, entries)) = listed? else {
            return Ok(None);
        };
        if format != options.format() {
            return Err("Created archive format does not match the requested format.".into());
        }
        Ok(Some((bytes, entries, format)))
    }

    pub async fn list_archive(
        bytes: Arc<Vec<u8>>,
    ) -> Result<Option<(peazip_core::ArchiveFormat, Vec<peazip_core::Entry>)>, String> {
        if bytes.is_empty() || bytes.len() as u64 > peazip_core::Limits::default().max_archive_bytes
        {
            return Err("Invalid archive size.".into());
        }
        run(
            move || unsafe { pc_list_start(bytes.as_ptr(), bytes.len()) },
            |_| {},
            None,
        )
        .await?
        .map(|metadata| super::decode_list(&metadata))
        .transpose()
    }

    pub async fn create_archive(
        options: peazip_core::CreateOptions,
        progress: impl FnMut(String),
    ) -> Reply {
        options.validate().map_err(|error| error.to_string())?;
        let (format_code, compression, level, label) = super::create_parameters(options);
        run(
            || unsafe { pc_create_archive_start(format_code, compression, level) },
            progress,
            Some(label),
        )
        .await
    }

    pub async fn test_archive(
        bytes: Arc<Vec<u8>>,
    ) -> Result<Option<peazip_core::TestReport>, String> {
        if bytes.is_empty() || bytes.len() as u64 > peazip_core::Limits::default().max_archive_bytes
        {
            return Err("Invalid archive size.".into());
        }
        run(
            move || unsafe { pc_test_start(bytes.as_ptr(), bytes.len()) },
            |_| {},
            None,
        )
        .await?
        .map(|report| super::decode_test_report(&report))
        .transpose()
    }

    async fn run(
        start: impl FnOnce() -> u32,
        mut progress: impl FnMut(String),
        create_label: Option<&'static str>,
    ) -> Reply {
        let (sender, mut receiver) = mpsc::unbounded();
        PENDING.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_some() {
                return Err("An archive operation is already running.".to_owned());
            }
            *slot = Some(Pending {
                sender,
                bytes: Vec::new(),
                create_label,
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
        options: peazip_core::ExtractOptions,
        progress: impl FnMut(String),
    ) -> Result<Option<String>, String> {
        let paths = match options.paths {
            peazip_core::ExtractPaths::Full => 0,
            peazip_core::ExtractPaths::FilesOnly => 1,
        };
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
            move || unsafe { pc_extract_with_options_start(packet.as_ptr(), packet.len(), paths) },
            progress,
            None,
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
        if total == 0 || total as usize > peazip_core::Limits::default().max_entries || done > total
        {
            return;
        }
        PENDING.with(|slot| {
            if let Some(pending) = slot.borrow().as_ref() {
                let message = match phase {
                    0 => format!("Reading files: {done}/{total}"),
                    1 if done == total => pending
                        .create_label
                        .unwrap_or("Compressing ZIP…")
                        .to_owned(),
                    2 => "Validating and extracting archive…".to_owned(),
                    3 => format!("Writing entries: {done}/{total}"),
                    _ => return,
                };
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
            peazip_core::ArchiveFormat::TarGzip => 2,
            peazip_core::ArchiveFormat::SevenZip => 3,
        };
        if unsafe { pc_download(bytes.as_ptr(), bytes.len(), code) } == 1 {
            Ok(())
        } else {
            Err("The browser could not start a download.".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::create_parameters;
    use peazip_core::{CreateOptions, SevenZipCompression, ZipCompression, ZipOptions};

    #[test]
    fn creation_wire_parameters_and_progress_labels_preserve_methods() {
        assert_eq!(
            create_parameters(CreateOptions::default()),
            (0, 0, 0, "Compressing ZIP…")
        );
        for (compression, code, level) in [
            (ZipCompression::Stored, 1, 0),
            (ZipCompression::Deflate { level: 1 }, 2, 1),
            (ZipCompression::Deflate { level: 9 }, 2, 9),
        ] {
            assert_eq!(
                create_parameters(CreateOptions::Zip(ZipOptions { compression })),
                (0, code, level, "Compressing ZIP…")
            );
        }
        assert_eq!(
            create_parameters(CreateOptions::Tar),
            (1, 0, 0, "Writing TAR…")
        );
        assert_eq!(
            create_parameters(CreateOptions::SevenZip(SevenZipCompression::Stored)),
            (3, 0, 0, "Writing 7Z…")
        );
        for level in 1..=9 {
            assert_eq!(
                create_parameters(CreateOptions::TarGzip { level }),
                (2, 0, u32::from(level), "Compressing TAR.GZ…")
            );
            assert_eq!(
                create_parameters(CreateOptions::SevenZip(SevenZipCompression::Lzma2 {
                    level
                })),
                (3, 1, u32::from(level), "Compressing 7Z…")
            );
        }
    }
}
