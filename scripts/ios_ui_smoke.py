"""Run native XCTest UI smoke against an already installed app on a dedicated simulator."""

import argparse
import datetime
import hashlib
import json
from pathlib import Path
import plistlib
import shutil
import subprocess


ROOT = Path(__file__).resolve().parents[1]
BUNDLE = "io.peazip.rust"


def output(*command):
    return subprocess.check_output(command, cwd=ROOT, text=True, stderr=subprocess.PIPE).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--udid", required=True)
    parser.add_argument("--device-name", required=True)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--fixture", type=Path, help="Seed sample.tar.gz in this simulator's local Files Downloads, then use the real picker")
    args = parser.parse_args()
    if args.fixture and (not args.fixture.is_file() or args.fixture.name != "sample.tar.gz"):
        parser.error("--fixture currently supports the real sample.tar.gz fixture only")
    if not args.device_name.startswith("PeaZip-"):
        parser.error("Only an explicitly named PeaZip- simulator may be used")
    devices = json.loads(output("xcrun", "simctl", "list", "-j", "devices"))["devices"]
    match = [(runtime, device) for runtime, rows in devices.items() for device in rows
             if device["udid"] == args.udid]
    if len(match) != 1 or match[0][1]["name"] != args.device_name:
        parser.error("The exact simulator UDID/name pair does not match")
    runtime, device = match[0]
    if device["state"] != "Booted" or not device.get("isAvailable"):
        parser.error("Boot this dedicated device with the base smoke script first")
    app = args.app.resolve()
    with (app / "Info.plist").open("rb") as stream:
        info = plistlib.load(stream)
    if info.get("CFBundleIdentifier") != BUNDLE or "iPhoneSimulator" not in info.get("CFBundleSupportedPlatforms", []):
        parser.error("--app must be the PeaZip simulator app")
    installed = Path(output("xcrun", "simctl", "get_app_container", args.udid, BUNDLE, "app"))
    executable = info["CFBundleExecutable"]
    digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
    if digest(app / executable) != digest(installed / executable):
        parser.error("Installed binary differs from --app; run the base smoke installation first")
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    evidence = (args.out or ROOT / "target" / f"ios-ui-smoke-{stamp}").resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    project = evidence / "project"
    shutil.copytree(ROOT / "scripts/ios-smoke", project)
    manifest = {"udid": args.udid, "device": device, "runtime": runtime, "app": str(app),
                "binary_sha256": digest(app / executable), "xcode": output("xcodebuild", "-version"),
                "git_head": output("git", "rev-parse", "HEAD"), "started_utc": stamp,
                "scope": "XCTest local-provider gzip TAR import and navigation" if args.fixture else "XCTest real Open archive button and UIDocumentPicker cancellation"}
    (evidence / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    (evidence / "source-status.txt").write_text(output("git", "status", "--short") + "\n")
    (evidence / "source.diff").write_text(output("git", "diff", "--binary"))
    source_hashes = {str(path.relative_to(ROOT)): digest(path)
                     for path in (ROOT / "scripts/ios-smoke").rglob("*") if path.is_file()}
    source_hashes["scripts/ios_ui_smoke.py"] = digest(Path(__file__).resolve())
    (evidence / "harness-sha256.json").write_text(json.dumps(source_hashes, indent=2) + "\n")
    common = ["xcodebuild", "-project", str(project / "Smoke.xcodeproj"), "-scheme", "Smoke",
              "-sdk", "iphonesimulator", "-destination", f"id={args.udid}",
              "-derivedDataPath", str(evidence / "DerivedData"),
              "-parallel-testing-enabled", "NO", "CODE_SIGNING_ALLOWED=NO"]
    print(f"XCTest evidence: {evidence}", flush=True)
    seeded = None
    try:
        if args.fixture:
            data = Path(device["dataPath"]).resolve()
            downloads = list((data / "Containers/Shared/AppGroup").glob("*/File Provider Storage/Downloads"))
            if len(downloads) != 1 or not downloads[0].resolve().is_relative_to(data):
                raise ValueError("Expected one existing local FileProvider Downloads inside this device; refusing other destinations")
            filename = f"peazip-smoke-{stamp}.tar.gz"
            candidate = downloads[0] / filename
            payload = args.fixture.read_bytes()
            manifest.update(fixture=str(args.fixture.resolve()), fixture_sha256=hashlib.sha256(payload).hexdigest(),
                            fixture_seed_method="local-provider-filesystem", fixture_seed_path=str(candidate))
            with candidate.open("xb") as stream:
                seeded = candidate
                stream.write(payload)
            swift = project / "Smoke.swift"
            swift.write_text(swift.read_text().replace("__FIXTURE_NAME__", filename))
        tested_hashes = {str(path.relative_to(project)): digest(path)
                         for path in project.rglob("*") if path.is_file()}
        (evidence / "tested-harness-sha256.json").write_text(json.dumps(tested_hashes, indent=2) + "\n")
        for action in ("build-for-testing", "test-without-building"):
            command = [*common, action]
            if action == "test-without-building":
                if args.fixture:
                    command += ["-only-testing:Smoke/Smoke/testImportAndNavigate",
                                "-only-testing:Smoke/Smoke/testOpenAndCancel"]
                else:
                    command += ["-only-testing:Smoke/Smoke/testOpenAndCancel"]
                command += ["-resultBundlePath", str(evidence / "Smoke.xcresult")]
            (evidence / f"{action}.command.json").write_text(json.dumps(command, indent=2) + "\n")
            with (evidence / f"{action}.log").open("w") as stream:
                subprocess.run(command, cwd=ROOT, stdout=stream, stderr=subprocess.STDOUT,
                               check=True, timeout=600)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        manifest.update(result="FAIL", error=str(error))
        raise
    else:
        manifest["result"] = "PASS"
    finally:
        if seeded is not None:
            try:
                if not seeded.exists() and not seeded.is_symlink():
                    manifest["fixture_cleanup"] = "already-absent"
                elif seeded.is_symlink() or digest(seeded) != manifest["fixture_sha256"]:
                    manifest.update(result="FAIL", fixture_cleanup="retained-modified-file")
                else:
                    seeded.unlink()
                    manifest["fixture_cleanup"] = "removed-after-sha256-match"
            except OSError as error:
                manifest.update(result="FAIL", fixture_cleanup=f"error: {error}")
        try:
            capture = subprocess.run(["xcrun", "simctl", "io", args.udid, "screenshot", str(evidence / "final.png")],
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30)
            manifest["final_screenshot_exit_code"] = capture.returncode
            capture.check_returncode()
            result = evidence / "Smoke.xcresult"
            if result.exists():
                with (evidence / "export.log").open("w") as stream:
                    exported = subprocess.run(["xcrun", "xcresulttool", "export", "attachments", "--path", str(result),
                                               "--output-path", str(evidence / "attachments")], stdout=stream,
                                              stderr=subprocess.STDOUT, timeout=60)
                    manifest["attachment_export_exit_code"] = exported.returncode
                    exported.check_returncode()
        except (OSError, subprocess.SubprocessError) as error:
            manifest.update(result="FAIL", evidence_error=str(error))
        manifest["finished_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        (evidence / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    if manifest["result"] != "PASS":
        raise RuntimeError(f"Smoke cleanup or evidence capture failed: {evidence}")
    print(f"PASS: {'real gzip TAR imported/navigated; ' if args.fixture else ''}picker opened/cancelled twice; {evidence}", flush=True)


if __name__ == "__main__":
    main()
