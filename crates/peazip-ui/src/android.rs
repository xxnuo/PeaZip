use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use futures_channel::oneshot;
use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JObjectArray, JString, JValue};
use jni::sys::jlong;
use jni::{JNIEnv, JavaVM};
use peazip_core::{ArchiveFormat, Limits, MemoryEntry};

static VM: OnceLock<JavaVM> = OnceLock::new();
static STATE: Mutex<State> = Mutex::new(State {
    serial: 0,
    binding: None,
    pending: None,
});

struct State {
    serial: jlong,
    binding: Option<Binding>,
    pending: Option<Pending>,
}

struct Binding {
    generation: jlong,
    activity: GlobalRef,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Pick,
    Save,
    Export,
}

enum Output {
    Files(Vec<MemoryEntry>),
    Name(String),
}

type Reply = Result<Option<Output>, String>;

struct Pending {
    id: jlong,
    generation: jlong,
    kind: Kind,
    sender: oneshot::Sender<Reply>,
    reply: Option<Reply>,
    export_status: String,
}

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|error| error.into_inner())
}

fn next_id(state: &mut State) -> Result<jlong, String> {
    state.serial = state
        .serial
        .checked_add(1)
        .ok_or("Android request IDs exhausted.")?;
    Ok(state.serial)
}

fn accepts(id: jlong, kind: Kind) -> bool {
    let state = state();
    state.pending.as_ref().is_some_and(|pending| {
        pending.id == id
            && pending.kind == kind
            && state
                .binding
                .as_ref()
                .is_some_and(|b| b.generation == pending.generation)
    })
}

fn complete(id: jlong, reply: Reply) {
    let pending = {
        let mut state = state();
        if state
            .pending
            .as_ref()
            .is_none_or(|pending| pending.id != id)
        {
            return;
        }
        state.pending.take()
    };
    if let Some(pending) = pending {
        let _ = pending.sender.send(reply);
    }
}

fn prepare(id: jlong, reply: Reply) {
    let mut state = state();
    if let Some(pending) = state.pending.as_mut()
        && pending.id == id
        && pending.reply.is_none()
    {
        pending.reply = Some(reply);
    }
}

fn cancel(pending: Option<Pending>) {
    if let Some(pending) = pending {
        let message = match pending.kind {
            Kind::Pick => "Android activity was destroyed; input selection was cancelled.".to_owned(),
            Kind::Save => {
                "Android activity was destroyed; saving was cancelled. A partial document may remain.".to_owned()
            }
            Kind::Export => format!("Android activity was destroyed; export cancelled. {}", pending.export_status),
        };
        let _ = pending.sender.send(Err(message));
    }
}

// JNI never unwinds into Java, and a pending Java exception never escapes into nsh.
fn boundary<T>(
    env: &mut JNIEnv<'_>,
    f: impl FnOnce(&mut JNIEnv<'_>) -> Result<T, String>,
) -> Result<T, String> {
    let result = catch_unwind(AssertUnwindSafe(|| f(env)))
        .unwrap_or_else(|_| Err("Android bridge panicked.".into()));
    match env.exception_check() {
        Ok(false) => result,
        _ => {
            let _ = env.exception_clear();
            Err("Android JNI call failed with a Java exception.".into())
        }
    }
}

fn jni_error(error: jni::errors::Error) -> String {
    format!("Android JNI error: {error}")
}

fn local_frame<T>(
    env: &mut JNIEnv<'_>,
    f: impl FnOnce(&mut JNIEnv<'_>) -> Result<T, String>,
) -> Result<T, String> {
    env.with_local_frame(8, |env| Ok::<_, jni::errors::Error>(f(env)))
        .map_err(jni_error)?
}

async fn request(kind: Kind, bytes: Option<Arc<Vec<u8>>>, format: ArchiveFormat) -> Reply {
    let (id, activity, receiver) = {
        let mut state = state();
        if state.pending.is_some() {
            return Err("An Android file operation is already running.".into());
        }
        let binding = state
            .binding
            .as_ref()
            .ok_or("Android activity is unavailable.")?;
        let activity = binding.activity.clone();
        let generation = binding.generation;
        let id = next_id(&mut state)?;
        let (sender, receiver) = oneshot::channel();
        state.pending = Some(Pending {
            id,
            generation,
            kind,
            sender,
            reply: None,
            export_status: "Root not yet known; completed 0 entries; unfinished path: not yet known. Documents may remain; nothing was deleted.".into(),
        });
        (id, activity, receiver)
    };
    let started = (|| {
        let vm = VM.get().ok_or("Android Java VM is unavailable.")?;
        let mut env = vm.attach_current_thread().map_err(jni_error)?;
        boundary(&mut env, |env| {
            local_frame(env, |env| {
                if !env
                    .call_method(activity.as_obj(), "archiveTransferAvailable", "()Z", &[])
                    .map_err(jni_error)?
                    .z()
                    .map_err(jni_error)?
                {
                    return Err(
                        "A previous Android transfer or its cleanup is still running.".into(),
                    );
                }
                let accepted = match bytes {
                    None => env
                        .call_method(
                            activity.as_obj(),
                            "pickArchiveInputs",
                            "(J)Z",
                            &[JValue::Long(id)],
                        )
                        .map_err(jni_error)?
                        .z()
                        .map_err(jni_error)?,
                    Some(bytes) => {
                        let array = env.byte_array_from_slice(&bytes).map_err(jni_error)?;
                        if kind == Kind::Export {
                            env.call_method(
                                activity.as_obj(),
                                "exportEntries",
                                "(J[B)Z",
                                &[JValue::Long(id), JValue::Object(array.as_ref())],
                            )
                        } else {
                            let format = match format {
                                ArchiveFormat::Zip => 0,
                                ArchiveFormat::Tar => 1,
                            };
                            env.call_method(
                                activity.as_obj(),
                                "saveArchive",
                                "(J[BI)Z",
                                &[
                                    JValue::Long(id),
                                    JValue::Object(array.as_ref()),
                                    JValue::Int(format),
                                ],
                            )
                        }
                        .map_err(jni_error)?
                        .z()
                        .map_err(jni_error)?
                    }
                };
                if !accepted {
                    return Err(
                        "An Android file transfer is already running or cleaning up.".into(),
                    );
                }
                Ok(())
            })
        })
    })();
    // Drop our Activity reference before awaiting; onDestroy releases the binding.
    drop(activity);
    if let Err(error) = started {
        complete(id, Err(error));
    }
    receiver
        .await
        .map_err(|_| "Android file operation ended unexpectedly.".to_owned())?
}

pub async fn pick_files() -> Result<Option<Vec<MemoryEntry>>, String> {
    match request(Kind::Pick, None, ArchiveFormat::Zip).await? {
        None => Ok(None),
        Some(Output::Files(files)) => Ok(Some(files)),
        _ => Err("Unexpected Android input response.".into()),
    }
}

pub async fn save_archive(
    bytes: Arc<Vec<u8>>,
    format: ArchiveFormat,
) -> Result<Option<String>, String> {
    if bytes.len() as u64 > Limits::default().max_archive_bytes {
        return Err("Archive exceeds the 32 MiB save limit.".into());
    }
    let reply = request(Kind::Save, Some(bytes), format)
        .await
        .map_err(|error| format!("{error} The destination may contain a partial document."))?;
    match reply {
        None => Ok(None),
        Some(Output::Name(name)) => Ok(Some(name)),
        _ => Err("Unexpected Android save response.".into()),
    }
}

// The caller must finish core::extract_selected_zip (including CRC checks) before exporting.
pub async fn export_entries(entries: Vec<MemoryEntry>) -> Result<Option<String>, String> {
    let limits = Limits::default();
    if entries.is_empty() || entries.len() > limits.max_entries {
        return Err("Invalid Android export entry count.".into());
    }
    let count = entries.len();
    let mut size = 4usize;
    let mut total = 0u64;
    for entry in &entries {
        if entry.path.is_empty()
            || entry.path.len() > limits.max_path_bytes
            || entry.data.len() as u64 > limits.max_entry_bytes
            || (entry.is_dir && !entry.data.is_empty())
        {
            return Err("Android export exceeds path or entry limits.".into());
        }
        total = total
            .checked_add(entry.data.len() as u64)
            .ok_or("Export size overflow.")?;
        size = size
            .checked_add(12 + entry.path.len() + entry.data.len())
            .ok_or("Export size overflow.")?;
    }
    if total > limits.max_total_bytes || size > i32::MAX as usize {
        return Err("Android export exceeds the 64 MiB total limit.".into());
    }
    let mut packet = Vec::new();
    packet
        .try_reserve_exact(size)
        .map_err(|_| "Cannot allocate Android export packet.")?;
    packet.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for entry in entries {
        packet.extend_from_slice(&u32::from(entry.is_dir).to_le_bytes());
        packet.extend_from_slice(&(entry.path.len() as u32).to_le_bytes());
        packet.extend_from_slice(&(entry.data.len() as u32).to_le_bytes());
        packet.extend_from_slice(entry.path.as_bytes());
        packet.extend_from_slice(&entry.data);
    }
    match request(Kind::Export, Some(Arc::new(packet)), ArchiveFormat::Zip).await? {
        None => Ok(None),
        Some(Output::Name(name)) => Ok(Some(format!("Extracted {count} entries to {name}"))),
        _ => Err("Unexpected Android export response.".into()),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_longbridge_nsh_PeaZipActivity_nativeBind(
    mut env: JNIEnv,
    activity: JObject,
) -> jlong {
    boundary(&mut env, |env| {
        let vm = env.get_java_vm().map_err(jni_error)?;
        let _ = VM.set(vm);
        let activity = env.new_global_ref(activity).map_err(jni_error)?;
        let (generation, old_binding, pending) = {
            let mut state = state();
            let generation = next_id(&mut state)?;
            let old_binding = state.binding.replace(Binding {
                generation,
                activity,
            });
            (generation, old_binding, state.pending.take())
        };
        drop(old_binding);
        cancel(pending);
        Ok(generation)
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_longbridge_nsh_PeaZipActivity_nativeUnbind(
    mut env: JNIEnv,
    _class: JClass,
    generation: jlong,
) {
    let _ = boundary(&mut env, |_| {
        let (binding, pending) = {
            let mut state = state();
            if state
                .binding
                .as_ref()
                .is_none_or(|b| b.generation != generation)
            {
                return Ok(());
            }
            (state.binding.take(), state.pending.take())
        };
        drop(binding);
        cancel(pending);
        Ok(())
    });
}

fn picked(
    env: &mut JNIEnv<'_>,
    names: JObjectArray,
    data: JObjectArray,
) -> Result<Vec<MemoryEntry>, String> {
    let limits = Limits::default();
    let count = env.get_array_length(&names).map_err(jni_error)?;
    if count <= 0
        || count as usize > limits.max_entries
        || env.get_array_length(&data).map_err(jni_error)? != count
    {
        return Err("Invalid Android input count.".into());
    }
    let mut files = Vec::with_capacity(count as usize);
    let mut total = 0u64;
    for index in 0..count {
        // Each item releases its JNI locals, including the String class lookup.
        let file = local_frame(env, |env| {
            let name = JString::from(
                env.get_object_array_element(&names, index)
                    .map_err(jni_error)?,
            );
            let path: String = env.get_string(&name).map_err(jni_error)?.into();
            if path.is_empty() || path.len() > limits.max_path_bytes {
                return Err("Android input name exceeds the path limit.".into());
            }
            let array = JByteArray::from(
                env.get_object_array_element(&data, index)
                    .map_err(jni_error)?,
            );
            let size = env.get_array_length(&array).map_err(jni_error)? as u64;
            total = total
                .checked_add(size)
                .ok_or("Android input total overflow.")?;
            if size > limits.max_entry_bytes || total > limits.max_total_bytes {
                return Err("Android input exceeds the entry or total byte limit.".into());
            }
            Ok(MemoryEntry {
                path,
                is_dir: false,
                data: env.convert_byte_array(&array).map_err(jni_error)?,
            })
        })?;
        files.push(file);
    }
    Ok(files)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_longbridge_nsh_PeaZipActivity_nativePicked(
    mut env: JNIEnv,
    _class: JClass,
    id: jlong,
    names: JObjectArray,
    data: JObjectArray,
) {
    let result = boundary(&mut env, |env| {
        if !accepts(id, Kind::Pick) {
            return Ok(None);
        }
        picked(env, names, data).map(|files| Some(Output::Files(files)))
    });
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if accepts(id, Kind::Pick) {
            prepare(id, result);
        }
    }));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_longbridge_nsh_PeaZipActivity_nativeSaved(
    mut env: JNIEnv,
    _class: JClass,
    id: jlong,
    name: JString,
) {
    let result = boundary(&mut env, |env| {
        if !accepts(id, Kind::Save) {
            return Ok(None);
        }
        let name: String = env.get_string(&name).map_err(jni_error)?.into();
        if name.is_empty() || name.len() > Limits::default().max_path_bytes {
            return Err(
                "Invalid Android saved document name. The document may already exist.".into(),
            );
        }
        Ok(Some(Output::Name(name)))
    });
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if accepts(id, Kind::Save) {
            prepare(id, result);
        }
    }));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_longbridge_nsh_PeaZipActivity_nativeExported(
    mut env: JNIEnv,
    _class: JClass,
    id: jlong,
    name: JString,
) {
    let result = boundary(&mut env, |env| {
        if !accepts(id, Kind::Export) {
            return Ok(None);
        }
        let name: String = env.get_string(&name).map_err(jni_error)?.into();
        let uuid = name
            .strip_prefix("PeaZip-")
            .ok_or("Invalid Android export root name.")?;
        if uuid.len() != 36
            || !uuid.bytes().enumerate().all(|(i, c)| {
                if [8, 13, 18, 23].contains(&i) {
                    c == b'-'
                } else {
                    c.is_ascii_hexdigit()
                }
            })
        {
            return Err("Invalid Android export root name.".into());
        }
        Ok(Some(Output::Name(name)))
    });
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if accepts(id, Kind::Export) {
            prepare(id, result);
        }
    }));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_longbridge_nsh_PeaZipActivity_nativeExportProgress(
    mut env: JNIEnv,
    _class: JClass,
    id: jlong,
    status: JString,
) {
    let _ = boundary(&mut env, |env| {
        if !accepts(id, Kind::Export) {
            return Ok(());
        }
        let status: String = env.get_string(&status).map_err(jni_error)?.into();
        if status.len() > 8192 {
            return Err("Invalid export progress length.".into());
        }
        let mut state = state();
        if let Some(pending) = state
            .pending
            .as_mut()
            .filter(|p| p.id == id && p.kind == Kind::Export)
        {
            pending.export_status = status;
        }
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_longbridge_nsh_PeaZipActivity_nativeCancelled(
    mut env: JNIEnv,
    _class: JClass,
    id: jlong,
) {
    let result = boundary(&mut env, |_| Ok(None));
    let _ = catch_unwind(AssertUnwindSafe(|| prepare(id, result)));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_longbridge_nsh_PeaZipActivity_nativeFailed(
    mut env: JNIEnv,
    _class: JClass,
    id: jlong,
    message: JString,
) {
    let result = boundary(&mut env, |env| {
        let message: String = env.get_string(&message).map_err(jni_error)?.into();
        Err::<Option<Output>, _>(message)
    });
    let _ = catch_unwind(AssertUnwindSafe(|| prepare(id, result)));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_longbridge_nsh_PeaZipActivity_nativeComplete(
    mut env: JNIEnv,
    _class: JClass,
    id: jlong,
) {
    let result = boundary(&mut env, |_| {
        let mut state = state();
        Ok(state
            .pending
            .as_mut()
            .filter(|pending| pending.id == id)
            .and_then(|pending| pending.reply.take()))
    });
    let _ = catch_unwind(AssertUnwindSafe(|| match result {
        Ok(Some(reply)) => complete(id, reply),
        Err(error) => complete(id, Err(error)),
        Ok(None) => {}
    }));
}
