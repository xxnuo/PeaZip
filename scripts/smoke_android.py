"""Run with uv run python scripts/smoke_android.py --serial emulator-5554."""

import argparse
import gzip
import os
from pathlib import Path
import re
import shutil
import shlex
import struct
import subprocess
import sys
import tarfile
import tempfile
import time
import uuid
import xml.etree.ElementTree as ET
import zipfile
import zlib

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

    def flat_view(self):
        tree = self.wait("archive view toggle", lambda t: any(
            self.match(n, "All entries") or self.match(n, "Folder view") for n in t.iter("node")))
        if any(self.match(n, "All entries") for n in tree.iter("node")):
            self.tap("All entries")
        self.label("Folder view")

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
        self.tap("Create archive…")
        self.label("Create archive")
        self.tap("Continue")
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

    def test_archive(self, filename, contents, checksummed_files, evidence, reject=False, test_action=None):
        def snapshot(tree):
            return [(n.get("content-desc"), n.get("checked"), n.get("text"))
                    for n in tree.iter("node")
                    if n.get("checkable") == "true" and n.get("package") == PACKAGE]
        tree = self.dump()
        before = snapshot(tree)
        selected_label = next(n.get("text") for n in tree.iter("node")
                              if n.get("package") == PACKAGE and
                              re.fullmatch(r"\d+ selected", n.get("text", "")))
        downloads = self.adb("shell", "ls", "-laR", "/sdcard/Download")
        documents = self.adb("shell", "ls", "-laR", "/sdcard/Documents", check=False)
        exports = self.export_snapshot(f"{evidence}-before") if self.export_owned else None
        files = sum(data is not None for data in contents.values())
        total = sum(len(data) for data in contents.values() if data is not None)
        expected = (f"Tested {len(contents)} entries ({total} bytes). "
                    f"Content checksums: {checksummed_files}/{files} files.")
        if test_action is None:
            self.tap("Test archive")
        else:
            test_action()
        def finished(tree):
            if any(n.get("package", "").endswith(".documentsui") or
                   n.get("resource-id") == "android:id/alertTitle" for n in tree.iter("node")):
                raise AssertionError("Test archive opened a dialog or DocumentsUI")
            return any(n.get("text", "").startswith(("Tested ", "Could not test archive:"))
                       for n in tree.iter("node"))
        tree = self.wait("archive test result", finished)
        report = next(n.get("text") for n in tree.iter("node") if
                      n.get("text", "").startswith(("Tested ", "Could not test archive:")))
        if (reject and not report.startswith("Could not test archive:")) or (not reject and report != expected):
            raise RuntimeError(f"Unexpected archive test result: {report}; expected {expected}")
        self.label(filename)
        self.label(selected_label)
        if snapshot(tree) != before:
            raise RuntimeError("Test archive changed archive rows or selection")
        if self.adb("shell", "ls", "-laR", "/sdcard/Download") != downloads:
            raise RuntimeError("Test archive changed Downloads")
        if self.adb("shell", "ls", "-laR", "/sdcard/Documents", check=False) != documents:
            raise RuntimeError("Test archive changed Documents")
        if exports is not None and self.export_snapshot(f"{evidence}-after") != exports:
            raise RuntimeError("Test archive changed previous export bytes")
        self.save(evidence)
        self.health()
        print(f"PASS read-only Test archive: {filename}: {report}; snapshot/selection preserved", flush=True)

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
                if self.match(node, "Extract…"):
                    while node.get("class") != "android.widget.Button" and node in parents:
                        node = parents[node]
                    return node.get("enabled") == str(enabled).lower()
            return False
        self.wait(f"Extract enabled={enabled}", valid)

    def extraction_state(self, filename, status=None, selected_entry="tree/"):
        self.label(filename)
        self.label("1 selected")
        if status:
            self.label(status)
        self.wait(f"{selected_entry} and descendants effectively checked; snapshot preserved", lambda t: {
            n.get("content-desc"): n.get("checked") for n in t.iter("node")
            if n.get("checkable") == "true" and n.get("package") == PACKAGE
        } == {name: str(name == selected_entry or
                       (selected_entry.endswith("/") and name.startswith(selected_entry))).lower()
              for name in self.entries})
        self.extraction_button(True)

    def export_snapshot(self, name):
        local = self.out / f"extract-{self.token}-{name}"
        self.adb("pull", self.export_parent, local)
        return {p.relative_to(local).as_posix(): None if p.is_dir() else p.read_bytes()
                for p in local.rglob("*")}

    def extract_dialog(self, scope=None, paths="Full paths"):
        self.tap("Extract…")
        self.label("Extract archive")
        if scope is not None:
            self.tap(scope)
        if paths is not None:
            self.tap(paths)

    def tree_picker(self, evidence="extract-tree-picker"):
        self.extract_dialog()
        self.tap("Continue")
        self.documents()
        self.save(evidence)
        self.wait("tree picker folder action", lambda t: any(
            n.get("text", "").lower() == "use this folder" for n in t.iter("node")))

    def export_destination(self, evidence="extract-parent-picker"):
        model = self.adb("shell", "getprop", "ro.product.model").strip()
        tree = self.documents()
        # DocumentsUI can restore the exact authorized parent with disabled ancestor breadcrumbs.
        breadcrumbs = [n.get("text") for n in tree.iter("node")
                       if n.get("resource-id", "").endswith(":id/breadcrumb_text")]
        if not breadcrumbs or breadcrumbs[-1] != Path(self.export_parent).name:
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
        self.extraction_button(True)
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
        self.tap("other.txt")
        self.test_archive(bad.name, {k: None if k.endswith("/") else v for k, v in contents.items()},
                          3, "test-zip-crc-rejected", reject=True)
        self.tap("other.txt")
        self.tap("tree/")
        self.extract_dialog()
        self.tap("Continue")
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
        with tarfile.open(path, "r:*") as archive:
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

    def sevenzip_contents(self, path):
        # Only the trusted fixture or its byte-identical saved copy reaches extraction.
        fixture = ROOT / "tests/fixtures/sample.7z"
        if path.read_bytes() != fixture.read_bytes():
            raise RuntimeError("Refusing CLI extraction of non-fixture 7Z bytes")
        command = shutil.which("7z") or shutil.which("7zz")
        if not command:
            raise RuntimeError("Independent 7Z verification requires system 7z or 7zz")
        with tempfile.TemporaryDirectory(prefix="7z-verify-", dir=self.out) as destination:
            for arguments in (("t", str(path)), ("x", str(path), f"-o{destination}", "-y")):
                result = subprocess.run([command, *arguments], capture_output=True,
                                        text=True, timeout=self.args.timeout)
                with (self.out / "7z-independent.log").open("a") as log:
                    log.write(result.stdout + result.stderr)
                if result.returncode:
                    raise RuntimeError(f"System 7z {arguments[0]} failed: {result.stdout} {result.stderr}")
            output = Path(destination)
            return {p.relative_to(output).as_posix() + ("/" if p.is_dir() else ""):
                    None if p.is_dir() else p.read_bytes() for p in output.rglob("*")}

    def tar_save_picker(self, evidence, kind="TAR", mime="application/x-tar"):
        self.tap(f"Save {kind}…")
        self.documents()
        if kind == "7Z":
            self.label("Archive.7z")
        activities = self.adb("shell", "dumpsys", "activity", "activities")
        (self.out / f"{evidence}-intent.log").write_text(activities)
        if not any("act=android.intent.action.CREATE_DOCUMENT" in line and
                   f"typ={mime}" in line for line in activities.splitlines()):
            raise RuntimeError(f"{kind} save picker lacks ACTION_CREATE_DOCUMENT {mime}")
        self.save(evidence)

    def tar_smoke(self, compressed=False, sevenzip=False):
        kind = "7Z" if sevenzip else "TAR.GZ" if compressed else "TAR"
        extension = "7z" if sevenzip else "tar.gz" if compressed else "tar"
        evidence = "7z" if sevenzip else "tar-gz" if compressed else "tar"
        mime = "application/x-7z-compressed" if sevenzip else "application/gzip" if compressed else "application/x-tar"
        fixture = ROOT / f"tests/fixtures/sample.{extension}"
        if compressed:
            data = fixture.read_bytes()
            assert data[:3] == b"\x1f\x8b\x08" and struct.unpack_from("<I", data, 4)[0] == 0
            assert gzip.decompress(data) == (ROOT / "tests/fixtures/sample.tar").read_bytes()
        contents = self.sevenzip_contents(fixture) if sevenzip else self.tar_contents(fixture)
        expected_contents = {"docs/": None, "docs/你好.txt": "tar 你好\n".encode(),
                            "docs/empty/": None, "docs/nested/data.bin": bytes((0, 127, 255)),
                            "other.txt": b"not selected\n"}
        if sevenzip:
            expected_contents["docs/nested/"] = None
            expected_contents["docs/你好.txt"] = "7z 你好\n".encode()
        assert contents == expected_contents
        selected_count = 5 if sevenzip else 4
        name = f"peazip-{self.token}-sample.{extension}"
        remote = f"/sdcard/Download/{name}"
        self.files.append(remote)
        self.adb("push", fixture, remote)
        self.scan(remote)
        self.entries = list(contents)
        self.choose(name)
        self.archive(name)
        tree = self.label(f"Save {kind}…")
        if any(self.match(n, "Save ZIP…") for n in tree.iter("node")):
            raise RuntimeError(f"Opened {kind} exposes Save ZIP")
        self.label("Create archive…")
        self.save(f"{evidence}-imported")
        self.tap("docs/")
        self.extraction_state(name, selected_entry="docs/")
        self.test_archive(name, contents, 0 if kind == "TAR" else 3, f"{evidence}-tested")
        before = self.adb("shell", "ls", "-1", "/sdcard/Download")
        self.tar_save_picker(f"{evidence}-save-cancel-picker", kind, mime)
        self.cancel_documents()
        self.extraction_state(name, "Save cancelled.", selected_entry="docs/")
        if self.adb("shell", "ls", "-1", "/sdcard/Download") != before:
            raise RuntimeError(f"{kind} save cancellation changed Downloads")
        self.save(f"{evidence}-save-cancelled")
        self.tar_save_picker(f"{evidence}-save-picker", kind, mime)
        self.downloads()
        tree = self.wait(f"{kind} save filename", lambda t: any(
            n.get("class") == "android.widget.EditText" for n in t.iter("node")))
        field = next(n for n in tree.iter("node") if n.get("class") == "android.widget.EditText")
        self.tap(field.get("text", ""), field.get("resource-id"))
        self.adb("shell", "input", "keyevent", "KEYCODE_MOVE_END")
        self.adb("shell", "input", "keyevent", *(["KEYCODE_DEL"] * len(field.get("text", ""))))
        saved = f"peazip-{self.token}-saved.{extension}"
        self.adb("shell", "input", "text", saved)
        self.label(saved)
        output = f"/sdcard/Download/{saved}"
        self.files.append(output)
        self.button(("Save", "SAVE"))
        self.extraction_state(name, f"Saved archive: {saved}", selected_entry="docs/")
        local = self.out / saved
        self.adb("pull", output, local)
        if local.read_bytes() != fixture.read_bytes():
            raise RuntimeError(f"Saved {kind} differs byte-for-byte from fixture")
        verified = self.sevenzip_contents(local) if sevenzip else self.tar_contents(local)
        if verified != contents:
            raise RuntimeError(f"Independent verification of saved {kind} failed")
        self.save(f"{evidence}-saved")
        baseline = self.export_snapshot(f"{evidence}-before")
        self.tree_picker(f"{evidence}-extract-cancel-picker")
        self.cancel_documents()
        self.extraction_state(name, "Extraction cancelled.", selected_entry="docs/")
        if self.export_snapshot(f"{evidence}-cancelled") != baseline:
            raise RuntimeError(f"{kind} tree cancellation changed export parent")
        self.save(f"{evidence}-extract-cancelled")
        self.tree_picker(f"{evidence}-extract-tree-picker")
        self.export_destination(f"{evidence}-extract-parent-picker")
        tree = self.wait(f"{kind} extraction report", lambda t: any(
            n.get("text", "").startswith(("Extracted ", "Could not extract archive:"))
            for n in t.iter("node")))
        report = next(n.get("text") for n in tree.iter("node") if
                      n.get("text", "").startswith(("Extracted ", "Could not extract archive:")))
        match = re.fullmatch(rf"Extracted {selected_count} entries to (PeaZip-([0-9a-fA-F-]{{36}}))", report)
        if not match:
            raise RuntimeError(f"Unexpected {kind} extraction report: {report}")
        uuid.UUID(match[2])
        folder = match[1]
        if folder in baseline:
            raise RuntimeError(f"{kind} extraction reused an existing export directory")
        expected = {**baseline, folder: None, f"{folder}/docs": None,
                    f"{folder}/docs/你好.txt": contents["docs/你好.txt"],
                    f"{folder}/docs/empty": None, f"{folder}/docs/nested": None,
                    f"{folder}/docs/nested/data.bin": contents["docs/nested/data.bin"]}
        if self.export_snapshot(f"{evidence}-success") != expected:
            raise RuntimeError(f"{kind} extraction directories/Unicode/binary/empty directory differ; "
                               "other.txt must be absent and previous exports unchanged")
        self.extraction_state(name, report, selected_entry="docs/")
        self.save(f"{evidence}-extracted")
        if compressed or sevenzip:
            bad = self.out / f"peazip-{self.token}-bad-crc.{extension}"
            damaged = bytearray(fixture.read_bytes())
            if sevenzip:
                assert damaged[:6] == b"7z\xbc\xaf\x27\x1c"
                assert struct.unpack_from("<I", damaged, 8)[0] == zlib.crc32(damaged[12:32])
                damaged[8] ^= 1
                assert struct.unpack_from("<I", damaged, 8)[0] != zlib.crc32(damaged[12:32])
            else:
                damaged[-8] ^= 1
                try:
                    gzip.decompress(damaged)
                except gzip.BadGzipFile:
                    pass
                else:
                    raise RuntimeError("Corrupted gzip CRC was accepted by Python")
            bad.write_bytes(damaged)
            bad_remote = f"/sdcard/Download/{bad.name}"
            self.files.append(bad_remote)
            self.adb("push", bad, bad_remote)
            self.scan(bad_remote)
            downloads_before = self.adb("shell", "ls", "-1", "/sdcard/Download")
            self.choose(bad.name)
            tree = self.wait(f"{kind} CRC rejection on open", lambda t: any(
                n.get("text", "").startswith("Could not open archive:")
                for n in t.iter("node")))
            error = next(n.get("text") for n in tree.iter("node")
                         if n.get("text", "").startswith("Could not open archive:"))
            self.extraction_state(name, error, selected_entry="docs/")
            if self.export_snapshot(f"{evidence}-crc-rejected") != expected:
                raise RuntimeError(f"{kind} CRC rejection changed previous exports")
            if self.adb("shell", "ls", "-1", "/sdcard/Download") != downloads_before:
                raise RuntimeError(f"{kind} CRC rejection changed Downloads")
            self.save(f"{evidence}-crc-rejected")
            print(f"PASS {kind} CRC rejected before output; archive/selection/exports preserved: {error}",
                  flush=True)
        self.health()
        result = (f"PASS {kind}: {len(contents)} entries excluding ./ root; Save {kind} without Save ZIP; "
                  f"real ACTION_CREATE_DOCUMENT {mime} on cancel and save; "
                  "save cancellation preserves snapshot/selection and Downloads; "
                  f"saved bytes identical to fixture, independently verified with {'system 7z' if sevenzip else 'Python tarfile'}; "
                  f"real SAF tree cancellation and docs/ extraction of {selected_count} entries; "
                  "Unicode/binary bytes and empty directory verified, other.txt absent; "
                  f"previous exports unchanged. {report}\n")
        (self.out / f"{evidence}-result.log").write_text(result)
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
        self.flat_view()
        self.health()
        sample, invalid = [Path(p).name for p in self.files]
        self.choose(sample)
        self.archive(sample)
        self.save("imported")
        with zipfile.ZipFile(fixture / "sample.zip") as archive:
            test_contents = {i.filename: None if i.is_dir() else archive.read(i)
                             for i in archive.infolist()}
        self.test_archive(sample, test_contents,
                          sum(v is not None for v in test_contents.values()), "zip-tested")
        self.choose(sample)
        self.archive(sample)
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
        self.tar_smoke(compressed=True)
        self.tar_smoke(sevenzip=True)


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
