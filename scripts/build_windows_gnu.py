"""Build the Windows GUI with the official dynamic WebView2 loader and GNU tools.

Run with uv run python. Requires Cargo/Rust Windows GNU target and MinGW tools
on PATH. No downloads, source patches, or Windows runtime execution.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import struct
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
TARGET = "x86_64-pc-windows-gnu"
SDK_VERSION = "1.0.3800.47"
LOADER_SHA256 = "86545b66cdb0603bc26b626fb9ad610cb6e71f28d468f5ea66df23b03dda96d5"
SDK_SHA256 = "56c9f26bdd07916a2d1949fb58a5c7e434dfa1173577dca879206050c4e718db"
SDK_URL = (f"https://api.nuget.org/v3-flatcontainer/microsoft.web.webview2/{SDK_VERSION}/"
           f"microsoft.web.webview2.{SDK_VERSION}.nupkg")


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


class PE:
    """Read only the PE headers, named exports and direct DLL imports we verify."""

    def __init__(self, path):
        self.data = path.read_bytes()
        if self.data[:2] != b"MZ":
            raise ValueError(f"Not a PE file: {path}")
        pe = self.unpack("<I", 0x3C)[0]
        if self.data[pe:pe + 4] != b"PE\0\0":
            raise ValueError(f"Invalid PE signature: {path}")
        machine, sections, _, _, _, optional_size, characteristics = self.unpack("<HHIIIHH", pe + 4)
        optional = pe + 24
        if machine != 0x8664 or self.unpack("<H", optional)[0] != 0x20B:
            raise ValueError(f"Expected x86-64 PE32+: {path}")
        if optional_size < 128 or self.unpack("<I", optional + 108)[0] < 2:
            raise ValueError("Missing PE import/export directories")
        self.subsystem = self.unpack("<H", optional + 68)[0]
        self.is_dll = bool(characteristics & 0x2000)
        self.sections = [self.unpack("<8sIIIIIIHHI", optional + optional_size + 40 * i)
                         for i in range(sections)]
        self.exports_rva, self.exports_size, self.imports_rva, self.imports_size = self.unpack(
            "<IIII", optional + 112)

    def unpack(self, fmt, offset):
        if offset < 0 or offset + struct.calcsize(fmt) > len(self.data):
            raise ValueError("PE structure outside file")
        return struct.unpack_from(fmt, self.data, offset)

    def offset(self, rva):
        for _, _, address, size, raw, *_ in self.sections:
            if address <= rva < address + size:
                offset = raw + rva - address
                if offset < len(self.data):
                    return offset
        raise ValueError(f"Unmapped PE RVA: {rva:#x}")

    def string(self, rva):
        start = self.offset(rva)
        end = self.data.find(b"\0", start, min(start + 4096, len(self.data)))
        if end < 0:
            raise ValueError("Unterminated PE name")
        return self.data[start:end].decode("ascii")

    def exports(self):
        if not self.exports_rva or self.exports_size < 40:
            raise ValueError("Loader has no export directory")
        directory = self.offset(self.exports_rva)
        count = self.unpack("<I", directory + 24)[0]
        names = self.offset(self.unpack("<I", directory + 32)[0])
        return [self.string(self.unpack("<I", names + 4 * i)[0]) for i in range(count)]

    def imports(self):
        names = []
        if not self.imports_rva:
            return names
        for displacement in range(0, self.imports_size - 19, 20):
            descriptor = self.unpack("<IIIII", self.offset(self.imports_rva + displacement))
            if not any(descriptor):
                return sorted(set(names), key=lambda name: (name.lower(), name))
            names.append(self.string(descriptor[3]))
        raise ValueError("Unterminated PE import directory")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--loader", type=Path, required=True,
                        help=f"Official x64 WebView2Loader.dll {SDK_VERSION}; SHA256 {LOADER_SHA256}")
    parser.add_argument("--loader-license", type=Path, required=True,
                        help="LICENSE.txt from the same official NuGet package")
    parser.add_argument("--loader-notice", type=Path, required=True,
                        help="NOTICE.txt from the same official NuGet package")
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target/windows-build/cargo-target")
    parser.add_argument("--output", type=Path, default=ROOT / "target/windows-build/release")
    parser.add_argument("--debug", action="store_true", help="Build dev profile instead of optimized release")
    parser.add_argument("--cc", default="x86_64-w64-mingw32-gcc")
    parser.add_argument("--cxx", default="x86_64-w64-mingw32-g++")
    parser.add_argument("--ar", default="x86_64-w64-mingw32-ar")
    parser.add_argument("--dlltool", default="x86_64-w64-mingw32-dlltool")
    args = parser.parse_args()
    for path in (args.loader, args.loader_license, args.loader_notice):
        if not path.is_file():
            parser.error(f"Required local input not found: {path}")
    if sha256(args.loader) != LOADER_SHA256:
        parser.error("Loader SHA256 differs from the pinned official x64 DLL")
    loader = PE(args.loader)
    if not loader.is_dll:
        parser.error("Loader must be a DLL")
    exports = loader.exports()
    if "CreateCoreWebView2EnvironmentWithOptions" not in exports or not all(
            re.fullmatch(r"[A-Za-z_]\w*", name) for name in exports):
        parser.error("Unexpected loader exports")
    target_dir, output = args.target_dir.resolve(), args.output.resolve()
    compat = target_dir / "gnu-webview2-loader"
    if any(char.isspace() for char in str(compat)):
        parser.error("Target directory must not contain whitespace (Cargo target RUSTFLAGS)")
    compat.mkdir(parents=True, exist_ok=True)
    output.mkdir(parents=True, exist_ok=True)
    definition = compat / "WebView2Loader.def"
    definition.write_text("LIBRARY WebView2Loader.dll\nEXPORTS\n" +
                          "".join(f"    {name}\n" for name in exports))
    env = os.environ.copy()
    # Scope flags to the Windows target; host build scripts retain their native toolchain.
    env.pop("RUSTFLAGS", None)
    env.pop("CARGO_ENCODED_RUSTFLAGS", None)
    env.update({"CARGO_TARGET_DIR": str(target_dir),
                "CC_x86_64_pc_windows_gnu": args.cc,
                "CXX_x86_64_pc_windows_gnu": args.cxx,
                "AR_x86_64_pc_windows_gnu": args.ar,
                "CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER": args.cc,
                "CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS": f"-Lnative={compat}"})
    log_path = output / "build.log"
    commands = []
    versions = [f"Python {sys.version}\n"]
    with log_path.open("w") as log:
        def run(command, cwd=ROOT):
            commands.append(command)
            log.write(f"cwd={cwd}\n$ {shlex.join(command)}\n")
            log.flush()
            subprocess.run(command, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)

        for command in (["rustc", "-Vv"], ["cargo", "-V"], ["rustup", "target", "list", "--installed"],
                        [args.cc, "--version"], [args.cxx, "--version"],
                        [args.ar, "--version"], [args.dlltool, "--version"]):
            result = subprocess.run(command, env=env, capture_output=True, text=True, check=True)
            versions.append(f"$ {shlex.join(command)}\n{result.stdout}{result.stderr}")
        run([args.dlltool, "--machine", "i386:x86-64", "--input-def", str(definition),
             "--dllname", "WebView2Loader.dll", "--output-lib", str(compat / "libWebView2LoaderStatic.a")],
            cwd=compat)
        lock_hash = sha256(ROOT / "Cargo.lock")
        print("Checking Windows GNU target...", flush=True)
        run(["cargo", "check", "--locked", "-p", "peazip-ui", "--target", TARGET])
        profile = "debug" if args.debug else "release"
        print(f"Building Windows GNU {profile}...", flush=True)
        run(["cargo", "build", "--locked", "-p", "peazip-ui", "--target", TARGET] +
            ([] if args.debug else ["--release"]))
        if sha256(ROOT / "Cargo.lock") != lock_hash:
            raise ValueError("Cargo.lock changed during build")
    binary = target_dir / TARGET / profile / "peazip-ui.exe"
    executable = PE(binary)
    if executable.is_dll or executable.subsystem != 2:
        raise ValueError("Expected a Windows GUI executable")
    dependencies = {"peazip-ui.exe": executable.imports(), "WebView2Loader.dll": loader.imports()}
    if "webview2loader.dll" not in {name.lower() for name in dependencies["peazip-ui.exe"]}:
        raise ValueError("Executable did not import the official dynamic WebView2 loader")
    shutil.copy2(binary, output / "peazip-ui.exe")
    shutil.copy2(args.loader, output / "WebView2Loader.dll")
    if sha256(output / "WebView2Loader.dll") != LOADER_SHA256:
        raise ValueError("Packaged loader changed during build")
    licenses = output / "licenses"
    licenses.mkdir(exist_ok=True)
    for source, name in ((args.loader_license, "WebView2-LICENSE.txt"),
                         (args.loader_notice, "WebView2-NOTICE.txt"), (ROOT / "LICENSE", "PeaZip-LICENSE")):
        shutil.copy2(source, licenses / name)
    shutil.copy2(definition, output / definition.name)
    (output / "tool-versions.txt").write_text("\n".join(versions))
    manifest = {"kind": "GNU compatibility build with official dynamic WebView2 loader",
                "source_patch": False, "windows_runtime_tested": False,
                "target": TARGET, "profile": profile, "machine": "x86-64", "subsystem": "Windows GUI",
                "cargo_lock_sha256": lock_hash, "build_script_sha256": sha256(Path(__file__)),
                "commands": commands,
                "target_environment": {key: env[key] for key in (
                    "CARGO_TARGET_DIR", "CC_x86_64_pc_windows_gnu", "CXX_x86_64_pc_windows_gnu",
                    "AR_x86_64_pc_windows_gnu", "CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER",
                    "CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS")},
                "loader": {"version": SDK_VERSION, "sha256": LOADER_SHA256,
                           "official_sdk_url": SDK_URL, "official_sdk_sha256": SDK_SHA256,
                           "official_sdk_entry": "build/native/x64/WebView2Loader.dll", "exports": exports},
                "direct_dll_imports": dependencies,
                "runtime": "Keep WebView2Loader.dll beside the exe; WebView2 Runtime is required separately. "
                           "Import lists do not cover dynamically loaded modules or prove Windows execution."}
    (output / "build-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    files = [output / name for name in ("peazip-ui.exe", "WebView2Loader.dll", "build-manifest.json",
                                       "build.log", "tool-versions.txt", definition.name)] + sorted(licenses.iterdir())
    (output / "SHA256SUMS").write_text("".join(
        f"{sha256(path)}  {path.relative_to(output)}\n" for path in files))
    print(f"PASS {output / 'peazip-ui.exe'} ({binary.stat().st_size} bytes); x86-64 Windows GUI")
    print("Official loader DLL packaged; Windows runtime not tested.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
