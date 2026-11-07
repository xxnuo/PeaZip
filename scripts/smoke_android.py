"""Run with uv run python scripts/smoke_android.py --serial emulator-5554."""

import argparse
import os
from pathlib import Path
import re
import shutil
import shlex
import subprocess
import sys
import time
import uuid
import xml.etree.ElementTree as ET
import zipfile

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = "io.peazip.rust"


class Smoke:
    def __init__(self, args):
        self.args = args
        self.out = ROOT / "target/android-smoke"
        self.out.mkdir(parents=True, exist_ok=True)
        self.token = uuid.uuid4().hex[:12]
        self.remote_xml = f"/sdcard/peazip-smoke-{self.token}.xml"
        self.files = []
        self.xml = ""
        self.pids = set()
        self.started = None

    def adb(self, *args, check=True, binary=False):
        result = subprocess.run([self.args.adb, "-s", self.args.serial, *map(str, args)],
                                capture_output=True, timeout=self.args.timeout,
                                text=not binary)
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
        self.tap("Open ZIP…")
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
            n.get("text", "").startswith("Saved ZIP: ") or self.match(n, "Replace")
            or n.get("text", "").startswith("Could not save ZIP") for n in t.iter("node")))
        saved = next((n.get("text")[len("Saved ZIP: "):] for n in tree.iter("node")
                      if n.get("text", "").startswith("Saved ZIP: ")), None)
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
        self.archive("Archive.zip", len(contents), f"Saved ZIP: {saved}", directory=False)
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
        self.label("Open ZIP…")
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
            n.get("text", "").startswith("Could not open ZIP:") for n in t.iter("node")))
        self.save("invalid")
        print("PASS invalid ZIP reports Could not open ZIP", flush=True)
        self.health()
        self.choose(sample)
        self.archive(sample)
        self.health()
        print("PASS reopening valid ZIP recovers", flush=True)
        self.create_oversized(sample)
        self.create_save()


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
    print(f"{'FAIL' if failed else 'PASS'} artifacts: {smoke.out}")
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
