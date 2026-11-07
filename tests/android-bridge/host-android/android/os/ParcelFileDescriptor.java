package android.os;

import java.io.IOException;

/** Models cached peer status surviving close, when canDetectErrors has become false. */
public final class ParcelFileDescriptor {
    public boolean reliable, peerError, closed;
    public int checks, checksAfterClose;
    private boolean consumed;
    public boolean canDetectErrors() { return reliable && !closed; }
    public void close() {
        if (closed) return;
        closed = true;
        // Model SDK close reading the now-exhausted peer channel after EOF consumed its status.
        if (reliable && consumed) peerError = true;
    }
    public void checkError() throws IOException {
        checks++;
        if (closed) checksAfterClose++; else consumed = true;
        if (reliable && peerError) throw new IOException("Reliable peer closeWithError (host model)");
    }
}
