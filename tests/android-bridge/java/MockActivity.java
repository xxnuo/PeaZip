public class MockActivity {
    public long lastId;
    public int opcode;
    public int format;
    public byte[] packet;
    public boolean failExport;
    public boolean archiveTransferAvailable() { return BridgeLifecycleCheck.available(); }
    public boolean pickArchiveInputs(long id) { opcode = 0; lastId = id; return true; }
    public boolean saveArchive(long id, byte[] bytes, int format) {
        if (format != 0 && format != 1) throw new IllegalArgumentException("Invalid format");
        this.format = format; packet = bytes;
        opcode = 1; lastId = id;
        if (bytes.length == 13) throw new IllegalStateException("Simulated provider exception");
        return true;
    }
    public boolean exportEntries(long id, byte[] bytes) {
        opcode = 2; lastId = id; packet = bytes;
        if (failExport) throw new IllegalStateException("Simulated export exception");
        return true;
    }
}
