package com.longbridge.nsh;

import android.content.ContentResolver;
import android.content.res.AssetFileDescriptor;
import android.database.Cursor;
import android.net.Uri;
import android.provider.DocumentsContract;
import android.provider.DocumentsContract.Document;
import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.function.Consumer;

/** All document IDs stay opaque; only DocumentsContract constructs descendant URIs. */
final class SafTreeExport implements TreeExport.Provider {
    private static final String[] COLUMNS = {Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME, Document.COLUMN_MIME_TYPE};
    private final ContentResolver resolver;
    private final Uri tree;
    private final PeaZipActivity.Task task;
    private final Consumer<String> progress;

    SafTreeExport(ContentResolver resolver, Uri tree, PeaZipActivity.Task task, Consumer<String> progress) throws IOException {
        requireTree(tree);
        this.resolver = resolver; this.tree = tree; this.task = task; this.progress = progress;
    }

    static void requireTree(Uri uri) throws IOException {
        if (uri == null || !"content".equals(uri.getScheme()) || uri.getAuthority() == null
                || uri.getAuthority().isEmpty() || uri.getAuthority().length() > 255 || !DocumentsContract.isTreeUri(uri)
                || uri.getQuery() != null || uri.getFragment() != null)
            throw new IOException("Only content:// tree URIs are accepted.");
        try {
            String id = DocumentsContract.getTreeDocumentId(uri);
            if (id.isEmpty() || id.length() > TreeExport.MAX_ID) throw new IllegalArgumentException();
        } catch (IllegalArgumentException e) { throw new IOException("Invalid tree URI.", e); }
    }

    static void requireScope(Uri tree, Uri uri) throws IOException {
        requireTree(uri);
        if (uri.getPathSegments().size() != 4 || !"document".equals(uri.getPathSegments().get(2)))
            throw new IOException("Expected a tree-scoped document URI.");
        if (!tree.getAuthority().equals(uri.getAuthority())
                || !DocumentsContract.getTreeDocumentId(tree).equals(DocumentsContract.getTreeDocumentId(uri)))
            throw new IOException("Provider redirected a document outside the selected authority/tree scope.");
        try {
            String id = DocumentsContract.getDocumentId(uri);
            if (id.isEmpty() || id.length() > TreeExport.MAX_ID) throw new IllegalArgumentException();
        } catch (IllegalArgumentException e) { throw new IOException("Invalid document URI.", e); }
    }

    static void requireComplete(android.os.Bundle extras) throws IOException {
        if (extras == null) return;
        String error = extras.getString(DocumentsContract.EXTRA_ERROR);
        if (extras.getBoolean(DocumentsContract.EXTRA_LOADING, false)
                || (error != null && !error.isEmpty()))
            throw new IOException("Document query is still loading or failed.");
    }

    public void check() throws IOException { task.check(); }
    public void progress(String status) { progress.accept(status); }

    Uri uri(TreeExport.Document document) throws IOException {
        Uri uri = DocumentsContract.buildDocumentUriUsingTree(tree, document.id);
        requireScope(tree, uri);
        return uri;
    }

    private TreeExport.Document row(Cursor cursor) throws IOException {
        String id = TreeExport.metadataField(cursor.getString(cursor.getColumnIndexOrThrow(Document.COLUMN_DOCUMENT_ID)), TreeExport.MAX_ID);
        String name = TreeExport.metadataField(cursor.getString(cursor.getColumnIndexOrThrow(Document.COLUMN_DISPLAY_NAME)), TreeExport.MAX_NAME);
        String mime = TreeExport.metadataField(cursor.getString(cursor.getColumnIndexOrThrow(Document.COLUMN_MIME_TYPE)), TreeExport.MAX_MIME);
        return new TreeExport.Document(id, name, mime);
    }

    private TreeExport.Document metadata(Uri uri) throws Exception {
        requireScope(tree, uri); // Reject redirects before querying or opening anything.
        check();
        try (Cursor cursor = resolver.query(uri, COLUMNS, null, null, null, task.signal)) {
            if (cursor == null) throw new IOException("Cannot query created document.");
            task.track(cursor);
            requireComplete(cursor.getExtras());
            if (!cursor.moveToFirst()) throw new IOException("Created document is missing.");
            TreeExport.Document document = row(cursor);
            if (!document.id.equals(DocumentsContract.getDocumentId(uri)) || cursor.moveToNext())
                throw new IOException("Provider returned aliased document metadata.");
            requireComplete(cursor.getExtras());
            check();
            return document;
        } finally { synchronized (task.resources) { task.resources.clear(); } }
    }

    public TreeExport.Document parent() throws Exception {
        return metadata(DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree)));
    }

    public List<TreeExport.Document> children(TreeExport.Document parent) throws Exception {
        Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, parent.id);
        check();
        try (Cursor cursor = resolver.query(children, COLUMNS, null, null, null, task.signal)) {
            if (cursor == null) throw new IOException("Cannot enumerate destination; absence is not established.");
            task.track(cursor);
            requireComplete(cursor.getExtras());
            ArrayList<TreeExport.Document> result = new ArrayList<>();
            int metadataBytes = 0;
            while (cursor.moveToNext()) {
                check();
                if (result.size() == 4096) throw new IOException("Destination enumeration exceeds 4096 documents.");
                TreeExport.Document child = row(cursor);
                metadataBytes = TreeExport.metadataTotal(metadataBytes, child);
                result.add(child);
            }
            requireComplete(cursor.getExtras());
            check();
            return result;
        } finally { synchronized (task.resources) { task.resources.clear(); } }
    }

    public TreeExport.Document create(TreeExport.Document parent, String name, String mime) throws Exception {
        check();
        Uri created = DocumentsContract.createDocument(resolver, uri(parent), mime, name);
        if (created == null) throw new IOException("Provider did not return a created document URI.");
        requireScope(tree, created);
        check();
        return metadata(created);
    }

    public void write(TreeExport.Document file, byte[] packet, int offset, int length) throws Exception {
        check();
        try (AssetFileDescriptor descriptor = resolver.openAssetFileDescriptor(uri(file), "wt", task.signal)) {
            if (descriptor == null) throw new IOException("Cannot open new destination document.");
            task.track(descriptor);
            PeaZipActivity.writeAsset(descriptor, packet, offset, length, task);
        } finally { synchronized (task.resources) { task.resources.clear(); } }
        check();
    }
}
