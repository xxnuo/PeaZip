"""Run with uv run python scripts/smoke_android_extract_options.py --serial SERIAL --apk APK."""

import hashlib
import re
import shutil
import sys
import uuid
import zipfile

import smoke_android


class ExtractOptionsSmoke(smoke_android.Smoke):
    FILES_ONLY = "Files only (no folders)"

    @staticmethod
    def snapshot(tree):
        nodes = [node for node in tree.iter("node") if node.get("package") == smoke_android.PACKAGE]
        return {
            "rows": sorted((node.get("content-desc"), node.get("checked"))
                           for node in nodes if node.get("checkable") == "true"),
            "selection": [node.get("text") for node in nodes
                          if re.fullmatch(r"\d+ selected", node.get("text", ""))],
        }

    def radio(self, label, checked, enabled=True):
        return self.wait(f"{label}: checked={checked}, enabled={enabled}", lambda tree: any(
            self.match(node, label) and node.get("package") == smoke_android.PACKAGE
            and node.get("checkable") == "true" and node.get("checked") == str(checked).lower()
            and node.get("enabled") == str(enabled).lower() for node in tree.iter("node")))

    def continue_state(self, enabled):
        return self.wait(f"Continue enabled={enabled}", lambda tree: any(
            self.match(node, "Continue") and node.get("class") == "android.widget.Button"
            and node.get("enabled") == str(enabled).lower() for node in tree.iter("node")))

    def unselected_rows(self, paths):
        self.label("0 selected")
        self.wait("current view has only the expected unselected rows", lambda tree:
                  self.snapshot(tree)["rows"] == sorted((path, "false") for path in paths))

    def dismissed(self):
        self.wait("extract dialog dismissed", lambda tree: not any(
            self.match(node, "Extract archive") for node in tree.iter("node")))

    def no_picker_marker(self, evidence):
        marker = f"extract-options-{self.token}-{evidence}"
        self.adb("shell", "log", "-p", "i", "-t", "PeaZipExtractOptions", marker)
        return marker

    def no_picker(self, marker, evidence):
        log = self.adb("logcat", "-d", "-v", "threadtime", "PeaZipExtractOptions:I",
                       "ActivityTaskManager:I", "ActivityManager:I", "*:S")
        (self.out / f"{evidence}-activities.log").write_text(log)
        if marker not in log:
            raise AssertionError("Missing activity marker; cannot prove no picker launch")
        launches = [line for line in log.split(marker, 1)[1].splitlines()
                    if "START" in line and re.search(
                        r"act=android\.intent\.action\.(?:OPEN_DOCUMENT|CREATE_DOCUMENT|OPEN_DOCUMENT_TREE)\b", line)]
        if launches:
            raise AssertionError(f"Unexpected document picker launch: {launches}")
        if any(node.get("package", "").endswith(".documentsui") for node in self.dump().iter("node")):
            raise AssertionError("DocumentsUI opened for a cancelled or rejected extraction")

    def preserved(self, name, before, evidence):
        self.label(name)
        self.wait("archive snapshot and selection preserved", lambda tree: self.snapshot(tree) == before)
        self.save(evidence)

    def fixture(self, stem):
        source = smoke_android.ROOT / f"crates/peazip-core/tests/fixtures/{stem}.zip"
        name = f"peazip-{self.token}-{stem}.zip"
        remote = f"/sdcard/Download/{name}"
        self.files.append(remote)
        self.adb("push", source, remote)
        self.scan(remote)
        with zipfile.ZipFile(source) as archive:
            contents = {info.filename: None if info.is_dir() else archive.read(info)
                        for info in archive.infolist()}
        self.entries = list(contents)
        self.choose(name)
        self.archive(name)
        return name, contents

    def authorize(self, expected_count, contents, before, evidence):
        self.tap("Continue")
        self.documents()
        self.wait("real SAF tree picker", lambda tree: any(
            node.get("text", "").lower() == "use this folder" for node in tree.iter("node")))
        self.save(f"{evidence}-tree-picker")
        self.export_destination(f"{evidence}-parent-picker")
        tree = self.wait("extraction result", lambda tree: any(
            node.get("text", "").startswith(("Extracted ", "Could not extract archive:"))
            for node in tree.iter("node")))
        report = next(node.get("text") for node in tree.iter("node")
                      if node.get("text", "").startswith(("Extracted ", "Could not extract archive:")))
        match = re.fullmatch(rf"Extracted {expected_count} entries to (PeaZip-([0-9a-fA-F-]{{36}}))", report)
        if not match:
            raise AssertionError(f"Unexpected result: {report}")
        uuid.UUID(match[2])
        folder = match[1]
        if folder in before:
            raise AssertionError("Extraction reused an existing export directory")
        expected = {**before, folder: None}
        for name, data in contents.items():
            path = f"{folder}/{name.rstrip('/')}"
            expected[path] = data
            for at, _ in enumerate(name.rstrip("/").split("/")[:-1], 1):
                expected[f"{folder}/{'/'.join(name.split('/')[:at])}"] = None
        actual = self.export_snapshot(f"{evidence}-bytes")
        if actual != expected:
            raise AssertionError(f"Extracted paths/bytes or previous outputs differ: expected={list(expected)}, actual={list(actual)}")
        self.save(evidence)
        self.health()
        print(f"PASS {evidence}: {report}; every exported byte verified, previous outputs preserved", flush=True)
        return actual

    def run(self):
        if not self.args.apk or not self.args.apk.is_file():
            raise RuntimeError("Pass --apk pointing to the final extraction-options APK")
        self.adb("get-state")
        if self.adb("emu", "avd", "name").replace("\r", "").splitlines()[0] != "PeaZip_API36":
            raise RuntimeError("Run only on the dedicated PeaZip_API36 emulator")
        if self.adb("shell", "getprop", "ro.build.version.sdk").strip() != "36":
            raise RuntimeError("Extraction smoke requires API 36")
        apk = self.out / "tested.apk"
        shutil.copyfile(self.args.apk.resolve(), apk)
        (self.out / "apk.sha256").write_text(f"{hashlib.sha256(apk.read_bytes()).hexdigest()}  {apk.name}\n")
        print(f"Extract-options artifacts: {self.out}", flush=True)
        self.adb("install", "-r", str(apk.resolve()))
        self.adb("shell", "am", "force-stop", smoke_android.PACKAGE)
        self.started = self.adb("shell", "date", "'+%m-%d %H:%M:%S.000'").strip()
        self.adb("shell", "am", "start", "-n", f"{smoke_android.PACKAGE}/com.longbridge.nsh.PeaZipActivity")
        self.label("Open archive…")
        self.flat_view()
        self.health()
        self.adb("shell", "mkdir", "-p", "/sdcard/Documents")
        self.adb("shell", "mkdir", self.export_parent)
        self.export_owned = True
        sentinel = self.out / "sentinel.bin"
        sentinel.write_bytes(b"PeaZip extraction options sentinel\0\xff\n")
        self.adb("push", sentinel, f"{self.export_parent}/sentinel.bin")
        exports = self.export_snapshot("initial")
        if exports != {"sentinel.bin": sentinel.read_bytes()}:
            raise AssertionError("Initial export sentinel differs")
        name, contents = self.fixture("extract-tree")
        before = self.snapshot(self.dump())
        for dismissal in ("Cancel", "Back"):
            marker = self.no_picker_marker(dismissal)
            self.extract_dialog(paths=None)
            self.radio("Displayed entries", True)
            self.radio("All entries", False)
            self.radio("Selected entries", False, enabled=False)
            if dismissal == "Cancel":
                self.tap("Cancel")
            else:
                self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
            self.dismissed()
            self.no_picker(marker, dismissal.lower())
            self.preserved(name, before, f"cancel-{dismissal.lower()}")
            if self.export_snapshot(f"cancel-{dismissal.lower()}") != exports:
                raise AssertionError("Modal cancellation changed previous exports")
            print(f"PASS Extract {dismissal}: no picker, snapshot/selection/output preserved", flush=True)

        self.extract_dialog(scope="All entries")
        self.radio("All entries", True)
        exports = self.authorize(len(contents), contents, exports, "all-full")
        self.preserved(name, before, "all-keeps-selection-empty")

        self.tap("docs/中文.txt")
        self.label("1 selected")
        selected = self.snapshot(self.dump())
        self.extract_dialog()
        self.radio("Selected entries", True)
        exports = self.authorize(1, {"docs/中文.txt": contents["docs/中文.txt"]}, exports, "selected-full")
        self.preserved(name, selected, "selected-keeps-snapshot")

        self.extract_dialog(scope="All entries", paths=self.FILES_ONLY)
        self.radio("All entries", True)
        flat = {path.rsplit("/", 1)[-1]: data for path, data in contents.items() if data is not None}
        exports = self.authorize(len(flat), flat, exports, "all-files-only")
        self.preserved(name, selected, "files-only-keeps-selection")

        self.tap("Folder view")
        self.tap("Open folder docs/")
        self.tap("Clear selection")
        self.unselected_rows(["docs/deep/", "docs/empty/", "docs/中文.txt"])
        docs_before = self.snapshot(self.dump())
        self.extract_dialog()
        self.label("Current view: /docs/")
        self.radio("Displayed entries", True)
        self.radio("Selected entries", False, enabled=False)
        self.radio("All entries", False)
        docs = {path: data for path, data in contents.items()
                if path.startswith("docs/") and path != "docs/"}
        exports = self.authorize(len(docs), docs, exports, "displayed-docs-full")
        self.preserved(name, docs_before, "displayed-docs-keeps-selection-empty")

        self.tap("Open folder docs/empty/")
        self.unselected_rows([])
        empty_before = self.snapshot(self.dump())
        marker = self.no_picker_marker("empty-displayed")
        self.extract_dialog()
        self.label("Current view: /docs/empty/")
        self.radio("Displayed entries", True, enabled=False)
        self.radio("Selected entries", False, enabled=False)
        self.radio("All entries", False)
        self.continue_state(False)
        self.save("empty-displayed-disabled")
        self.tap("Cancel")
        self.dismissed()
        self.preserved(name, empty_before, "empty-displayed-cancelled")
        self.extract_dialog()
        self.radio("Displayed entries", True, enabled=False)
        self.continue_state(False)
        self.tap("All entries")
        self.radio("All entries", True)
        self.continue_state(True)
        self.save("empty-folder-explicit-all-enabled")
        self.tap("Cancel")
        self.dismissed()
        self.no_picker(marker, "empty-displayed-no-picker")
        self.preserved(name, empty_before, "empty-explicit-all-cancelled")
        if self.export_snapshot("empty-displayed-no-output") != exports:
            raise AssertionError("An empty displayed scope changed previous exports")
        print("PASS empty folder: default Displayed disables Continue; Cancel preserves state; explicit All enables Continue", flush=True)

        # The remembered folder is empty; flat Displayed must nevertheless cover the archive.
        self.flat_view()
        self.unselected_rows(contents)
        flat_before = self.snapshot(self.dump())
        self.extract_dialog()
        self.label("All entries view")
        self.radio("Displayed entries", True)
        self.radio("All entries", False)
        exports = self.authorize(len(contents), contents, exports, "displayed-flat-full")
        self.preserved(name, flat_before, "displayed-flat-keeps-selection-empty")

        name, contents = self.fixture("extract-collisions")
        self.tap("a/")
        self.tap("b/")
        self.label("2 selected")
        selected = self.snapshot(self.dump())
        marker = self.no_picker_marker("collision")
        self.extract_dialog(paths=self.FILES_ONLY)
        self.radio("Selected entries", True)
        self.tap("Continue")
        tree = self.wait("basename collision rejection", lambda tree: any(
            node.get("text", "").startswith("Could not extract archive:") for node in tree.iter("node")))
        message = next(node.get("text") for node in tree.iter("node")
                       if node.get("text", "").startswith("Could not extract archive:"))
        if "duplicate or conflicting archive path" not in message:
            raise AssertionError(f"Unexpected collision failure: {message}")
        self.no_picker(marker, "collision-rejected")
        self.preserved(name, selected, "collision-snapshot-preserved")
        if self.export_snapshot("collision-no-output") != exports:
            raise AssertionError("Collision rejection changed exports or left partial files")
        print("PASS FilesOnly collision: rejected before SAF; snapshot, selection and exports unchanged", flush=True)
        self.extract_dialog(paths="Full paths")
        self.radio("Selected entries", True)
        chosen = {path: data for path, data in contents.items() if path.startswith(("a/", "b/"))}
        self.authorize(len(chosen), chosen, exports, "collision-full-retry")
        self.preserved(name, selected, "full-retry-keeps-selection")
        self.health()
        print("PASS Extract options: modal Cancel/Back no picker; default Displayed without selection; "
              "docs Displayed exports only actual descendants; empty Displayed disables Continue with safe Cancel/explicit All; "
              "flat Displayed covers the archive; explicit All and default Selected; Full paths and FilesOnly actual SAF bytes including Unicode/empty files; "
              "FilesOnly omits directories; basename collision before SAF with no partial output; Full paths retry succeeds", flush=True)


if __name__ == "__main__":
    smoke_android.Smoke = ExtractOptionsSmoke
    sys.exit(smoke_android.main())
