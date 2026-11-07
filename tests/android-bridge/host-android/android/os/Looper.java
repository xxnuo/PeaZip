package android.os;

import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/** A distinct host UI thread, never the JNI invocation or transfer worker. */
public final class Looper {
    private static final Looper MAIN = new Looper();
    private static volatile Thread ui;
    final ExecutorService executor = Executors.newSingleThreadExecutor(work -> {
        Thread thread = new Thread(work, "host-android-main");
        thread.setDaemon(true);
        ui = thread;
        return thread;
    });
    public static Looper getMainLooper() { return MAIN; }
    public static Looper myLooper() { return Thread.currentThread() == ui ? MAIN : null; }
}
