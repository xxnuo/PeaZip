package android.os;

/** Host double; provider cancellation itself still needs Android device coverage. */
public final class CancellationSignal {
    private volatile boolean cancelled;
    private OnCancelListener listener;
    public interface OnCancelListener { void onCancel(); }
    public boolean isCanceled() { return cancelled; }
    public synchronized void setOnCancelListener(OnCancelListener listener) { this.listener = listener; }
    public void cancel() {
        OnCancelListener callback;
        synchronized (this) { cancelled = true; callback = listener; }
        if (callback != null) callback.onCancel();
    }
}
