import java.io.*;
import java.lang.reflect.*;
import java.util.Arrays;

public class BridgeReadCheck {
    static Class<?> taskClass;
    static Constructor<?> taskConstructor;
    static Method read;
    static Field destroyed;
    static int checks;

    static Object task() throws Exception { return taskConstructor.newInstance(1L, 1, null); }
    static byte[] read(InputStream input, int limit, Object task) throws Exception {
        try { return (byte[]) read.invoke(null, input, limit, task); }
        catch (InvocationTargetException e) {
            if (e.getCause() instanceof Exception ex) throw ex;
            throw e;
        }
    }
    static void rejects(InputStream input, int limit, Object task) throws Exception {
        try { read(input, limit, task); throw new AssertionError("Expected IOException"); }
        catch (IOException expected) { checks++; }
    }
    static InputStream repeated(int size) {
        return new InputStream() {
            int left = size;
            public int available() { return Integer.MAX_VALUE; }
            public int read() { if (left == 0) return -1; left--; return 7; }
            public int read(byte[] b, int off, int len) {
                if (left == 0) return -1;
                int n = Math.min(left, len); Arrays.fill(b, off, off+n, (byte) 7); left -= n; return n;
            }
        };
    }
    public static void main(String[] args) throws Exception {
        Class<?> activity = Class.forName("com.longbridge.nsh.PeaZipActivity");
        taskClass = Class.forName("com.longbridge.nsh.PeaZipActivity$Task");
        taskConstructor = taskClass.getDeclaredConstructor(long.class, int.class, byte[].class);
        taskConstructor.setAccessible(true);
        destroyed = taskClass.getDeclaredField("destroyed"); destroyed.setAccessible(true);
        read = activity.getDeclaredMethod("readBounded", InputStream.class, int.class, taskClass);
        read.setAccessible(true);
        int max = 16 * 1024 * 1024;
        if (read(repeated(max), max, task()).length != max) throw new AssertionError(); checks++;
        rejects(repeated(max + 1), max, task());
        if (read(repeated(0), 0, task()).length != 0) throw new AssertionError(); checks++;
        rejects(repeated(1), 0, task());
        rejects(repeated(101), 100, task());
        if (read(repeated(99), 100, task()).length != 99) throw new AssertionError(); checks++;
        InputStream zeroReads = new ByteArrayInputStream(new byte[] {1, 2, 3}) {
            public synchronized int read(byte[] b, int off, int len) { return 0; }
        };
        if (!Arrays.equals(read(zeroReads, 3, task()), new byte[] {1, 2, 3})) throw new AssertionError(); checks++;
        rejects(new InputStream() { public int read() throws IOException { throw new IOException("Provider failed"); } }, 100, task());
        Object cancelled = task(); destroyed.setBoolean(cancelled, true);
        rejects(repeated(1), 1, cancelled);
        Object duringRead = task();
        rejects(new ByteArrayInputStream(new byte[] {1}) {
            public synchronized int read(byte[] b, int off, int len) {
                try { destroyed.setBoolean(duringRead, true); } catch (Exception e) { throw new AssertionError(e); }
                return super.read(b, off, len);
            }
        }, 1, duringRead);
        Object duringProbe = task();
        rejects(new ByteArrayInputStream(new byte[] {1}) {
            public synchronized int read() {
                try { destroyed.setBoolean(duringProbe, true); } catch (Exception e) { throw new AssertionError(e); }
                return super.read();
            }
        }, 1, duringProbe);
        System.out.println("PASS: " + checks + " bounded-read checks");
    }
}
