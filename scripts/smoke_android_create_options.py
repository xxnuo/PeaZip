"""Run with uv run python scripts/smoke_android_create_options.py --serial SERIAL --apk APK."""

import hashlib
import json
import re
import shutil
import struct
import sys
import zipfile
import zlib

import smoke_android


class CreateOptionsSmoke(smoke_android.Smoke):
    STORE = "Store (no compression)"

    @staticmethod
    def snapshot(tree):
        nodes = [n for n in tree.iter("node") if n.get("package") == smoke_android.PACKAGE]
        return {
            "rows": sorted((n.get("content-desc"), n.get("checked"))
                           for n in nodes if n.get("checkable") == "true"),
            "selection": [n.get("text") for n in nodes
                          if re.fullmatch(r"\d+ selected", n.get("text", ""))],
        }

    @staticmethod
    def level_fields(tree):
        return [field for wrapper in tree.iter("node")
                if smoke_android.Smoke.match(wrapper, "Compression level")
                for field in wrapper.iter("node")
                if field.get("class") == "android.widget.EditText"]

    def tap_node(self, node):
        bounds = re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", node.get("bounds", ""))
        if not bounds or node.get("enabled") == "false":
            raise AssertionError(f"Unusable input: {node.attrib}")
        x1, y1, x2, y2 = map(int, bounds.groups())
        if x2 <= x1 or y2 <= y1:
            raise AssertionError(f"Empty input bounds: {node.attrib}")
        self.adb("shell", "input", "tap", (x1 + x2) // 2, (y1 + y2) // 2)

    def replace_field(self, field, value):
        self.tap_node(field)
        self.adb("shell", "input", "keyevent", "KEYCODE_MOVE_END")
        if field.get("text"):
            self.adb("shell", "input", "keyevent", *(["KEYCODE_DEL"] * len(field.get("text"))))
        self.adb("shell", "input", "text", value)

    def set_level(self, level):
        tree = self.wait("Compression level native input", lambda t: bool(self.level_fields(t)))
        self.replace_field(self.level_fields(tree)[0], str(level))
        self.wait(f"Compression level becomes {level}", lambda t: any(
            field.get("text") == str(level) for field in self.level_fields(t)))

    def modal(self, compression, level=None):
        self.tap("Create ZIP…")
        self.label("Create archive")
        self.tap(compression)
        if compression == "Deflate":
            self.label("Compression level")
            if level is not None:
                self.set_level(level)

    def no_picker_marker(self, evidence):
        marker = f"create-options-{self.token}-{evidence}"
        self.adb("shell", "log", "-p", "i", "-t", "PeaZipCreateOptions", marker)
        return marker

    def assert_no_picker(self, marker, evidence):
        log = self.adb("logcat", "-d", "-v", "threadtime", "PeaZipCreateOptions:I",
                       "ActivityTaskManager:I", "ActivityManager:I", "*:S")
        (self.out / f"{evidence}-activities.log").write_text(log)
        if marker not in log:
            raise AssertionError("Activity log marker was lost; cannot prove no picker launch")
        launches = [line for line in log.split(marker, 1)[1].splitlines()
                    if "START" in line and re.search(
                        r"act=android\.intent\.action\.(?:OPEN_DOCUMENT|CREATE_DOCUMENT|OPEN_DOCUMENT_TREE)\b", line)]
        if launches:
            raise AssertionError(f"Modal action launched a document picker: {launches}")
        if any(n.get("package", "").endswith(".documentsui") for n in self.dump().iter("node")):
            raise AssertionError("DocumentsUI is visible after a modal-only action")

    def preserved(self, name, before, evidence):
        self.label(name)
        self.wait("old snapshot and selection preserved", lambda t: self.snapshot(t) == before)
        self.save(evidence)

    def ime_shown(self, evidence):
        state = self.adb("shell", "dumpsys", "input_method")
        (self.out / f"{evidence}-ime.log").write_text(state)
        if not re.search(r"\bmInputShown=true\b", state):
            raise AssertionError("Expected the real numeric keyboard to remain open")
        self.save(f"{evidence}-with-ime")

    def pick_input(self, name, evidence):
        if self.level_fields(self.dump()):
            self.ime_shown(evidence)
        self.tap("Continue")
        self.documents()
        self.tap("Search")
        self.wait("focused DocumentsUI search", lambda t: any(
            n.get("focused") == "true" and n.get("class") in
            ("android.widget.EditText", "android.widget.AutoCompleteTextView") for n in t.iter("node")))
        self.adb("shell", "input", "text", name)
        self.adb("shell", "input", "keyevent", "KEYCODE_ENTER")
        self.tap(name, "android:id/title", long=True)
        self.save(f"{evidence}-input-selected")
        self.button(("Open", "OPEN", "Select", "SELECT"))

    @staticmethod
    def verify_zip(path, contents, method):
        raw = path.read_bytes()
        result = {}
        with zipfile.ZipFile(path) as archive:
            if set(archive.namelist()) != set(contents) or len(archive.infolist()) != len(contents):
                raise AssertionError(f"ZIP names differ: {archive.namelist()}")
            if archive.testzip() is not None:
                raise AssertionError("Saved ZIP CRC failure")
            for info in archive.infolist():
                if archive.read(info) != contents[info.filename]:
                    raise AssertionError(f"Saved bytes differ: {info.filename}")
                if method is not None and info.compress_type != method:
                    raise AssertionError(f"Actual compression method {info.compress_type}, expected {method}")
                name_size, extra_size = struct.unpack_from("<HH", raw, info.header_offset + 26)
                start = info.header_offset + 30 + name_size + extra_size
                packed = raw[start:start + info.compress_size]
                if info.compress_type == zipfile.ZIP_DEFLATED:
                    if zlib.decompress(packed, -15) != contents[info.filename]:
                        raise AssertionError("Raw DEFLATE stream differs")
                elif info.compress_type == zipfile.ZIP_STORED and packed != contents[info.filename]:
                    raise AssertionError("Stored data differs")
                result[info.filename] = {"method": info.compress_type, "size": info.file_size,
                                         "compressed_size": info.compress_size, "crc32": f"{info.CRC:08x}",
                                         "packed_sha256": hashlib.sha256(packed).hexdigest()}
        return result

    def save_zip(self, evidence, contents, method=None, expected_bytes=None):
        requested = f"peazip-{self.token}-{evidence}.zip"
        self.tap("Save ZIP…")
        self.documents()
        self.downloads()
        tree = self.wait("save filename", lambda t: any(
            n.get("class") == "android.widget.EditText" for n in t.iter("node")))
        self.replace_field(next(n for n in tree.iter("node") if n.get("class") == "android.widget.EditText"), requested)
        self.button(("Save", "SAVE"))
        tree = self.wait("saved ZIP result", lambda t: any(
            n.get("text", "").startswith(("Saved archive: ", "Could not save archive:")) or
            self.match(n, "Replace") for n in t.iter("node")))
        saved = next((n.get("text")[len("Saved archive: "):] for n in tree.iter("node")
                      if n.get("text", "").startswith("Saved archive: ")), None)
        if not saved or not re.fullmatch(rf"{re.escape(requested[:-4])}(?: \(\d+\))?\.zip", saved):
            raise AssertionError(f"Save failed or asked to replace a file: {self.xml}")
        remote = f"/sdcard/Download/{saved}"
        self.files.append(remote)
        local = self.out / saved
        self.adb("pull", remote, local)
        if expected_bytes is not None and local.read_bytes() != expected_bytes:
            raise AssertionError("Modal cancellation or creation failure changed the old archive bytes")
        report = self.verify_zip(local, contents, method)
        (self.out / f"{evidence}-zip.json").write_text(json.dumps(report, ensure_ascii=False, indent=2))
        self.save(evidence)
        print(f"PASS saved {evidence}: {report}", flush=True)
        return local, report

    def run(self):
        if not self.args.apk or not self.args.apk.is_file():
            raise RuntimeError("Pass --apk pointing to the final creation-options APK")
        self.adb("get-state")
        if "PeaZip_API36" not in self.adb("emu", "avd", "name"):
            raise RuntimeError("Run only on the dedicated PeaZip_API36 emulator")
        if self.adb("shell", "getprop", "ro.build.version.sdk").strip() != "36":
            raise RuntimeError("Creation-options smoke requires API 36")
        apk = self.out / "tested.apk"
        shutil.copyfile(self.args.apk.resolve(), apk)
        (self.out / "apk.sha256").write_text(f"{hashlib.sha256(apk.read_bytes()).hexdigest()}  {apk.name}\n")
        print(f"Create-options artifacts: {self.out}", flush=True)
        self.adb("install", "-r", str(apk.resolve()))
        self.adb("shell", "am", "force-stop", smoke_android.PACKAGE)
        self.started = self.adb("shell", "date", "'+%m-%d %H:%M:%S.000'").strip()
        self.adb("shell", "am", "start", "-n", f"{smoke_android.PACKAGE}/com.longbridge.nsh.PeaZipActivity")
        self.label("Open archive…")
        self.flat_view()
        fixture = smoke_android.ROOT / "web/tests/fixtures/sample.zip"
        with zipfile.ZipFile(fixture) as archive:
            baseline_contents = {info.filename: archive.read(info) for info in archive.infolist()}
        baseline_name = f"peazip-{self.token}-baseline.zip"
        payload_name = f"peazip-{self.token}-payload.bin"
        bomb_name = f"peazip-{self.token}-zeros.bin"
        sentence = b"The quick brown fox jumps over the lazy dog. Archive compression level test.\n"
        data = b"".join(sentence * 16 + hashlib.sha256(n.to_bytes(4, "little")).digest() for n in range(512))
        sources = {baseline_name: fixture.read_bytes(), payload_name: data, bomb_name: bytes(1024 * 1024)}
        for name, body in sources.items():
            local = self.out / name
            local.write_bytes(body)
            remote = f"/sdcard/Download/{name}"
            self.files.append(remote)
            self.adb("push", local, remote)
            self.scan(remote)
        self.entries = list(baseline_contents)
        self.choose(baseline_name)
        self.archive(baseline_name)
        self.tap("Select all")
        before = self.snapshot(self.dump())
        for dismissal in ("Cancel", "Back"):
            inventory = self.adb("shell", "ls", "-laR", "/sdcard/Download")
            marker = self.no_picker_marker(dismissal)
            self.modal("Deflate", 1)
            self.ime_shown(f"modal-{dismissal.lower()}")
            self.save(f"modal-before-{dismissal.lower()}")
            if dismissal == "Back":
                self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
            else:
                self.tap("Cancel")
            self.wait("creation modal dismissed", lambda t: not any(
                self.match(n, "Create archive") for n in t.iter("node")))
            self.assert_no_picker(marker, f"modal-{dismissal.lower()}")
            self.preserved(baseline_name, before, f"modal-{dismissal.lower()}")
            if self.adb("shell", "ls", "-laR", "/sdcard/Download") != inventory:
                raise AssertionError("Modal cancellation changed Downloads")
            print(f"PASS modal {dismissal}: no picker; snapshot, selection and Downloads unchanged", flush=True)
        self.save_zip("cancel-preserved", baseline_contents, expected_bytes=fixture.read_bytes())

        self.modal(self.STORE)
        self.pick_input(payload_name, "stored")
        self.entries = [payload_name]
        self.archive("Archive.zip", status="Created ZIP: 1 entries", directory=False)
        self.save_zip("stored", {payload_name: data}, zipfile.ZIP_STORED)

        marker = self.no_picker_marker("invalid-level")
        self.modal("Deflate", 0)
        self.label("Enter a Deflate level from 1 to 9.")
        tree = self.label("Continue")
        if not any(self.match(n, "Continue") and n.get("enabled") == "false" for n in tree.iter("node")):
            raise AssertionError("Invalid level did not disable Continue")
        self.assert_no_picker(marker, "invalid-level")
        self.save("invalid-level")
        self.set_level(1)
        self.pick_input(payload_name, "deflate-1")
        self.archive("Archive.zip", status="Created ZIP: 1 entries", directory=False)
        _, fast = self.save_zip("deflate-1", {payload_name: data}, zipfile.ZIP_DEFLATED)
        self.modal("Deflate", 9)
        self.pick_input(payload_name, "deflate-9")
        self.archive("Archive.zip", status="Created ZIP: 1 entries", directory=False)
        old_zip, best = self.save_zip("deflate-9", {payload_name: data}, zipfile.ZIP_DEFLATED)
        if best[payload_name]["compressed_size"] >= fast[payload_name]["compressed_size"]:
            raise AssertionError(f"Selected levels had no expected effect: level 1={fast}; level 9={best}")

        self.tap("Select all")
        before = self.snapshot(self.dump())
        self.modal("Deflate", 9)
        self.pick_input(bomb_name, "ratio-error")
        tree = self.wait("strict Deflate ratio error", lambda t: any(
            n.get("text", "").startswith("Could not create ZIP:") for n in t.iter("node")))
        error = next(n.get("text") for n in tree.iter("node") if n.get("text", "").startswith("Could not create ZIP:"))
        if "compression ratio" not in error:
            raise AssertionError(f"Unexpected creation error: {error}")
        self.preserved("Archive.zip", before, "ratio-error-preserved")
        self.save_zip("ratio-error-old-snapshot", {payload_name: data}, zipfile.ZIP_DEFLATED,
                      expected_bytes=old_zip.read_bytes())
        self.modal(self.STORE)
        self.pick_input(bomb_name, "stored-retry")
        self.entries = [bomb_name]
        self.archive("Archive.zip", status="Created ZIP: 1 entries", directory=False)
        self.save_zip("stored-retry", {bomb_name: sources[bomb_name]}, zipfile.ZIP_STORED)
        self.health()
        print("PASS Create options: Cancel/Back never launch picker and preserve saved snapshot bytes; "
              "invalid level disabled then corrected; real Store/Deflate 1/9 bytes, methods and CRCs; "
              "strict ratio error preserves old snapshot, Store retry succeeds; "
              "Continue and Cancel were tapped with the real numeric IME open.", flush=True)


if __name__ == "__main__":
    smoke_android.Smoke = CreateOptionsSmoke
    sys.exit(smoke_android.main())
