import os
from pathlib import Path
import plistlib
import tempfile
import unittest
from unittest.mock import patch

import build_ios


class IOSBuildTests(unittest.TestCase):
    def test_reject_clt_and_missing_xcode_without_global_changes(self):
        original = os.environ.copy()
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "Command Line Tools"):
                build_ios.developer_path(directory)
            with patch.object(build_ios, "output") as command:
                with self.assertRaisesRegex(ValueError, "找不到 iphonesimulator"):
                    build_ios.sdk_environment("iphonesimulator", directory)
                command.assert_not_called()
        self.assertEqual(dict(os.environ), original)

    def test_app_path_with_spaces_and_scoped_sdk(self):
        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory) / "Xcode Preview.app"
            developer = (app / "Contents/Developer").resolve()
            sdk = developer / "Platforms/iPhoneSimulator.platform/Developer/SDKs/iPhoneSimulator.sdk"
            sdk.mkdir(parents=True)
            (sdk / "SDKSettings.plist").write_bytes(plistlib.dumps({}))
            clang = developer / "usr/bin/clang"
            clang.parent.mkdir(parents=True)
            clang.touch()
            (clang.parent / "xcodebuild").touch()
            original = os.environ.copy()
            with patch.object(build_ios, "output", side_effect=[str(sdk), str(clang)]):
                env, linker = build_ios.sdk_environment("iphonesimulator", str(app))
            self.assertEqual(env["SDKROOT"], str(sdk))
            self.assertEqual(env["DEVELOPER_DIR"], str(developer))
            self.assertEqual(linker, str(clang))
            self.assertEqual(dict(os.environ), original)

    def test_sdk_lookup_must_return_real_sdk(self):
        with tempfile.TemporaryDirectory() as directory:
            developer = Path(directory)
            (developer / "Platforms").mkdir()
            (developer / "usr/bin").mkdir(parents=True)
            (developer / "usr/bin/xcodebuild").touch()
            with patch.object(build_ios, "output", return_value=str(developer / "missing.sdk")):
                with self.assertRaisesRegex(ValueError, "SDK 路径无效"):
                    build_ios.sdk_environment("iphoneos", directory)

    def test_arm64_host_or_device_binary_is_not_simulator_evidence(self):
        with patch.object(build_ios.subprocess, "run"), patch.object(build_ios, "output") as command:
            for wrong_platform in ("MACOS", "IOS"):
                command.return_value = f"platform {wrong_platform}\nminos 15.0"
                with self.assertRaisesRegex(ValueError, "不是 IOSSIMULATOR"):
                    build_ios.validate_binary(Path("binary"), "arm64", "IOSSIMULATOR", {})
            command.return_value = "platform IOSSIMULATOR\nminos 15.0"
            build_ios.validate_binary(Path("binary"), "arm64", "IOSSIMULATOR", {})

    def test_plist_serializes_correct_platform_and_shared_app_id(self):
        for sdk, platform in (("iphoneos", "iPhoneOS"), ("iphonesimulator", "iPhoneSimulator")):
            info = plistlib.loads(plistlib.dumps(build_ios.info_plist(sdk)))
            self.assertEqual(info["CFBundleIdentifier"], "io.peazip.rust")
            self.assertEqual(info["CFBundleSupportedPlatforms"], [platform])
            self.assertEqual(info["UIDeviceFamily"], [1, 2])
            self.assertTrue(info["UIFileSharingEnabled"])
            self.assertTrue(info["LSSupportsOpeningDocumentsInPlace"])
            self.assertNotIn("ProvisioningProfile", info)


if __name__ == "__main__":
    unittest.main()
