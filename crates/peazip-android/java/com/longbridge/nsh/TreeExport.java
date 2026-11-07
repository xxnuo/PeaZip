package com.longbridge.nsh;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.text.Normalizer;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.UUID;

/** Packet validation and export policy, independent of Android for provider fault injection. */
public final class TreeExport {
    static final int MAX_PACKET = 64 * 1024 * 1024 + 4096 * (12 + 1024) + 4;
    // Bounds are UTF-16 code units; budgets include two bytes per unit plus object overhead.
    static final int MAX_ID = 4096, MAX_NAME = 1024, MAX_MIME = 256;
    static final int MAX_METADATA = 4 * 1024 * 1024;
    static final String DIRECTORY = "vnd.android.document/directory";
    static final String FILE = "application/octet-stream";

    public static final class Document {
        public final String id, name, mime;
        public Document(String id, String name, String mime) throws IOException {
            this.id = metadataField(id, MAX_ID);
            this.name = metadataField(name, MAX_NAME);
            this.mime = metadataField(mime, MAX_MIME);
        }
    }

    public interface Provider {
        void check() throws IOException;
        Document parent() throws Exception;
        List<Document> children(Document parent) throws Exception;
        Document create(Document parent, String name, String mime) throws Exception;
        void write(Document file, byte[] packet, int offset, int length) throws Exception;
        void progress(String status);
    }

    static String metadataField(String value, int limit) throws IOException {
        if (value == null || value.isEmpty() || value.length() > limit)
            throw new IOException("Provider document metadata field is missing or exceeds length limit.");
        return value;
    }

    static int metadataTotal(int used, Document document) throws IOException {
        int next = used + 256 + 2 * (document.id.length() + document.name.length() + document.mime.length());
        if (next > MAX_METADATA) throw new IOException("Provider children metadata exceeds 4 MiB budget.");
        return next;
    }

    private static List<Document> checkedChildren(Provider provider, Document parent) throws Exception {
        List<Document> children = provider.children(parent);
        if (children == null || children.size() > 4096) throw new IOException("Invalid provider child count.");
        int bytes = 0;
        for (Document child : children) bytes = metadataTotal(bytes, child);
        return children;
    }

    private int knownIdBytes;
    private boolean rememberId(Set<String> ids, String id) throws IOException {
        if (ids.contains(id)) return false;
        int next = knownIdBytes + 128 + 2 * id.length();
        if (next > MAX_METADATA) throw new IOException("Provider document identities exceed 4 MiB budget.");
        ids.add(id);
        knownIdBytes = next;
        return true;
    }

    private static final class Item {
        final String path, parent, name;
        final boolean dir;
        boolean explicit;
        int offset, length;
        Item(String path, String parent, String name, boolean dir) {
            this.path = path; this.parent = parent; this.name = name; this.dir = dir;
        }
    }

    private String root = "not yet known", unfinished = "not yet known";
    private int completed;

    public synchronized String status() {
        return "Root: " + root + "; completed " + completed + " entries; unfinished path: " + unfinished
                + ". Documents may remain; nothing was deleted.";
    }
    private synchronized void progress(Provider provider, String path, boolean done) {
        unfinished = path;
        if (done) completed++;
        provider.progress(status());
    }

    static String key(String value) {
        return Normalizer.normalize(value, Normalizer.Form.NFKC).toUpperCase(Locale.ROOT).toLowerCase(Locale.ROOT);
    }

    private static void component(String value) throws IOException {
        String stem = value.split("\\.", -1)[0].toUpperCase(Locale.ROOT);
        if (value.isEmpty() || value.equals(".") || value.equals("..") || value.endsWith(".") || value.endsWith(" ")
                || value.chars().anyMatch(c -> Character.isISOControl(c) || "/\\:<>\"|?*".indexOf(c) >= 0)
                || stem.matches("CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9]"))
            throw new IOException("Unsafe export path component.");
    }

    private static int number(ByteBuffer input) throws IOException {
        if (input.remaining() < 4) throw new IOException("Truncated export packet.");
        int n = input.getInt();
        if (n < 0) throw new IOException("Invalid unsigned export length.");
        return n;
    }

    private static LinkedHashMap<String, Item> parse(byte[] packet) throws IOException {
        if (packet == null || packet.length > MAX_PACKET) throw new IOException("Export packet exceeds byte limit.");
        ByteBuffer input = ByteBuffer.wrap(packet).order(ByteOrder.LITTLE_ENDIAN);
        int count = number(input);
        if (count == 0 || count > 4096) throw new IOException("Export count exceeds limit.");
        LinkedHashMap<String, Item> items = new LinkedHashMap<>();
        long total = 0;
        for (int i = 0; i < count; i++) {
            int kind = number(input), pathLength = number(input), length = number(input);
            if (kind > 1 || pathLength == 0 || pathLength > 1024 || length > 16 * 1024 * 1024
                    || (kind == 1 && length != 0) || (total += length) > 64 * 1024 * 1024
                    || (long) pathLength + length > input.remaining()) throw new IOException("Invalid export entry limits.");
            String path;
            try {
                path = StandardCharsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT)
                        .onUnmappableCharacter(CodingErrorAction.REPORT)
                        .decode(ByteBuffer.wrap(packet, input.position(), pathLength)).toString();
            } catch (java.nio.charset.CharacterCodingException e) { throw new IOException("Invalid UTF-8 export path.", e); }
            input.position(input.position() + pathLength);
            if (kind == 1) {
                if (!path.endsWith("/")) throw new IOException("Directory path must end with slash.");
                path = path.substring(0, path.length() - 1);
            }
            String[] parts = path.split("/", -1);
            String parent = "", prefix = "";
            for (int j = 0; j < parts.length; j++) {
                component(parts[j]);
                component(Normalizer.normalize(parts[j], Normalizer.Form.NFKC));
                prefix = parent.isEmpty() ? parts[j] : parent + "/" + parts[j];
                boolean last = j == parts.length - 1, dir = !last || kind == 1;
                String key = key(prefix);
                Item item = items.get(key);
                if (item == null) {
                    if (items.size() == 4096) throw new IOException("Export including implicit directories exceeds 4096 entries.");
                    item = new Item(prefix, parent, parts[j], dir);
                    items.put(key, item);
                } else if (!item.path.equals(prefix) || !item.dir || !dir || (last && item.explicit)) {
                    throw new IOException("Duplicate, case/NFKC alias or file-parent export conflict.");
                }
                if (last) { item.explicit = true; item.offset = input.position(); item.length = length; }
                parent = prefix;
            }
            input.position(input.position() + length);
        }
        if (input.hasRemaining()) throw new IOException("Trailing export packet bytes.");
        return items;
    }

    private Document fresh(Provider provider, Document parent, String name, String mime, Set<String> ids)
            throws Exception {
        provider.check();
        List<Document> before = checkedChildren(provider, parent);
        Set<String> siblings = new HashSet<>();
        for (Document child : before) {
            if (!siblings.add(child.id)) throw new IOException("Provider returned same-ID aliases.");
            rememberId(ids, child.id);
            if (key(child.name).equals(key(name))) throw new IOException("Destination already exists; refusing to merge or overwrite.");
        }
        provider.check();
        // SAF has no cross-provider atomic no-replace primitive; these checks are best effort.
        Document child = provider.create(parent, name, mime);
        provider.check();
        // Providers may infer a regular file MIME from its extension. Directory identity must not change.
        if (child == null || !child.name.equals(name) || child.mime.equals(DIRECTORY) != mime.equals(DIRECTORY) || !rememberId(ids, child.id))
            throw new IOException("Provider renamed, changed type or aliased the new document (returned name: "
                    + (child == null ? "unknown" : child.name) + ").");
        int matches = 0;
        for (Document listed : checkedChildren(provider, parent)) {
            if (key(listed.name).equals(key(name)) || listed.id.equals(child.id)) {
                if (!listed.id.equals(child.id) || !listed.name.equals(name) || listed.mime.equals(DIRECTORY) != mime.equals(DIRECTORY))
                    throw new IOException("Provider returned conflicting name or document ID after creation.");
                matches++;
            }
        }
        if (matches != 1) throw new IOException("New document is missing or aliased in parent.");
        if (mime.equals(DIRECTORY) && !checkedChildren(provider, child).isEmpty())
            throw new IOException("New directory is not empty; refusing to merge.");
        provider.check();
        return child;
    }

    public String run(byte[] packet, Provider provider) throws Exception {
        try {
            // Validate the entire packet, including implicit parents, before any provider call.
            LinkedHashMap<String, Item> items = parse(packet);
            provider.check();
            Document parent = provider.parent();
            if (!parent.mime.equals(DIRECTORY)) throw new IOException("Selected parent is not a directory.");
            Set<String> ids = new HashSet<>();
            rememberId(ids, parent.id);
            synchronized (this) { root = "PeaZip-" + UUID.randomUUID(); }
            progress(provider, root, false);
            Document destination = fresh(provider, parent, root, DIRECTORY, ids);
            Map<String, Document> directories = new HashMap<>();
            directories.put("", destination);
            for (Item item : items.values()) {
                progress(provider, item.path, false);
                Document document = fresh(provider, directories.get(item.parent), item.name,
                        item.dir ? DIRECTORY : FILE, ids);
                if (item.dir) directories.put(item.path, document);
                else provider.write(document, packet, item.offset, item.length);
                provider.check(); // write() must have closed the writer and descriptor successfully.
                if (item.explicit) progress(provider, item.path, true);
            }
            progress(provider, "none", false);
            return root;
        } catch (Exception | OutOfMemoryError e) {
            throw new IOException((e.getMessage() == null ? e.getClass().getSimpleName() : e.getMessage()) + " " + status(), e);
        }
    }
}
