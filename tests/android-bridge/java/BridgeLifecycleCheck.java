import android.content.Intent;
import android.os.CancellationSignal;
import android.os.Handler;
import android.os.Looper;
import com.longbridge.nsh.PeaZipActivity;
import java.io.*;
import java.lang.ref.WeakReference;
import java.lang.reflect.*;
import java.util.concurrent.*;

/** Exercises production scheduling/cancellation; only Android primitives are host doubles. */
public class BridgeLifecycleCheck {
    static final Class<?> ACTIVITY = PeaZipActivity.class;
    static final Class<?> TASK;
    static PeaZipActivity owner;
    static PeaZipActivity replacement;
    static Object task;
    static volatile String failure;
    static volatile boolean picked, completed;
    static CountDownLatch delivery;
    static {
        try { TASK = Class.forName("com.longbridge.nsh.PeaZipActivity$Task"); }
        catch (Exception e) { throw new AssertionError(e); }
    }
    static Field field(Class<?> type, String name) throws Exception {
        Field field = type.getDeclaredField(name); field.setAccessible(true); return field;
    }
    static Object get(Object object, String name) throws Exception { return field(object.getClass(), name).get(object); }
    static void set(Object object, String name, Object value) throws Exception { field(object.getClass(), name).set(object, value); }
    static Object call(Class<?> type, Object object, String name, Class<?>[] types, Object... args) throws Exception {
        Method method = type.getDeclaredMethod(name, types); method.setAccessible(true);
        try { return method.invoke(object, args); }
        catch (InvocationTargetException e) {
            if (e.getCause() instanceof Exception cause) throw cause;
            throw new AssertionError(e.getCause());
        }
    }
    static void await(CountDownLatch latch) throws Exception {
        if (!latch.await(10, TimeUnit.SECONDS)) throw new AssertionError("Host bridge check timed out");
    }
    static ThreadPoolExecutor executor(String name) throws Exception {
        return (ThreadPoolExecutor) field(ACTIVITY, name).get(null);
    }
    static Object prepare(PeaZipActivity activity, long id, boolean save) throws Exception {
        return prepare(activity, id, save, false);
    }
    static Object prepare(PeaZipActivity activity, long id, boolean save, boolean exporting) throws Exception {
        owner = activity;
        Constructor<?> constructor = TASK.getDeclaredConstructor(long.class, int.class, byte[].class, boolean.class);
        constructor.setAccessible(true);
        Object next = constructor.newInstance(id, 1, save || exporting ? new byte[1] : null, exporting);
        set(next, "owner", new WeakReference<>(activity));
        set(next, "started", true);
        set(activity, "destroyed", false);
        set(activity, "pending", next);
        Object lock = field(ACTIVITY, "TRANSFERS").get(null);
        synchronized (lock) {
            if (field(ACTIVITY, "active").get(null) != null) throw new AssertionError("Transfer admission leaked");
            field(ACTIVITY, "active").set(null, next);
        }
        task = next;
        return next;
    }
    public static void checkSaveFormats(PeaZipActivity picker) throws Exception {
        for (int invalid : new int[] {-1, 4, Integer.MIN_VALUE, Integer.MAX_VALUE}) {
            try {
                picker.saveArchive(1, new byte[1], invalid);
                throw new AssertionError("Invalid format accepted: " + invalid);
            } catch (IllegalArgumentException expected) { }
            if (field(ACTIVITY, "pending").get(picker) != null || !picker.archiveTransferAvailable())
                throw new AssertionError("Invalid format launched picker or retained admission");
        }
        Constructor<?> constructor = TASK.getDeclaredConstructor(long.class, int.class, byte[].class, boolean.class, int.class);
        constructor.setAccessible(true);
        for (int format : new int[] {0, 1, 2, 3}) {
            byte[] bytes = new byte[] {'M', 'Z'};
            Object current = constructor.newInstance(10L + format, 1, bytes, false, format);
            Intent intent = (Intent) call(ACTIVITY, null, "pickerIntent", new Class<?>[] {TASK}, current);
            if (!Intent.ACTION_CREATE_DOCUMENT.equals(intent.getAction())
                    || !(format == 3 ? "application/x-7z-compressed" : format == 2 ? "application/gzip" : format == 0 ? "application/zip" : "application/x-tar").equals(intent.getType())
                    || !(format == 3 ? "Archive.7z" : format == 2 ? "Archive.tar.gz" : format == 0 ? "Archive.zip" : "Archive.tar").equals(intent.getStringExtra(Intent.EXTRA_TITLE)))
                throw new AssertionError("Wrong archive action/MIME/title for format " + format);
            if (get(current, "bytes") != bytes) throw new AssertionError("Archive bytes changed");
        }
        System.out.println("PASS: ZIP/TAR/TAR.GZ/7Z ACTION_CREATE_DOCUMENT MIME/title from explicit format, SFX bytes preserved; invalid codes rejected before admission");
    }
    public static boolean available() { return owner == null || owner.archiveTransferAvailable(); }
    public static void startDelivery(PeaZipActivity activity, long id) throws Exception {
        Object current = prepare(activity, id, false);
        delivery = new CountDownLatch(1);
        picked = completed = false;
        failure = null;
        byte[] bytes = new byte[16 * 1024 * 1024];
        executor("IO").execute(() -> {
            try {
                call(ACTIVITY, null, "deliver", new Class<?>[] {TASK, String[].class, byte[][].class, String.class, String.class},
                        current, new String[] {"a", "b", "c", "d"}, new byte[][] {bytes, bytes, bytes, bytes}, null, null);
            } catch (Throwable e) { failure = e.toString(); delivery.countDown(); }
        });
    }
    public static void startExportDelivery(PeaZipActivity activity, long id) throws Exception {
        Object current = prepare(activity, id, false, true);
        delivery = new CountDownLatch(1);
        picked = completed = false;
        failure = null;
        executor("IO").execute(() -> {
            try {
                call(ACTIVITY, null, "deliver", new Class<?>[] {TASK, String[].class, byte[][].class, String.class, String.class},
                        current, null, null, "PeaZip-12345678-1234-4234-8234-123456789abc", null);
            } catch (Throwable e) { failure = e.toString(); delivery.countDown(); }
        });
    }
    public static void awaitExportDelivery() throws Exception {
        await(delivery);
        if (failure != null || !picked || !completed) throw new AssertionError("Export background handoff failed: " + failure);
        System.out.println("PASS: export JNI staging/completion off UI; shared admission released before Rust future completion");
    }
    // Called by the real JVM's registered JNI entry points, before production conversion/commit.
    public static void record(boolean commit) {
        try {
            if (Looper.myLooper() == Looper.getMainLooper() || !Thread.currentThread().getName().equals("PeaZip-documents"))
                throw new AssertionError("JNI conversion/completion ran on UI or an unexpected thread");
            if (get(owner, "pending") != null || !((Boolean) get(task, "ready")))
                throw new AssertionError("JNI ran before UI pending cleanup");
            if (commit != available()) throw new AssertionError("Admission must be released only after JNI staging");
            if (commit) completed = true; else picked = true;
        } catch (Throwable e) { failure = e.toString(); }
    }
    public static void afterComplete() { delivery.countDown(); }
    public static void awaitDelivery() throws Exception {
        await(delivery);
        if (failure != null || !picked || !completed) throw new AssertionError("Background handoff failed: " + failure);
        System.out.println("PASS: 64 MiB JNI conversion and completion off UI; UI cleanup and admission release precede future completion");
    }
    static void workerFinished(Object current) throws Exception {
        call(ACTIVITY, null, "workerFinished", new Class<?>[] {TASK}, current);
    }
    static void idle() throws Exception {
        CountDownLatch idle = new CountDownLatch(3);
        for (String name : new String[] {"IO", "CANCEL", "CLOSE"}) executor(name).execute(idle::countDown);
        await(idle);
        if (!available()) throw new AssertionError("Cleanup retained admission after all workers stopped");
    }
    static void destroy(PeaZipActivity activity) throws Exception {
        CountDownLatch done = new CountDownLatch(1);
        new Handler(Looper.getMainLooper()).post(() -> {
            try { call(ACTIVITY, activity, "onDestroy", new Class<?>[0]); }
            catch (Exception e) {
                // android.jar's superclass has no executable onDestroy implementation on a host JVM.
                if (!"Stub!".equals(e.getMessage())) failure = e.toString();
            } finally { done.countDown(); }
        });
        await(done);
        if (failure != null) throw new AssertionError(failure);
    }
    static final class Blocked implements Closeable {
        final CountDownLatch entered = new CountDownLatch(1), io = new CountDownLatch(1);
        final CountDownLatch closing = new CountDownLatch(1), cancelling = new CountDownLatch(1);
        final CountDownLatch allowClose, allowCancel;
        volatile String closeThread, cancelThread;
        Blocked(boolean hold) { allowClose = new CountDownLatch(hold ? 1 : 0); allowCancel = new CountDownLatch(hold ? 1 : 0); }
        void block() {
            entered.countDown();
            try { await(io); } catch (Exception e) { throw new AssertionError(e); }
        }
        void cancel() {
            cancelThread = Thread.currentThread().getName(); cancelling.countDown();
            try { await(allowCancel); } catch (Exception e) { throw new AssertionError(e); }
        }
        public void close() {
            synchronized (this) { if (closeThread == null) closeThread = Thread.currentThread().getName(); }
            closing.countDown();
            try { await(allowClose); } catch (Exception e) { throw new AssertionError(e); }
            io.countDown();
        }
    }
    static void blockedIO(PeaZipActivity activity, boolean write, boolean hold) throws Exception {
        blockedIO(activity, write, hold, false);
    }
    static void blockedIO(PeaZipActivity activity, boolean write, boolean hold, boolean exporting) throws Exception {
        Object current = prepare(activity, 9000, write, exporting);
        Blocked blocked = new Blocked(hold);
        CancellationSignal signal = (CancellationSignal) get(current, "signal");
        signal.setOnCancelListener(blocked::cancel);
        InputStream input = new InputStream() {
            public int read() { blocked.block(); return -1; }
            public void close() { blocked.close(); }
        };
        OutputStream output = new OutputStream() {
            public void write(int value) { blocked.block(); }
            public void close() { blocked.close(); }
        };
        Closeable stream = write ? output : input;
        call(TASK, current, "track", new Class<?>[] {Closeable.class}, stream);
        CountDownLatch done = new CountDownLatch(1);
        executor("IO").execute(() -> {
            try {
                if (write) { output.write(1); call(TASK, current, "check", new Class<?>[0]); }
                else call(ACTIVITY, null, "readBounded", new Class<?>[] {InputStream.class, int.class, TASK}, input, 1, current);
                failure = "Destroyed I/O returned success";
            } catch (Exception e) { if (!(e instanceof IOException)) failure = e.toString(); }
            finally {
                try { stream.close(); workerFinished(current); } catch (Exception e) { failure = e.toString(); }
                done.countDown();
            }
        });
        await(blocked.entered);
        destroy(activity); // Must return even when both cancellation and close are blocked.
        await(blocked.cancelling); await(blocked.closing);
        if (!signal.isCanceled() || !"PeaZip-cancel".equals(blocked.cancelThread) || !"PeaZip-close".equals(blocked.closeThread))
            throw new AssertionError("Underlying cancel/close was not dispatched off UI");
        if (hold) {
            for (int i = 0; i < 200; i++) {
                if (replacement.pickArchiveInputs(i) || replacement.saveArchive(i, new byte[1])
                        || replacement.exportEntries(i, new byte[1])) throw new AssertionError("Old cleanup admitted another Activity transfer");
            }
            blocked.allowClose.countDown();
            await(done);
            if (available()) throw new AssertionError("Blocked cancellation no longer held admission");
            blocked.allowCancel.countDown();
        }
        await(done); idle();
        if (failure != null) throw new AssertionError(failure);
    }
    public static void checkCancellation(PeaZipActivity activity, PeaZipActivity next) throws Exception {
        replacement = next;
        blockedIO(activity, false, false);
        blockedIO(activity, true, false);
        blockedIO(activity, false, true);
        blockedIO(activity, true, true, true);
        for (String name : new String[] {"IO", "CANCEL", "CLOSE"}) {
            ThreadPoolExecutor executor = executor(name);
            if (executor.getLargestPoolSize() > 1 || executor.getQueue().size() > 1)
                throw new AssertionError("Unbounded transfer/cleanup executor");
        }
        System.out.println("PASS: blocked read/save/export cancel and close off UI; blocked cleanup rejects 200 cross-Activity Pick/Save/Export admissions; workers bounded to 1+1+1");
    }
}
