"""Build and run separate bridge instrumentation on an explicitly selected test device."""

import argparse
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess

import check_android_bridge


def check(args):
    args.build_device_tests = True
    check_android_bridge.check(args)
    root = check_android_bridge.ROOT
    sdk_default = Path.home() / ("Library/Android/sdk" if platform.system() == "Darwin" else "Android/Sdk")
    sdk = Path(args.sdk or os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
               or sdk_default).expanduser().resolve()
    adb = shutil.which("adb") or str(sdk / "platform-tools" / ("adb.exe" if os.name == "nt" else "adb"))
    package = "io.peazip.rust.bridgechecks"
    artifact = root / "target/android-bridge/device/peazip-bridge-checks.apk"

    def device(*command):
        result = subprocess.run([adb, "-s", args.serial, *map(str, command)], check=True,
                                capture_output=True, text=True, timeout=120)
        return result.stdout

    if args.apk:
        device("install", "-r", args.apk)
    device("install", "-r", artifact)
    try:
        output = device("shell", "am", "instrument", "-w", "-r",
                        f"{package}/{package}.ReliablePipeChecks")
        (artifact.parent / "result.txt").write_text(output)
        print(output)
        if not re.search(r"INSTRUMENTATION_RESULT: stream=PASS: \d+ real Android reliable-pipe/regular-file checks", output) or "INSTRUMENTATION_CODE: -1" not in output:
            raise ValueError("Device checks failed; see target/android-bridge/device/result.txt")
    finally:
        device("uninstall", package)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--apk", type=Path, help="Install this app APK before checking")
    parser.add_argument("--sdk")
    parser.add_argument("--jdk")
    parser.add_argument("--api", type=int, default=35)
    try:
        check(parser.parse_args())
    except (OSError, ValueError, StopIteration, subprocess.SubprocessError) as error:
        parser.exit(1, f"Android device checks failed: {error}\n")
