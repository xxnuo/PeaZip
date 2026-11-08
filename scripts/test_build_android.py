"""Run with uv run python -m unittest discover -s scripts -p test_build_android.py."""

import os
from pathlib import Path
import tempfile
import time
import unittest
import zipfile

from build_android import append_apk_files


class ApkTimestamps(unittest.TestCase):
    def test_old_registry_license_is_clamped_without_changing_bytes_source_or_existing_entries(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            license = root / "LICENSE"
            library = root / "lib.so"
            license.write_bytes(b"Third-party license\nCopyright 1973\n")
            library.write_bytes(b"native library\0\xff")
            os.utime(license, (0, 0))
            timestamp = time.mktime((2020, 2, 4, 6, 8, 10, 0, 0, -1))
            os.utime(library, (timestamp, timestamp))
            original = license.stat().st_mtime_ns
            apk = root / "unsigned.apk"
            with zipfile.ZipFile(apk, "w") as archive:
                archive.writestr("AndroidManifest.xml", b"manifest")
            append_apk_files(apk, [
                (license, "assets/generic-array-0.14.7-LICENSE", zipfile.ZIP_DEFLATED),
                (library, "lib/arm64-v8a/libnsh.so", zipfile.ZIP_STORED),
            ])
            with zipfile.ZipFile(apk) as archive:
                self.assertIsNone(archive.testzip())
                self.assertEqual(len(archive.infolist()), 3)
                self.assertEqual(archive.read("AndroidManifest.xml"), b"manifest")
                old = archive.getinfo("assets/generic-array-0.14.7-LICENSE")
                self.assertEqual(old.date_time, (1980, 1, 1, 0, 0, 0))
                self.assertEqual(old.compress_type, zipfile.ZIP_DEFLATED)
                self.assertEqual(archive.read(old), license.read_bytes())
                native = archive.getinfo("lib/arm64-v8a/libnsh.so")
                self.assertEqual(native.date_time, (2020, 2, 4, 6, 8, 10))
                self.assertEqual(native.compress_type, zipfile.ZIP_STORED)
                self.assertEqual(archive.read(native), library.read_bytes())
            self.assertEqual(license.stat().st_mtime_ns, original)


if __name__ == "__main__":
    unittest.main()
