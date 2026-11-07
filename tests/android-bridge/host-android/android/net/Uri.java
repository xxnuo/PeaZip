package android.net;

import java.net.URI;
import java.net.URLDecoder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;

/** Host URI double; only URI policy is tested here, never Android provider behavior. */
public final class Uri {
    private final URI uri;
    private Uri(String value) { uri = URI.create(value); }
    public static Uri parse(String value) { return new Uri(value); }
    public String getScheme() { return uri.getScheme(); }
    public String getAuthority() { return uri.getRawAuthority(); }
    public String getQuery() { return uri.getRawQuery(); }
    public String getFragment() { return uri.getRawFragment(); }
    public List<String> getPathSegments() {
        ArrayList<String> result = new ArrayList<>();
        for (String part : uri.getRawPath().split("/")) if (!part.isEmpty())
            result.add(URLDecoder.decode(part.replace("+", "%2B"), StandardCharsets.UTF_8));
        return result;
    }
}
