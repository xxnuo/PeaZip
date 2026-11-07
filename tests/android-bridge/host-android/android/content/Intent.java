package android.content;

import java.util.HashMap;
import java.util.Map;

/** Host storage for production picker Intent assertions. */
public class Intent {
    private final String action;
    private String type;
    private final Map<String, Object> extras = new HashMap<>();
    public Intent(String action) { this.action = action; }
    public Intent addCategory(String category) { return this; }
    public Intent setType(String type) { this.type = type; return this; }
    public Intent putExtra(String key, String value) { extras.put(key, value); return this; }
    public Intent putExtra(String key, boolean value) { extras.put(key, value); return this; }
    public String getAction() { return action; }
    public String getType() { return type; }
    public String getStringExtra(String key) { return (String) extras.get(key); }
}
