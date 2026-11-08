"""Run with uv run python scripts/smoke_android_packed.py --serial SERIAL --apk APK."""

import hashlib
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import zipfile

import smoke_android
from smoke_android_extract_options import ExtractOptionsSmoke
from smoke_android_menus import MenuSmoke


class PackedSmoke(MenuSmoke):
    def fixtures(self):
        source = self.out / "packed.zip"
        with zipfile.ZipFile(source, "w") as archive:
            archive.writestr("folder/", b"")
            archive.writestr("empty", b"")
            archive.writestr("a.txt", b"packed alpha bravo charlie\n" * 64,
                             compress_type=zipfile.ZIP_DEFLATED, compresslevel=6)
            archive.writestr("plain.bin", bytes(range(128)))
            archive.writestr("中文.txt", "Packed Unicode 中文\n".encode() * 24,
                             compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)
        with zipfile.ZipFile(source) as archive:
            assert archive.testzip() is None
            contents = {info.filename: None if info.is_dir() else archive.read(info)
                        for info in archive.infolist()}
            packed = {info.filename: None if info.is_dir() else info.compress_size
                      for info in archive.infolist()}
        assert packed["empty"] == 0 and 0 < packed["a.txt"] < len(contents["a.txt"])
        yield "zip", source, contents, packed, "a.txt"

        root = smoke_android.ROOT / "crates/peazip-core/tests/fixtures"
        for kind, name in (("tar", "ustar.tar"), ("tgz", "ustar.tar.gz")):
            source = root / name
            contents = self.tar_contents(source)
            packed = {path: None if data is None or kind == "tgz" else (len(data) + 511) // 512 * 512
                      for path, data in contents.items()}
            assert contents["empty"] == b"" and contents["binary"] == bytes((0, 255, 128))
            assert packed["binary"] == (512 if kind == "tar" else None)
            yield kind, source, contents, packed, "binary"

        for kind, name in (("solid7z", "7z-lzma2-on.7z"), ("copy7z", "7z-copy.7z")):
            source = root / name
            contents = self.trusted_sevenzip(source)
            assert contents == {"empty-dir/": None, "folder/": None, "empty": b"",
                                "binary": bytes((0, 255, 128)),
                                "folder/中文.txt": "hello 7z 中文\n".encode(),
                                "folder/second.bin": bytes(range(256)) * 3}
            packed = {path: None if data is None or (kind == "solid7z" and data) else len(data)
                      for path, data in contents.items()}
            assert packed["empty"] == 0 and packed["binary"] == (None if kind == "solid7z" else 3)
            yield kind, source, contents, packed, "binary"

    def trusted_sevenzip(self, source):
        # Only these checked-in fixtures are ever passed to system extraction.
        root = smoke_android.ROOT / "crates/peazip-core/tests/fixtures"
        if source not in (root / "7z-lzma2-on.7z", root / "7z-copy.7z"):
            raise AssertionError("Refusing non-fixture 7z extraction")
        command = shutil.which("7z") or shutil.which("7zz")
        if not command:
            raise RuntimeError("Independent fixture verification requires system 7z")
        with tempfile.TemporaryDirectory(prefix="packed-7z-", dir=self.out) as directory:
            for arguments in (("t", str(source)), ("x", str(source), f"-o{directory}", "-y")):
                result = subprocess.run([command, *arguments], capture_output=True, text=True,
                                        timeout=self.args.timeout)
                with (self.out / "7z-independent.log").open("a") as log:
                    log.write(result.stdout + result.stderr)
                if result.returncode:
                    raise AssertionError(f"System 7z failed: {result.stdout} {result.stderr}")
            destination = Path(directory)
            return {path.relative_to(destination).as_posix() + ("/" if path.is_dir() else ""):
                    None if path.is_dir() else path.read_bytes() for path in destination.rglob("*")}

    @staticmethod
    def entry_rows(tree):
        return [node for node in tree.iter("node") if node.get("package") == smoke_android.PACKAGE
                and node.get("checkable") == "true"]

    def cells(self, contents, packed, column):
        header = "Packed (bytes)" if column == "Packed" else "Size (bytes)"
        other = "Size (bytes)" if column == "Packed" else "Packed (bytes)"

        def correct(tree):
            if not any(self.match(node, header) for node in tree.iter("node")):
                return False
            if any(self.match(node, other) for node in tree.iter("node")):
                raise AssertionError(f"Compact layout displays both columns: {header} and {other}")
            rows = self.entry_rows(tree)
            if len(rows) != len(contents) or {row.get("content-desc") for row in rows} != set(contents):
                return False
            for row in rows:
                path = row.get("content-desc")
                data = contents[path]
                value = packed[path] if column == "Packed" else None if data is None else len(data)
                suffix = "not applicable" if data is None else "unavailable" if value is None else str(value)
                label = f"{'Packed bytes' if column == 'Packed' else 'Size in bytes'}: {suffix}"
                if not any(self.match(node, label) for node in row.iter("node")):
                    return False
                # Android exposes labelled text through content-desc; screenshots retain the visible numeral/dash.
                if any(node.get("content-desc", "").startswith(
                        "Size in bytes:" if column == "Packed" else "Packed bytes:")
                       for node in row.iter("node")):
                    raise AssertionError(f"Hidden column still rendered in {path}")
            return True

        return self.wait(f"compact {column} cells match independently read fixture values", correct)

    def sorted_rows(self, contents, packed, descending):
        expected = sorted(contents, key=lambda path: (
            contents[path] is not None, packed[path] is None,
            -(packed[path] or 0) if descending else packed[path] or 0, path))
        self.wait(f"packed {'descending' if descending else 'ascending'} order: {expected}", lambda tree:
                  [row.get("content-desc") for row in sorted(self.entry_rows(tree),
                   key=lambda node: self.bounds(node)[1])] == expected)

    def run(self):
        if not self.args.apk or not self.args.apk.is_file():
            raise RuntimeError("Pass --apk pointing to the rebuilt Packed APK")
        if self.adb("emu", "avd", "name").replace("\r", "").splitlines()[0] != "PeaZip_API36":
            raise RuntimeError("Run only on the dedicated PeaZip_API36 emulator")
        if self.adb("shell", "getprop", "ro.build.version.sdk").strip() != "36":
            raise RuntimeError("Packed smoke requires API 36")
        apk = self.out / "tested.apk"
        shutil.copyfile(self.args.apk.resolve(), apk)
        digest = hashlib.sha256(apk.read_bytes()).hexdigest()
        (self.out / "apk.sha256").write_text(f"{digest}  {apk.name}\n")
        print(f"Packed smoke artifacts: {self.out}; APK SHA256 {digest}", flush=True)
        fixtures = list(self.fixtures())
        self.adb("install", "-r", str(apk.resolve()))
        self.adb("shell", "am", "force-stop", smoke_android.PACKAGE)
        self.started = self.adb("shell", "date", "'+%m-%d %H:%M:%S.000'").strip()
        self.adb("shell", "am", "start", "-n",
                 f"{smoke_android.PACKAGE}/com.longbridge.nsh.PeaZipActivity")
        self.label("Open archive…")
        self.flat_view()
        self.health()
        self.adb("shell", "mkdir", "-p", "/sdcard/Documents")
        self.adb("shell", "mkdir", self.export_parent)
        self.export_owned = True
        sentinel = self.out / "sentinel.bin"
        sentinel.write_bytes(b"Packed smoke preserves previous exports\0\xff\n")
        self.adb("push", sentinel, f"{self.export_parent}/sentinel.bin")
        exports = self.export_snapshot("initial")
        assert exports == {"sentinel.bin": sentinel.read_bytes()}

        for kind, source, contents, packed, selected in fixtures:
            print(f"RUN {kind}: import, compact Size, Packed sorting, SAF export, restore columns", flush=True)
            name = f"peazip-{self.token}-{kind}{''.join(source.suffixes)}"
            remote = f"/sdcard/Download/{name}"
            self.files.append(remote)
            self.adb("push", source, remote)
            self.scan(remote)
            self.entries = list(contents)
            self.choose(name)
            self.archive(name, directory=False)
            self.menu_select("Layout", "Sort by name")
            self.cells(contents, packed, "Size")
            self.save(f"{kind}-compact-original-size")
            self.tap(selected)
            self.tap("empty")
            states = {path: path in (selected, "empty") for path in contents}
            self.rows(states)
            self.label("2 selected")
            for descending in (False, True):
                self.menu_select("Layout", "Sort by packed size")
                self.cells(contents, packed, "Packed")
                self.sorted_rows(contents, packed, descending)
                self.rows(states)
                self.label("2 selected")
                unknown = any(data is not None and packed[path] is None for path, data in contents.items())
                tree = self.dump()
                if any(self.match(node, "— means the file's packed size is unavailable.")
                       for node in tree.iter("node")) != unknown:
                    raise AssertionError(f"{kind} unknown-size explanation does not match actual coverage")
                self.save(f"{kind}-packed-{'descending' if descending else 'ascending'}")
                print(f"PASS {kind} Packed {'descending' if descending else 'ascending'}: "
                      "all values/order and selection verified", flush=True)
            self.extract_dialog(scope="Selected entries", paths="Full paths")
            exports = ExtractOptionsSmoke.authorize(
                self, 2, {selected: contents[selected], "empty": b""}, exports, f"{kind}-packed-export")
            self.rows(states)
            self.label("2 selected")
            for restore in ("Sort by size", "Sort by name"):
                self.menu_select("Layout", restore)
                self.cells(contents, packed, "Size")
                self.rows(states)
                self.label("2 selected")
                self.save(f"{kind}-restored-{restore.rsplit(' ', 1)[-1]}")
            self.label(name)
            if self.adb("exec-out", "cat", remote, binary=True) != source.read_bytes():
                raise AssertionError(f"{kind} source changed after sorting/extraction")
            self.health()
            print(f"PASS {kind}: packed={packed}; both sort orders, compact columns, selection, "
                  "source bytes and actual SAF export verified", flush=True)
        if hashlib.sha256(self.args.apk.read_bytes()).hexdigest() != digest:
            raise AssertionError("APK changed during smoke; rerun with the final build")


if __name__ == "__main__":
    smoke_android.Smoke = PackedSmoke
    sys.exit(smoke_android.main())
