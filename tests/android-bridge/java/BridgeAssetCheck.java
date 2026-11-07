import android.content.res.AssetFileDescriptor;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.Arrays;

public class BridgeAssetCheck {
    static final Class<?> ACTIVITY = BridgeLifecycleCheck.ACTIVITY, TASK = BridgeLifecycleCheck.TASK;
    static Object task() throws Exception {
        var constructor = TASK.getDeclaredConstructor(long.class, int.class, byte[].class);
        constructor.setAccessible(true); return constructor.newInstance(1L, 1, null);
    }
    static AssetFileDescriptor asset(Path file, long offset, long length) throws Exception {
        // The extra constructor belongs only to the host platform double.
        return AssetFileDescriptor.class.getConstructor(File.class, long.class, long.class).newInstance(file.toFile(), offset, length);
    }
    static byte[] read(AssetFileDescriptor asset, int budget) throws Exception {
        return (byte[]) BridgeLifecycleCheck.call(ACTIVITY, null, "readAsset",
                new Class<?>[] {AssetFileDescriptor.class, int.class, TASK}, asset, budget, task());
    }
    static void write(AssetFileDescriptor asset, byte[] bytes) throws Exception {
        BridgeLifecycleCheck.call(ACTIVITY, null, "writeAsset",
                new Class<?>[] {AssetFileDescriptor.class, byte[].class, TASK}, asset, bytes, task());
    }
    interface Action { void run() throws Exception; }
    static void rejects(Action action) throws Exception {
        try { action.run(); throw new AssertionError("Expected asset rejection"); }
        catch (IOException expected) {}
    }
    public static void main(String[] args) throws Exception {
        Path file = Files.createTempFile("peazip-asset-", ".bin");
        try {
            Files.writeString(file, "XXabcSECRET", StandardCharsets.UTF_8);
            if (!Arrays.equals(read(asset(file, 2, 3), 3), "abc".getBytes(StandardCharsets.UTF_8)))
                throw new AssertionError("Read escaped the selected offset/length slice");
            AssetFileDescriptor oversized = asset(file, 0, 4);
            rejects(() -> read(oversized, 3));
            if ((int) BridgeLifecycleCheck.get(oversized, "inputOpens") != 0) throw new AssertionError("Oversized asset was opened");
            rejects(() -> read(asset(file, 0, -1), 3)); // Unknown length still needs the extra probe.
            AssetFileDescriptor bounded = asset(file, 0, 3), shifted = asset(file, 2, -1);
            byte[] zip = {1, 2, 3, 4};
            rejects(() -> write(bounded, zip));
            rejects(() -> write(shifted, zip));
            if ((int) BridgeLifecycleCheck.get(bounded, "outputOpens") != 0
                    || (int) BridgeLifecycleCheck.get(shifted, "outputOpens") != 0
                    || !Files.readString(file).equals("XXabcSECRET")) throw new AssertionError("Rejected output touched the document");
            write(asset(file, 0, -1), zip);
            if (!Arrays.equals(Files.readAllBytes(file), zip)) throw new AssertionError("Whole document save was incomplete");
            byte[] packet = {9, 8, 1, 2, 3, 4, 7};
            BridgeLifecycleCheck.call(ACTIVITY, null, "writeAsset",
                    new Class<?>[] {AssetFileDescriptor.class, byte[].class, int.class, int.class, TASK},
                    asset(file, 0, -1), packet, 2, 4, task());
            if (!Arrays.equals(Files.readAllBytes(file), zip)) throw new AssertionError("Export write escaped packet slice");
            AssetFileDescriptor closeFailure = asset(file, 0, -1);
            BridgeLifecycleCheck.set(closeFailure, "failClose", true);
            rejects(() -> write(closeFailure, zip));
            if (!Files.exists(file)) throw new AssertionError("Failed save deleted the destination");
            AssetFileDescriptor reliableWrite = asset(file, 0, -1);
            var writeParcel = reliableWrite.getParcelFileDescriptor();
            writeParcel.getClass().getField("reliable").setBoolean(writeParcel, true);
            writeParcel.getClass().getField("peerError").setBoolean(writeParcel, true);
            rejects(() -> write(reliableWrite, zip));
            if (writeParcel.canDetectErrors() || writeParcel.getClass().getField("checksAfterClose").getInt(writeParcel) != 1)
                throw new AssertionError("Cached write peer error was not checked after close");
            for (long length : new long[] {-1, zip.length}) {
                AssetFileDescriptor reliableRead = asset(file, 0, length);
                var readParcel = reliableRead.getParcelFileDescriptor();
                readParcel.getClass().getField("reliable").setBoolean(readParcel, true);
                readParcel.getClass().getField("peerError").setBoolean(readParcel, true);
                rejects(() -> read(reliableRead, zip.length));
                int afterClose = readParcel.getClass().getField("checksAfterClose").getInt(readParcel);
                if (readParcel.canDetectErrors() || readParcel.getClass().getField("checks").getInt(readParcel) != 1
                        || afterClose != (length < 0 ? 0 : 1))
                    throw new AssertionError("Cached read peer error was not checked after close");
            }
            AssetFileDescriptor reliableOK = asset(file, 0, -1);
            var okParcel = reliableOK.getParcelFileDescriptor();
            okParcel.getClass().getField("reliable").setBoolean(okParcel, true);
            write(reliableOK, zip);
            if (okParcel.getClass().getField("checks").getInt(okParcel) != 1)
                throw new AssertionError("Reliable successful write skipped status check");
            AssetFileDescriptor reliableWholeRead = asset(file, 0, -1);
            var wholeParcel = reliableWholeRead.getParcelFileDescriptor();
            wholeParcel.getClass().getField("reliable").setBoolean(wholeParcel, true);
            if (!Arrays.equals(read(reliableWholeRead, zip.length), zip)
                    || wholeParcel.getClass().getField("checks").getInt(wholeParcel) != 1
                    || wholeParcel.getClass().getField("checksAfterClose").getInt(wholeParcel) != 0)
                throw new AssertionError("Physical EOF peer status was checked twice");
            System.out.println("PASS: 13 asset checks: selected slice/probe, declared budget, bounded/offset save rejection, whole save, export packet slice, close failure and cached reliable peer errors (host model)");
        } finally { Files.deleteIfExists(file); }
    }
}
