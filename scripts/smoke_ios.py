"""Install, launch and capture PeaZip on an explicitly named, dedicated iOS simulator."""

import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import re
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
APP_ID = "io.peazip.rust"


def dedicated_device(devices, udid):
    for runtime, rows in devices.items():
        for device in rows:
            if device["udid"] == udid:
                if not device["name"].startswith("PeaZip-") or not device.get("isAvailable", False):
                    raise ValueError("The selected device must be an available, dedicated PeaZip-* simulator")
                return {**device, "runtime": runtime}
    raise ValueError("The requested simulator UDID does not exist")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--udid", required=True, help="Explicit UDID of a dedicated PeaZip-* simulator")
    parser.add_argument("--app", type=Path, default=ROOT / "target/ios/aarch64-apple-ios-sim/release/PeaZip.app")
    parser.add_argument("--output", type=Path, default=ROOT / "target/ios-smoke")
    args = parser.parse_args()
    app = args.app.resolve()
    plist = plistlib.loads((app / "Info.plist").read_bytes())
    if plist.get("CFBundleIdentifier") != APP_ID or plist.get("CFBundleSupportedPlatforms") != ["iPhoneSimulator"]:
        raise ValueError("Expected the PeaZip iPhoneSimulator app, not a device build")
    binary = app / plist["CFBundleExecutable"]
    original_hash = hashlib.sha256(binary.read_bytes()).hexdigest()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)

    def run(*command):
        result = subprocess.run(command, cwd=ROOT, text=True, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, timeout=180)
        with (output / "commands.log").open("a") as log:
            log.write(json.dumps(list(command)) + "\n" + result.stdout + "\n")
        result.check_returncode()
        return result.stdout

    devices = json.loads(run("xcrun", "simctl", "list", "devices", "-j"))["devices"]
    device = dedicated_device(devices, args.udid)
    booted_here = device["state"] == "Shutdown"
    if device["state"] not in ("Booted", "Shutdown"):
        raise ValueError(f"Simulator is transitioning: {device['state']}")
    started_here = False
    try:
        run("codesign", "--verify", "--strict", str(app))
        build = run("xcrun", "vtool", "-show-build", str(binary))
        if not re.search(r"\bplatform\s+IOSSIMULATOR\b", build):
            raise ValueError("The executable is not built for iOS Simulator")
        if booted_here:
            run("xcrun", "simctl", "boot", args.udid)
            started_here = True
        run("xcrun", "simctl", "bootstatus", args.udid, "-b")
        run("xcrun", "simctl", "install", args.udid, str(app))
        launched = run("xcrun", "simctl", "launch", "--terminate-running-process", args.udid, APP_ID)
        match = re.search(rf"{re.escape(APP_ID)}: (\d+)", launched)
        if not match:
            raise ValueError("simctl did not report the application PID")
        pid = int(match[1])
        for name in ("launch", "settled"):
            time.sleep(2)
            processes = run("xcrun", "simctl", "spawn", args.udid, "launchctl", "list")
            if not any(line.split()[:2] == [str(pid), "0"] and APP_ID in line
                       for line in processes.splitlines()):
                raise ValueError("PeaZip did not remain running after launch")
            run("xcrun", "simctl", "io", args.udid, "screenshot", str(output / f"{name}.png"))
        if hashlib.sha256(binary.read_bytes()).hexdigest() != original_hash:
            raise ValueError("The app changed during verification")
        source_files = sorted((ROOT / "crates/peazip-ui/src").glob("*.rs"))
        source_files += [ROOT / "Cargo.lock", ROOT / "crates/peazip-ios/src/main.rs", Path(__file__).resolve()]
        evidence = {
            "scope": "SDK platform/signature, installation, sustained process and screenshots; no picker interaction",
            "commit": run("git", "rev-parse", "HEAD").strip(),
            "device": device, "pid": pid, "binary_sha256": original_hash,
            "xcode": run("xcodebuild", "-version").strip(),
            "source_sha256": {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in source_files},
            "screenshots": {name: hashlib.sha256((output / name).read_bytes()).hexdigest()
                            for name in ("launch.png", "settled.png")},
        }
    finally:
        if started_here:
            run("xcrun", "simctl", "shutdown", args.udid)
    evidence["shutdown_performed"] = started_here
    (output / "evidence.json").write_text(json.dumps(evidence, ensure_ascii=False, indent=2) + "\n")
    print(f"PASS iOS Simulator install/launch PID {pid}: {output}")


if __name__ == "__main__":
    main()
