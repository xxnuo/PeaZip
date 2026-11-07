package android.content.res;

import java.io.*;

/** Platform double: a selected asset is a slice, not its entire backing file. */
public final class AssetFileDescriptor implements Closeable {
    public static final long UNKNOWN_LENGTH = -1;
    private final File file;
    private final long offset, length;
    public int inputOpens, outputOpens;
    public boolean failClose;
    private final android.os.ParcelFileDescriptor parcel = new android.os.ParcelFileDescriptor();
    public android.os.ParcelFileDescriptor getParcelFileDescriptor() { return parcel; }
    public AssetFileDescriptor(File file, long offset, long length) {
        this.file = file; this.offset = offset; this.length = length;
    }
    public long getDeclaredLength() { return length; }
    public long getStartOffset() { return offset; }
    public void close() { parcel.close(); }
    public FileInputStream createInputStream() throws IOException {
        inputOpens++;
        return new FileInputStream(file) {
            long left = length;
            { getChannel().position(offset); }
            public void close() throws IOException { try { super.close(); } finally { parcel.close(); } }
            public int read() throws IOException {
                if (left == 0) return -1;
                int value = super.read();
                if (value == -1 && parcel.canDetectErrors()) parcel.checkError();
                if (value >= 0 && left > 0) left--; return value;
            }
            public int read(byte[] bytes, int at, int count) throws IOException {
                if (left == 0) return -1;
                int size = super.read(bytes, at, left < 0 ? count : (int) Math.min(count, left));
                if (size == -1 && parcel.canDetectErrors()) parcel.checkError();
                if (size > 0 && left > 0) left -= size;
                return size;
            }
        };
    }
    public FileOutputStream createOutputStream() throws IOException {
        outputOpens++;
        return new FileOutputStream(file) {
            public void close() throws IOException {
                try { super.close(); } finally { parcel.close(); }
                if (failClose) throw new IOException("Provider close failed");
            }
        };
    }
}
