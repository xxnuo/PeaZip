package android.provider;

import android.net.Uri;
import java.net.URLEncoder;
import java.nio.charset.StandardCharsets;
import java.util.List;

/** Minimal SDK URI-helper double. Does not implement or simulate a SAF provider. */
public final class DocumentsContract {
    public static boolean isTreeUri(Uri uri) {
        List<String> parts = uri.getPathSegments();
        return parts.size() >= 2 && parts.get(0).equals("tree");
    }
    public static String getTreeDocumentId(Uri uri) {
        if (!isTreeUri(uri)) throw new IllegalArgumentException();
        return uri.getPathSegments().get(1);
    }
    public static String getDocumentId(Uri uri) {
        List<String> parts = uri.getPathSegments();
        if (parts.size() >= 4 && parts.get(0).equals("tree") && parts.get(2).equals("document")) return parts.get(3);
        throw new IllegalArgumentException();
    }
    public static Uri buildDocumentUriUsingTree(Uri tree, String id) {
        return Uri.parse("content://" + tree.getAuthority() + "/tree/" + encode(getTreeDocumentId(tree)) + "/document/" + encode(id));
    }
    private static String encode(String value) { return URLEncoder.encode(value, StandardCharsets.UTF_8).replace("+", "%20"); }
}
