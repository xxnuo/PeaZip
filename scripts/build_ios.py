"""Build the locked Rust iOS host and package an SDK-validated .app (no device signing)."""

import argparse
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import subprocess
import tempfile

from archive_licenses import archive_licenses

ROOT = Path(__file__).resolve().parents[1]
APP_ID = "io.peazip.rust"
MINIMUM_OS = "15.0"
TARGETS = {
    "aarch64-apple-ios-sim": ("iphonesimulator", "arm64", "IOSSIMULATOR"),
    "x86_64-apple-ios": ("iphonesimulator", "x86_64", "IOSSIMULATOR"),
    "aarch64-apple-ios": ("iphoneos", "arm64", "IOS"),
}


def output(args, env=None):
    return subprocess.check_output(args, cwd=ROOT, env=env, text=True, stderr=subprocess.PIPE).strip()


def developer_path(path):
    path = Path(path).expanduser().resolve()
    if path.suffix == ".app":
        path /= "Contents/Developer"
    if not (path / "Platforms").is_dir() or not (path / "usr/bin/xcodebuild").is_file():
        raise ValueError(f"完整 Xcode Developer 目录无效：{path}（Command Line Tools 不含 iOS SDK）")
    return path


def sdk_environment(sdk, requested=None):
    explicit = requested or os.environ.get("DEVELOPER_DIR")
    candidates = [explicit] if explicit else []
    if not explicit:
        candidates.append(output(["xcode-select", "-p"]))
        candidates.extend(sorted(Path("/Applications").glob("Xcode*.app")))
        # Finder aliases are not filesystem symlinks; resolve read-only without mounting volumes.
        alias = Path("/Applications/Xcode")
        if alias.is_file() and shutil.which("swift"):
            code = ('import Foundation; do { print(try URL(resolvingAliasFileAt: '
                    'URL(fileURLWithPath: "/Applications/Xcode"), options: '
                    '[.withoutUI, .withoutMounting]).path) } '
                    'catch { fputs("Xcode alias: \\(error)\\n", stderr); exit(1) }')
            try:
                candidates.append(output(["swift", "-e", code]))
            except subprocess.CalledProcessError:
                pass
    errors = []
    for candidate in candidates:
        try:
            developer = developer_path(candidate)
            env = {**os.environ, "DEVELOPER_DIR": str(developer)}
            sdk_path = Path(output(["xcrun", "--sdk", sdk, "--show-sdk-path"], env))
            if not sdk_path.is_dir() or not (sdk_path / "SDKSettings.plist").is_file():
                raise ValueError(f"SDK 路径无效：{sdk_path}")
            clang = output(["xcrun", "--sdk", sdk, "--find", "clang"], env)
            if not Path(clang).is_file():
                raise ValueError(f"clang 不存在：{clang}")
            env.update(SDKROOT=str(sdk_path), IPHONEOS_DEPLOYMENT_TARGET=MINIMUM_OS)
            return env, clang
        except (ValueError, subprocess.CalledProcessError) as error:
            errors.append(str(error))
    raise ValueError(f"找不到 {sdk} SDK；请通过 --developer-dir 或 DEVELOPER_DIR 指定完整 Xcode。"
                     "\n" + "\n".join(errors))


def info_plist(sdk):
    return {
        "CFBundleDevelopmentRegion": "en",
        "CFBundleExecutable": "peazip-ios",
        "CFBundleIdentifier": APP_ID,
        "CFBundleName": "PeaZip",
        "CFBundleDisplayName": "PeaZip",
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": "0.1.0",
        "CFBundleVersion": "1",
        "CFBundleSupportedPlatforms": ["iPhoneOS" if sdk == "iphoneos" else "iPhoneSimulator"],
        "MinimumOSVersion": MINIMUM_OS,
        "UIDeviceFamily": [1, 2],
        "LSRequiresIPhoneOS": True,
        "UILaunchScreen": {},
        "UIApplicationSupportsMultipleScenes": False,
        "CADisableMinimumFrameDurationOnPhone": True,
    }


def validate_binary(binary, arch, expected_platform, env):
    subprocess.run(["xcrun", "lipo", "-verify_arch", arch, str(binary)], env=env, check=True)
    build = output(["xcrun", "vtool", "-show-build", str(binary)], env)
    if not re.search(rf"\bplatform\s+{expected_platform}\b", build):
        raise ValueError(f"产物不是 {expected_platform} Mach-O：{binary}\n{build}")


def build(target, debug=False, developer_dir=None):
    if platform.system() != "Darwin":
        raise ValueError("iOS 构建需要 macOS 与完整 Xcode iOS SDK")
    sdk, arch, expected_platform = TARGETS[target]
    for tool in ("xcode-select", "xcrun", "cargo", "rustup"):
        if not shutil.which(tool):
            raise ValueError(f"缺少工具：{tool}")
    env, clang = sdk_environment(sdk, developer_dir)
    if target not in output(["rustup", "target", "list", "--installed"], env).splitlines():
        raise ValueError(f"缺少 Rust target；请运行 rustup target add {target}")
    if sdk == "iphonesimulator" and not shutil.which("codesign"):
        raise ValueError("缺少 codesign，无法为模拟器应用执行 ad-hoc 签名")
    env[f"CARGO_TARGET_{target.upper().replace('-', '_')}_LINKER"] = clang
    metadata = json.loads(output(["cargo", "metadata", "--filter-platform", target, "--format-version", "1", "--locked"], env))
    if not any(p["name"] == "peazip-ios" and p["id"] in metadata["workspace_members"]
               for p in metadata["packages"]):
        raise ValueError("peazip-ios 必须加入 workspace，且 Cargo.lock 必须与依赖一致")
    command = ["cargo", "build", "--locked", "-p", "peazip-ios", "--bin", "peazip-ios", "--target", target]
    if not debug:
        command.append("--release")
    subprocess.run(command, cwd=ROOT, env=env, check=True)
    profile = "debug" if debug else "release"
    target_dir = Path(metadata["target_directory"])
    binary = target_dir / target / profile / "peazip-ios"
    validate_binary(binary, arch, expected_platform, env)
    destination = target_dir / "ios" / target / profile
    destination.mkdir(parents=True, exist_ok=True)
    app = destination / "PeaZip.app"
    with tempfile.TemporaryDirectory(dir=destination) as directory:
        staged = Path(directory) / "PeaZip.app"
        (staged / "nsh/assets").mkdir(parents=True)
        shutil.copy2(binary, staged / "peazip-ios")
        shutil.copyfile(ROOT / "LICENSE", staged / "PeaZip-LICENSE")
        runtime = next(package for package in metadata["packages"] if package["name"] == "nsh-ios-app")
        shutil.copyfile(Path(runtime["manifest_path"]).parent.parent.parent / "LICENSE-APACHE", staged / "NSH-LICENSE-APACHE")
        for name, source in archive_licenses(metadata):
            shutil.copyfile(source, staged / name)
        with (staged / "Info.plist").open("wb") as stream:
            plistlib.dump(info_plist(sdk), stream, sort_keys=True)
        (staged / "PkgInfo").write_bytes(b"APPL????")
        if sdk == "iphonesimulator":
            subprocess.run(["codesign", "--force", "--sign", "-", "--timestamp=none", str(staged)], env=env, check=True)
            subprocess.run(["codesign", "--verify", "--strict", str(staged)], env=env, check=True)
        if app.exists():
            shutil.rmtree(app)
        staged.rename(app)
    print(app)
    if sdk == "iphoneos":
        print("真机 .app 未签名、无 provisioning profile；不是可安装或可发布产物。")
    else:
        print("模拟器 .app 已 ad-hoc 签名；未自动安装或启动。")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=TARGETS, default="aarch64-apple-ios-sim")
    parser.add_argument("--developer-dir", help="本次构建使用的 Xcode.app 或 Contents/Developer")
    parser.add_argument("--debug", action="store_true")
    args = parser.parse_args()
    try:
        build(args.target, args.debug, args.developer_dir)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        detail = error.stderr.strip() if isinstance(error, subprocess.CalledProcessError) and error.stderr else str(error)
        parser.exit(1, f"iOS 构建失败：{detail}\n")


if __name__ == "__main__":
    main()
