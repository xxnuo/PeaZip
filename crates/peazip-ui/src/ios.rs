use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

use futures_channel::oneshot;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::{Allocated, Retained},
    runtime::{AnyClass, AnyObject},
    sel,
};
use objc2_foundation::{
    NSArray, NSHomeDirectory, NSNotification, NSNotificationCenter, NSObject, NSObjectProtocol,
    NSString, NSURL,
};
use objc2_ui_kit::{UIApplication, UIViewController};
use peazip_core::{ArchiveFormat, Limits, MemoryEntry};

enum Output {
    Files(Vec<MemoryEntry>),
    Name(String),
}
type Reply = Result<Option<Output>, String>;
const SHEET_CLOSING: &str = "The previous system sheet is still closing. Try again.";

thread_local! {
    // UIKit holds its document delegate weakly.
    static ACTIVE: RefCell<Option<Retained<DocumentDelegate>>> = const { RefCell::new(None) };
    static SERIAL: Cell<u64> = const { Cell::new(0) };
    static READ_SCOPES: RefCell<BTreeMap<u64, Vec<Retained<NSURL>>>> = const { RefCell::new(BTreeMap::new()) };
}

struct Ivars {
    id: u64,
    export: bool,
    answered: Cell<bool>,
    reading: Cell<bool>,
    sender: Arc<Mutex<Option<oneshot::Sender<Reply>>>>,
    staging: RefCell<Option<tempfile::TempDir>>,
    picker: Retained<UIViewController>,
}

fn complete(id: u64, reply: Reply, uncertain: bool) {
    ACTIVE.with_borrow_mut(|active| {
        if active
            .as_ref()
            .is_none_or(|delegate| delegate.ivars().id != id)
        {
            return;
        }
        let delegate = active.take().unwrap();
        unsafe {
            NSNotificationCenter::defaultCenter().removeObserver(&*delegate);
        }
        if uncertain {
            // A disconnected scene does not prove the provider stopped reading the source.
            if let Some(staging) = delegate.ivars().staging.borrow_mut().take() {
                let _retained_source = staging.keep();
            }
        }
        if let Some(sender) = delegate
            .ivars()
            .sender
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = sender.send(reply);
        }
    });
}

fn deliver(id: u64, reply: Reply, uncertain: bool) {
    dispatch2::DispatchQueue::main().exec_async(move || {
        let scopes = READ_SCOPES.with_borrow_mut(|scopes| scopes.remove(&id));
        for url in scopes.into_iter().flatten() {
            unsafe {
                url.stopAccessingSecurityScopedResource();
            }
        }
        complete(id, reply, uncertain);
    });
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "PeaZipDocumentDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct DocumentDelegate;

    unsafe impl NSObjectProtocol for DocumentDelegate {}

    impl DocumentDelegate {
        #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
        fn picked(&self, _picker: &UIViewController, urls: &NSArray<NSURL>) {
            if self.ivars().answered.replace(true) { return; }
            let id = self.ivars().id;
            if self.ivars().export {
                if urls.count() != 1 {
                    deliver(id, Err("The system returned an unexpected export result; the destination may contain a copy.".into()), true);
                } else {
                    let name = urls.firstObject().and_then(|url| url.lastPathComponent())
                        .map(|name| name.to_string()).unwrap_or_else(|| "archive".into());
                    deliver(id, Ok(Some(Output::Name(name))), false);
                }
                return;
            }
            self.ivars().reading.set(true);
            if urls.count() > Limits::default().max_entries {
                deliver(id, Err("Selected files exceed the entry limit.".into()), false);
                return;
            }
            let paths = urls.iter().map(|url| if url.isFileURL() { url.path().map(|path| PathBuf::from(path.to_string())) } else { None })
                .collect::<Option<Vec<_>>>();
            let Some(paths) = paths else {
                deliver(id, Err("The picker did not return local file copies.".into()), false);
                return;
            };
            let scopes = urls.iter().filter(|url| unsafe { url.startAccessingSecurityScopedResource() }).collect();
            READ_SCOPES.with_borrow_mut(|active| { active.insert(id, scopes); });
            let started = std::thread::Builder::new().name("peazip-ios-input".into()).spawn(move || {
                let reply = crate::mobile_files::read_files(&paths, Limits::default())
                    .map(|files| if files.is_empty() { None } else { Some(Output::Files(files)) });
                deliver(id, reply, false);
            });
            if let Err(error) = started { deliver(id, Err(error.to_string()), false); }
        }

        #[unsafe(method(documentPickerWasCancelled:))]
        fn cancelled(&self, _picker: &UIViewController) {
            if !self.ivars().answered.replace(true) { deliver(self.ivars().id, Ok(None), false); }
        }

        #[unsafe(method(presentationControllerDidDismiss:))]
        fn dismissed(&self, _controller: &AnyObject) {
            if !self.ivars().answered.replace(true) { deliver(self.ivars().id, Ok(None), false); }
        }

        #[unsafe(method(peaZipSceneDisconnected:))]
        fn disconnected(&self, _notification: &NSNotification) {
            if self.ivars().answered.get() && !self.ivars().reading.get() { return; }
            self.ivars().answered.set(true);
            complete(self.ivars().id, Err("The scene closed before the system confirmed the result. The destination may contain a copy.".into()), true);
        }
    }
);

fn controller(
    mtm: MainThreadMarker,
) -> Result<(Retained<UIViewController>, Retained<AnyObject>), String> {
    let app = UIApplication::sharedApplication(mtm);
    unsafe {
        let scenes: Retained<AnyObject> = msg_send![&app, connectedScenes];
        let scenes: Retained<NSArray<AnyObject>> = msg_send![&scenes, allObjects];
        for scene in scenes.iter() {
            let state: isize = msg_send![&scene, activationState];
            let supports_windows: bool = msg_send![&scene, respondsToSelector: sel!(windows)];
            if state > 1 || state < 0 || !supports_windows {
                continue;
            }
            let windows: Retained<NSArray<AnyObject>> = msg_send![&scene, windows];
            for window in windows.iter() {
                let key: bool = msg_send![&window, isKeyWindow];
                if !key {
                    continue;
                }
                let root: Option<Retained<UIViewController>> =
                    msg_send![&window, rootViewController];
                if let Some(top) = root {
                    // A form may still be presented before its removal transaction starts.
                    if top.presentedViewController().is_some()
                        || top.isBeingDismissed()
                        || top.viewIfLoaded().and_then(|view| view.window()).is_none()
                    {
                        return Err(SHEET_CLOSING.into());
                    }
                    return Ok((top, scene));
                }
            }
        }
    }
    Err("No active iOS window can present the file picker.".into())
}

async fn present(staged: Option<(tempfile::TempDir, PathBuf)>) -> Reply {
    let (sender, receiver) = oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(sender)));
    dispatch2::DispatchQueue::main().exec_async(move || schedule(staged, sender, 0));
    receiver
        .await
        .map_err(|_| "The iOS file request ended without a confirmed result.".to_owned())?
}

fn schedule(
    staged: Option<(tempfile::TempDir, PathBuf)>,
    sender: Arc<Mutex<Option<oneshot::Sender<Reply>>>>,
    attempts: u8,
) {
    if sender
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_none_or(|sender| sender.is_canceled())
    {
        return;
    }
    let ready = MainThreadMarker::new()
        .ok_or_else(|| "The file picker must run on the main thread.".to_owned())
        .and_then(|mtm| controller(mtm).map(|_| ()));
    if ready.as_ref().is_err_and(|error| error == SHEET_CLOSING) && attempts < 20 {
        // Closing an nsh modal is a separate UI transaction; wait for an attached presenter.
        let error_sender = Arc::clone(&sender);
        let when =
            dispatch2::DispatchTime::try_from(std::time::Duration::from_millis(100)).unwrap();
        if let Err(error) = dispatch2::DispatchQueue::main()
            .after(when, move || schedule(staged, sender, attempts + 1))
        {
            if let Some(sender) = error_sender
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
            {
                let _ = sender.send(Err(format!("Cannot schedule the iOS picker: {error:?}")));
            }
        }
        return;
    }
    let result = ready.and_then(|_| start(staged, Arc::clone(&sender)));
    if let Err(error) = result {
        if let Some(sender) = sender.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = sender.send(Err(error));
        }
    }
}

fn start(
    staged: Option<(tempfile::TempDir, PathBuf)>,
    sender: Arc<Mutex<Option<oneshot::Sender<Reply>>>>,
) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("The file picker must run on the main thread.")?;
    if ACTIVE.with_borrow(|active| active.is_some()) {
        return Err("A file picker is already active.".into());
    }
    let (top, scene) = controller(mtm)?;
    let id = SERIAL.with(|serial| {
        let next = serial
            .get()
            .checked_add(1)
            .ok_or("iOS request IDs exhausted.")?;
        serial.set(next);
        Ok::<_, String>(next)
    })?;
    let picker_class = AnyClass::get(c"UIDocumentPickerViewController")
        .ok_or("iOS document picker unavailable.")?;
    let export = staged.is_some();
    let picker: Retained<UIViewController> = unsafe {
        let alloc: Allocated<AnyObject> = msg_send![picker_class, alloc];
        let picker: Option<Retained<AnyObject>> = if let Some((_, path)) = &staged {
            let path = NSString::from_str(path.to_str().ok_or("Invalid temporary filename.")?);
            let url = NSURL::fileURLWithPath(&path);
            let urls = NSArray::from_retained_slice(&[url]);
            let picker: Option<Retained<AnyObject>> =
                msg_send![alloc, initForExportingURLs: &*urls, asCopy: true];
            if let Some(picker) = &picker {
                let documents = NSHomeDirectory()
                    .stringByAppendingPathComponent(&NSString::from_str("Documents"));
                let directory = NSURL::fileURLWithPath(&documents);
                let _: () = msg_send![picker, setDirectoryURL: &*directory];
            }
            picker
        } else {
            static TYPES_LOADED: OnceLock<bool> = OnceLock::new();
            if !*TYPES_LOADED.get_or_init(|| !libc::dlopen(c"/System/Library/Frameworks/UniformTypeIdentifiers.framework/UniformTypeIdentifiers".as_ptr(), libc::RTLD_LAZY).is_null()) {
                return Err("iOS content types are unavailable.".into());
            }
            let class = AnyClass::get(c"UTType").ok_or("iOS content types are unavailable.")?;
            let data = NSString::from_str("public.data");
            let kind: Option<Retained<AnyObject>> = msg_send![class, typeWithIdentifier: &*data];
            let types = NSArray::from_retained_slice(&[
                kind.ok_or("iOS file content type is unavailable.")?
            ]);
            let picker: Option<Retained<AnyObject>> =
                msg_send![alloc, initForOpeningContentTypes: &*types, asCopy: true];
            if let Some(picker) = &picker {
                let _: () = msg_send![picker, setAllowsMultipleSelection: true];
            }
            picker
        };
        Retained::cast_unchecked(picker.ok_or("Cannot create the iOS document picker.")?)
    };
    let delegate = DocumentDelegate::alloc(mtm).set_ivars(Ivars {
        id,
        export,
        answered: Cell::new(false),
        reading: Cell::new(false),
        sender,
        staging: RefCell::new(staged.map(|(directory, _)| directory)),
        picker,
    });
    let delegate: Retained<DocumentDelegate> = unsafe { msg_send![super(delegate), init] };
    unsafe {
        let _: () = msg_send![&delegate.ivars().picker, setDelegate: &*delegate];
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            &*delegate,
            sel!(peaZipSceneDisconnected:),
            Some(&NSString::from_str("UISceneDidDisconnectNotification")),
            Some(&*scene),
        );
        if let Some(sheet) = delegate.ivars().picker.presentationController() {
            let _: () = msg_send![&sheet, setDelegate: &*delegate];
        }
    }
    ACTIVE.with_borrow_mut(|active| *active = Some(delegate.clone()));
    top.presentViewController_animated_completion(&delegate.ivars().picker, true, None);
    if let Some(sheet) = delegate.ivars().picker.presentationController() {
        unsafe {
            let _: () = msg_send![&sheet, setDelegate: &*delegate];
        }
    }
    Ok(())
}

pub async fn pick_files() -> Result<Option<Vec<MemoryEntry>>, String> {
    match present(None).await? {
        None => Ok(None),
        Some(Output::Files(files)) => Ok(Some(files)),
        _ => Err("Unexpected iOS input response.".into()),
    }
}

pub async fn save_archive(
    bytes: Arc<Vec<u8>>,
    format: ArchiveFormat,
) -> Result<Option<String>, String> {
    if bytes.is_empty() || bytes.len() as u64 > Limits::default().max_archive_bytes {
        return Err("Archive exceeds the export byte limit.".into());
    }
    let staged = nsh_core::prelude::blocking(move || {
        let documents = PathBuf::from(NSHomeDirectory().to_string()).join("Documents");
        std::fs::create_dir_all(documents).map_err(|e| e.to_string())?;
        let directory = tempfile::Builder::new()
            .prefix("peazip-export-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let path = directory.path().join(crate::archive_filename(format));
        std::fs::write(&path, bytes.as_slice()).map_err(|e| e.to_string())?;
        Ok::<_, String>((directory, path))
    })
    .await
    .map_err(|e| e.to_string())??;
    match present(Some(staged)).await? {
        None => Ok(None),
        Some(Output::Name(name)) => Ok(Some(name)),
        _ => Err("Unexpected iOS export response.".into()),
    }
}
