"""Run with uv run python scripts/smoke_android_create_formats.py --serial SERIAL --apk APK."""

import gzip
import hashlib
import io
import json
import re
import shutil
import sys
import tarfile
import zipfile

import smoke_android
from smoke_android_create_options import CreateOptionsSmoke
from smoke_android_extract_options import ExtractOptionsSmoke
from smoke_android_menus import MenuSmoke


class CreateFormatsSmoke(CreateOptionsSmoke, MenuSmoke):
    @staticmethod
    def entry_rows(tree):
        contents = next((node for node in tree.iter("node")
                         if node.get("package") == smoke_android.PACKAGE
                         and smoke_android.Smoke.match(node, "Archive contents")), None)
        return [] if contents is None else [node for node in contents.iter("node")
                                           if node.get("checkable") == "true"]

    @staticmethod
    def snapshot(tree):
        return {"rows": sorted((node.get("content-desc"), node.get("checked"))
                               for node in CreateFormatsSmoke.entry_rows(tree)),
                "selection": [node.get("text") for node in tree.iter("node")
                              if node.get("package") == smoke_android.PACKAGE
                              and re.fullmatch(r"\d+ selected", node.get("text", ""))]}

    def archive(self, filename, selected=0, status=None, directory=False):
        self.label(filename)
        self.label(status or f"{len(self.entries)} entries")
        self.label(f"{selected} selected")
        self.wait("archive rows and selection (excluding filter controls)", lambda tree:
                  [(node.get("content-desc"), node.get("checked"))
                   for node in sorted(self.entry_rows(tree), key=lambda node: node.get("content-desc", ""))]
                  == sorted((name, str(bool(selected)).lower()) for name in self.entries))

    @staticmethod
    def level_fields(tree):
        return [field for wrapper in tree.iter("node")
                if smoke_android.Smoke.match(wrapper, "GZIP compression level")
                for field in wrapper.iter("node") if field.get("class") == "android.widget.EditText"]

    def radio(self, label, checked=True):
        return self.wait(f"{label} checked={checked}", lambda tree: any(
            self.match(node, label) and node.get("checkable") == "true"
            and node.get("checked") == str(checked).lower() for node in tree.iter("node")))

    def modal(self, kind, level=None):
        self.tap("Create archive…")
        self.label("Create archive")
        self.tap(kind)
        self.radio(kind)
        if level is not None:
            self.set_level(level)

    def pick_files(self, query, names, evidence):
        if any(field.get("focused") == "true" for field in self.level_fields(self.dump())):
            self.ime_shown(evidence)
        self.tap("Continue")
        self.documents()
        self.tap("Search")
        self.wait("focused input search", lambda tree: any(
            node.get("focused") == "true" and node.get("class") in
            ("android.widget.EditText", "android.widget.AutoCompleteTextView") for node in tree.iter("node")))
        self.adb("shell", "input", "text", query)
        self.adb("shell", "input", "keyevent", "KEYCODE_ENTER")
        for at, name in enumerate(names):
            self.tap(name, "android:id/title", long=at == 0)
        self.save(f"{evidence}-input-selected")
        self.button(("Open", "OPEN", "Select", "SELECT"))

    @staticmethod
    def archive_name(kind):
        return "Archive.tar.gz" if kind == "TAR.GZ" else "Archive.tar"

    def created(self, kind, contents):
        self.entries = list(contents)
        self.archive(self.archive_name(kind), status=f"Created {kind}: {len(contents)} entries", directory=False)

    @staticmethod
    def verify_tar(path, contents, kind):
        raw = path.read_bytes()
        if kind == "TAR.GZ":
            assert raw[:4] == b"\x1f\x8b\x08\0" and raw[4:8] == bytes(4)
            decoded = gzip.decompress(raw)  # Independently verifies the complete CRC and ISIZE.
        else:
            assert not raw.startswith(b"\x1f\x8b")
            decoded = raw
        assert len(decoded) % 512 == 0 and decoded[-1024:] == bytes(1024)
        actual = {}
        with tarfile.open(fileobj=io.BytesIO(decoded), mode="r:") as archive:
            for member in archive:
                assert member.isfile() or member.isdir()
                assert member.uid == member.gid == member.mtime == 0
                assert member.mode == (0o755 if member.isdir() else 0o644)
                name = member.name.rstrip("/") + ("/" if member.isdir() else "")
                assert name not in actual
                actual[name] = None if member.isdir() else archive.extractfile(member).read()
        if actual != contents:
            raise AssertionError(f"Saved {kind} names/payloads differ: {list(actual)} vs {list(contents)}")
        return {"format": kind, "bytes": len(raw), "tar_bytes": len(decoded),
                "sha256": hashlib.sha256(raw).hexdigest(),
                "files": {name: None if data is None else {"size": len(data), "sha256": hashlib.sha256(data).hexdigest()}
                          for name, data in actual.items()}}

    def save_format(self, kind, evidence, contents, expected_bytes=None):
        extension = ".tar.gz" if kind == "TAR.GZ" else ".tar"
        requested = f"peazip-{self.token}-{evidence}{extension}"
        self.tar_save_picker(f"{evidence}-save-picker", kind,
                             "application/gzip" if kind == "TAR.GZ" else "application/x-tar")
        self.downloads()
        tree = self.wait("save filename", lambda tree: any(
            node.get("class") == "android.widget.EditText" for node in tree.iter("node")))
        self.replace_field(next(node for node in tree.iter("node")
                                if node.get("class") == "android.widget.EditText"), requested)
        self.button(("Save", "SAVE"))
        tree = self.wait("saved archive result", lambda tree: any(
            node.get("text", "").startswith(("Saved archive: ", "Could not save archive:"))
            or self.match(node, "Replace") for node in tree.iter("node")))
        saved = next((node.get("text")[len("Saved archive: "):] for node in tree.iter("node")
                      if node.get("text", "").startswith("Saved archive: ")), None)
        if saved != requested:
            raise AssertionError(f"Save failed or attempted replacement: {saved!r}")
        remote = f"/sdcard/Download/{saved}"
        self.files.append(remote)
        local = self.out / saved
        self.adb("pull", remote, local)
        if expected_bytes is not None and local.read_bytes() != expected_bytes:
            raise AssertionError("Cancelled or failed creation changed the previous archive bytes")
        report = self.verify_tar(local, contents, kind)
        (self.out / f"{evidence}.json").write_text(json.dumps(report, ensure_ascii=False, indent=2))
        self.save(evidence)
        self.health()
        print(f"PASS {evidence}: real {kind} MIME/save, {len(contents)} entries, "
              f"all bytes/metadata verified; archive bytes={report['bytes']}", flush=True)
        return local

    def run(self):
        if not self.args.apk or not self.args.apk.is_file():
            raise RuntimeError("Pass --apk pointing to the rebuilt creation-formats APK")
        if self.adb("emu", "avd", "name").replace("\r", "").splitlines()[0] != "PeaZip_API36":
            raise RuntimeError("Use only the dedicated PeaZip_API36 emulator")
        if self.adb("shell", "getprop", "ro.build.version.sdk").strip() != "36":
            raise RuntimeError("Creation-formats smoke requires API 36")
        apk = self.out / "tested.apk"
        shutil.copyfile(self.args.apk.resolve(), apk)
        digest = hashlib.sha256(apk.read_bytes()).hexdigest()
        (self.out / "apk.sha256").write_text(f"{digest}  {apk.name}\n")
        print(f"Create-formats artifacts: {self.out}; APK SHA256 {digest}", flush=True)
        self.adb("install", "-r", str(apk.resolve()))
        self.adb("shell", "am", "force-stop", smoke_android.PACKAGE)
        self.started = self.adb("shell", "date", "'+%m-%d %H:%M:%S.000'").strip()
        self.adb("shell", "am", "start", "-n", f"{smoke_android.PACKAGE}/com.longbridge.nsh.PeaZipActivity")
        self.label("Open archive…")
        self.flat_view()
        self.health()

        fixture = smoke_android.ROOT / "web/tests/fixtures/sample.zip"
        with zipfile.ZipFile(fixture) as archive:
            baseline = {info.filename: None if info.is_dir() else archive.read(info) for info in archive.infolist()}
        baseline_name = f"peazip-{self.token}-baseline.zip"
        prefix = f"peazip-{self.token}-format-"
        sentence = b"The quick brown fox jumps over the lazy dog. Archive compression level test.\n"
        data = b"".join(sentence * 16 + hashlib.sha256(at.to_bytes(4, "little")).digest() for at in range(512))
        contents = {f"{prefix}中文.bin": data, f"{prefix}binary.bin": bytes(range(256)), f"{prefix}empty.txt": b""}
        bomb_name = f"peazip-{self.token}-zeros.bin"
        sources = {baseline_name: fixture.read_bytes(), **contents, bomb_name: bytes(1024 * 1024)}
        for name, body in sources.items():
            local = self.out / name
            local.write_bytes(body)
            remote = f"/sdcard/Download/{name}"
            self.files.append(remote)
            self.adb("push", local, remote)
            self.scan(remote)
        self.entries = list(baseline)
        self.choose(baseline_name)
        self.archive(baseline_name, directory=False)
        self.tap("Select all")
        before = self.snapshot(self.dump())
        self.tap("Create archive…")
        self.label("Create archive")
        self.radio("ZIP")
        self.radio("Automatic")
        self.save("default-zip-automatic")
        self.tap("Cancel")
        self.preserved(baseline_name, before, "default-zip-cancelled")
        for kind, dismissal in (("TAR", "Cancel"), ("TAR.GZ", "Back")):
            inventory = self.adb("shell", "ls", "-1", "/sdcard/Download")
            marker = self.no_picker_marker(f"{kind}-{dismissal}")
            self.modal(kind)
            if dismissal == "Back":
                self.adb("shell", "input", "keyevent", "KEYCODE_BACK")
            else:
                self.tap("Cancel")
            self.assert_no_picker(marker, f"modal-{kind}-{dismissal}")
            self.preserved(baseline_name, before, f"cancel-{kind}-{dismissal}")
            assert self.adb("shell", "ls", "-1", "/sdcard/Download") == inventory
        self.modal("TAR")
        self.tap("Continue")
        self.documents()
        self.cancel_documents()
        self.preserved(baseline_name, before, "input-picker-cancelled")
        self.save_zip("cancel-old-zip-bytes", {name: b"" if value is None else value for name, value in baseline.items()}, expected_bytes=fixture.read_bytes())
        print("PASS default ZIP/Automatic, modal Cancel/Back and input-picker cancellation preserve original ZIP bytes/selection", flush=True)

        saved = {}
        for kind, level, evidence in (("TAR", None, "created-tar"), ("TAR.GZ", 1, "gzip-1"), ("TAR.GZ", 9, "gzip-9")):
            print(f"RUN {evidence}: real Unicode/binary/empty inputs, create/save/readback", flush=True)
            self.modal(kind, 0 if level == 1 else level)
            if level == 1:
                self.label("Enter a GZIP level from 1 to 9.")
                tree = self.label("Continue")
                assert any(self.match(node, "Continue") and node.get("enabled") == "false" for node in tree.iter("node"))
                self.save("gzip-level-zero-disabled")
                self.set_level(1)
            self.pick_files(prefix, contents, evidence)
            self.created(kind, contents)
            self.tap(next(iter(contents)))
            selected = self.snapshot(self.dump())
            if evidence != "gzip-9":
                inventory = self.adb("shell", "ls", "-1", "/sdcard/Download")
                self.tar_save_picker(f"{evidence}-cancel-save", kind,
                                     "application/gzip" if kind == "TAR.GZ" else "application/x-tar")
                self.cancel_documents()
                self.preserved(self.archive_name(kind), selected, f"{evidence}-save-cancelled")
                assert self.adb("shell", "ls", "-1", "/sdcard/Download") == inventory
            saved[evidence] = self.save_format(kind, evidence, contents)
            if evidence != "gzip-1":
                remote = f"/sdcard/Download/{saved[evidence].name}"
                self.scan(remote)
                self.choose(saved[evidence].name)
                self.archive(saved[evidence].name, directory=False)
                self.save(f"{evidence}-reimported")
        assert saved["gzip-9"].stat().st_size < saved["gzip-1"].stat().st_size
        print("PASS gzip level 0 disabled, corrected 1 and 9 used; level 9 has smaller real gzip bytes", flush=True)

        selected_name = next(iter(contents))
        self.tap(selected_name)
        self.adb("shell", "mkdir", "-p", "/sdcard/Documents")
        self.adb("shell", "mkdir", self.export_parent)
        self.export_owned = True
        sentinel = self.out / "sentinel.bin"
        sentinel.write_bytes(b"create formats SAF sentinel\0\xff")
        self.adb("push", sentinel, f"{self.export_parent}/sentinel.bin")
        exports = self.export_snapshot("initial")
        assert exports == {"sentinel.bin": sentinel.read_bytes()}
        self.extract_dialog(scope="Selected entries", paths="Full paths")
        ExtractOptionsSmoke.authorize(self, 1, {selected_name: contents[selected_name]}, exports, "created-gzip-selected-export")

        before = self.snapshot(self.dump())
        self.modal("TAR.GZ", 9)
        self.pick_files(bomb_name, [bomb_name], "gzip-ratio-error")
        tree = self.wait("gzip ratio error", lambda tree: any(
            node.get("text", "").startswith("Could not create TAR.GZ:") for node in tree.iter("node")))
        error = next(node.get("text") for node in tree.iter("node") if node.get("text", "").startswith("Could not create TAR.GZ:"))
        assert "compression ratio" in error
        self.preserved(saved["gzip-9"].name, before, "ratio-error-preserved")
        self.save_format("TAR.GZ", "ratio-old-bytes", contents, expected_bytes=saved["gzip-9"].read_bytes())
        print("PASS gzip ratio failure preserves snapshot/selection and exact previously saved archive bytes", flush=True)
        self.modal("TAR")
        self.pick_files(bomb_name, [bomb_name], "tar-retry")
        self.created("TAR", {bomb_name: sources[bomb_name]})
        self.save_format("TAR", "tar-retry", {bomb_name: sources[bomb_name]})
        for name, body in sources.items():
            if self.adb("exec-out", "cat", f"/sdcard/Download/{name}", binary=True) != body:
                raise AssertionError(f"Source changed: {name}")
        assert hashlib.sha256(self.args.apk.read_bytes()).hexdigest() == digest, "APK changed during smoke"
        self.health()
        print("PASS TAR/TAR.GZ create/save/reimport, gzip levels, cancellations, ratio-preserving failure, TAR retry and actual SAF bytes", flush=True)


if __name__ == "__main__":
    smoke_android.Smoke = CreateFormatsSmoke
    sys.exit(smoke_android.main())
