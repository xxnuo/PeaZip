"""Verify real Android menu sheets; run only after the APK and AVD are ready."""

import re
import sys

import smoke_android


class MenuSmoke(smoke_android.Smoke):
    @staticmethod
    def menu_items(tree, label=None):
        # Menu items are actionable (or disabled); the named menu-bar Group is neither.
        return [n for n in tree.iter("node") if n.get("package") == smoke_android.PACKAGE
                and n.get("class") == "android.view.View" and n.get("content-desc")
                and (n.get("clickable") == "true" or n.get("enabled") == "false")
                and (label is None or n.get("content-desc") == label)]

    def menu_open(self, group, first):
        self.tap(group)
        return self.wait(f"{group} menu sheet", lambda t: bool(self.menu_items(t, first)))

    def menu_item(self, label, enabled=True):
        tree = self.wait(f"menu item {label}", lambda t: bool(self.menu_items(t, label)))
        nodes = self.menu_items(tree, label)
        if len(nodes) != 1 or nodes[0].get("enabled") != str(enabled).lower():
            raise AssertionError(f"Unexpected menu item state: {label}: {[n.attrib for n in nodes]}")
        return nodes[0]

    def menu_select(self, group, label):
        self.menu_open(group, label)
        node = self.menu_item(label)
        if node.get("clickable") != "true":
            raise AssertionError(f"Menu item is not clickable: {label}")
        x1, y1, x2, y2 = self.bounds(node)
        self.adb("shell", "input", "tap", (x1 + x2) // 2, (y1 + y2) // 2)
        self.menu_closed()

    @staticmethod
    def bounds(node):
        match = re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", node.get("bounds", ""))
        if not match:
            raise AssertionError(f"Invalid menu bounds: {node.attrib}")
        x1, y1, x2, y2 = map(int, match.groups())
        if x2 <= x1 or y2 <= y1:
            raise AssertionError(f"Empty menu bounds: {node.attrib}")
        return x1, y1, x2, y2

    def menu_closed(self):
        self.wait("menu sheet dismissed", lambda t: not self.menu_items(t))

    def picker(self):
        self.menu_select("File", "Open archive…")
        self.documents()

    def rows(self, expected):
        self.wait("menu action preserves visible rows and selection", lambda t: {
            n.get("content-desc"): n.get("checked") == "true" for n in t.iter("node")
            if n.get("package") == smoke_android.PACKAGE and n.get("checkable") == "true"
        } == expected)

    def path_fields(self, tree):
        return [field for wrapper in tree.iter("node")
                if wrapper.get("package") == smoke_android.PACKAGE
                and self.match(wrapper, "Archive path")
                for field in wrapper.iter("node")
                if field.get("class") == "android.widget.EditText"]

    def state(self):
        return [(n.get("text"), n.get("content-desc"), n.get("checked"))
                for n in self.dump().iter("node") if n.get("package") == smoke_android.PACKAGE
                and (n.get("text") or n.get("content-desc"))]

    def run(self):
        if not self.args.apk or not self.args.apk.is_file():
            raise RuntimeError("Pass --apk pointing to the rebuilt menu APK")
        self.adb("get-state")
        self.adb("install", "-r", str(self.args.apk.resolve()))
        self.adb("shell", "am", "force-stop", smoke_android.PACKAGE)
        self.started = self.adb("shell", "date", "'+%m-%d %H:%M:%S.000'").strip()
        self.adb("shell", "am", "start", "-n",
                 f"{smoke_android.PACKAGE}/com.longbridge.nsh.PeaZipActivity")
        self.label("Open archive…")
        self.health()
        self.rows({})
        self.menu_open("File", "Close archive")
        self.menu_item("Close archive", enabled=False)
        self.menu_item("Save archive…", enabled=False)
        self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
        self.menu_closed()
        self.menu_open("Tools", "Test archive")
        self.menu_item("Test archive", enabled=False)
        self.menu_item("Extract selected…", enabled=False)
        self.save("menus-empty-disabled")
        self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
        self.menu_closed()

        fixture = smoke_android.ROOT / "tests/fixtures/sample.tar"
        contents = self.tar_contents(fixture)
        self.entries = list(contents)
        name = f"peazip-{self.token}-menus.tar"
        remote = f"/sdcard/Download/{name}"
        self.files.append(remote)
        self.adb("push", fixture, remote)
        self.scan(remote)
        self.choose(name)
        self.label(name)
        self.label(f"{len(contents)} entries")
        self.label("0 selected")
        tree = self.dump()
        if any(self.match(n, "Folder view") for n in tree.iter("node")):
            self.tap("Folder view")
        self.rows({"docs/": False, "other.txt": False})
        self.tap("Open folder docs/")
        visible = {"docs/empty/": False, "docs/nested/": False, "docs/你好.txt": False}
        self.rows(visible)
        self.menu_select("Browser", "Archive root")
        self.rows({"docs/": False, "other.txt": False})
        self.menu_select("Browser", "Back")
        self.rows(visible)
        self.menu_select("Browser", "Archive root")
        self.rows({"docs/": False, "other.txt": False})
        self.menu_select("Browser", "Go to path")
        tree = self.wait("menu Go to path focuses input after sheet dismissal", lambda t: any(
            field.get("focused") == "true" for field in self.path_fields(t)))
        field = next(field for field in self.path_fields(tree) if field.get("focused") == "true")
        (self.out / "menus-path-focused.xml").write_text(self.xml)
        # Do not tap the field: focus must come from the menu command itself.
        self.adb("shell", "input", "keyevent", "KEYCODE_MOVE_END")
        if field.get("text"):
            self.adb("shell", "input", "keyevent", *(["KEYCODE_DEL"] * len(field.get("text", ""))))
        self.adb("shell", "input", "text", "docs/")
        self.wait("menu-focused path receives input", lambda t: any(
            field.get("focused") == "true" and field.get("text") == "docs/"
            for field in self.path_fields(t)))
        self.adb("shell", "input", "keyevent", "KEYCODE_ENTER")
        self.rows(visible)
        self.wait("valid path Enter ends editing", lambda t: any(
            field.get("focused") == "false" and field.get("text") == "docs/"
            for field in self.path_fields(t)))
        self.label("0 selected")
        self.save("menus-path-enter-blurred")
        self.menu_select("Edit", "Select all")
        self.rows(dict.fromkeys(visible, True))
        self.label("3 selected")
        self.menu_select("Edit", "Clear selection")
        self.rows(visible)
        self.label("0 selected")
        self.tap("docs/你好.txt")
        visible["docs/你好.txt"] = True
        self.rows(visible)
        self.label("1 selected")
        self.test_archive(name, contents, 0, "menus-tested",
                          test_action=lambda: self.menu_select("Tools", "Test archive"))

        self.menu_open("Layout", "All entries")
        if self.menu_item("All entries").get("checked") != "false":
            raise AssertionError("Folder view must leave All entries unchecked")
        self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
        self.menu_closed()
        self.menu_select("Layout", "All entries")
        self.rows({path: path == "docs/你好.txt" for path in contents})
        self.menu_open("Layout", "All entries")
        if self.menu_item("All entries").get("checked") != "true":
            raise AssertionError("Flat view must check All entries")
        self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
        self.menu_closed()
        self.menu_select("Layout", "All entries")
        self.rows(visible)
        self.menu_select("Layout", "Sort by size")
        self.rows(visible)
        self.menu_select("Layout", "Sort by name")
        self.rows(visible)
        self.label("1 selected")

        before = self.state()
        self.menu_open("File", "Close archive")
        self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
        self.menu_closed()
        if self.state() != before:
            raise AssertionError("Back dismissal changed archive state")
        tree = self.menu_open("File", "Close archive")
        items = self.menu_items(tree)
        left, top, right, _ = self.bounds(items[0])
        top = min(self.bounds(n)[1] for n in items)
        if top <= 100:
            raise AssertionError("No safe backdrop coordinate above menu sheet")
        self.adb("shell", "input", "tap", (left + right) // 2, top - 100)
        self.menu_closed()
        if self.state() != before:
            raise AssertionError("Outside dismissal changed archive state")

        downloads = self.adb("shell", "ls", "-laR", "/sdcard/Download")
        self.menu_select("File", "Save archive…")
        self.documents()
        self.label("Archive.tar")
        activities = self.adb("shell", "dumpsys", "activity", "activities")
        if not any("act=android.intent.action.CREATE_DOCUMENT" in line and
                   "typ=application/x-tar" in line for line in activities.splitlines()):
            raise AssertionError("Menu Save did not launch the TAR document intent")
        self.save("menus-save-picker")
        self.cancel_documents()
        self.label("Save cancelled.")
        self.label(name)
        self.label("1 selected")
        self.rows(visible)
        if self.adb("shell", "ls", "-laR", "/sdcard/Download") != downloads:
            raise AssertionError("Menu Save cancellation changed Downloads")
        if self.adb("exec-out", "cat", remote, binary=True) != fixture.read_bytes():
            raise AssertionError("Menu actions changed archive bytes")

        self.menu_select("File", "Close archive")
        self.label("Open an archive to view its contents.")
        self.label("0 selected")
        self.rows({})
        self.menu_open("File", "Close archive")
        self.menu_item("Close archive", enabled=False)
        self.menu_item("Save archive…", enabled=False)
        self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
        self.menu_closed()
        self.menu_open("Tools", "Test archive")
        self.menu_item("Test archive", enabled=False)
        self.menu_item("Extract selected…", enabled=False)
        self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
        self.menu_closed()
        before = self.state()
        self.menu_select("Help", "About PeaZip")
        self.label("About PeaZip")
        self.save("menus-about")
        self.tap("Close")
        self.wait("About dismissed", lambda t: not any(
            self.match(n, "About PeaZip") for n in t.iter("node")))
        if self.state() != before:
            raise AssertionError("About dismissal changed the empty archive state")
        self.save("menus-complete")
        self.health()
        print("PASS Android menu sheets: disabled states, File Open/Save cancel/Close, "
              "Browser navigation/path focus/Enter blur, Edit selection, Layout view/sort, Tools read-only Test, "
              "Back/outside dismissal, About", flush=True)


if __name__ == "__main__":
    smoke_android.Smoke = MenuSmoke
    sys.exit(smoke_android.main())
