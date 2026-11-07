package io.peazip.rust.bridgechecks;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.res.AssetFileDescriptor;
import android.os.Bundle;
import android.os.ParcelFileDescriptor;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.lang.reflect.Constructor;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.util.Arrays;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;

/** Device-only instrumentation: real Android reliable pipes, production asset methods, no Activity or test entry in the app. */
public final class ReliablePipeChecks extends Instrumentation {
    private static final byte[] DATA = {1, 2, 3};
    private static final String PEER_ERROR = "peazip-injected-peer-error";
    private Class<?> activity, task;
    private Constructor<?> constructor;
    private Method write, read;
    private int checks;

    @Override public void onCreate(Bundle args) { super.onCreate(args); start(); }

    private static void await(CountDownLatch latch) throws IOException {
        try {
            if (!latch.await(10, TimeUnit.SECONDS)) throw new IOException("Reliable pipe barrier timed out");
        } catch (InterruptedException e) { Thread.currentThread().interrupt(); throw new IOException(e); }
    }

    private Object invoke(Method method, Object... args) throws Exception {
        try { return method.invoke(null, args); }
        catch (InvocationTargetException e) {
            if (e.getCause() instanceof Exception) throw (Exception) e.getCause();
            throw new AssertionError(e.getCause());
        }
    }

    private void write(AssetFileDescriptor descriptor) throws Exception {
        invoke(write, descriptor, DATA, constructor.newInstance(1L, 1, DATA));
    }

    private byte[] read(AssetFileDescriptor descriptor) throws Exception {
        return (byte[]) invoke(read, descriptor, DATA.length, constructor.newInstance(1L, 1, null));
    }

    private static void peerFinished(Thread peer, AtomicReference<Throwable> failure) throws Exception {
        peer.join(10000);
        if (peer.isAlive()) throw new AssertionError("Reliable pipe peer did not stop");
        if (failure.get() != null) throw new AssertionError("Reliable pipe peer failed", failure.get());
    }

    private void output(boolean error) throws Exception {
        ParcelFileDescriptor[] pipe = ParcelFileDescriptor.createReliablePipe();
        CountDownLatch flushed = new CountDownLatch(1), peerClosed = new CountDownLatch(1);
        AtomicReference<Throwable> peerFailure = new AtomicReference<>();
        AtomicBoolean streamClosed = new AtomicBoolean();
        Thread peer = new Thread(() -> {
            try (InputStream input = new ParcelFileDescriptor.AutoCloseInputStream(pipe[0])) {
                for (byte value : DATA) if (input.read() != (value & 255)) throw new AssertionError("Wrong pipe bytes");
                await(flushed);
                if (error) pipe[0].closeWithError(PEER_ERROR); else pipe[0].close();
            } catch (Throwable e) { peerFailure.set(e); }
            finally { peerClosed.countDown(); }
        }, "PeaZip-test-peer");
        peer.setDaemon(true);
        peer.start();
        try (ParcelFileDescriptor reader = pipe[0]; ParcelFileDescriptor writer = pipe[1];
             AssetFileDescriptor descriptor = new AssetFileDescriptor(writer, 0, AssetFileDescriptor.UNKNOWN_LENGTH) {
                 @Override public FileOutputStream createOutputStream() {
                     return new ParcelFileDescriptor.AutoCloseOutputStream(writer) {
                         @Override public void flush() throws IOException {
                             super.flush();
                             flushed.countDown();
                             await(peerClosed); // Peer error is available before local close, after successful write/flush.
                         }
                         @Override public void close() throws IOException { super.close(); streamClosed.set(true); }
                     };
                 }
             }) {
            IOException failure = null;
            try { write(descriptor); } catch (IOException e) { failure = e; }
            peerFinished(peer, peerFailure);
            if (!streamClosed.get()) throw new AssertionError("Output stream close itself did not succeed");
            if (writer.canDetectErrors()) throw new AssertionError("Closed writer still claims a live comm FD");
            if (error && (failure == null || !failure.getMessage().contains(PEER_ERROR)))
                throw new AssertionError("Cached reliable output peer error was ignored", failure);
            if (!error && failure != null) throw failure;
        } finally { flushed.countDown(); pipe[0].close(); pipe[1].close(); }
        checks++;
    }

    private void input(boolean bounded, boolean error) throws Exception {
        ParcelFileDescriptor[] pipe = ParcelFileDescriptor.createReliablePipe();
        try (ParcelFileDescriptor reader = pipe[0]; ParcelFileDescriptor writer = pipe[1]) {
            FileOutputStream remote = new ParcelFileDescriptor.AutoCloseOutputStream(writer);
            remote.write(DATA);
            remote.flush();
            if (error) writer.closeWithError(PEER_ERROR); else writer.close();
            try (AssetFileDescriptor descriptor = new AssetFileDescriptor(reader, 0,
                    bounded ? DATA.length : AssetFileDescriptor.UNKNOWN_LENGTH)) {
                IOException failure = null;
                byte[] result = null;
                try { result = read(descriptor); } catch (IOException e) { failure = e; }
                if (error && (failure == null || !failure.getMessage().contains(PEER_ERROR)))
                    throw new AssertionError("Reliable " + (bounded ? "bounded" : "whole") + " input error was ignored", failure);
                if (!error && (failure != null || !Arrays.equals(result, DATA)))
                    throw new AssertionError("Valid reliable input was rejected", failure);
            } finally { remote.close(); }
        }
        checks++;
    }

    private void regularFile() throws Exception {
        File file = File.createTempFile("peazip-pipe-check-", ".bin", getTargetContext().getCacheDir());
        try {
            try (AssetFileDescriptor descriptor = new AssetFileDescriptor(ParcelFileDescriptor.open(file,
                    ParcelFileDescriptor.MODE_READ_WRITE | ParcelFileDescriptor.MODE_TRUNCATE), 0, AssetFileDescriptor.UNKNOWN_LENGTH)) {
                write(descriptor);
            }
            try (AssetFileDescriptor descriptor = new AssetFileDescriptor(ParcelFileDescriptor.open(file,
                    ParcelFileDescriptor.MODE_READ_ONLY), 0, DATA.length)) {
                if (!Arrays.equals(read(descriptor), DATA)) throw new AssertionError("Regular asset changed");
            }
        } finally { if (!file.delete()) throw new IOException("Cannot delete test-owned file"); }
        checks++;
    }

    @Override public void onStart() {
        Bundle result = new Bundle();
        try {
            ClassLoader loader = getTargetContext().getClassLoader();
            activity = loader.loadClass("com.longbridge.nsh.PeaZipActivity");
            task = loader.loadClass("com.longbridge.nsh.PeaZipActivity$Task");
            constructor = task.getDeclaredConstructor(long.class, int.class, byte[].class);
            constructor.setAccessible(true);
            write = activity.getDeclaredMethod("writeAsset", AssetFileDescriptor.class, byte[].class, task);
            read = activity.getDeclaredMethod("readAsset", AssetFileDescriptor.class, int.class, task);
            write.setAccessible(true); read.setAccessible(true);
            output(true); output(false);
            input(false, true); input(true, true); input(false, false); input(true, false);
            regularFile();
            result.putString("stream", "PASS: " + checks + " real Android reliable-pipe/regular-file checks\n");
            finish(Activity.RESULT_OK, result);
        } catch (Throwable e) {
            result.putString("stream", "FAIL: " + android.util.Log.getStackTraceString(e) + "\n");
            finish(Activity.RESULT_CANCELED, result);
        }
    }
}
