package android.content.res;

import java.io.*;

/** Platform double: a selected asset is a slice, not its entire backing file. */
public final class AssetFileDescriptor implements Closeable {
    public static final long UNKNOWN_LENGTH = -1;
    private final File file;
    private final long offset, length;
    public int inputOpens, outputOpens;
    public boolean failClose;
    public AssetFileDescriptor(File file, long offset, long length) {
        this.file = file; this.offset = offset; this.length = length;
    }
    public long getDeclaredLength() { return length; }
    public long getStartOffset() { return offset; }
    public void close() {}
    public FileInputStream createInputStream() throws IOException {
        inputOpens++;
        return new FileInputStream(file) {
            long left = length;
            { getChannel().position(offset); }
            public int read() throws IOException {
                if (left == 0) return -1;
                int value = super.read(); if (value >= 0 && left > 0) left--; return value;
            }
            public int read(byte[] bytes, int at, int count) throws IOException {
                if (left == 0) return -1;
                int size = super.read(bytes, at, left < 0 ? count : (int) Math.min(count, left));
                if (size > 0 && left > 0) left -= size;
                return size;
            }
        };
    }
    public FileOutputStream createOutputStream() throws IOException {
        outputOpens++;
        return new FileOutputStream(file) {
            public void close() throws IOException {
                super.close();
                if (failClose) throw new IOException("Provider close failed");
            }
        };
    }
}
