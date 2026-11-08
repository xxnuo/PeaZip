"""Run native XCTest UI smoke against an already installed app on a dedicated simulator."""

import argparse
import datetime
import gzip
import hashlib
import io
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import tarfile
import zipfile


ROOT = Path(__file__).resolve().parents[1]
BUNDLE = "io.peazip.rust"
CREATE_INPUTS = {"输入你好.txt": "iOS 创建你好\n".encode(),
                 "data.bin": bytes([0, 127, 255]) + bytes(range(256)), "empty.txt": b""}


def read_export_tree(root):
    files, directories = {}, set()
    for path in root.rglob("*"):
        if path.is_symlink():
            raise ValueError(f"Unexpected symlink in real directory export: {path}")
        name = path.relative_to(root).as_posix()
        if path.is_dir():
            directories.add(name)
        elif path.is_file():
            files[name] = path.read_bytes()
        else:
            raise ValueError(f"Unexpected export entry: {path}")
    return files, directories


def output(*command):
    return subprocess.check_output(command, cwd=ROOT, text=True, stderr=subprocess.PIPE).strip()


def local_downloads(device):
    data = Path(device["dataPath"]).resolve()
    roots = []
    for storage in (data / "Containers/Shared/AppGroup").glob("*/File Provider Storage"):
        metadata = storage.parent / ".com.apple.mobile_container_manager.metadata.plist"
        if metadata.is_file() and plistlib.loads(metadata.read_bytes()).get("MCMMetadataIdentifier") == "group.com.apple.FileProvider.LocalStorage":
            roots.append(storage)
    if len(roots) != 1 or not roots[0].resolve().is_relative_to(data):
        raise ValueError("Expected exactly one system local FileProvider inside this device; refusing other destinations")
    downloads = roots[0] / "Downloads"
    if downloads.is_symlink():
        raise ValueError("Refusing a symlink as the fixture destination")
    downloads.mkdir(exist_ok=True)
    return downloads


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--udid", required=True)
    parser.add_argument("--device-name", required=True)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--fixture", type=Path, help="Seed sample.tar.gz in this simulator's local Files Downloads, then use the real picker")
    parser.add_argument("--create-save", action="store_true", help="Create and export ZIP/TAR/TAR.GZ through real system pickers; independently verify every exported byte")
    parser.add_argument("--extract", action="store_true", help="Import TAR.GZ/7Z and verify Selected/Files only/Displayed/All directory exports and cancellations")
    parser.add_argument("--extract-case", choices=("selected", "files-only", "displayed", "all"), help="Run one --extract case for focused diagnosis")
    parser.add_argument("--cancel-export-only", action="store_true", help="Dismiss the current real export sheet by gesture and verify the existing archive snapshot/selection")
    parser.add_argument("--save-existing-zip", action="store_true", help="Export the existing three-file ZIP snapshot to the local provider root and independently verify it")
    args = parser.parse_args()
    if args.extract_case and not args.extract:
        parser.error("--extract-case requires --extract")
    if args.extract and any((args.create_save, args.cancel_export_only, args.save_existing_zip, args.fixture)):
        parser.error("--extract seeds its own real archive fixtures and runs independently")
    if args.cancel_export_only and (args.fixture or args.create_save):
        parser.error("--cancel-export-only uses the existing app snapshot and cannot seed/create another archive")
    if args.save_existing_zip and (args.fixture or args.create_save or args.cancel_export_only):
        parser.error("--save-existing-zip is a single existing-snapshot export probe")
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
    installed_info = plistlib.loads((installed / "Info.plist").read_bytes())
    for key in ("CFBundleIdentifier", "CFBundleExecutable", "UIFileSharingEnabled", "LSSupportsOpeningDocumentsInPlace"):
        if installed_info.get(key) != info.get(key):
            parser.error("Installed app metadata differs from --app; reinstall the latest package first")
    if (args.create_save or args.save_existing_zip) and not all(installed_info.get(key) for key in ("UIFileSharingEnabled", "LSSupportsOpeningDocumentsInPlace")):
        parser.error("Create/save smoke requires the production Files-sharing package")
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
                "binary_sha256": digest(app / executable), "info_plist_sha256": digest(app / "Info.plist"),
                "installed_info_plist_sha256": digest(installed / "Info.plist"), "xcode": output("xcodebuild", "-version"),
                "production_source_sha256": {name: digest(ROOT / name) for name in (
                    "crates/peazip-core/src/lib.rs", "crates/peazip-ui/src/lib.rs",
                    "crates/peazip-ui/src/ios.rs", "crates/peazip-ui/src/mobile_export.rs")
                    if (ROOT / name).is_file()},
                "git_head": output("git", "rev-parse", "HEAD"), "started_utc": stamp,
                "scope": "; ".join(scope for enabled, scope in [
                    (args.create_save, "XCTest real multi-file ZIP/TAR/TAR.GZ creation/export and cancellation; host independent byte verification"),
                    (bool(args.fixture), "XCTest local-provider gzip TAR import/navigation and repeated picker cancellation"),
                    (args.cancel_export_only, "XCTest real export sheet gesture dismissal; existing archive snapshot/selection retained"),
                    (args.save_existing_zip, "XCTest single existing ZIP export to the local provider root; independent byte verification"),
                    (args.extract, "XCTest selected directory extraction/export via real pickers; host independent tree/byte verification"),
                    (not any((args.fixture, args.create_save, args.cancel_export_only, args.save_existing_zip, args.extract)), "XCTest real Open archive button and repeated picker cancellation")
                ] if enabled)}
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
    seeds = []
    created_directories = []
    exported_root = None
    try:
        if args.fixture or args.create_save or args.save_existing_zip or args.extract:
            downloads = local_downloads(device)
        if args.extract:
            exported_root = downloads.parent
            export_inventory_before = {path.name for path in exported_root.iterdir()}
            replacements = {}
            fixtures = []
            for suffix, token in (("tar.gz", "__EXTRACT_TAR_NAME__"), ("7z", "__EXTRACT_7Z_NAME__")):
                source = ROOT / f"tests/fixtures/sample.{suffix}"
                candidate = downloads / f"peazip-extract-{stamp}.{suffix}"
                payload = source.read_bytes()
                with candidate.open("xb") as stream:
                    seeds.append((candidate, hashlib.sha256(payload).hexdigest()))
                    stream.write(payload)
                replacements[token] = candidate.name
                fixtures.append({"source": str(source), "seed": str(candidate), "sha256": digest(source)})
            swift = project / "Smoke.swift"
            swift_source = swift.read_text()
            for token, name in replacements.items():
                swift_source = swift_source.replace(token, name)
            swift.write_text(swift_source)
            directories = {"docs", "docs/empty", "docs/nested"}
            tar_files = {"docs/你好.txt": "tar 你好\n".encode(), "docs/nested/data.bin": bytes([0, 127, 255])}
            seven_files = {**tar_files, "docs/你好.txt": "7z 你好\n".encode()}
            extract_cases = {
                "selected": ("testExtractSelectedDirectory", tar_files, directories),
                "files-only": ("testExtractFilesOnly", {Path(name).name: data for name, data in seven_files.items()}, set()),
                "displayed": ("testExtractDisplayedFrozen", {"docs/你好.txt": tar_files["docs/你好.txt"]}, {"docs"}),
                "all": ("testExtractAll", {**seven_files, "other.txt": b"not selected\n"}, directories),
            }
            if args.extract_case:
                extract_cases = {args.extract_case: extract_cases[args.extract_case]}
            manifest.update(extract_fixtures=fixtures, export_root=str(exported_root), extract_cases=list(extract_cases))
        if args.fixture:
            filename = f"peazip-smoke-{stamp}.tar.gz"
            candidate = downloads / filename
            payload = args.fixture.read_bytes()
            manifest.update(fixture=str(args.fixture.resolve()), fixture_sha256=hashlib.sha256(payload).hexdigest(),
                            fixture_seed_method="local-provider-filesystem", fixture_seed_path=str(candidate))
            with candidate.open("xb") as stream:
                seeds.append((candidate, hashlib.sha256(payload).hexdigest()))
                stream.write(payload)
            swift = project / "Smoke.swift"
            swift.write_text(swift.read_text().replace("__FIXTURE_NAME__", filename))
        if args.create_save:
            input_root = downloads / f"peazip-input-{stamp}"
            exported_root = downloads.parent
            export_inventory_before = {path.name for path in exported_root.iterdir()}
            for filename in ("Archive.zip", "Archive.tar", "Archive.tar.gz"):
                if (exported_root / filename).exists() or (exported_root / filename).is_symlink():
                    raise ValueError(f"Refusing to overwrite an existing test destination: {filename}")
            input_root.mkdir()
            created_directories.append(input_root)
            for name, payload in CREATE_INPUTS.items():
                path = input_root / name
                with path.open("xb") as stream:
                    seeds.append((path, hashlib.sha256(payload).hexdigest()))
                    stream.write(payload)
            manifest.update(create_seed_method="local-provider-filesystem", create_input_root=str(input_root),
                            export_root=str(exported_root), expected_input_sha256={name: hashlib.sha256(payload).hexdigest()
                                                                                 for name, payload in CREATE_INPUTS.items()})
            swift = project / "Smoke.swift"
            swift.write_text(swift.read_text().replace("__CREATE_INPUT_FOLDER__", input_root.name)
                             .replace("__CREATE_OUTPUT_FOLDER__", exported_root.name))
        if args.save_existing_zip:
            exported_root = downloads.parent
            export_inventory_before = {path.name for path in exported_root.iterdir()}
            if (exported_root / "Archive.zip").exists() or (exported_root / "Archive.zip").is_symlink():
                raise ValueError("Refusing to overwrite an existing ZIP in the dedicated provider")
            manifest["export_root"] = str(exported_root)
        tested_hashes = {str(path.relative_to(project)): digest(path)
                         for path in project.rglob("*") if path.is_file()}
        (evidence / "tested-harness-sha256.json").write_text(json.dumps(tested_hashes, indent=2) + "\n")
        for action in ("build-for-testing", "test-without-building"):
            command = [*common, action]
            if action == "test-without-building":
                if args.extract:
                    command += [f"-only-testing:Smoke/Smoke/{case[0]}" for case in extract_cases.values()]
                if args.save_existing_zip:
                    command += ["-only-testing:Smoke/Smoke/testSaveExistingZip"]
                if args.cancel_export_only:
                    command += ["-only-testing:Smoke/Smoke/testExportCancellationGesture"]
                if args.create_save:
                    command += ["-only-testing:Smoke/Smoke/testCreateAndSaveFormats", "-only-testing:Smoke/Smoke/testExportCancellationGesture"]
                if args.fixture:
                    command += ["-only-testing:Smoke/Smoke/testImportAndNavigate",
                                "-only-testing:Smoke/Smoke/testOpenAndCancel"]
                elif not any((args.create_save, args.cancel_export_only, args.save_existing_zip, args.extract)):
                    command += ["-only-testing:Smoke/Smoke/testOpenAndCancel"]
                command += ["-resultBundlePath", str(evidence / "Smoke.xcresult")]
            (evidence / f"{action}.command.json").write_text(json.dumps(command, indent=2) + "\n")
            with (evidence / f"{action}.log").open("w") as stream:
                subprocess.run(command, cwd=ROOT, stdout=stream, stderr=subprocess.STDOUT,
                               check=True, timeout=600)
        if args.extract:
            new_paths = [path for path in exported_root.iterdir() if path.name not in export_inventory_before]
            if len(new_paths) != len(extract_cases):
                raise ValueError(f"Expected {len(extract_cases)} real directory exports, got {new_paths}")
            unmatched = dict(extract_cases)
            verified = []
            for path in new_paths:
                if path.is_symlink() or not path.is_dir() or not path.name.startswith("PeaZip-"):
                    raise ValueError(f"Expected a randomly named PeaZip directory: {path}")
                actual_files, actual_directories = read_export_tree(path)
                matches = [name for name, (_, files, directories) in unmatched.items()
                           if actual_files == files and actual_directories == directories]
                if len(matches) != 1:
                    raise ValueError(f"Export does not match one expected tree: {path}, files={list(actual_files)}, directories={sorted(actual_directories)}")
                case = matches[0]
                del unmatched[case]
                shutil.copytree(path, evidence / path.name)
                verified.append({"case": case, "path": str(path), "directories": sorted(actual_directories),
                    "files": {name: {"size": len(data), "sha256": hashlib.sha256(data).hexdigest()}
                              for name, data in actual_files.items()}})
                for name, data in actual_files.items():
                    seeds.append((path / name, hashlib.sha256(data).hexdigest()))
                created_directories.extend([path, *sorted((path / name for name in actual_directories), key=lambda p: len(p.parts))])
            manifest["verified_extract_exports"] = verified
            (evidence / "verified-extract-exports.json").write_text(json.dumps(verified, ensure_ascii=False, indent=2) + "\n")
        if args.create_save or args.save_existing_zip:
            verified = []
            filenames = ("Archive.zip",) if args.save_existing_zip else ("Archive.zip", "Archive.tar", "Archive.tar.gz")
            for filename in filenames:
                path = exported_root / filename
                if path.is_symlink() or not path.is_file():
                    raise ValueError(f"Real export missing from the selected provider folder: {path}")
                if filename.endswith(".zip"):
                    with zipfile.ZipFile(path) as archive:
                        if len(archive.infolist()) != len(CREATE_INPUTS):
                            raise ValueError("Exported ZIP has missing or duplicate entries")
                        actual = {entry.filename: archive.read(entry) for entry in archive.infolist()}
                else:
                    raw = path.read_bytes()
                    if filename.endswith(".gz"):
                        raw = gzip.decompress(raw)
                    with tarfile.open(fileobj=io.BytesIO(raw), mode="r:") as archive:
                        members = archive.getmembers()
                        if len(members) != len(CREATE_INPUTS) or any(not entry.isfile() for entry in members):
                            raise ValueError("Exported TAR has unexpected entry types or counts")
                        actual = {entry.name: archive.extractfile(entry).read() for entry in members}
                if actual != CREATE_INPUTS:
                    raise ValueError(f"Exported archive contents differ from the three real source files: {path}")
                saved = evidence / filename
                with saved.open("xb") as stream:
                    stream.write(path.read_bytes())
                expected = digest(path)
                verified.append({"path": str(path), "artifact": filename, "sha256": expected,
                                 "entries": {name: {"size": len(data), "sha256": hashlib.sha256(data).hexdigest()}
                                             for name, data in actual.items()}})
                seeds.append((path, expected))
            if args.create_save:
                for name, payload in CREATE_INPUTS.items():
                    if (input_root / name).read_bytes() != payload:
                        raise ValueError(f"Creating/saving changed a source: {name}")
            if {path.name for path in exported_root.iterdir()} != export_inventory_before | set(filenames):
                raise ValueError("Unexpected files were published by the export/cancellation flow")
            manifest["verified_exports"] = verified
            (evidence / "verified-exports.json").write_text(json.dumps(verified, ensure_ascii=False, indent=2) + "\n")
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        manifest.update(result="FAIL", error=str(error))
        raise
    else:
        manifest["result"] = "PASS"
    finally:
        cleanup = []
        for seeded, expected_hash in seeds:
            try:
                if not seeded.exists() and not seeded.is_symlink():
                    state = "already-absent"
                elif seeded.is_symlink() or digest(seeded) != expected_hash:
                    state = "retained-modified-file"
                    manifest["result"] = "FAIL"
                else:
                    seeded.unlink()
                    state = "removed-after-sha256-match"
            except OSError as error:
                state = f"error: {error}"
                manifest["result"] = "FAIL"
            cleanup.append({"path": str(seeded), "result": state})
        if args.fixture and cleanup:
            manifest["fixture_cleanup"] = cleanup[0]["result"]
        for directory in reversed(created_directories):
            try:
                directory.rmdir()
                cleanup.append({"path": str(directory), "result": "removed-own-empty-directory"})
            except OSError:
                cleanup.append({"path": str(directory), "result": "retained-nonempty-directory"})
        manifest["cleanup"] = cleanup
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
    print(f"PASS: {'ZIP/TAR/TAR.GZ created/exported and independently verified; ' if args.create_save else ''}"
          f"{'selected directory exported and independently verified; ' if args.extract else ''}"
          f"{'real gzip TAR imported/navigated; ' if args.fixture else ''}"
          f"{'export gesture cancellation verified; ' if args.cancel_export_only else ''}"
          f"{'existing ZIP exported and independently verified; ' if args.save_existing_zip else ''}"
          f"{'picker opened/cancelled twice; ' if args.fixture or not any((args.create_save, args.cancel_export_only, args.save_existing_zip, args.extract)) else ''}{evidence}", flush=True)


if __name__ == "__main__":
    main()
