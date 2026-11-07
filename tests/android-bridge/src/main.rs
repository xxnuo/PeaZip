#[path = "../../../crates/peazip-ui/src/android.rs"]
mod android;
use android::*;
use jni::objects::{JClass, JObject, JObjectArray, JValue};
use jni::{InitArgsBuilder, JNIEnv, JNIVersion, JavaVM, NativeMethod};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
};
struct Noop;
static OBSERVER_FAILED: AtomicBool = AtomicBool::new(false);

fn observe(env: &mut JNIEnv, method: &str, commit: Option<bool>) {
    let args = commit.map(|value| [JValue::Bool(value as u8)]);
    if env
        .call_static_method(
            "BridgeLifecycleCheck",
            method,
            if commit.is_some() { "(Z)V" } else { "()V" },
            args.as_ref().map_or(&[], |args| args.as_slice()),
        )
        .is_err()
    {
        OBSERVER_FAILED.store(true, Ordering::SeqCst);
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
}

extern "system" fn observed_picked(
    mut env: JNIEnv,
    class: JClass,
    id: i64,
    names: JObjectArray,
    data: JObjectArray,
) {
    observe(&mut env, "record", Some(false));
    Java_com_longbridge_nsh_PeaZipActivity_nativePicked(env, class, id, names, data);
}

extern "system" fn observed_exported(
    mut env: JNIEnv,
    class: JClass,
    id: i64,
    name: jni::objects::JString,
) {
    observe(&mut env, "record", Some(false));
    Java_com_longbridge_nsh_PeaZipActivity_nativeExported(env, class, id, name);
}

extern "system" fn observed_complete(mut env: JNIEnv, class: JClass, id: i64) {
    observe(&mut env, "record", Some(true));
    Java_com_longbridge_nsh_PeaZipActivity_nativeComplete(unsafe { env.unsafe_clone() }, class, id);
    observe(&mut env, "afterComplete", None);
}
impl Wake for Noop {
    fn wake(self: Arc<Self>) {}
}
fn poll<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(Noop));
    future.poll(&mut Context::from_waker(&waker))
}
fn class() -> JClass<'static> {
    JClass::from(JObject::null())
}
fn id(env: &mut JNIEnv, activity: &JObject) -> i64 {
    env.get_field(activity, "lastId", "J").unwrap().j().unwrap()
}
fn arrays<'a>(
    env: &mut JNIEnv<'a>,
    count: i32,
    name: &str,
    bytes: &[u8],
) -> (JObjectArray<'a>, JObjectArray<'a>) {
    let names = env
        .new_object_array(count, "java/lang/String", JObject::null())
        .unwrap();
    let data = env.new_object_array(count, "[B", JObject::null()).unwrap();
    for i in 0..count {
        env.with_local_frame(8, |env| -> jni::errors::Result<()> {
            let name = env.new_string(name)?;
            let bytes = env.byte_array_from_slice(bytes)?;
            env.set_object_array_element(&names, i, name)?;
            env.set_object_array_element(&data, i, bytes)?;
            Ok(())
        })
        .unwrap();
    }
    (names, data)
}
fn main() {
    let classpath = std::env::args()
        .nth(1)
        .expect("Pass the compiled Java test classes directory");
    let args = InitArgsBuilder::new()
        .version(JNIVersion::V8)
        .option(format!("-Djava.class.path={classpath}"))
        .option("-Xcheck:jni")
        .option("-XX:MaxJNILocalCapacity=512")
        .build()
        .unwrap();
    let vm = JavaVM::new(args).unwrap();
    let mut env = vm.attach_current_thread().unwrap();
    let activity = env.new_object("MockActivity", "()V", &[]).unwrap();
    let generation = Java_com_longbridge_nsh_PeaZipActivity_nativeBind(
        unsafe { env.unsafe_clone() },
        env.new_local_ref(&activity).unwrap(),
    );
    assert!(generation > 0);
    let mut checks = 0;
    {
        let mut pick = Box::pin(pick_files());
        assert!(poll(pick.as_mut()).is_pending());
        let mut reentry = Box::pin(pick_files());
        assert!(matches!(poll(reentry.as_mut()), Poll::Ready(Err(_))));
        checks += 1;
        let (names, data) = arrays(&mut env, 4096, "file.txt", &[]);
        Java_com_longbridge_nsh_PeaZipActivity_nativePicked(
            unsafe { env.unsafe_clone() },
            class(),
            id(&mut env, &activity),
            names,
            data,
        );
        assert!(
            poll(pick.as_mut()).is_pending(),
            "JNI staging must not complete the future"
        );
        Java_com_longbridge_nsh_PeaZipActivity_nativeComplete(
            unsafe { env.unsafe_clone() },
            class(),
            id(&mut env, &activity),
        );
        match poll(pick.as_mut()) {
            Poll::Ready(Ok(Some(files))) => assert!(
                files.len() == 4096
                    && files
                        .iter()
                        .all(|f| !f.is_dir && f.path == "file.txt" && f.data.is_empty())
            ),
            _ => panic!("4096 items failed"),
        }
        checks += 1;
    }
    for (count, name, bytes) in [
        (4097, "a", vec![]),
        (1, &*"a".repeat(1025), vec![]),
        (1, "a", vec![0; 16 * 1024 * 1024 + 1]),
    ] {
        let mut pick = Box::pin(pick_files());
        assert!(poll(pick.as_mut()).is_pending());
        let (names, data) = arrays(&mut env, count, name, &bytes);
        Java_com_longbridge_nsh_PeaZipActivity_nativePicked(
            unsafe { env.unsafe_clone() },
            class(),
            id(&mut env, &activity),
            names,
            data,
        );
        Java_com_longbridge_nsh_PeaZipActivity_nativeComplete(
            unsafe { env.unsafe_clone() },
            class(),
            id(&mut env, &activity),
        );
        assert!(matches!(poll(pick.as_mut()), Poll::Ready(Err(_))));
        checks += 1;
    }
    {
        let mut pick = Box::pin(pick_files());
        assert!(poll(pick.as_mut()).is_pending());
        let names = env
            .new_object_array(5, "java/lang/String", JObject::null())
            .unwrap();
        let data = env.new_object_array(5, "[B", JObject::null()).unwrap();
        for i in 0..5 {
            env.with_local_frame(8, |env| -> jni::errors::Result<()> {
                let name = env.new_string(format!("file{i}"))?;
                let bytes =
                    env.byte_array_from_slice(&vec![0; if i < 4 { 16 * 1024 * 1024 } else { 1 }])?;
                env.set_object_array_element(&names, i, name)?;
                env.set_object_array_element(&data, i, bytes)?;
                Ok(())
            })
            .unwrap();
        }
        Java_com_longbridge_nsh_PeaZipActivity_nativePicked(
            unsafe { env.unsafe_clone() },
            class(),
            id(&mut env, &activity),
            names,
            data,
        );
        Java_com_longbridge_nsh_PeaZipActivity_nativeComplete(
            unsafe { env.unsafe_clone() },
            class(),
            id(&mut env, &activity),
        );
        assert!(matches!(poll(pick.as_mut()), Poll::Ready(Err(_))));
        checks += 1;
    }
    {
        let mut save = Box::pin(save_zip(Arc::new(vec![0; 32 * 1024 * 1024 + 1])));
        assert!(matches!(poll(save.as_mut()), Poll::Ready(Err(_))));
        checks += 1;
    }
    {
        let mut save = Box::pin(save_zip(Arc::new(vec![0; 13])));
        assert!(matches!(poll(save.as_mut()), Poll::Ready(Err(_))));
        assert!(!env.exception_check().unwrap());
        checks += 1;
    }
    {
        let mut save = Box::pin(save_zip(Arc::new(vec![0; 1])));
        assert!(poll(save.as_mut()).is_pending());
        let name = env.new_string("Archive.zip").unwrap();
        Java_com_longbridge_nsh_PeaZipActivity_nativeSaved(
            unsafe { env.unsafe_clone() },
            class(),
            id(&mut env, &activity),
            name,
        );
        assert!(poll(save.as_mut()).is_pending());
        Java_com_longbridge_nsh_PeaZipActivity_nativeComplete(
            unsafe { env.unsafe_clone() },
            class(),
            id(&mut env, &activity),
        );
        assert!(matches!(poll(save.as_mut()), Poll::Ready(Ok(Some(name))) if name=="Archive.zip"));
        checks += 1;
    }
    {
        let mut pick = Box::pin(pick_files());
        assert!(poll(pick.as_mut()).is_pending());
        let old_id = id(&mut env, &activity);
        let next = env.new_object("MockActivity", "()V", &[]).unwrap();
        let next_generation = Java_com_longbridge_nsh_PeaZipActivity_nativeBind(
            unsafe { env.unsafe_clone() },
            env.new_local_ref(&next).unwrap(),
        );
        assert!(next_generation > old_id);
        assert!(matches!(poll(pick.as_mut()), Poll::Ready(Err(_))));
        checks += 1;
        let mut pick = Box::pin(pick_files());
        assert!(poll(pick.as_mut()).is_pending());
        Java_com_longbridge_nsh_PeaZipActivity_nativeCancelled(
            unsafe { env.unsafe_clone() },
            class(),
            old_id,
        );
        Java_com_longbridge_nsh_PeaZipActivity_nativeUnbind(
            unsafe { env.unsafe_clone() },
            class(),
            generation,
        );
        assert!(poll(pick.as_mut()).is_pending());
        checks += 1;
        Java_com_longbridge_nsh_PeaZipActivity_nativeCancelled(
            unsafe { env.unsafe_clone() },
            class(),
            id(&mut env, &next),
        );
        Java_com_longbridge_nsh_PeaZipActivity_nativeComplete(
            unsafe { env.unsafe_clone() },
            class(),
            id(&mut env, &next),
        );
        assert!(matches!(poll(pick.as_mut()), Poll::Ready(Ok(None))));
        checks += 1;
        let mut save = Box::pin(save_zip(Arc::new(vec![0; 1])));
        assert!(poll(save.as_mut()).is_pending());
        Java_com_longbridge_nsh_PeaZipActivity_nativeUnbind(
            unsafe { env.unsafe_clone() },
            class(),
            next_generation,
        );
        assert!(
            matches!(poll(save.as_mut()), Poll::Ready(Err(error)) if error.contains("partial"))
        );
        checks += 1;
        let mut unavailable = Box::pin(pick_files());
        assert!(matches!(poll(unavailable.as_mut()), Poll::Ready(Err(_))));
        checks += 1;
    }
    // Export uses the same admission and two-stage callbacks, with a distinct opcode.
    {
        let binding = Java_com_longbridge_nsh_PeaZipActivity_nativeBind(
            unsafe { env.unsafe_clone() },
            env.new_local_ref(&activity).unwrap(),
        );
        let entry = || peazip_core::MemoryEntry {
            path: "目录/a".into(),
            is_dir: false,
            data: vec![1, 2, 3],
        };
        for entries in [
            vec![],
            vec![peazip_core::MemoryEntry {
                path: "a".repeat(1025),
                ..entry()
            }],
            vec![peazip_core::MemoryEntry {
                data: vec![0; 16 * 1024 * 1024 + 1],
                ..entry()
            }],
            vec![peazip_core::MemoryEntry {
                is_dir: true,
                ..entry()
            }],
            (0..4097).map(|_| entry()).collect(),
            (0..5)
                .map(|_| peazip_core::MemoryEntry {
                    data: vec![0; 16 * 1024 * 1024],
                    ..entry()
                })
                .collect(),
        ] {
            let mut export = Box::pin(export_entries(entries));
            assert!(matches!(poll(export.as_mut()), Poll::Ready(Err(_))));
            checks += 1;
        }
        let mut export = Box::pin(export_entries(vec![entry()]));
        assert!(poll(export.as_mut()).is_pending());
        let export_id = id(&mut env, &activity);
        assert_eq!(
            env.get_field(&activity, "opcode", "I")
                .unwrap()
                .i()
                .unwrap(),
            2
        );
        let packet = jni::objects::JByteArray::from(
            env.get_field(&activity, "packet", "[B")
                .unwrap()
                .l()
                .unwrap(),
        );
        let mut expected = Vec::new();
        for n in [1u32, 0, "目录/a".len() as u32, 3] {
            expected.extend_from_slice(&n.to_le_bytes());
        }
        expected.extend_from_slice("目录/a".as_bytes());
        expected.extend_from_slice(&[1, 2, 3]);
        assert_eq!(env.convert_byte_array(packet).unwrap(), expected);
        let mut pick = Box::pin(pick_files());
        assert!(matches!(poll(pick.as_mut()), Poll::Ready(Err(_))));
        let mut save = Box::pin(save_zip(Arc::new(vec![1])));
        assert!(matches!(poll(save.as_mut()), Poll::Ready(Err(_))));
        let wrong_name = env.new_string("Archive.zip").unwrap();
        Java_com_longbridge_nsh_PeaZipActivity_nativeSaved(
            unsafe { env.unsafe_clone() },
            class(),
            export_id,
            wrong_name,
        );
        Java_com_longbridge_nsh_PeaZipActivity_nativeComplete(
            unsafe { env.unsafe_clone() },
            class(),
            export_id,
        );
        assert!(
            poll(export.as_mut()).is_pending(),
            "Save callback must not complete Export"
        );
        let root = "PeaZip-12345678-1234-4234-8234-123456789abc";
        let name = env.new_string(root).unwrap();
        Java_com_longbridge_nsh_PeaZipActivity_nativeExported(
            unsafe { env.unsafe_clone() },
            class(),
            export_id,
            name,
        );
        assert!(poll(export.as_mut()).is_pending());
        Java_com_longbridge_nsh_PeaZipActivity_nativeComplete(
            unsafe { env.unsafe_clone() },
            class(),
            export_id,
        );
        assert!(
            matches!(poll(export.as_mut()), Poll::Ready(Ok(Some(name))) if name == format!("Extracted 1 entries to {root}"))
        );
        checks += 1;
        let mut cancelled = Box::pin(export_entries(vec![entry()]));
        assert!(poll(cancelled.as_mut()).is_pending());
        let current_id = id(&mut env, &activity);
        let stale_name = env.new_string(root).unwrap();
        Java_com_longbridge_nsh_PeaZipActivity_nativeExported(
            unsafe { env.unsafe_clone() },
            class(),
            export_id,
            stale_name,
        );
        Java_com_longbridge_nsh_PeaZipActivity_nativeComplete(
            unsafe { env.unsafe_clone() },
            class(),
            export_id,
        );
        assert!(poll(cancelled.as_mut()).is_pending());
        Java_com_longbridge_nsh_PeaZipActivity_nativeCancelled(
            unsafe { env.unsafe_clone() },
            class(),
            current_id,
        );
        assert!(poll(cancelled.as_mut()).is_pending());
        Java_com_longbridge_nsh_PeaZipActivity_nativeComplete(
            unsafe { env.unsafe_clone() },
            class(),
            current_id,
        );
        assert!(matches!(poll(cancelled.as_mut()), Poll::Ready(Ok(None))));
        checks += 1;
        let mut invalid = Box::pin(export_entries(vec![entry()]));
        assert!(poll(invalid.as_mut()).is_pending());
        let name = env.new_string("renamed").unwrap();
        Java_com_longbridge_nsh_PeaZipActivity_nativeExported(
            unsafe { env.unsafe_clone() },
            class(),
            id(&mut env, &activity),
            name,
        );
        Java_com_longbridge_nsh_PeaZipActivity_nativeComplete(
            unsafe { env.unsafe_clone() },
            class(),
            id(&mut env, &activity),
        );
        assert!(matches!(poll(invalid.as_mut()), Poll::Ready(Err(_))));
        checks += 1;
        env.set_field(&activity, "failExport", "Z", JValue::Bool(1))
            .unwrap();
        let mut failed = Box::pin(export_entries(vec![entry()]));
        assert!(matches!(poll(failed.as_mut()), Poll::Ready(Err(_))));
        assert!(!env.exception_check().unwrap());
        env.set_field(&activity, "failExport", "Z", JValue::Bool(0))
            .unwrap();
        checks += 1;
        let mut destroyed = Box::pin(export_entries(vec![entry()]));
        assert!(poll(destroyed.as_mut()).is_pending());
        let destroyed_id = id(&mut env, &activity);
        let status = env.new_string(format!("Root: {root}; completed 1 entries; unfinished path: a. Documents may remain; nothing was deleted.")).unwrap();
        Java_com_longbridge_nsh_PeaZipActivity_nativeExportProgress(
            unsafe { env.unsafe_clone() },
            class(),
            destroyed_id,
            status,
        );
        let next = env.new_object("MockActivity", "()V", &[]).unwrap();
        let next_binding = Java_com_longbridge_nsh_PeaZipActivity_nativeBind(
            unsafe { env.unsafe_clone() },
            env.new_local_ref(&next).unwrap(),
        );
        assert!(
            matches!(poll(destroyed.as_mut()), Poll::Ready(Err(error)) if error.contains(root) && error.contains("completed 1") && error.contains("unfinished path: a") && error.contains("may remain"))
        );
        let mut current = Box::pin(export_entries(vec![entry()]));
        assert!(poll(current.as_mut()).is_pending());
        Java_com_longbridge_nsh_PeaZipActivity_nativeUnbind(
            unsafe { env.unsafe_clone() },
            class(),
            binding,
        );
        let stale = env.new_string(root).unwrap();
        Java_com_longbridge_nsh_PeaZipActivity_nativeExported(
            unsafe { env.unsafe_clone() },
            class(),
            destroyed_id,
            stale,
        );
        Java_com_longbridge_nsh_PeaZipActivity_nativeComplete(
            unsafe { env.unsafe_clone() },
            class(),
            destroyed_id,
        );
        assert!(poll(current.as_mut()).is_pending());
        Java_com_longbridge_nsh_PeaZipActivity_nativeUnbind(
            unsafe { env.unsafe_clone() },
            class(),
            next_binding,
        );
        assert!(
            matches!(poll(current.as_mut()), Poll::Ready(Err(error)) if error.contains("completed 0") && error.contains("not yet known"))
        );
        checks += 1;
    }
    assert!(!env.exception_check().unwrap());
    println!("PASS: {checks} JNI bridge checks, including 4096 items with 512 JNI local capacity");
    env.register_native_methods(
        "com/longbridge/nsh/PeaZipActivity",
        &[
            NativeMethod {
                name: "nativePicked".into(),
                sig: "(J[Ljava/lang/String;[[B)V".into(),
                fn_ptr: observed_picked as *mut c_void,
            },
            NativeMethod {
                name: "nativeComplete".into(),
                sig: "(J)V".into(),
                fn_ptr: observed_complete as *mut c_void,
            },
            NativeMethod {
                name: "nativeExported".into(),
                sig: "(JLjava/lang/String;)V".into(),
                fn_ptr: observed_exported as *mut c_void,
            },
            NativeMethod {
                name: "nativeExportProgress".into(),
                sig: "(JLjava/lang/String;)V".into(),
                fn_ptr: Java_com_longbridge_nsh_PeaZipActivity_nativeExportProgress as *mut c_void,
            },
            NativeMethod {
                name: "nativeUnbind".into(),
                sig: "(J)V".into(),
                fn_ptr: Java_com_longbridge_nsh_PeaZipActivity_nativeUnbind as *mut c_void,
            },
            NativeMethod {
                name: "nativeFailed".into(),
                sig: "(JLjava/lang/String;)V".into(),
                fn_ptr: Java_com_longbridge_nsh_PeaZipActivity_nativeFailed as *mut c_void,
            },
            NativeMethod {
                name: "nativeCancelled".into(),
                sig: "(J)V".into(),
                fn_ptr: Java_com_longbridge_nsh_PeaZipActivity_nativeCancelled as *mut c_void,
            },
        ],
    )
    .unwrap();
    // Allocate the host Activity without running android.jar's stub constructor.
    let owner = env
        .alloc_object("com/longbridge/nsh/PeaZipActivity")
        .unwrap();
    let replacement = env
        .alloc_object("com/longbridge/nsh/PeaZipActivity")
        .unwrap();
    let mock = env.new_object("MockActivity", "()V", &[]).unwrap();
    let generation = Java_com_longbridge_nsh_PeaZipActivity_nativeBind(
        unsafe { env.unsafe_clone() },
        env.new_local_ref(&mock).unwrap(),
    );
    let mut pick = Box::pin(pick_files());
    assert!(poll(pick.as_mut()).is_pending());
    let request_id = id(&mut env, &mock);
    env.call_static_method(
        "BridgeLifecycleCheck",
        "startDelivery",
        "(Lcom/longbridge/nsh/PeaZipActivity;J)V",
        &[JValue::Object(&owner), JValue::Long(request_id)],
    )
    .unwrap();
    env.call_static_method("BridgeLifecycleCheck", "awaitDelivery", "()V", &[])
        .unwrap();
    assert!(
        matches!(poll(pick.as_mut()), Poll::Ready(Ok(Some(files))) if files.len() == 4 && files.iter().all(|file| file.data.len() == 16*1024*1024))
    );
    assert!(!OBSERVER_FAILED.load(Ordering::SeqCst));
    let mut export = Box::pin(export_entries(vec![peazip_core::MemoryEntry {
        path: "empty/".into(),
        is_dir: true,
        data: vec![],
    }]));
    assert!(poll(export.as_mut()).is_pending());
    let export_id = id(&mut env, &mock);
    env.call_static_method(
        "BridgeLifecycleCheck",
        "startExportDelivery",
        "(Lcom/longbridge/nsh/PeaZipActivity;J)V",
        &[JValue::Object(&owner), JValue::Long(export_id)],
    )
    .unwrap();
    env.call_static_method("BridgeLifecycleCheck", "awaitExportDelivery", "()V", &[])
        .unwrap();
    assert!(
        matches!(poll(export.as_mut()), Poll::Ready(Ok(Some(name))) if name == "Extracted 1 entries to PeaZip-12345678-1234-4234-8234-123456789abc")
    );
    assert!(!OBSERVER_FAILED.load(Ordering::SeqCst));
    Java_com_longbridge_nsh_PeaZipActivity_nativeUnbind(
        unsafe { env.unsafe_clone() },
        class(),
        generation,
    );
    env.call_static_method(
        "BridgeLifecycleCheck",
        "checkCancellation",
        "(Lcom/longbridge/nsh/PeaZipActivity;Lcom/longbridge/nsh/PeaZipActivity;)V",
        &[JValue::Object(&owner), JValue::Object(&replacement)],
    )
    .unwrap();
}
