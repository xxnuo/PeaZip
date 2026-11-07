"""Run with uv run python scripts/smoke_android.py --serial emulator-5554."""

import argparse
import os
from pathlib import Path
import re
import shutil
import shlex
import struct
import subprocess
import sys
import tarfile
import time
import uuid
import xml.etree.ElementTree as ET
import zipfile

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = "io.peazip.rust"


class Smoke:
    def __init__(self, args):
        self.args = args
        self.token = uuid.uuid4().hex[:12]
        self.out = ROOT / "target/android-smoke" / self.token
        self.out.mkdir(parents=True, exist_ok=True)
        self.remote_xml = f"/sdcard/peazip-smoke-{self.token}.xml"
        self.files = []
        self.xml = ""
        self.pids = set()
        self.started = None
        self.export_parent = f"/sdcard/Documents/peazip-{self.token}-exports"
        self.export_owned = False

    def adb(self, *args, check=True, binary=False):
        result = subprocess.run([self.args.adb, "-s", self.args.serial, *map(str, args)],
                                capture_output=True, timeout=self.args.timeout,
                                text=not binary)
        with (self.out / f"adb-{self.token}.log").open("a") as log:
            log.write(f"{time.strftime('%Y-%m-%d %H:%M:%S')} {shlex.join(list(map(str, args)))} "
                      f"exit={result.returncode}\n")
            if not binary and args[:2] != ("exec-out", "cat"):
                log.write(result.stdout + result.stderr)
        if check and result.returncode:
            raise RuntimeError(f"adb {args}: {result.stderr!r} {result.stdout!r}")
        return result.stdout

    def dump(self):
        self.adb("shell", "uiautomator", "dump", self.remote_xml)
        self.xml = self.adb("exec-out", "cat", self.remote_xml)
        return ET.fromstring(self.xml)

    @staticmethod
    def match(node, label):
        return label in (node.get("text"), node.get("content-desc"))

    def wait(self, description, predicate):
        deadline = time.monotonic() + self.args.timeout
        last = ""
        while time.monotonic() < deadline:
            try:
                tree = self.dump()
                if predicate(tree):
                    return tree
            except (RuntimeError, ET.ParseError, subprocess.TimeoutExpired) as error:
                last = str(error)
            time.sleep(0.3)
        labels = [(n.get("text"), n.get("content-desc")) for n in ET.fromstring(self.xml).iter("node")] if self.xml else []
        raise RuntimeError(f"Timeout: {description}; last error={last}; visible labels={labels}")

    def label(self, label):
        return self.wait(label, lambda t: any(self.match(n, label) for n in t.iter("node")))

    def tap(self, label, resource=None, long=False):
        tree = self.wait(label, lambda t: any(self.match(n, label) and
                         (resource is None or n.get("resource-id") == resource)
                         for n in t.iter("node")))
        parents = {child: parent for parent in tree.iter() for child in parent}
        node = next(n for n in tree.iter("node") if self.match(n, label)
                    and (resource is None or n.get("resource-id") == resource))
        while node.get("clickable") != "true" and node in parents:
            node = parents[node]
        if node.get("enabled") == "false" or node.get("clickable") != "true":
            raise RuntimeError(f"Not an enabled clickable node: {label}")
        bounds = re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", node.get("bounds", ""))
        if not bounds:
            raise RuntimeError(f"Invalid bounds for {label}: {node.attrib}")
        x1, y1, x2, y2 = map(int, bounds.groups())
        if x2 <= x1 or y2 <= y1:
            raise RuntimeError(f"Empty bounds: {label}")
        x, y = (x1 + x2) // 2, (y1 + y2) // 2
        if long:
            self.adb("shell", "input", "swipe", x, y, x, y, 800)
        else:
            self.adb("shell", "input", "tap", x, y)

    def picker(self):
        self.tap("Open archive…")
        self.wait("system DocumentsUI picker", lambda t: any(
            n.get("package", "").endswith(".documentsui") for n in t.iter("node")))

    def choose(self, filename):
        self.picker()
        self.tap("Search")
        self.wait("picker search field", lambda t: any(
            n.get("class") in ("android.widget.EditText", "android.widget.AutoCompleteTextView") and n.get("focused") == "true"
            for n in t.iter("node")))
        self.adb("shell", "input", "text", filename.split(" ", 1)[0])
        self.adb("shell", "input", "keyevent", "KEYCODE_ENTER")
        self.tap(filename, "android:id/title")

    def archive(self, filename, selected=0, status=None, directory=True):
        self.label(filename)
        self.label(status or f"{len(self.entries)} entries")
        self.label(f"{selected} selected")
        def valid(tree):
            rows = [n for n in tree.iter("node") if n.get("checkable") == "true"
                    and n.get("package") == PACKAGE]
            return (set(n.get("content-desc") for n in rows) == set(self.entries)
                    and len(rows) == len(self.entries)
                    and all(n.get("checked") == str(bool(selected)).lower() for n in rows)
                    and (not directory or any(self.match(n, "—") for n in tree.iter("node"))))
        self.wait("entry names, empty directory marker and checkbox states", valid)

    def documents(self):
        return self.wait("system DocumentsUI", lambda t: any(
            n.get("package", "").endswith(".documentsui") for n in t.iter("node")))

    def cancel_documents(self):
        # Back first navigates up when DocumentsUI restores a nested directory.
        for _ in range(12):
            tree = self.dump()
            if not any(n.get("package", "").endswith(".documentsui")
                       for n in tree.iter("node")):
                self.wait("return to archive", lambda t: any(
                    n.get("package") == PACKAGE for n in t.iter("node")))
                return
            self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
        raise RuntimeError("DocumentsUI did not close after bounded Back navigation")

    def button(self, labels):
        tree = self.wait(" / ".join(labels), lambda t: any(
            any(self.match(n, label) for label in labels) and n.get("enabled") == "true"
            for n in t.iter("node")))
        self.tap(next(label for label in labels if any(
            self.match(n, label) and n.get("enabled") == "true" for n in tree.iter("node"))))

    def downloads(self):
        self.tap("Show roots")
        self.tap("Downloads", "android:id/title")

    def scan(self, remote):
        self.adb("shell", "am", "broadcast", "-a", "android.intent.action.MEDIA_SCANNER_SCAN_FILE",
                 "-d", shlex.quote(f"file://{remote}"))

    def create_inputs(self, query, names, evidence):
        self.tap("Create ZIP…")
        self.documents()
        self.tap("Search")
        self.wait("focused search", lambda t: any(n.get("focused") == "true" and
                  n.get("class") in ("android.widget.EditText", "android.widget.AutoCompleteTextView")
                  for n in t.iter("node")))
        self.adb("shell", "input", "text", query)
        self.adb("shell", "input", "keyevent", "KEYCODE_ENTER")
        for index, name in enumerate(names):
            self.tap(name, "android:id/title", long=index == 0)
        self.save(evidence)
        self.button(("Open", "OPEN", "Select", "SELECT"))

    def create_oversized(self, sample):
        name = f"peazip-{self.token}-oversized.bin"
        local = self.out / name
        with local.open("wb") as stream:
            stream.truncate(17 * 1024 * 1024)
        remote = f"/sdcard/Download/{name}"
        self.files.append(remote)
        self.adb("push", local, remote)
        self.scan(remote)
        self.tap("Select all")
        self.archive(sample, len(self.entries))
        self.create_inputs(name, [name], "create-oversized-selected")
        tree = self.wait("metadata preflight rejection", lambda t: any(
            n.get("text", "").startswith("Could not create ZIP: Input metadata exceeds")
            for n in t.iter("node")))
        error = next(n.get("text") for n in tree.iter("node") if
                     n.get("text", "").startswith("Could not create ZIP: Input metadata exceeds"))
        self.archive(sample, len(self.entries), status=error)
        self.save("create-oversized-rejected")
        self.health()
        print(f"PASS 17 MiB metadata rejection preserves archive and selection: {error}", flush=True)

    def create_save(self):
        prefix = f"peazip-{self.token}-source-"
        contents = {f"{prefix}ascii.txt": b"PeaZip smoke\n",
                    f"{prefix}中文.bin": bytes(range(256)),
                    f"{prefix}empty.txt": b""}
        for name, data in contents.items():
            local = self.out / name
            local.write_bytes(data)
            remote = f"/sdcard/Download/{name}"
            self.files.append(remote)
            self.adb("push", local, remote)
            self.scan(remote)
        self.create_inputs(prefix, contents, "create-multiselected")
        self.entries = list(contents)
        self.archive("Archive.zip", status=f"Created ZIP: {len(contents)} entries", directory=False)
        self.save("created")
        self.tap("Select all")
        before = self.adb("shell", "ls", "-1", "/sdcard/Download")
        self.tap("Save ZIP…")
        self.documents()
        self.label("Archive.zip")
        self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
        self.archive("Archive.zip", len(contents), "Save cancelled.", directory=False)
        after = self.adb("shell", "ls", "-1", "/sdcard/Download")
        if before != after:
            raise RuntimeError("Save cancellation changed Downloads contents")
        self.save("save-cancelled")
        print("PASS Save cancelled: entries and selection preserved; no Downloads output", flush=True)
        output_name = f"peazip-{self.token}-saved.zip"
        sentinel_remote = f"/sdcard/Download/{output_name}"
        sentinel = self.out / output_name
        sentinel.write_bytes(b"PeaZip smoke sentinel: must not overwrite\n")
        self.files.append(sentinel_remote)
        self.adb("push", sentinel, sentinel_remote)
        self.scan(sentinel_remote)
        self.tap("Save ZIP…")
        self.documents()
        self.downloads()
        tree = self.wait("save filename", lambda t: any(n.get("class") == "android.widget.EditText"
                         for n in t.iter("node")))
        field = next(n for n in tree.iter("node") if n.get("class") == "android.widget.EditText")
        self.tap(field.get("text", ""), field.get("resource-id"))
        self.adb("shell", "input", "keyevent", "KEYCODE_MOVE_END")
        self.adb("shell", "input", "keyevent", *(["KEYCODE_DEL"] * len(field.get("text", ""))))
        self.adb("shell", "input", "text", output_name)
        self.label(output_name)
        self.button(("Save", "SAVE"))
        tree = self.wait("save result (never confirm Replace)", lambda t: any(
            n.get("text", "").startswith("Saved archive: ") or self.match(n, "Replace")
            or n.get("text", "").startswith("Could not save archive:") for n in t.iter("node")))
        saved = next((n.get("text")[len("Saved archive: "):] for n in tree.iter("node")
                      if n.get("text", "").startswith("Saved archive: ")), None)
        if not saved:
            raise RuntimeError(f"Save did not complete; no Replace confirmation: {self.xml}")
        if not re.fullmatch(rf"peazip-{self.token}-saved(?: \(\d+\))?\.zip", saved):
            raise RuntimeError(f"Unexpected saved display name: {saved!r}")
        remote = f"/sdcard/Download/{saved}"
        if remote != sentinel_remote:
            self.files.append(remote)
        if self.adb("exec-out", "cat", shlex.quote(sentinel_remote), binary=True) != sentinel.read_bytes():
            raise RuntimeError("Save silently overwrote the same-name sentinel")
        if saved == output_name:
            raise RuntimeError("Save reused the sentinel name")
        self.archive("Archive.zip", len(contents), f"Saved archive: {saved}", directory=False)
        local = self.out / saved
        self.adb("pull", remote, local)
        with zipfile.ZipFile(local) as archive:
            if len(archive.infolist()) != len(contents) or set(archive.namelist()) != set(contents):
                raise RuntimeError(f"Saved ZIP names differ: {archive.namelist()}")
            if archive.testzip() is not None:
                raise RuntimeError("Saved ZIP CRC failure")
            for name, data in contents.items():
                if archive.read(name) != data:
                    raise RuntimeError(f"Saved ZIP content differs: {name}")
        self.save("saved")
        self.scan(remote)
        self.choose(saved)
        self.archive(saved, directory=False)
        self.health()
        print(f"PASS Create / Save / reimport: {saved}; names, content and CRC verified; same-name sentinel preserved", flush=True)

    def save(self, name):
        self.dump()
        (self.out / f"{name}.xml").write_text(self.xml)
        (self.out / f"{name}.png").write_bytes(self.adb("exec-out", "screencap", "-p", binary=True))

    def extraction_button(self, enabled):
        def valid(tree):
            parents = {child: parent for parent in tree.iter() for child in parent}
            for node in tree.iter("node"):
                if self.match(node, "Extract selected…"):
                    while node.get("class") != "android.widget.Button" and node in parents:
                        node = parents[node]
                    return node.get("enabled") == str(enabled).lower()
            return False
        self.wait(f"Extract selected enabled={enabled}", valid)

    def extraction_state(self, filename, status=None, selected_entry="tree/"):
        self.label(filename)
        self.label("1 selected")
        if status:
            self.label(status)
        self.wait(f"only {selected_entry} selected; archive snapshot preserved", lambda t: {
            n.get("content-desc"): n.get("checked") for n in t.iter("node")
            if n.get("checkable") == "true" and n.get("package") == PACKAGE
        } == {name: str(name == selected_entry).lower() for name in self.entries})
        self.extraction_button(True)

    def export_snapshot(self, name):
        local = self.out / f"extract-{self.token}-{name}"
        self.adb("pull", self.export_parent, local)
        return {p.relative_to(local).as_posix(): None if p.is_dir() else p.read_bytes()
                for p in local.rglob("*")}

    def tree_picker(self, evidence="extract-tree-picker"):
        self.tap("Extract selected…")
        self.documents()
        self.save(evidence)
        self.wait("tree picker folder action", lambda t: any(
            n.get("text", "").lower() == "use this folder" for n in t.iter("node")))

    def export_destination(self, evidence="extract-parent-picker"):
        model = self.adb("shell", "getprop", "ro.product.model").strip()
        tree = self.documents()
        if any(self.match(n, "Show roots") for n in tree.iter("node")):
            self.tap("Show roots")
            tree = self.wait("internal storage root", lambda t: any(
                n.get("resource-id") == "android:id/title" and
                n.get("text") in (model, "Internal storage", "Internal shared storage")
                for n in t.iter("node")))
            title = next(n.get("text") for n in tree.iter("node") if
                         n.get("resource-id") == "android:id/title" and
                         n.get("text") in (model, "Internal storage", "Internal shared storage"))
            self.tap(title, "android:id/title")
        elif not any(self.match(n, "Documents") and n.get("resource-id") == "android:id/title"
                     for n in tree.iter("node")):
            breadcrumb = next((n for n in tree.iter("node") if self.match(n, model)
                               and n.get("resource-id", "").endswith(":id/breadcrumb_text")), None)
            if breadcrumb is None:
                raise RuntimeError("Tree picker has no internal storage navigation")
            self.tap(model, breadcrumb.get("resource-id"))
        self.tap("Documents", "android:id/title")
        self.tap(Path(self.export_parent).name, "android:id/title")
        self.save(evidence)
        self.button(("Use this folder", "USE THIS FOLDER"))
        self.button(("Allow", "ALLOW"))

    def extract_selected(self):
        sdk = self.adb("shell", "getprop", "ro.build.version.sdk").strip()
        if sdk != "36":
            raise RuntimeError(f"Extraction smoke requires Android 36, got {sdk}")
        payload = b"PeaZip selected nested payload\x00\xff\n"
        text_payload = b"PeaZip plain text MIME smoke\n"
        contents = {"tree/": b"", "tree/child.bin": payload,
                    "tree/note.txt": text_payload, "tree/empty/": b"",
                    "other.txt": b"must not be exported\n"}
        good = self.out / f"peazip-{self.token}-nested.zip"
        with zipfile.ZipFile(good, "w", compression=zipfile.ZIP_STORED) as archive:
            for name, data in contents.items():
                archive.writestr(name, data)
        bad = self.out / f"peazip-{self.token}-crc.zip"
        damaged = bytearray(good.read_bytes())
        with zipfile.ZipFile(good) as archive:
            offset = archive.getinfo("tree/child.bin").header_offset
        name_len, extra_len = struct.unpack_from("<HH", damaged, offset + 26)
        damaged[offset + 30 + name_len + extra_len] ^= 1
        bad.write_bytes(damaged)
        with zipfile.ZipFile(bad) as archive:
            assert archive.namelist() == list(contents)
            assert archive.testzip() == "tree/child.bin"
        for local in (good, bad):
            remote = f"/sdcard/Download/{local.name}"
            self.files.append(remote)
            self.adb("push", local, remote)
            self.scan(remote)
        # mkdir must fail on a collision, so cleanup can only own a new directory.
        self.adb("shell", "mkdir", "-p", "/sdcard/Documents")
        self.adb("shell", "mkdir", self.export_parent)
        self.export_owned = True
        sentinel = self.out / f"extract-{self.token}-sentinel.bin"
        sentinel.write_bytes(b"PeaZip export parent sentinel\x00\xff\n")
        self.adb("push", sentinel, f"{self.export_parent}/sentinel.bin")
        baseline = self.export_snapshot("before")
        assert baseline == {"sentinel.bin": sentinel.read_bytes()}
        self.entries = list(contents)
        self.choose(good.name)
        self.archive(good.name)
        self.extraction_button(False)
        self.tap("tree/")
        self.extraction_state(good.name)
        self.tree_picker()
        self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
        self.extraction_state(good.name, "Extraction cancelled.")
        if self.export_snapshot("cancelled") != baseline:
            raise RuntimeError("Tree picker cancellation changed the export parent")
        self.save("extract-cancelled")
        print("PASS real tree picker cancellation preserves snapshot/selection; no output", flush=True)
        self.choose(bad.name)
        self.archive(bad.name)
        self.tap("tree/")
        self.tap("Extract selected…")
        def rejected(tree):
            if any(n.get("package", "").endswith(".documentsui") for n in tree.iter("node")):
                raise AssertionError("CRC-invalid payload opened DocumentsUI before rejection")
            return any(n.get("text", "").startswith("Could not extract archive:")
                       for n in tree.iter("node"))
        tree = self.wait("CRC rejection before tree picker", rejected)
        error = next(n.get("text") for n in tree.iter("node")
                     if n.get("text", "").startswith("Could not extract archive:"))
        self.extraction_state(bad.name, error)
        if self.export_snapshot("crc-rejected") != baseline:
            raise RuntimeError("CRC rejection wrote output")
        self.save("extract-crc-rejected")
        print(f"PASS selected CRC-invalid payload rejected before picker: {error}", flush=True)
        self.choose(good.name)
        self.archive(good.name)
        self.tap("tree/")
        self.tree_picker()
        self.export_destination()
        tree = self.wait("extraction report", lambda t: any(
            n.get("text", "").startswith("Extracted ") or
            n.get("text", "").startswith("Could not extract archive:") for n in t.iter("node")))
        report = next(n.get("text") for n in tree.iter("node") if
                      n.get("text", "").startswith(("Extracted ", "Could not extract archive:")))
        match = re.fullmatch(r"Extracted 4 entries to (PeaZip-([0-9a-fA-F-]{36}))", report)
        if not match:
            raise RuntimeError(f"Unexpected extraction report: {report}")
        uuid.UUID(match[2])
        folder = match[1]
        expected = {**baseline, folder: None, f"{folder}/tree": None,
                    f"{folder}/tree/child.bin": payload,
                    f"{folder}/tree/note.txt": text_payload, f"{folder}/tree/empty": None}
        if self.export_snapshot("success") != expected:
            raise RuntimeError("Extracted files/directories/content or sentinel differ from expected")
        self.save("extracted")
        self.health()
        (self.out / f"extract-{self.token}-result.log").write_text(
            f"Android SDK: {sdk}\nSerial: {self.args.serial}\nParent: {self.export_parent}\n"
            f"{report}\nVerified via adb pull: selected binary and .txt bytes, empty directory, "
            "unselected file absent, sentinel unchanged.\n"
            "Real DocumentsUI tree cancellation: snapshot/selection preserved, no output.\n"
            f"CRC preflight rejection before picker: {error}\n"
            "Recovery: reopened valid ZIP and extracted successfully.\n")
        print(f"PASS Android {sdk} real SAF: {report}; nested bytes/empty directory verified; "
              "unselected other.txt absent; sentinel unchanged; recovery after CRC rejection", flush=True)

    @staticmethod
    def tar_contents(path):
        contents = {}
        with tarfile.open(path, "r:") as archive:
            for member in archive:
                name = member.name.removeprefix("./")
                if name == ".":
                    continue
                if not (member.isdir() or member.isfile()):
                    raise RuntimeError(f"Unexpected TAR member type: {member.name}")
                name = name.rstrip("/") + "/" if member.isdir() else name
                if name in contents:
                    raise RuntimeError(f"Duplicate TAR member: {name}")
                contents[name] = None if member.isdir() else archive.extractfile(member).read()
        return contents

    def tar_save_picker(self, evidence):
        self.tap("Save TAR…")
        self.documents()
        activities = self.adb("shell", "dumpsys", "activity", "activities")
        (self.out / f"{evidence}-intent.log").write_text(activities)
        if not any("act=android.intent.action.CREATE_DOCUMENT" in line and
                   "typ=application/x-tar" in line for line in activities.splitlines()):
            raise RuntimeError("TAR save picker lacks ACTION_CREATE_DOCUMENT application/x-tar")
        self.save(evidence)

    def tar_smoke(self):
        fixture = ROOT / "tests/fixtures/sample.tar"
        contents = self.tar_contents(fixture)
        assert contents == {"docs/": None, "docs/你好.txt": "tar 你好\n".encode(),
                            "docs/empty/": None, "docs/nested/data.bin": bytes((0, 127, 255)),
                            "other.txt": b"not selected\n"}
        name = f"peazip-{self.token}-sample.tar"
        remote = f"/sdcard/Download/{name}"
        self.files.append(remote)
        self.adb("push", fixture, remote)
        self.scan(remote)
        self.entries = list(contents)
        self.choose(name)
        self.archive(name)
        tree = self.label("Save TAR…")
        if any(self.match(n, "Save ZIP…") for n in tree.iter("node")):
            raise RuntimeError("Opened TAR exposes Save ZIP")
        self.label("Create ZIP…")
        self.save("tar-imported")
        self.tap("docs/")
        self.extraction_state(name, selected_entry="docs/")
        before = self.adb("shell", "ls", "-1", "/sdcard/Download")
        self.tar_save_picker("tar-save-cancel-picker")
        self.cancel_documents()
        self.extraction_state(name, "Save cancelled.", selected_entry="docs/")
        if self.adb("shell", "ls", "-1", "/sdcard/Download") != before:
            raise RuntimeError("TAR save cancellation changed Downloads")
        self.save("tar-save-cancelled")
        self.tar_save_picker("tar-save-picker")
        self.downloads()
        tree = self.wait("TAR save filename", lambda t: any(
            n.get("class") == "android.widget.EditText" for n in t.iter("node")))
        field = next(n for n in tree.iter("node") if n.get("class") == "android.widget.EditText")
        self.tap(field.get("text", ""), field.get("resource-id"))
        self.adb("shell", "input", "keyevent", "KEYCODE_MOVE_END")
        self.adb("shell", "input", "keyevent", *(["KEYCODE_DEL"] * len(field.get("text", ""))))
        saved = f"peazip-{self.token}-saved.tar"
        self.adb("shell", "input", "text", saved)
        self.label(saved)
        output = f"/sdcard/Download/{saved}"
        self.files.append(output)
        self.button(("Save", "SAVE"))
        self.extraction_state(name, f"Saved archive: {saved}", selected_entry="docs/")
        local = self.out / saved
        self.adb("pull", output, local)
        if local.read_bytes() != fixture.read_bytes():
            raise RuntimeError("Saved TAR differs byte-for-byte from fixture")
        if self.tar_contents(local) != contents:
            raise RuntimeError("Python tarfile verification of saved TAR failed")
        self.save("tar-saved")
        baseline = self.export_snapshot("tar-before")
        self.tree_picker("tar-extract-cancel-picker")
        self.cancel_documents()
        self.extraction_state(name, "Extraction cancelled.", selected_entry="docs/")
        if self.export_snapshot("tar-cancelled") != baseline:
            raise RuntimeError("TAR tree cancellation changed export parent")
        self.save("tar-extract-cancelled")
        self.tree_picker("tar-extract-tree-picker")
        self.export_destination("tar-extract-parent-picker")
        tree = self.wait("TAR extraction report", lambda t: any(
            n.get("text", "").startswith(("Extracted ", "Could not extract archive:"))
            for n in t.iter("node")))
        report = next(n.get("text") for n in tree.iter("node") if
                      n.get("text", "").startswith(("Extracted ", "Could not extract archive:")))
        match = re.fullmatch(r"Extracted 4 entries to (PeaZip-([0-9a-fA-F-]{36}))", report)
        if not match:
            raise RuntimeError(f"Unexpected TAR extraction report: {report}")
        uuid.UUID(match[2])
        folder = match[1]
        if folder in baseline:
            raise RuntimeError("TAR extraction reused an existing export directory")
        expected = {**baseline, folder: None, f"{folder}/docs": None,
                    f"{folder}/docs/你好.txt": contents["docs/你好.txt"],
                    f"{folder}/docs/empty": None, f"{folder}/docs/nested": None,
                    f"{folder}/docs/nested/data.bin": contents["docs/nested/data.bin"]}
        if self.export_snapshot("tar-success") != expected:
            raise RuntimeError("TAR extraction directories/Unicode/binary/empty directory differ; "
                               "other.txt must be absent and previous exports unchanged")
        self.extraction_state(name, report, selected_entry="docs/")
        self.save("tar-extracted")
        self.health()
        result = ("PASS TAR: five entries excluding ./ root; Save TAR without Save ZIP; "
                  "real ACTION_CREATE_DOCUMENT application/x-tar on cancel and save; "
                  "save cancellation preserves snapshot/selection and Downloads; "
                  "saved bytes identical to fixture, independently verified with Python tarfile; "
                  "real SAF tree cancellation and docs/ extraction of four entries; "
                  "Unicode/binary bytes and empty directory verified, other.txt absent; "
                  f"previous exports unchanged. {report}\n")
        (self.out / "tar-result.log").write_text(result)
        print(result, end="", flush=True)

    def health(self):
        pid = self.adb("shell", "pidof", PACKAGE).strip()
        if not pid:
            raise RuntimeError("App process is absent")
        current = set(pid.split())
        if self.pids and current != self.pids:
            raise RuntimeError(f"App PID changed: {self.pids} -> {current}")
        self.pids = current
        log = self.adb("logcat", "-d", "-v", "threadtime", "-T", self.started,
                       "AndroidRuntime:E", "*:S")
        (self.out / "android-runtime.log").write_text(log)
        blocks = re.split(r"(?=\d\d-\d\d .*FATAL EXCEPTION)", log)
        if any("FATAL EXCEPTION" in b and (PACKAGE in b or any(
                re.search(rf"\s{pid}\s", line) for pid in self.pids for line in b.splitlines())) for b in blocks):
            raise RuntimeError(f"App fatal in AndroidRuntime: {log}")
        print(f"PASS app pid={pid}; no app AndroidRuntime fatal", flush=True)

    def run(self):
        self.adb("get-state")
        self.started = self.adb("shell", "date", "'+%m-%d %H:%M:%S.000'").strip()
        if self.args.apk:
            self.adb("install", "-r", str(Path(self.args.apk).resolve()))
        fixture = ROOT / "web/tests/fixtures"
        with zipfile.ZipFile(fixture / "sample.zip") as archive:
            self.entries = archive.namelist()
            assert "你好.txt" in self.entries and "空目录/" in self.entries
            assert not any(i.filename.startswith("空目录/") and not i.is_dir() for i in archive.infolist())
        for kind in ("sample", "invalid"):
            remote = f"/sdcard/Download/peazip-{self.token}-{kind}.zip"
            self.files.append(remote)
            self.adb("push", fixture / f"{kind}.zip", remote)
            self.adb("shell", "am", "broadcast", "-a", "android.intent.action.MEDIA_SCANNER_SCAN_FILE",
                     "-d", f"file://{remote}")
        tree = self.dump()
        for _ in range(4):
            if not any(n.get("package", "").endswith(".documentsui") for n in tree.iter("node")):
                break
            self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
            tree = self.dump()
        self.adb("shell", "am", "start", "-n", f"{PACKAGE}/com.longbridge.nsh.PeaZipActivity")
        self.label("Open archive…")
        self.health()
        sample, invalid = [Path(p).name for p in self.files]
        self.choose(sample)
        self.archive(sample)
        self.save("imported")
        print(f"PASS system picker import: {sample}; entries={self.entries}", flush=True)
        self.tap("Select all")
        self.archive(sample, len(self.entries))
        self.save("selected-all")
        self.tap("Clear selection")
        self.archive(sample)
        print("PASS Select all / Clear selection, counters and all checkbox states", flush=True)
        self.picker()
        self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
        self.archive(sample)
        print("PASS cancelling picker preserves archive and entries", flush=True)
        self.choose(invalid)
        self.wait("invalid ZIP error", lambda t: any(
            n.get("text", "").startswith("Could not open archive:") for n in t.iter("node")))
        self.save("invalid")
        print("PASS invalid ZIP reports Could not open archive", flush=True)
        self.health()
        self.choose(sample)
        self.archive(sample)
        self.health()
        print("PASS reopening valid ZIP recovers", flush=True)
        self.create_oversized(sample)
        self.create_save()
        self.extract_selected()
        self.tar_smoke()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--apk", type=Path)
    sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
    default_adb = os.environ.get("ADB") or shutil.which("adb")
    if not default_adb:
        default_adb = str((Path(sdk) if sdk else Path.home() / "Library/Android/sdk") / "platform-tools/adb")
    parser.add_argument("--adb", default=default_adb)
    parser.add_argument("--timeout", type=float, default=30)
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    smoke = Smoke(args)
    failed = False
    try:
        smoke.run()
    except Exception as error:
        failed = True
        print(f"FAIL: {error}", file=sys.stderr)
    finally:
        for action in (lambda: smoke.save("final"),
                       lambda: smoke.health() if smoke.started else None):
            try:
                action()
            except Exception as error:
                failed = True
                print(f"FAIL evidence/health: {error}", file=sys.stderr)
        for remote in [*smoke.files, smoke.remote_xml]:
            try:
                smoke.adb("shell", "rm", "-f", shlex.quote(remote))
            except Exception as error:
                failed = True
                print(f"FAIL cleanup {remote}: {error}", file=sys.stderr)
        if smoke.export_owned:
            try:
                smoke.adb("shell", "rm", "-rf", smoke.export_parent)
            except Exception as error:
                failed = True
                print(f"FAIL cleanup {smoke.export_parent}: {error}", file=sys.stderr)
    print(f"{'FAIL' if failed else 'PASS'} artifacts: {smoke.out}")
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
