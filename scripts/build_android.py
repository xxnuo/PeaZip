"""Build an optimized, debug-signed Android APK from the locked Rust and Java nsh sources."""

import argparse
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[1]
ABIS = {
    "arm64-v8a": ("aarch64-linux-android", "aarch64-linux-android"),
    "armeabi-v7a": ("armv7-linux-androideabi", "armv7a-linux-androideabi"),
    "x86_64": ("x86_64-linux-android", "x86_64-linux-android"),
}


def latest(directory):
    versions = [path for path in directory.iterdir() if path.is_dir() and re.fullmatch(r"\d+(?:\.\d+)*", path.name)]
    if not versions:
        raise ValueError(f"No installed stable tool version in {directory}")
    return max(versions, key=lambda path: tuple(map(int, path.name.split("."))))


def build(args):
    sdk_default = Path.home() / ("Library/Android/sdk" if platform.system() == "Darwin" else "Android/Sdk")
    sdk = Path(args.sdk or os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT") or sdk_default).expanduser().resolve()
    ndk = Path(args.ndk).expanduser().resolve() if args.ndk else latest(sdk / "ndk")
    build_tools = latest(sdk / "build-tools")
    jar = sdk / "platforms" / f"android-{args.api}" / "android.jar"
    if not jar.is_file():
        raise ValueError(f"Install Android SDK platform android-{args.api}: {jar}")
    studio_jdk = Path("/Applications/Android Studio.app/Contents/jbr/Contents/Home")
    java = shutil.which("javac")
    jdk_default = studio_jdk if studio_jdk.is_dir() else (Path(java).resolve().parent.parent if java else None)
    jdk_value = args.jdk or os.environ.get("JAVA_HOME") or jdk_default
    if not jdk_value:
        raise ValueError("Set JAVA_HOME or --jdk to a JDK 17 or newer")
    jdk = Path(jdk_value).expanduser().resolve()
    suffix = ".exe" if os.name == "nt" else ""
    if not (jdk / "bin" / f"javac{suffix}").is_file():
        raise ValueError(f"JDK compiler missing: {jdk}")
    host = {"Darwin": "darwin-x86_64", "Linux": "linux-x86_64", "Windows": "windows-x86_64"}[platform.system()]
    toolchain = ndk / "toolchains" / "llvm" / "prebuilt" / host / "bin"
    target, clang_target = ABIS[args.abi]
    clang = toolchain / f"{clang_target}28-clang{'.cmd' if os.name == 'nt' else ''}"
    if not clang.is_file():
        raise ValueError(f"NDK compiler missing: {clang}")
    env = dict(os.environ)
    env["JAVA_HOME"] = str(jdk)
    env["PATH"] = str(jdk / "bin") + os.pathsep + env.get("PATH", "")
    env[f"CARGO_TARGET_{target.upper().replace('-', '_')}_LINKER"] = str(clang)
    env[f"CC_{target.replace('-', '_')}"] = str(clang)
    env[f"AR_{target.replace('-', '_')}"] = str(toolchain / f"llvm-ar{suffix}")

    def run(command):
        subprocess.run(list(map(str, command)), cwd=ROOT, env=env, check=True)

    def tool(name):
        extension = (".bat" if name in ("d8", "apksigner") else ".exe") if os.name == "nt" else ""
        return build_tools / (name + extension)

    profile = "dev" if args.debug else "release"
    run(["cargo", "build", "-p", "peazip-android", "--target", target, "--profile", profile, "--locked"])
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", target,
    ], cwd=ROOT, env=env, text=True))
    runtime = next(package for package in metadata["packages"] if package["name"] == "nsh-platform-android")
    sources = sorted((Path(runtime["manifest_path"]).parent / "java").rglob("*.java"))
    if not sources:
        raise ValueError("Locked nsh Android Java sources are missing")
    out = ROOT / "target" / "android" / args.abi
    out.mkdir(parents=True, exist_ok=True)
    classes = out / "classes"
    if classes.exists():
        shutil.rmtree(classes)
    classes.mkdir()
    source_list = out / "sources.args"
    source_list.write_text("\n".join('"' + str(path).replace("\\", "\\\\").replace('"', '\\"') + '"' for path in sources), encoding="utf-8")
    run([jdk / "bin" / f"javac{suffix}", "--release", "17", "-Xlint:-options", "-encoding", "UTF-8", "-classpath", jar, "-d", classes, f"@{source_list}"])
    java_jar = out / "classes.jar"
    run([jdk / "bin" / f"jar{suffix}", "cf", java_jar, "-C", classes, "."])
    run([tool("d8"), "--min-api", "28", "--lib", jar, "--output", out, java_jar])
    unsigned = out / "unsigned.apk"
    run([tool("aapt2"), "link", "--manifest", ROOT / "crates/peazip-android/AndroidManifest.xml", "-I", jar,
         "--min-sdk-version", "28", "--target-sdk-version", args.api, "--version-code", "1", "--version-name", "0.1.0",
         "--debug-mode", "-o", unsigned])
    library = Path(metadata["target_directory"]) / target / ("debug" if args.debug else "release") / "libpeazip_android.so"
    with zipfile.ZipFile(unsigned, "a") as apk:
        apk.write(out / "classes.dex", "classes.dex", compress_type=zipfile.ZIP_DEFLATED)
        apk.write(library, f"lib/{args.abi}/libnsh.so", compress_type=zipfile.ZIP_STORED)
        apk.write(ROOT / "LICENSE", "assets/PeaZip-LICENSE", compress_type=zipfile.ZIP_DEFLATED)
        apk.write(Path(runtime["manifest_path"]).parent.parent.parent / "LICENSE-APACHE", "assets/NSH-LICENSE-APACHE", compress_type=zipfile.ZIP_DEFLATED)
    aligned = out / "aligned.apk"
    run([tool("zipalign"), "-P", "16", "-f", "4", unsigned, aligned])
    keystore = ROOT / "target/android/debug.keystore"
    if not keystore.exists():
        run([jdk / "bin" / f"keytool{suffix}", "-genkeypair", "-noprompt", "-keystore", keystore,
             "-storepass", "android", "-keypass", "android", "-alias", "androiddebugkey", "-keyalg", "RSA", "-keysize", "2048",
             "-validity", "10000", "-dname", "CN=PeaZip Development,O=Development,C=US"])
    artifact = out / f"peazip-{args.abi}-debug.apk"
    run([tool("apksigner"), "sign", "--ks", keystore, "--ks-key-alias", "androiddebugkey", "--ks-pass", "pass:android",
         "--key-pass", "pass:android", "--out", artifact, aligned])
    run([tool("apksigner"), "verify", artifact])
    run([tool("zipalign"), "-c", "-P", "16", "4", artifact])
    print(artifact)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--abi", choices=ABIS, default="arm64-v8a")
    parser.add_argument("--sdk")
    parser.add_argument("--ndk")
    parser.add_argument("--jdk")
    parser.add_argument("--api", type=int, default=35)
    parser.add_argument("--debug", action="store_true", help="Use the Rust development profile")
    try:
        build(parser.parse_args())
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Android build failed: {error}\n")
