public class MockActivity {
    public long lastId;
    public boolean archiveTransferAvailable() { return BridgeLifecycleCheck.available(); }
    public boolean pickArchiveInputs(long id) { lastId = id; return true; }
    public boolean saveArchive(long id, byte[] bytes) {
        lastId = id;
        if (bytes.length == 13) throw new IllegalStateException("Simulated provider exception");
        return true;
    }
}
