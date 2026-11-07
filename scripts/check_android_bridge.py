"""Opt-in host checks: HotSpot JDK 17+, Android SDK platform 35, Rust/Cargo; no NDK or device."""

import argparse
import json
import os
from pathlib import Path
import platform
import shlex
import shutil
import subprocess
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
TESTS = ROOT / "tests/android-bridge"


def build_device_checks(sdk, jdk, jar, suffix, run):
    """Build only. A separate instrumentation APK exercises real pipes without app test hooks."""
    versions = [path for path in (sdk / "build-tools").iterdir()
                if path.is_dir() and all(part.isdigit() for part in path.name.split("."))]
    build_tools = max(versions, key=lambda path: tuple(map(int, path.name.split("."))))
    out = ROOT / "target/android-bridge/device"
    out.mkdir(parents=True, exist_ok=True)
    keystore = ROOT / "target/android/debug.keystore"
    if not keystore.is_file():
        raise ValueError("Build the app APK first to create its matching debug signing key")

    def tool(name):
        extension = (".bat" if name in ("d8", "apksigner") else ".exe") if os.name == "nt" else ""
        return build_tools / (name + extension)

    with tempfile.TemporaryDirectory(prefix="classes-", dir=out) as directory:
        classes = Path(directory)
        run([jdk / "bin" / f"javac{suffix}", "--release", "17", "-encoding", "UTF-8", "-classpath", jar,
             "-d", classes, *sorted((TESTS / "device").rglob("*.java"))])
        java_jar = out / "checks.jar"
        run([jdk / "bin" / f"jar{suffix}", "cf", java_jar, "-C", classes, "."])
        run([tool("d8"), "--min-api", "28", "--lib", jar, "--output", out, java_jar])
    unsigned = out / "unsigned.apk"
    run([tool("aapt2"), "link", "--manifest", TESTS / "device/AndroidManifest.xml", "-I", jar,
         "--min-sdk-version", "28", "--target-sdk-version", "35", "--debug-mode", "-o", unsigned])
    with zipfile.ZipFile(unsigned, "a") as apk:
        apk.write(out / "classes.dex", "classes.dex", compress_type=zipfile.ZIP_DEFLATED)
    aligned = out / "aligned.apk"
    run([tool("zipalign"), "-P", "16", "-f", "4", unsigned, aligned])
    artifact = out / "peazip-bridge-checks.apk"
    run([tool("apksigner"), "sign", "--ks", keystore, "--ks-key-alias", "androiddebugkey", "--ks-pass", "pass:android",
         "--key-pass", "pass:android", "--out", artifact, aligned])
    run([tool("apksigner"), "verify", artifact])
    run([tool("zipalign"), "-c", "-P", "16", "4", artifact])
    print(f"READY (not installed/run): {artifact}", flush=True)
    print("Device command after installing app and test APKs: adb shell am instrument -w "
          "io.peazip.rust.bridgechecks/io.peazip.rust.bridgechecks.ReliablePipeChecks", flush=True)


def check(args):
    sdk_default = Path.home() / ("Library/Android/sdk" if platform.system() == "Darwin" else "Android/Sdk")
    sdk = Path(args.sdk or os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT") or sdk_default).expanduser().resolve()
    jar = sdk / "platforms" / f"android-{args.api}" / "android.jar"
    if not jar.is_file():
        raise ValueError(f"Install Android SDK platform android-{args.api}: {jar}")
    studio_jdk = Path("/Applications/Android Studio.app/Contents/jbr/Contents/Home")
    javac = shutil.which("javac")
    fallback = studio_jdk if studio_jdk.is_dir() else (Path(javac).resolve().parent.parent if javac else None)
    jdk_value = args.jdk or os.environ.get("JAVA_HOME") or fallback
    if not jdk_value:
        raise ValueError("Set JAVA_HOME or --jdk to a JDK 17 or newer")
    jdk = Path(jdk_value).expanduser().resolve()
    suffix = ".exe" if os.name == "nt" else ""
    for name in ("java", "javac"):
        if not (jdk / "bin" / f"{name}{suffix}").is_file():
            raise ValueError(f"JDK tool missing: {jdk / 'bin' / name}")
    env = dict(os.environ)
    env["JAVA_HOME"] = str(jdk)
    env["PATH"] = str(jdk / "bin") + os.pathsep + env.get("PATH", "")

    def run(command):
        command = list(map(str, command))
        print("+ " + shlex.join(command), flush=True)
        subprocess.run(command, cwd=ROOT, env=env, check=True)

    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", "aarch64-linux-android",
    ], cwd=ROOT, env=env, text=True))
    runtime = next(package for package in metadata["packages"] if package["name"] == "nsh-platform-android")
    sources = sorted((Path(runtime["manifest_path"]).parent / "java").rglob("*.java"))
    if not sources:
        raise ValueError("Locked nsh Android Java sources are missing")
    sources += sorted((ROOT / "crates/peazip-android/java").rglob("*.java"))
    sources += sorted((TESTS / "java").rglob("*.java"))
    # Keep these checks separate from the APK/device-smoke build artifacts.
    out = ROOT / "target/android-bridge"
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="java-", dir=out) as directory:
        classes = Path(directory)
        source_list = classes / "sources.args"
        source_list.write_text("\n".join('"' + str(path).replace("\\", "\\\\").replace('"', '\\"') + '"' for path in sources), encoding="utf-8")
        run([jdk / "bin" / f"javac{suffix}", "--release", "17", "-encoding", "UTF-8",
             "-classpath", jar, "-d", classes, f"@{source_list}"])
        # Only the host harness replaces these Android primitives; APK builds use the SDK.
        run([jdk / "bin" / f"javac{suffix}", "--release", "17", "-d", classes,
             *sorted((TESTS / "host-android").rglob("*.java"))])
        bytecode = subprocess.check_output([str(jdk / "bin" / f"javap{suffix}"), "-private", "-c", "-classpath", str(classes),
                                            "com.longbridge.nsh.PeaZipActivity"], cwd=ROOT, env=env, text=True)
        for method in ("ContentResolver.query:", "ContentResolver.openAssetFileDescriptor:"):
            if not any(method in line and "CancellationSignal;" in line for line in bytecode.splitlines()):
                raise ValueError(f"Production bytecode is missing cancellable {method}")
        if not all(method in bytecode for method in
                ("AssetFileDescriptor.createInputStream:", "AssetFileDescriptor.createOutputStream:", "AssetFileDescriptor.getDeclaredLength:", "AssetFileDescriptor.getStartOffset:",
                 "AssetFileDescriptor.getParcelFileDescriptor:", "ParcelFileDescriptor.canDetectErrors:", "ParcelFileDescriptor.checkError:")):
            raise ValueError("Production bytecode must preserve asset stream boundaries")
        tree_bytecode = subprocess.check_output([str(jdk / "bin" / f"javap{suffix}"), "-private", "-c", "-classpath", str(classes),
                                                 "com.longbridge.nsh.SafTreeExport"], cwd=ROOT, env=env, text=True)
        for method in ("ContentResolver.query:", "ContentResolver.openAssetFileDescriptor:"):
            if not any(method in line and "CancellationSignal;" in line for line in tree_bytecode.splitlines()):
                raise ValueError(f"Tree export is missing cancellable {method}")
        if not all(method in tree_bytecode for method in ("DocumentsContract.buildDocumentUriUsingTree:",
                "DocumentsContract.buildChildDocumentsUriUsingTree:", "DocumentsContract.createDocument:", "PeaZipActivity.writeAsset:")):
            raise ValueError("Tree export must use opaque document IDs and the shared whole-document writer")
        print("PASS: compiled query/open use CancellationSignal; input/output use bounded asset stream factories", flush=True)
        run([jdk / "bin" / f"java{suffix}", "-ea", "-classpath",
             os.pathsep.join(map(str, [classes, jar])), "BridgeReadCheck"])
        run([jdk / "bin" / f"java{suffix}", "-ea", "-classpath",
             os.pathsep.join(map(str, [classes, jar])), "BridgeAssetCheck"])
        run([jdk / "bin" / f"java{suffix}", "-ea", "-classpath",
             os.pathsep.join(map(str, [classes, jar])), "com.longbridge.nsh.TreeExportCheck"])
        host = next(line.removeprefix("host: ") for line in subprocess.check_output(
            ["rustc", "-vV"], cwd=ROOT, env=env, text=True).splitlines() if line.startswith("host: "))
        run(["cargo", "run", "--locked", "--manifest-path", TESTS / "Cargo.toml",
             "--target", host, "--target-dir", out / "rust", "--", os.pathsep.join(map(str, [classes, jar]))])

    if args.build_device_tests:
        build_device_checks(sdk, jdk, jar, suffix, run)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-device-tests", action="store_true", help="Also build a separate reliable-pipe instrumentation APK; never installs/runs it")
    parser.add_argument("--sdk", help="Android SDK root (or ANDROID_HOME / ANDROID_SDK_ROOT)")
    parser.add_argument("--jdk", help="Host JDK root, including libjvm (or JAVA_HOME)")
    parser.add_argument("--api", type=int, default=35)
    try:
        check(parser.parse_args())
    except (OSError, ValueError, StopIteration, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Android bridge checks failed: {error}\n")
