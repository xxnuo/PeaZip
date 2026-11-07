"""Regenerate with uv run python crates/peazip-core/tests/fixtures/generate_extract.py (requires 7z)."""

import argparse
import gzip
import io
from pathlib import Path
import shutil
import struct
import subprocess
import tarfile
import tempfile
import zipfile
import zlib

CASES = {
    "extract-tree": {"docs/": None, "docs/empty/": None, "docs/deep/": None,
                     "docs/deep/data.bin": bytes([0, 255, 128]),
                     "docs/中文.txt": "flatten 中文\n".encode(), "empty-file": b"",
                     "outside.txt": b"outside\n"},
    "extract-collisions": {"a/": None, "b/": None, "c/": None, "a/same.txt": b"one",
                           "b/same.txt": b"two", "c/SAME.TXT": b"three", "clean.txt": b"clean"},
    "extract-directories": {"empty/": None, "nested/": None, "nested/empty/": None},
    "extract-unsafe": {"aa/": None, "../file": b"unsafe source", "clean.txt": b"clean"},
}


def generate(sevenzip):
    output = Path(__file__).resolve().parent
    for stem, contents in CASES.items():
        with zipfile.ZipFile(output / f"{stem}.zip", "w", compression=zipfile.ZIP_STORED) as archive:
            for name, data in contents.items():
                info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
                info.create_system = 3
                info.external_attr = ((0o40755 if data is None else 0o100644) << 16) | (0x10 if data is None else 0)
                archive.writestr(info, data or b"")
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w", format=tarfile.USTAR_FORMAT) as archive:
            for name, data in contents.items():
                info = tarfile.TarInfo(name)
                info.mode = 0o755 if data is None else 0o644
                info.type = tarfile.DIRTYPE if data is None else tarfile.REGTYPE
                info.size = 0 if data is None else len(data)
                archive.addfile(info, None if data is None else io.BytesIO(data))
        raw = buffer.getvalue()
        (output / f"{stem}.tar").write_bytes(raw)
        compressed = io.BytesIO()
        with gzip.GzipFile(filename="", mode="wb", fileobj=compressed, mtime=0, compresslevel=9) as stream:
            stream.write(raw)
        (output / f"{stem}.tar.gz").write_bytes(compressed.getvalue())
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "input"
            root.mkdir()
            for name, data in contents.items():
                path = root / ("aa/file" if name == "../file" else name)
                if data is None:
                    path.mkdir(parents=True, exist_ok=True)
                    path.chmod(0o755)
                else:
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(data)
                    path.chmod(0o644)
            archive = Path(directory) / "fixture.7z"
            subprocess.run([sevenzip, "a", "-t7z", "-m0=Copy", "-ms=off", "-mhc=off",
                            "-mtc=off", "-mta=off", "-mtm=off", str(archive), "."],
                           cwd=root, capture_output=True, check=True)
            raw = bytearray(archive.read_bytes())
        if stem == "extract-unsafe":
            # Keep the plain header's lengths and payload CRCs intact: replace aa/file by ../file.
            start = 32 + struct.unpack_from("<Q", raw, 12)[0]
            before, after = (name.encode("utf-16le") for name in ("aa/file", "../file"))
            assert len(before) == len(after) and raw[start:].count(before) == 1
            offset = raw.index(before, start)
            raw[offset:offset + len(before)] = after
            struct.pack_into("<I", raw, 28, zlib.crc32(raw[start:]))
            struct.pack_into("<I", raw, 8, zlib.crc32(raw[12:32]))
        (output / f"{stem}.7z").write_bytes(raw)
    print("Generated 16 extraction fixtures using Python zipfile/tarfile/gzip and system 7z")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sevenzip", default=shutil.which("7z"))
    args = parser.parse_args()
    if not args.sevenzip:
        parser.error("Pass --sevenzip pointing to an installed 7z executable")
    generate(args.sevenzip)
