"""Verify archive navigation and SAF on the dedicated Android smoke device."""

from pathlib import Path
import sys
import re
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import smoke_android
from smoke_android_menus import MenuSmoke


class NavigationSmoke(smoke_android.Smoke):
    def run(self):
        if not self.args.apk or not self.args.apk.is_file():
            raise RuntimeError("Pass --apk pointing to the rebuilt final APK")
        self.adb("get-state")
        self.adb("install", "-r", str(self.args.apk.resolve()))
        self.adb("shell", "am", "force-stop", smoke_android.PACKAGE)
        self.started = self.adb("shell", "date", "'+%m-%d %H:%M:%S.000'").strip()
        tree = self.dump()
        for _ in range(4):
            if not any(n.get("package", "").endswith(".documentsui") for n in tree.iter("node")):
                break
            self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
            tree = self.dump()
        self.adb("shell", "am", "start", "-n",
                 f"{smoke_android.PACKAGE}/com.longbridge.nsh.PeaZipActivity")
        self.label("Open archive…")
        self.health()
        sdk = self.adb("shell", "getprop", "ro.build.version.sdk").strip()
        if sdk != "36":
            raise RuntimeError(f"Extraction smoke requires Android 36, got {sdk}")
        self.adb("shell", "mkdir", "-p", "/sdcard/Documents")
        # Only a successful exclusive mkdir grants cleanup ownership.
        self.adb("shell", "mkdir", self.export_parent)
        self.export_owned = True
        sentinel = self.out / f"extract-{self.token}-sentinel.bin"
        sentinel.write_bytes(b"PeaZip export parent sentinel\x00\xff\n")
        self.adb("push", sentinel, f"{self.export_parent}/sentinel.bin")
        if self.export_snapshot("before") != {"sentinel.bin": sentinel.read_bytes()}:
            raise RuntimeError("Initial export sentinel differs")
        fixture = smoke_android.ROOT / "tests/fixtures/sample.tar"
        contents = self.tar_contents(fixture)
        self.entries = list(contents)
        name = f"peazip-{self.token}-navigation.tar"
        remote = f"/sdcard/Download/{name}"
        self.files.append(remote)
        self.adb("push", fixture, remote)
        self.scan(remote)
        self.choose(name)
        self.wait_archive_ready(name)
        tree = self.wait("archive view toggle", lambda t: any(
            self.match(n, "All entries") or self.match(n, "Folder view") for n in t.iter("node")))
        if any(self.match(n, "Folder view") for n in tree.iter("node")):
            self.tap("Folder view")
        self.label("All entries")
        self.navigation_rows({"docs/": False, "other.txt": False})
        self.label("0 selected")
        self.column_visibility(name, contents, remote)
        self.save("navigation-root")
        self.tap("Open folder docs/")
        docs = {"docs/empty/": False, "docs/nested/": False, "docs/你好.txt": False}
        self.navigation_rows(docs)
        self.tap("Open folder docs/nested/")
        self.navigation_rows({"docs/nested/data.bin": False})
        self.tap("Up")
        self.navigation_rows(docs)
        self.tap("Back")
        self.navigation_rows({"docs/nested/data.bin": False})
        self.tap("Forward")
        self.navigation_rows(docs)
        self.save("navigation-history")
        # TAR has no explicit docs/nested/ entry: selecting it must select its real descendant.
        self.set_checkbox("docs/nested/", True)
        self.label("1 selected")
        self.tap("Open folder docs/nested/")
        self.navigation_rows({"docs/nested/data.bin": True})
        self.tap("Archive root")
        self.navigation_rows({"docs/": False, "other.txt": False})
        self.label("1 selected")
        self.go_path("docs/nested/")
        self.navigation_rows({"docs/nested/data.bin": True})
        self.save("navigation-path-selected")
        baseline = self.export_snapshot("navigation-before")
        self.tree_picker("navigation-extract-cancel-picker")
        self.cancel_documents()
        self.navigation_rows({"docs/nested/data.bin": True})
        self.label("1 selected")
        self.label("Extraction cancelled.")
        if self.export_snapshot("navigation-cancelled") != baseline:
            raise RuntimeError("Navigation extraction cancellation changed previous output")
        self.tree_picker("navigation-extract-picker")
        self.export_destination("navigation-extract-parent")
        tree = self.wait("navigation extraction result", lambda t: any(
            n.get("text", "").startswith(("Extracted ", "Could not extract archive:"))
            for n in t.iter("node")))
        report = next(n.get("text") for n in tree.iter("node") if
                      n.get("text", "").startswith(("Extracted ", "Could not extract archive:")))
        match = re.fullmatch(r"Extracted 1 entries to (PeaZip-([0-9a-fA-F-]{36}))", report)
        if not match:
            raise RuntimeError(f"Unexpected implicit-directory extraction: {report}")
        uuid.UUID(match[2])
        folder = match[1]
        if folder in baseline:
            raise RuntimeError("Navigation extraction reused an existing directory")
        expected = {**baseline, folder: None, f"{folder}/docs": None,
                    f"{folder}/docs/nested": None,
                    f"{folder}/docs/nested/data.bin": contents["docs/nested/data.bin"]}
        if self.export_snapshot("navigation-success") != expected:
            raise RuntimeError("Implicit-directory output differs; unrelated files or previous output changed")
        self.navigation_rows({"docs/nested/data.bin": True})
        self.label("1 selected")
        self.save("navigation-extracted")
        # View preference persists across opening archives, while navigation/selection resets.
        self.choose(name)
        self.wait_archive_ready(name)
        self.label("All entries")
        self.navigation_rows({"docs/": False, "other.txt": False})
        self.set_checkbox("docs/", True)
        self.label("1 selected")
        self.navigation_rows({"docs/": True, "other.txt": False})
        self.tap("Open folder docs/")
        self.navigation_rows({"docs/empty/": True, "docs/nested/": True, "docs/你好.txt": True})
        self.tap("Open folder docs/nested/")
        self.navigation_rows({"docs/nested/data.bin": True})
        self.label("1 selected")
        self.set_checkbox("docs/nested/data.bin", False)
        self.navigation_rows({"docs/nested/data.bin": False})
        self.label("2 selected")
        self.tap("Up")
        self.navigation_rows({"docs/empty/": True, "docs/nested/": False, "docs/你好.txt": True})
        self.tap("Archive root")
        self.navigation_rows({"docs/": False, "other.txt": False})
        self.label("2 selected")
        self.save("navigation-parent-child-excluded")
        baseline = self.export_snapshot("navigation-excluded-before")
        self.tree_picker("navigation-excluded-picker")
        self.export_destination("navigation-excluded-parent")
        tree = self.wait("excluded-child extraction result", lambda t: any(
            n.get("text", "").startswith(("Extracted ", "Could not extract archive:"))
            for n in t.iter("node")))
        report = next(n.get("text") for n in tree.iter("node") if
                      n.get("text", "").startswith(("Extracted ", "Could not extract archive:")))
        match = re.fullmatch(r"Extracted 2 entries to (PeaZip-([0-9a-fA-F-]{36}))", report)
        if not match:
            raise RuntimeError(f"Unexpected parent-minus-child extraction: {report}")
        uuid.UUID(match[2])
        folder = match[1]
        if folder in baseline:
            raise RuntimeError("Excluded-child extraction reused an existing directory")
        expected = {**baseline, folder: None, f"{folder}/docs": None,
                    f"{folder}/docs/empty": None,
                    f"{folder}/docs/你好.txt": contents["docs/你好.txt"]}
        if self.export_snapshot("navigation-excluded-success") != expected:
            raise RuntimeError("Deselected child was exported or remaining output/sentinel changed")
        self.navigation_rows({"docs/": False, "other.txt": False})
        self.label("2 selected")
        self.save("navigation-parent-child-extracted")
        self.choose(name)
        self.wait_archive_ready(name)
        self.navigation_rows({"docs/": False, "other.txt": False})
        self.flat_view()
        self.navigation_rows(dict.fromkeys(self.entries, False))
        self.choose(name)
        self.wait_archive_ready(name)
        self.label("Folder view")
        self.navigation_rows(dict.fromkeys(self.entries, False))
        self.save("navigation-flat-persisted")
        self.tap("Folder view")
        self.check_path_submit()
        self.auto_adjust_columns(name, contents, remote)
        self.health()
        print("PASS folder rows/history/path; implicit selection; SAF cancellation/extraction; "
              "parent selection checks descendants; deselected child absent from real extraction; "
              "unselected files absent, sentinel preserved; view preference persists", flush=True)

    def auto_adjust_columns(self, name, contents, remote):
        self.tap("Archive root")
        self.navigation_rows({"docs/": False, "other.txt": False})
        self.set_checkbox("other.txt", True)
        self.label("1 selected")
        before = self.dump()
        headers = [(node.get("text"), node.get("bounds")) for node in before.iter("node")
                   if node.get("text") in ("Name", "Size (bytes)")]
        self.tap("Layout")
        tree = self.wait("Layout Auto adjust columns", lambda tree: bool(MenuSmoke.menu_items(tree, "Auto adjust columns")))
        items = MenuSmoke.menu_items(tree, "Auto adjust columns")
        if len(items) != 1 or items[0].get("enabled") != "true" or items[0].get("clickable") != "true":
            raise AssertionError("Auto adjust columns is not an enabled real menu item")
        self.save("navigation-auto-adjust-menu")
        x1, y1, x2, y2 = MenuSmoke.bounds(items[0])
        self.adb("shell", "input", "tap", (x1 + x2) // 2, (y1 + y2) // 2)
        self.wait("Layout dismissed", lambda tree: not MenuSmoke.menu_items(tree))
        self.navigation_rows({"docs/": False, "other.txt": True})
        self.label("1 selected")
        self.label(name)
        after = self.dump()
        assert [(node.get("text"), node.get("bounds")) for node in after.iter("node")
                if node.get("text") in ("Name", "Size (bytes)")] == headers, "Narrow columns changed after Auto adjust"
        assert not any(node.get("content-desc", "").startswith("Resize ") for node in after.iter("node"))
        row = next(node for node in after.iter("node") if node.get("class") == "android.widget.CheckBox"
                   and node.get("content-desc") == "other.txt")
        assert any(node.get("content-desc") == f"Size in bytes: {len(contents['other.txt'])}"
                   for node in row.iter("node"))
        assert self.adb("exec-out", "cat", remote, binary=True) == (smoke_android.ROOT / "tests/fixtures/sample.tar").read_bytes()
        self.save("navigation-auto-adjust-narrow-preserved")
        print("PASS narrow Layout Auto adjust columns: real menu tap; Name/Size bounds, selection, raw size label and source bytes preserved", flush=True)

    def layout_command(self, label, checked=None, evidence=None):
        self.tap("Layout")
        tree = self.wait(label, lambda tree: bool(MenuSmoke.menu_items(tree, label)))
        items = MenuSmoke.menu_items(tree, label)
        assert len(items) == 1 and items[0].get("enabled") == "true" and items[0].get("clickable") == "true"
        if checked is not None:
            assert items[0].get("checked") == str(checked).lower(), (label, items[0].attrib)
        if evidence:
            self.save(evidence)
        x1, y1, x2, y2 = MenuSmoke.bounds(items[0])
        self.adb("shell", "input", "tap", (x1 + x2) // 2, (y1 + y2) // 2)
        self.wait("Layout dismissed", lambda tree: not MenuSmoke.menu_items(tree))

    def visibility_state(self, name, contents, remote, size, packed, evidence):
        self.navigation_rows({"docs/": False, "other.txt": True})
        self.label(name)
        self.label("1 selected")
        tree = self.dump()
        for label, shown in (("Size (bytes)", size), ("Packed (bytes)", packed), ("Type", False)):
            assert any(self.match(node, label) for node in tree.iter("node")) == shown, (label, shown)
        row = next(node for node in tree.iter("node") if node.get("class") == "android.widget.CheckBox"
                   and node.get("content-desc") == "other.txt")
        assert any(node.get("content-desc") == f"Size in bytes: {len(contents['other.txt'])}"
                   for node in row.iter("node")) == size
        assert any(node.get("content-desc") == "Packed bytes: 512" for node in row.iter("node")) == packed
        assert any(node.get("text") == "other.txt" for node in row.iter("node"))
        assert self.adb("exec-out", "cat", remote, binary=True) == (smoke_android.ROOT / "tests/fixtures/sample.tar").read_bytes()
        self.save(evidence)

    def column_visibility(self, name, contents, remote):
        self.set_checkbox("other.txt", True)
        self.layout_command("Sort by size")
        self.layout_command("Show size column", checked=True, evidence="visibility-size-menu-360")
        self.layout_command("Show packed column", checked=True, evidence="visibility-packed-menu-360")
        self.visibility_state(name, contents, remote, False, False, "visibility-name-only-360")
        self.layout_command("Show packed column", checked=False)
        self.visibility_state(name, contents, remote, False, True, "visibility-packed-only-360")
        self.tap("Layout")
        tree = self.wait("Layout checked state", lambda tree: bool(MenuSmoke.menu_items(tree, "Show packed column")))
        for label, checked in (("Show size column", False), ("Show packed column", True), ("Sort by size", True), ("Sort by packed size", False)):
            item = MenuSmoke.menu_items(tree, label)
            assert len(item) == 1 and item[0].get("checked") == str(checked).lower(), label
        self.save("visibility-packed-sort-preserved-menu-360")
        self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
        self.wait("Layout dismissed", lambda tree: not MenuSmoke.menu_items(tree))
        self.layout_command("Show size column", checked=False)
        self.visibility_state(name, contents, remote, True, False, "visibility-both-size-view-360")
        self.layout_command("Sort by packed size")
        self.visibility_state(name, contents, remote, False, True, "visibility-both-packed-view-360")
        self.layout_command("Sort by name")
        self.visibility_state(name, contents, remote, True, False, "visibility-both-restored-360")
        self.set_checkbox("other.txt", False)
        self.label("0 selected")
        print("PASS 360-wide column visibility: real menu checked states; Name-only/Packed-only/Both; full size/packed labels, sort preference, selection and source bytes preserved", flush=True)

    def set_checkbox(self, path, checked):
        tree = self.dump()
        node = next(n for n in tree.iter("node") if n.get("checkable") == "true"
                    and n.get("package") == smoke_android.PACKAGE and self.match(n, path))
        if node.get("checked") == str(checked).lower():
            raise AssertionError(f"Expected a real checkbox transition for {path}, already {checked}")
        bounds = re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", node.get("bounds", ""))
        if not bounds or node.get("enabled") != "true" or node.get("clickable") != "true":
            raise RuntimeError(f"Invalid checkbox target: {node.attrib}")
        x1, y1, x2, y2 = map(int, bounds.groups())
        if x2 <= x1 or y2 <= y1:
            raise RuntimeError(f"Empty checkbox bounds for {path}")
        evidence = f"selection-{path.replace('/', '-')}-{checked}"
        (self.out / f"{evidence}-before.xml").write_text(self.xml)
        with (self.out / "selection-clicks.log").open("a") as log:
            log.write(f"{path}: {node.get('checked')} -> {checked}; bounds={node.get('bounds')}; "
                      f"single tap=({(x1 + x2) // 2}, {(y1 + y2) // 2})\n")
        self.adb("shell", "input", "tap", (x1 + x2) // 2, (y1 + y2) // 2)
        self.dump()
        (self.out / f"{evidence}-after.xml").write_text(self.xml)
        self.wait(f"single click changes {path} to checked={checked}", lambda t: any(
            n.get("checkable") == "true" and n.get("package") == smoke_android.PACKAGE
            and self.match(n, path) and n.get("checked") == str(checked).lower()
            for n in t.iter("node")))

    def wait_archive_ready(self, filename):
        def ready(tree):
            nodes = list(tree.iter("node"))
            if not all(any(self.match(n, label) for n in nodes)
                       for label in (filename, f"{len(self.entries)} entries", "0 selected")):
                return False
            parents = {child: parent for parent in tree.iter() for child in parent}
            for node in nodes:
                if self.match(node, "Open archive…"):
                    while node.get("class") != "android.widget.Button" and node in parents:
                        node = parents[node]
                    return node.get("enabled") == "true"
            return False
        self.wait("archive load completed, idle and selection reset", ready)
        self.path_unfocused("reopened")

    def navigation_rows(self, expected):
        self.wait("immediate children and selection", lambda t: {
            n.get("content-desc"): n.get("checked") == "true" for n in t.iter("node")
            if n.get("class") == "android.widget.CheckBox"
            and n.get("checkable") == "true" and n.get("package") == smoke_android.PACKAGE
        } == expected)

    def path_fields(self, tree):
        return [child for wrapper in tree.iter("node")
                if self.match(wrapper, "Archive path")
                for child in wrapper.iter("node")
                if child.get("class") == "android.widget.EditText"]

    def path_unfocused(self, evidence, expected_path=None):
        def valid(tree):
            fields = self.path_fields(tree)
            return bool(fields) and all(
                field.get("focused") == "false" and
                (expected_path is None or field.get("text") == expected_path)
                for field in fields)
        self.wait(f"Archive path unfocused after {evidence}", valid)
        (self.out / f"navigation-path-unfocused-{evidence}.xml").write_text(self.xml)

    def check_path_submit(self):
        self.go_path("docs/nested/", enter=True)
        self.navigation_rows({"docs/nested/data.bin": False})
        self.save("navigation-enter-valid")
        self.go_path("missing/", enter=True, valid=False)
        self.label("Archive directory not found")
        self.wait("invalid path remains editable", lambda t: any(
            n.get("class") == "android.widget.EditText" and n.get("focused") == "true"
            for n in t.iter("node")))
        self.save("navigation-enter-invalid")
        self.adb("shell", "input", "keyevent", "KEYCODE_BACK")

    def go_path(self, path, enter=False, valid=True):
        tree = self.wait("Archive path input", lambda t: bool(self.path_fields(t)))
        field = self.path_fields(tree)[0]
        bounds = re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", field.get("bounds", ""))
        if not bounds or field.get("enabled") == "false":
            raise RuntimeError(f"Invalid Archive path input: {field.attrib}")
        x1, y1, x2, y2 = map(int, bounds.groups())
        if x2 <= x1 or y2 <= y1:
            raise RuntimeError("Empty Archive path input bounds")
        self.adb("shell", "input", "tap", (x1 + x2) // 2, (y1 + y2) // 2)
        self.adb("shell", "input", "keyevent", "KEYCODE_MOVE_END")
        if field.get("text"):
            self.adb("shell", "input", "keyevent", *(["KEYCODE_DEL"] * len(field.get("text", ""))))
        self.adb("shell", "input", "text", path)
        if enter:
            self.adb("shell", "input", "keyevent", "KEYCODE_ENTER")
        else:
            self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
            self.tap("Go")
        if valid:
            self.path_unfocused("go", expected_path=path)


if __name__ == "__main__":
    # Reuse the original CLI, final evidence/health checks and owned-file cleanup.
    smoke_android.Smoke = NavigationSmoke
    sys.exit(smoke_android.main())
