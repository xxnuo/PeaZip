package com.longbridge.nsh;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/** Fake provider policy tests, not a real SAF/device test. */
public final class TreeExportCheck {
    record Entry(String path, boolean dir, int length) {}
    static Entry file(String path) { return new Entry(path, false, 3); }
    static Entry dir(String path) { return new Entry(path, true, 0); }
    static byte[] packet(Entry... entries) {
        int size = 4;
        for (Entry entry : entries) size += 12 + entry.path.getBytes(StandardCharsets.UTF_8).length + entry.length;
        ByteBuffer output = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN).putInt(entries.length);
        for (Entry entry : entries) {
            byte[] path = entry.path.getBytes(StandardCharsets.UTF_8);
            output.putInt(entry.dir ? 1 : 0).putInt(path.length).putInt(entry.length).put(path);
            for (int i = 0; i < entry.length; i++) output.put((byte) (i % 127));
        }
        return output.array();
    }
    static List<TreeExport.Document> largeMetadata(int batch) throws IOException {
        ArrayList<TreeExport.Document> result = new ArrayList<>();
        int rowBytes = 256 + 2 * (TreeExport.MAX_ID + TreeExport.MAX_NAME + TreeExport.MAX_MIME);
        for (int i = 0; i <= TreeExport.MAX_METADATA / rowBytes; i++) {
            String prefix = batch + ":" + i + ":";
            result.add(new TreeExport.Document(prefix + "x".repeat(TreeExport.MAX_ID - prefix.length()),
                    "n".repeat(TreeExport.MAX_NAME), "m".repeat(TreeExport.MAX_MIME)));
        }
        return result;
    }
    static final class Fake implements TreeExport.Provider {
        final Map<String, List<TreeExport.Document>> children = new HashMap<>();
        final List<String> writes = new ArrayList<>();
        final TreeExport.Document parent;
        int calls, creates, listings;
        String fault = "", status, rootName;
        boolean destroyed;
        Fake() throws Exception {
            parent = new TreeExport.Document("opaque:parent/path", "parent", TreeExport.DIRECTORY);
            children.put(parent.id, new ArrayList<>());
        }
        public void check() throws IOException { calls++; if (destroyed) throw new IOException("cancelled"); }
        public TreeExport.Document parent() { calls++; return parent; }
        public List<TreeExport.Document> children(TreeExport.Document parent) throws Exception {
            calls++;
            listings++;
            if (fault.equals("metadata-id")) return List.of(new TreeExport.Document("x".repeat(TreeExport.MAX_ID + 1), "n", TreeExport.FILE));
            if (fault.equals("metadata-name")) return List.of(new TreeExport.Document("id", "x".repeat(TreeExport.MAX_NAME + 1), TreeExport.FILE));
            if (fault.equals("metadata-mime")) return List.of(new TreeExport.Document("id", "n", "x".repeat(TreeExport.MAX_MIME + 1)));
            if (fault.equals("metadata-budget") || (fault.equals("metadata-postcreate") && creates == 2)) return largeMetadata(0);
            if (fault.equals("identity-budget")) {
                List<TreeExport.Document> result = new ArrayList<>(children.get(parent.id));
                if (listings == 3) return result; // The newly created root must be empty.
                // Each query fits its own budget, but varying IDs must not grow the global alias set forever.
                List<TreeExport.Document> batch = largeMetadata(listings);
                result.addAll(batch.subList(0, batch.size() * 3 / 4));
                return result;
            }
            if (fault.equals("permission")) throw new SecurityException("Permission denied");
            if (fault.equals("notfound")) throw new java.io.FileNotFoundException("Cannot enumerate");
            if (fault.equals("exists") && creates == 0)
                return List.of(new TreeExport.Document("existing", rootName, TreeExport.DIRECTORY));
            List<TreeExport.Document> result = new ArrayList<>(children.get(parent.id));
            if (fault.equals("listmime")) {
                for (int i = 0; i < result.size(); i++) {
                    TreeExport.Document child = result.get(i);
                    if (!child.mime.equals(TreeExport.DIRECTORY))
                        result.set(i, new TreeExport.Document(child.id, child.name, "text/plain"));
                }
            }
            return result;
        }
        public TreeExport.Document create(TreeExport.Document parent, String name, String mime) throws Exception {
            creates++;
            if (fault.equals("create")) throw new IOException("create failed");
            String id = "opaque:" + creates + "/not-a-path";
            if (fault.equals("idalias")) id = parent.id;
            if (fault.equals("siblingalias") && creates == 3) id = "opaque:2/not-a-path";
            String actual = fault.equals("rename") ? name + " (1)" : name;
            String type = fault.equals("type") ? TreeExport.FILE : mime;
            if (fault.equals("plain") && mime.equals(TreeExport.FILE)) type = "text/plain";
            if (fault.equals("filedir") && mime.equals(TreeExport.FILE)) type = TreeExport.DIRECTORY;
            TreeExport.Document child = new TreeExport.Document(id, actual, type);
            children.get(parent.id).add(child);
            children.putIfAbsent(id, new ArrayList<>());
            if (fault.equals("nonempty") && creates == 1)
                children.get(id).add(new TreeExport.Document("old", "existing", TreeExport.FILE));
            if (fault.equals("listingalias")) children.get(parent.id).add(child);
            if (fault.equals("raced")) children.get(parent.id).add(new TreeExport.Document("race", name, mime));
            return child;
        }
        public void write(TreeExport.Document document, byte[] packet, int offset, int length) throws Exception {
            if (length != 3 || packet[offset] != 0 || packet[offset + 2] != 2) throw new AssertionError("Wrong packet slice");
            if (fault.equals("write")) throw new IOException("write failed");
            if (fault.equals("close") && !writes.isEmpty()) throw new IOException("close failed");
            if (fault.equals("cancel")) { destroyed = true; check(); }
            writes.add(document.name);
        }
        public void progress(String value) {
            status = value;
            if (value.contains("Root: PeaZip-")) rootName = value.substring(6, 49);
        }
    }
    static int checks;
    static void invalid(byte[] packet) throws Exception {
        invalid(packet, null);
    }
    static void invalid(byte[] packet, String reason) throws Exception {
        Fake provider = new Fake();
        try { new TreeExport().run(packet, provider); throw new AssertionError("Accepted invalid packet"); }
        catch (IOException expected) {
            if (provider.calls != 0 || provider.creates != 0 || !provider.writes.isEmpty()) throw new AssertionError("Validation touched provider");
            if (reason != null && !expected.getMessage().startsWith(reason)) throw expected;
            if (!expected.getMessage().contains("completed 0") || !expected.getMessage().contains("may remain")) throw expected;
        }
        checks++;
    }
    static void parsedItems(byte[] packet, int count) throws Exception {
        // Inspect only parsing so large valid trees do not exercise provider metadata budgets.
        var parse = TreeExport.class.getDeclaredMethod("parse", byte[].class);
        parse.setAccessible(true);
        Map<?, ?> items = (Map<?, ?>) parse.invoke(null, (Object) packet);
        if (items.size() != count) throw new AssertionError("Wrong physical item count: " + items.size());
        checks++;
    }
    public static void main(String[] args) throws Exception {
        Fake provider = new Fake();
        String root = new TreeExport().run(packet(file("目录/中文.txt"), dir("empty/"), dir("目录/"), file("zero")), provider);
        if (!root.matches("PeaZip-[0-9a-f-]{36}") || provider.creates != 5 || provider.writes.size() != 2
                || !provider.status.contains("completed 4") || !provider.status.contains("unfinished path: none"))
            throw new AssertionError("Nested/implicit/explicit/empty directory export failed");
        checks++;
        for (String mode : List.of("plain", "listmime")) {
            Fake normalized = new Fake(); normalized.fault = mode;
            new TreeExport().run(packet(file("other.txt")), normalized);
            if (!normalized.writes.equals(List.of("other.txt")) || !normalized.status.contains("completed 1"))
                throw new AssertionError("Provider file MIME normalization rejected");
            checks++;
        }
        for (String path : List.of("../x", "/x", "a//x", "a/./x", "a/../x", "a\\x", "C:x", "a\u0000x",
                "a\nx", "a.", "a ", "CON", "com1.txt", "a/", "a/／x", "．．/x", "x".repeat(1025))) invalid(packet(file(path)));
        for (Entry[] entries : List.of(new Entry[] {file("a"), file("a")}, new Entry[] {file("A"), file("a")},
                new Entry[] {file("Ａ"), file("a")}, new Entry[] {file("ß"), file("ss")},
                new Entry[] {file("A/x"), file("a/y")}, new Entry[] {file("a"), file("a/b")},
                new Entry[] {file("a/b"), file("a")}, new Entry[] {dir("a/"), dir("a/")})) invalid(packet(entries));
        invalid(packet());
        invalid(packet(new Entry("a/", true, 1)));
        invalid(packet(new Entry("a", true, 0)));
        byte[] valid = packet(file("a"));
        for (int cut : new int[] {0, 3, 8, 15, valid.length - 1}) invalid(Arrays.copyOf(valid, cut));
        invalid(Arrays.copyOf(valid, valid.length + 1));
        byte[] bad = valid.clone(); bad[16] = (byte) 0xff; invalid(bad);
        bad = valid.clone(); bad[4] = 2; invalid(bad);
        bad = valid.clone(); bad[12] = (byte) 0xff; bad[15] = (byte) 0xff; invalid(bad);
        invalid(packet(new Entry("a", false, 16 * 1024 * 1024 + 1)));
        invalid(packet(new Entry("a", false, 16 * 1024 * 1024), new Entry("b", false, 16 * 1024 * 1024),
                new Entry("c", false, 16 * 1024 * 1024), new Entry("d", false, 16 * 1024 * 1024), file("e")));
        Entry[] medium = new Entry[2050];
        for (int i = 0; i < medium.length; i++) medium[i] = file("d" + i + "/f");
        parsedItems(packet(medium), 4100);
        Entry[] many = new Entry[4096];
        for (int i = 0; i < many.length; i++) many[i] = file("d" + i + "/x");
        parsedItems(packet(many), 8192);
        for (int i = 0; i < many.length; i++) many[i] = file("d" + i + "/a/b/f");
        parsedItems(packet(many), 16384);
        many[many.length - 1] = file("d4095/a/b/c/f");
        invalid(packet(many), "Export including implicit directories exceeds 16384 items.");
        Entry[] deep = new Entry[257];
        for (int i = 0; i < deep.length; i++) deep[i] = file("d" + i + "/" + "x/".repeat(62) + "f");
        invalid(packet(deep), "Export including implicit directories exceeds 16384 items.");
        Entry[] tooMany = new Entry[4097];
        for (int i = 0; i < tooMany.length; i++) tooMany[i] = file("f" + i);
        invalid(packet(tooMany), "Export count exceeds limit.");
        for (String fault : List.of("permission", "notfound", "exists", "create", "rename", "type", "filedir", "idalias", "siblingalias",
                "listingalias", "raced", "nonempty", "write", "close", "cancel")) {
            Fake fake = new Fake(); fake.fault = fault;
            try { new TreeExport().run(packet(file("a"), file("b")), fake); throw new AssertionError("Accepted " + fault); }
            catch (IOException expected) {
                String message = expected.getMessage();
                if (!message.contains("PeaZip-") || !message.contains("unfinished path:") || !message.contains("nothing was deleted"))
                    throw new AssertionError("Missing progress for " + fault, expected);
                if (fault.equals("filedir") && (!fake.writes.isEmpty() || !message.contains("completed 0")))
                    throw new AssertionError("File converted to directory was written or counted");
                if (fault.equals("close") && (!message.contains("completed 1") || !message.contains("unfinished path: b")))
                    throw new AssertionError("Close failure counted unfinished file", expected);
                if ((fault.equals("permission") || fault.equals("notfound") || fault.equals("exists")) && fake.creates != 0)
                    throw new AssertionError("Failure treated as absence");
            }
            checks++;
        }
        for (String fault : List.of("metadata-id", "metadata-name", "metadata-mime", "metadata-budget", "metadata-postcreate", "identity-budget")) {
            Fake fake = new Fake(); fake.fault = fault;
            try { new TreeExport().run(packet(file("a"), file("b")), fake); throw new AssertionError("Accepted " + fault); }
            catch (IOException expected) {
                if (!expected.getMessage().contains(fault.equals("identity-budget") ? "identities" : "metadata")) throw expected;
                int expectedCreates = fault.equals("metadata-postcreate") ? 2 : fault.equals("identity-budget") ? 1 : 0;
                if (fake.creates != expectedCreates || !fake.writes.isEmpty())
                    throw new AssertionError("Metadata rejection continued create/write for " + fault, expected);
                if (!expected.getMessage().contains("completed 0") || !expected.getMessage().contains("nothing was deleted")) throw expected;
            }
            checks++;
        }
        TreeExport.Document boundary = new TreeExport.Document("i".repeat(TreeExport.MAX_ID),
                "n".repeat(TreeExport.MAX_NAME), "m".repeat(TreeExport.MAX_MIME));
        int rowBytes = TreeExport.metadataTotal(0, boundary);
        if (TreeExport.metadataTotal(TreeExport.MAX_METADATA - rowBytes, boundary) != TreeExport.MAX_METADATA)
            throw new AssertionError("Exact metadata budget rejected");
        try { TreeExport.metadataTotal(TreeExport.MAX_METADATA - rowBytes + 1, boundary); throw new AssertionError("One-byte metadata overflow accepted"); }
        catch (IOException expected) { checks++; }
        checks++;
        android.net.Uri longTree = android.net.Uri.parse("content://chosen/tree/" + "t".repeat(TreeExport.MAX_ID));
        SafTreeExport adapter = new SafTreeExport(null, longTree, null, null);
        android.net.Uri longChild = adapter.uri(boundary);
        if (!android.provider.DocumentsContract.getDocumentId(longChild).equals(boundary.id)
                || !android.provider.DocumentsContract.getTreeDocumentId(longChild).equals("t".repeat(TreeExport.MAX_ID)))
            throw new AssertionError("Lazy opaque document URI changed scope or identity");
        checks++;
        for (String uri : List.of("content://chosen/tree/" + "t".repeat(TreeExport.MAX_ID + 1),
                "content://" + "a".repeat(256) + "/tree/root")) {
            try { SafTreeExport.requireTree(android.net.Uri.parse(uri)); throw new AssertionError("Unbounded tree metadata accepted"); }
            catch (IOException expected) { checks++; }
        }
        android.net.Uri tree = android.net.Uri.parse("content://chosen/tree/root%3Aopaque%2Fid");
        SafTreeExport.requireTree(tree);
        android.net.Uri child = android.provider.DocumentsContract.buildDocumentUriUsingTree(tree, "child:/opaque/id");
        SafTreeExport.requireScope(tree, child);
        if (!android.provider.DocumentsContract.getDocumentId(child).equals("child:/opaque/id"))
            throw new AssertionError("Opaque document ID was treated as a filesystem path");
        checks++;
        for (String uri : List.of("file:///tmp/tree/root/document/x", "content://app.fileprovider/tree/root%3Aopaque%2Fid/document/x",
                "content://other/tree/root%3Aopaque%2Fid/document/x", "content://chosen/tree/other/document/x",
                "content://chosen/document/x", "content://chosen/tree/root%3Aopaque%2Fid/document/x?redirect=1",
                "content://chosen/tree/root%3Aopaque%2Fid/document/x#fragment",
                "content://chosen/tree/root%3Aopaque%2Fid/document/x/children")) {
            try { SafTreeExport.requireScope(tree, android.net.Uri.parse(uri)); throw new AssertionError("Accepted URI redirect: " + uri); }
            catch (IOException expected) { checks++; }
        }
        SafTreeExport.requireComplete(null);
        android.os.Bundle info = new android.os.Bundle();
        info.putString(android.provider.DocumentsContract.EXTRA_INFO, "informational only");
        SafTreeExport.requireComplete(info);
        checks += 2;
        for (String clearedError : new String[] {null, ""}) {
            android.os.Bundle cleared = new android.os.Bundle();
            cleared.putString(android.provider.DocumentsContract.EXTRA_ERROR, clearedError);
            SafTreeExport.requireComplete(cleared);
            checks++;
        }
        for (int scenario = 0; scenario < 3; scenario++) {
            android.os.Bundle extras = new android.os.Bundle();
            if (scenario == 0) {
                extras.putBoolean(android.provider.DocumentsContract.EXTRA_LOADING, true);
                extras.putString(android.provider.DocumentsContract.EXTRA_ERROR, "");
            }
            else {
                extras.putString(android.provider.DocumentsContract.EXTRA_ERROR, "Permission denied");
                if (scenario == 2) extras.putString(android.provider.DocumentsContract.EXTRA_INFO, "info does not clear error");
            }
            try { SafTreeExport.requireComplete(extras); throw new AssertionError("Error/loading cursor was treated as absence"); }
            catch (IOException expected) { checks++; }
        }
        System.out.println("PASS: " + checks + " fake-provider export checks: full packet validation before I/O; Unicode/paths/limits; empty and implicit directories; permission/create/write/close/cancel/alias faults (not real SAF)");
    }
}
