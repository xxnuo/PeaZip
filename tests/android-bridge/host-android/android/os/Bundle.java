package android.os;

import java.util.HashMap;

/** Host double for cursor query-status policy; not an Android Binder implementation. */
public final class Bundle {
    private final HashMap<String, Object> values = new HashMap<>();
    public void putString(String key, String value) { values.put(key, value); }
    public void putBoolean(String key, boolean value) { values.put(key, value); }
    public String getString(String key) { return (String) values.get(key); }
    public boolean getBoolean(String key, boolean fallback) {
        Object value = values.get(key);
        return value == null ? fallback : (Boolean) value;
    }
}
