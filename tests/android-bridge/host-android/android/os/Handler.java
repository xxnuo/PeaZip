package android.os;

public final class Handler {
    private final Looper looper;
    public Handler(Looper looper) { this.looper = looper; }
    public boolean post(Runnable action) { looper.executor.execute(action); return true; }
}
