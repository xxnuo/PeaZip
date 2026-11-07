"""Build the Rust WebAssembly app with the JavaScript host from the same locked nsh revision."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def build(debug=False):
    profile = "dev" if debug else "web"
    subprocess.run([
        "cargo", "build", "--locked", "-p", "peazip-web", "-p", "peazip-worker",
        "--target", "wasm32-unknown-unknown", "--profile", profile,
    ], cwd=ROOT, check=True)
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--format-version", "1", "--locked",
        "--filter-platform", "wasm32-unknown-unknown",
    ], cwd=ROOT, text=True))
    platform = next(package for package in metadata["packages"] if package["name"] == "nsh-platform-web")
    host = Path(platform["manifest_path"]).parent / "web"
    target = Path(metadata["target_directory"])
    output = ROOT / "target" / "web-dist"
    output.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(target / "wasm32-unknown-unknown" / ("debug" if debug else "web") / "peazip_web.wasm", output / "peazip.wasm")
    shutil.copyfile(host / "nsh.js", output / "nsh.js")
    shutil.copyfile(ROOT / "web" / "index.html", output / "index.html")
    for name in ["archive-bridge.js", "archive-worker.js", "archive-limits.js", "directory-export.js"]:
        shutil.copyfile(ROOT / "web" / name, output / name)
    worker = target / "wasm32-unknown-unknown" / ("debug" if debug else "web") / "peazip_worker.wasm"
    bindgen_version = next(package["version"] for package in metadata["packages"] if package["name"] == "wasm-bindgen")
    local_tool = ROOT / "target" / "web-tools" / "bin" / ("wasm-bindgen.exe" if os.name == "nt" else "wasm-bindgen")
    bindgen = str(local_tool) if local_tool.exists() else shutil.which("wasm-bindgen")
    if not bindgen or subprocess.check_output([bindgen, "--version"], text=True).strip() != f"wasm-bindgen {bindgen_version}":
        raise SystemExit(f"Install the matching tool: cargo install wasm-bindgen-cli --version {bindgen_version} --locked --root target/web-tools")
    subprocess.run([bindgen, str(worker), "--target", "web", "--out-dir", str(output), "--out-name", "worker-core", "--no-typescript"], check=True)
    shutil.copyfile(ROOT / "LICENSE", output / "LICENSE")
    shutil.copyfile(host.parent.parent.parent / "LICENSE-APACHE", output / "NSH-LICENSE-APACHE")
    print(output)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--debug", action="store_true", help="Build a faster, unoptimized development artifact")
    build(parser.parse_args().debug)
