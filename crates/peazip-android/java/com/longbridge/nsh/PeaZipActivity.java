package com.longbridge.nsh;

import android.app.Activity;
import android.content.ClipData;
import android.content.ContentResolver;
import android.content.Intent;
import android.content.res.AssetFileDescriptor;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.os.CancellationSignal;
import android.os.Handler;
import android.os.Looper;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.Closeable;
import java.io.InputStream;
import java.io.OutputStream;
import java.lang.ref.WeakReference;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;

/** App-owned SAF bridge; nsh's Java sources remain unmodified. */
public final class PeaZipActivity extends Activity {
    private static final int MAX_FILES = 4096;
    private static final int MAX_PATH = 1024;
    private static final int MAX_ENTRY = 16 * 1024 * 1024;
    private static final int MAX_TOTAL = 64 * 1024 * 1024;
    private static final int MAX_ARCHIVE = 32 * 1024 * 1024;
    private static final Object TRANSFERS = new Object();
    private static final ThreadPoolExecutor IO = executor("PeaZip-documents");
    private static final ThreadPoolExecutor CANCEL = executor("PeaZip-cancel");
    private static final ThreadPoolExecutor CLOSE = executor("PeaZip-close");
    private static Task active;
    private NshView view;
    private long generation;
    private volatile boolean destroyed;
    private int requestCode;
    private Task pending;

    private native long nativeBind();
    private static native void nativeUnbind(long generation);
    private static native void nativePicked(long id, String[] names, byte[][] bytes);
    private static native void nativeSaved(long id, String displayName);
    private static native void nativeExported(long id, String rootName);
    private static native void nativeExportProgress(long id, String status);
    private static native void nativeCancelled(long id);
    private static native void nativeFailed(long id, String message);
    private static native void nativeComplete(long id);

    private static ThreadPoolExecutor executor(String name) {
        return new ThreadPoolExecutor(1, 1, 0, TimeUnit.MILLISECONDS, new ArrayBlockingQueue<>(1), work -> {
            Thread thread = new Thread(work, name);
            thread.setDaemon(true);
            return thread;
        });
    }

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        DevActivity.edgeToEdge(this);
        view = new NshView(this);
        view.useStandaloneLimits();
        setContentView(view);
        view.handleNotificationIntent(getIntent());
        if (state == null) view.handleUrlSchemeIntent(getIntent());
        // NshView has loaded libnsh; bind before Rust builds the first UI.
        generation = nativeBind();
        if (rustApp()) view.runRustApp("nsh");
        else view.loadAssetBundle("nsh", true);
    }

    private boolean rustApp() {
        try {
            android.content.pm.ApplicationInfo info = getPackageManager().getApplicationInfo(
                    getPackageName(), android.content.pm.PackageManager.GET_META_DATA);
            return info.metaData != null && info.metaData.getBoolean("com.longbridge.nsh.RUST", false);
        } catch (android.content.pm.PackageManager.NameNotFoundException e) {
            return false;
        }
    }

    @Override
    public void onConfigurationChanged(android.content.res.Configuration config) {
        super.onConfigurationChanged(config);
        DevActivity.systemBars(this);
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        if (view != null) {
            view.handleNotificationIntent(intent);
            view.handleUrlSchemeIntent(intent);
        }
    }

    @Override
    @SuppressWarnings("deprecation")
    public void onBackPressed() {
        if (view == null || !view.handleBack()) super.onBackPressed();
    }

    @Override
    protected void onDestroy() {
        destroyed = true;
        synchronized (TRANSFERS) {
            if (active != null && active.owner.get() == this) cancelTask(active);
        }
        pending = null;
        try {
            nativeUnbind(generation);
        } finally {
            try {
                if (view != null) view.destroy();
            } finally {
                super.onDestroy();
            }
        }
    }

    /** Called by Rust from either the nsh thread or the UI thread. */
    public boolean archiveTransferAvailable() {
        synchronized (TRANSFERS) { return active == null; }
    }

    public boolean pickArchiveInputs(long id) {
        return begin(id, null);
    }

    public boolean saveArchive(long id, byte[] bytes) {
        return saveArchive(id, bytes, 0);
    }

    public boolean saveArchive(long id, byte[] bytes, int format) {
        if (format != 0 && format != 1) throw new IllegalArgumentException("Unsupported archive format code: " + format);
        if (bytes == null || bytes.length > MAX_ARCHIVE) throw new IllegalArgumentException("Invalid archive byte count.");
        return begin(id, bytes, false, format);
    }

    public boolean exportEntries(long id, byte[] packet) {
        if (packet == null || packet.length > TreeExport.MAX_PACKET) throw new IllegalArgumentException("Invalid export packet size.");
        return begin(id, packet, true);
    }

    private boolean begin(long id, byte[] bytes) {
        return begin(id, bytes, false);
    }

    private boolean begin(long id, byte[] bytes, boolean export) {
        return begin(id, bytes, export, 0);
    }

    private boolean begin(long id, byte[] bytes, boolean export, int format) {
        Task task;
        synchronized (TRANSFERS) {
            if (destroyed || active != null) return false;
            task = new Task(id, 0, bytes, export, format);
            task.owner = new WeakReference<>(this);
            active = task;
        }
        try {
            runOnUiThread(() -> start(task));
        } catch (RuntimeException | Error e) {
            cancelTask(task);
            workerFinished(task);
            throw e;
        }
        return true;
    }

    @SuppressWarnings("deprecation")
    private void start(Task task) {
        if (task.destroyed) return;
        if (destroyed || generation == 0) {
            finish(task, null, null, null, "Android activity is unavailable.");
            return;
        }
        if (pending != null) {
            finish(task, null, null, null, "An Android file operation is already running.");
            return;
        }
        // Never reuse a request code in this Activity: delayed picker results stay stale.
        if (requestCode == 65535) {
            finish(task, null, null, null, "Android picker request codes exhausted; reopen the app.");
            return;
        }
        task.requestCode = ++requestCode;
        pending = task;
        try {
            Intent intent = pickerIntent(task);
            startActivityForResult(intent, task.requestCode);
        } catch (Exception e) {
            finish(task, null, null, null, message(e));
        }
    }

    private static Intent pickerIntent(Task task) {
        Intent intent = new Intent(task.export != null ? Intent.ACTION_OPEN_DOCUMENT_TREE
                : task.save ? Intent.ACTION_CREATE_DOCUMENT : Intent.ACTION_OPEN_DOCUMENT);
        if (task.export == null) {
            intent.addCategory(Intent.CATEGORY_OPENABLE);
            intent.setType(task.save ? (task.format == 1 ? "application/x-tar" : "application/zip") : "*/*");
            if (task.save) intent.putExtra(Intent.EXTRA_TITLE, task.format == 1 ? "Archive.tar" : "Archive.zip");
            else intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true);
        }
        return intent;
    }

    @Override
    @SuppressWarnings("deprecation")
    protected void onActivityResult(int code, int result, Intent data) {
        super.onActivityResult(code, result, data);
        Task task = pending;
        if (destroyed || task == null || code != task.requestCode || task.started) return;
        if (result == RESULT_CANCELED) {
            finish(task, null, null, null, null);
            return;
        }
        try {
            if (result != RESULT_OK || data == null) throw new IOException("Document picker returned no document.");
            ClipData clip = data.getClipData();
            int count = clip == null ? (data.getData() == null ? 0 : 1) : clip.getItemCount();
            if (count <= 0 || count > MAX_FILES || ((task.save || task.export != null) && count != 1)) {
                throw new IOException("Invalid document count (maximum 4096 input files).");
            }
            Uri[] uris = new Uri[count];
            for (int i = 0; i < count; i++) {
                Uri uri = clip == null ? data.getData() : clip.getItemAt(i).getUri();
                if (uri == null || !"content".equals(uri.getScheme())) {
                    throw new IOException("Only content:// document URIs are accepted.");
                }
                if (task.export != null) SafTreeExport.requireTree(uri);
                uris[i] = uri;
            }
            task.started = true;
            ContentResolver resolver = getApplicationContext().getContentResolver();
            IO.execute(() -> transfer(resolver, task, uris));
        } catch (Exception e) {
            finish(task, null, null, null, message(e));
        }
    }

    static final class Task {
        final long id;
        int requestCode;
        final boolean save;
        final int format;
        final TreeExport export;
        volatile boolean destroyed;
        volatile byte[] bytes;
        boolean started;
        WeakReference<PeaZipActivity> owner = new WeakReference<>(null);
        final CancellationSignal signal = new CancellationSignal();
        final ArrayList<Closeable> resources = new ArrayList<>();
        final CountDownLatch cleared = new CountDownLatch(1);
        volatile boolean ready;
        boolean workerDone;
        boolean cancelDone = true;
        boolean closeDone = true;

        Task(long id, int code, byte[] bytes) {
            this(id, code, bytes, false);
        }

        Task(long id, int code, byte[] bytes, boolean exporting) {
            this(id, code, bytes, exporting, 0);
        }

        Task(long id, int code, byte[] bytes, boolean exporting, int format) {
            this.format = format;
            this.id = id;
            this.requestCode = code;
            this.save = bytes != null && !exporting;
            this.export = exporting ? new TreeExport() : null;
            this.bytes = bytes;
        }

        void check() throws IOException {
            if (destroyed) throw new IOException("Activity was destroyed; operation cancelled.");
        }

        void track(Closeable resource) throws IOException {
            synchronized (resources) { resources.add(resource); }
            check();
        }
    }

    private static void release(Task task) {
        if (active == task && task.workerDone && task.cancelDone && task.closeDone) active = null;
    }

    private static void workerFinished(Task task) {
        synchronized (TRANSFERS) {
            task.workerDone = true;
            release(task);
        }
    }

    private static void cancelTask(Task task) {
        synchronized (TRANSFERS) {
            if (task.destroyed) return;
            task.destroyed = true;
            task.bytes = null;
            task.cleared.countDown();
            if (active != task) return;
            task.cancelDone = false;
            task.closeDone = false;
            if (!task.started) task.workerDone = true;
            // cancel() may block in a provider; close must get its own bounded worker.
            CANCEL.execute(() -> {
                try { task.signal.cancel(); }
                finally {
                    synchronized (TRANSFERS) { task.cancelDone = true; release(task); }
                }
            });
            CLOSE.execute(() -> {
                try {
                    Closeable[] resources;
                    synchronized (task.resources) { resources = task.resources.toArray(new Closeable[0]); }
                    for (Closeable resource : resources) {
                        try { resource.close(); } catch (Exception ignored) { }
                    }
                } finally {
                    synchronized (TRANSFERS) { task.closeDone = true; release(task); }
                }
            });
        }
    }

    private static final class Metadata {
        final String name;
        final long size;

        Metadata(String name, long size) {
            this.name = name;
            this.size = size;
        }
    }

    private static Metadata metadata(ContentResolver resolver, Uri uri, Task task) throws IOException {
        task.check();
        try (Cursor cursor = resolver.query(uri,
                new String[] {OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE}, null, null, null, task.signal)) {
            if (cursor != null) task.track(cursor);
            task.check();
            if (cursor == null || !cursor.moveToFirst()) throw new IOException("Cannot query document metadata.");
            int nameColumn = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
            int sizeColumn = cursor.getColumnIndex(OpenableColumns.SIZE);
            String name = nameColumn < 0 || cursor.isNull(nameColumn) ? null : cursor.getString(nameColumn);
            if (name == null || name.isEmpty() || name.length() > MAX_PATH
                    || name.getBytes(StandardCharsets.UTF_8).length > MAX_PATH) {
                throw new IOException("Document name exceeds the 1024-byte UTF-8 path limit or is missing.");
            }
            long size = sizeColumn < 0 || cursor.isNull(sizeColumn) ? -1 : cursor.getLong(sizeColumn);
            return new Metadata(name, size);
        } finally {
            synchronized (task.resources) { task.resources.clear(); }
        }
    }

    private static byte[] readBounded(InputStream input, int limit, Task task) throws IOException {
        ByteArrayOutputStream output = new ByteArrayOutputStream(Math.min(limit, 8192));
        byte[] buffer = new byte[8192];
        int remaining = limit;
        while (remaining > 0) {
            task.check();
            int count = input.read(buffer, 0, Math.min(buffer.length, remaining));
            task.check();
            if (count < 0) return output.toByteArray();
            if (count == 0) {
                int one = input.read();
                task.check();
                if (one < 0) return output.toByteArray();
                output.write(one);
                remaining--;
            } else {
                if (count > Math.min(buffer.length, remaining)) throw new IOException("Invalid provider read length.");
                output.write(buffer, 0, count);
                remaining -= count;
            }
        }
        task.check();
        int probe = input.read();
        task.check();
        if (probe != -1) throw new IOException("Input exceeds the 16 MiB entry or 64 MiB total byte limit.");
        return output.toByteArray();
    }

    private static byte[] readAsset(AssetFileDescriptor descriptor, int budget, Task task) throws IOException {
        long length = descriptor.getDeclaredLength();
        if (length < AssetFileDescriptor.UNKNOWN_LENGTH || length > budget) {
            throw new IOException("Selected asset exceeds the entry or total byte limit.");
        }
        ParcelFileDescriptor parcel = descriptor.getParcelFileDescriptor();
        boolean reliable = parcel.canDetectErrors();
        byte[] result;
        // The asset's offset/length bounds are authoritative; SIZE metadata is only a hint.
        try (InputStream input = descriptor.createInputStream()) {
            task.track(input);
            result = readBounded(input, budget, task);
        }
        // Logical bounded EOF bypasses PFD's check; physical EOF already checked and consumed its status.
        if (reliable && length >= 0 && result.length == length) parcel.checkError();
        task.check();
        return result;
    }

    private static void writeAsset(AssetFileDescriptor descriptor, byte[] bytes, Task task) throws IOException {
        writeAsset(descriptor, bytes, 0, bytes.length, task);
    }

    static void writeAsset(AssetFileDescriptor descriptor, byte[] bytes, int start, int count, Task task) throws IOException {
        if (descriptor.getDeclaredLength() != AssetFileDescriptor.UNKNOWN_LENGTH || descriptor.getStartOffset() != 0) {
            throw new IOException("Writing requires a whole document with unknown asset length and zero offset.");
        }
        ParcelFileDescriptor parcel = descriptor.getParcelFileDescriptor();
        // Capture before close: closing clears the communication FD, but retains its received status.
        boolean reliable = parcel.canDetectErrors();
        try (OutputStream output = descriptor.createOutputStream()) {
            task.track(output);
            for (int offset = start; offset < start + count; ) {
                task.check();
                int length = Math.min(8192, start + count - offset);
                output.write(bytes, offset, length);
                offset += length;
            }
            task.check();
            output.flush();
            task.check();
        }
        // Only errors already observable at local close can be checked; this is not remote commit acknowledgement.
        if (reliable) parcel.checkError();
        task.check();
    }

    private static void transfer(ContentResolver resolver, Task task, Uri[] uris) {
        String[] names = null;
        byte[][] contents = null;
        String saved = null;
        String failure = null;
        try {
            task.check();
            if (task.export != null) {
                byte[] packet = task.bytes;
                task.bytes = null;
                saved = task.export.run(packet, new SafTreeExport(resolver, uris[0], task,
                        status -> nativeExportProgress(task.id, status)));
            } else if (task.save) {
                String name = metadata(resolver, uris[0], task).name;
                byte[] bytes = task.bytes;
                task.bytes = null;
                task.check();
                if (bytes == null || bytes.length > MAX_ARCHIVE) throw new IOException("Invalid archive byte count.");
                try (AssetFileDescriptor descriptor = resolver.openAssetFileDescriptor(uris[0], "wt", task.signal)) {
                    if (descriptor == null) throw new IOException("Cannot open destination document.");
                    task.track(descriptor);
                    writeAsset(descriptor, bytes, task);
                } finally {
                    synchronized (task.resources) { task.resources.clear(); }
                }
                task.check();
                saved = name;
            } else {
                names = new String[uris.length];
                contents = new byte[uris.length][];
                long declared = 0;
                for (int i = 0; i < uris.length; i++) {
                    Metadata info = metadata(resolver, uris[i], task);
                    names[i] = info.name;
                    if (info.size > MAX_ENTRY) throw new IOException("Input metadata exceeds the 16 MiB entry limit.");
                    if (info.size >= 0) declared += info.size;
                    if (declared > MAX_TOTAL) throw new IOException("Input metadata exceeds the 64 MiB total limit.");
                }
                int total = 0;
                for (int i = 0; i < uris.length; i++) {
                    task.check();
                    try (AssetFileDescriptor descriptor = resolver.openAssetFileDescriptor(uris[i], "r", task.signal)) {
                        if (descriptor == null) throw new IOException("Cannot open input document.");
                        task.track(descriptor);
                        contents[i] = readAsset(descriptor, Math.min(MAX_ENTRY, MAX_TOTAL - total), task);
                    } finally {
                        synchronized (task.resources) { task.resources.clear(); }
                    }
                    total += contents[i].length;
                }
                task.check();
            }
        } catch (Exception | OutOfMemoryError e) {
            failure = message(e);
            names = null;
            contents = null;
            saved = null;
        } finally {
            task.bytes = null;
        }
        deliver(task, names, contents, saved, failure);
    }

    private void finish(Task task, String[] names, byte[][] contents, String saved, String failure) {
        task.started = true;
        IO.execute(() -> deliver(task, names, contents, saved, failure));
    }

    private static boolean clearActivity(Task task) throws InterruptedException {
        if (task.destroyed) return false;
        new Handler(Looper.getMainLooper()).post(() -> {
            try {
                PeaZipActivity activity = task.owner.get();
                if (activity != null && !activity.destroyed && !task.destroyed
                        && (activity.pending == task || activity.pending == null)) {
                    activity.pending = null;
                    task.bytes = null;
                    task.ready = true;
                }
            } finally { task.cleared.countDown(); }
        });
        task.cleared.await();
        return task.ready && !task.destroyed;
    }

    private static void deliver(Task task, String[] names, byte[][] contents, String saved, String failure) {
        boolean staged = false;
        try {
            if (Looper.myLooper() == Looper.getMainLooper()) throw new IllegalStateException("JNI transfer must run off the UI thread.");
            if (!clearActivity(task)) return;
            if (failure != null) {
                if (task.export != null && !failure.contains("Documents may remain")) failure += " " + task.export.status();
                if (task.save) failure += " The destination may contain a partial document; it was not deleted.";
                nativeFailed(task.id, failure);
            } else if (names != null) nativePicked(task.id, names, contents);
            else if (saved != null) {
                if (task.export != null) nativeExported(task.id, saved);
                else nativeSaved(task.id, saved);
            }
            else nativeCancelled(task.id);
            staged = true;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            cancelTask(task);
        } finally {
            // Release admission before Rust can wake a future that starts another request.
            workerFinished(task);
            if (staged && !task.destroyed) nativeComplete(task.id);
        }
    }

    private static String message(Throwable error) {
        String detail = error.getMessage();
        return detail == null || detail.isEmpty() ? error.getClass().getSimpleName() : detail;
    }
}
