"""Run uv run python crates/peazip-core/tests/fixtures/generate_packed.py; requires system 7z."""

import argparse
import hashlib
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile


def generate(sevenzip):
    output = Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        data = bytearray()
        for index in range(1024):
            position = 32 * index
            block = b"\x90\x90\x90\xe8" + struct.pack("<i", ((index + 17) % 1024) * 32 - (position + 8))
            block += b"\xe9" + struct.pack("<i", ((index + 31) % 1024) * 32 - (position + 13))
            data.extend(block + hashlib.sha256(index.to_bytes(4, "little")).digest()[:32 - len(block)])
        source = root / "branches.bin"
        source.write_bytes(data)
        source.chmod(0o644)
        archive = root / "bcj2.7z"
        common = ["-t7z", "-mhc=off", "-mtc=off", "-mta=off", "-mtm=off"]
        subprocess.run([sevenzip, "a", *common, "-m0=BCJ2", "-m1=LZMA2:d=1m",
                        "-m2=LZMA2:d=1m", "-m3=LZMA2:d=1m", "-mb0:1", "-mb0s1:2",
                        "-mb0s2:3", "-ms=off", str(archive), source.name],
                       cwd=root, check=True, capture_output=True)
        (output / "7z-bcj2-single.7z").write_bytes(archive.read_bytes())
        mixed = root / "mixed"
        mixed.mkdir()
        (mixed / "folder").mkdir(mode=0o755)
        for name, body in [("a", b"alpha\n" * 80), ("b", b"beta\n" * 80),
                           ("c", b"gamma\n" * 80), ("empty", b"")]:
            path = mixed / name
            path.write_bytes(body)
            path.chmod(0o644)
        archive = root / "mixed.7z"
        subprocess.run([sevenzip, "a", *common, "-m0=LZMA2:d=1m", "-ms=2f", str(archive), "."],
                       cwd=mixed, check=True, capture_output=True)
        (output / "7z-mixed-blocks.7z").write_bytes(archive.read_bytes())
    print("Generated single-stream BCJ2 with four packed streams and mixed solid/single-stream blocks")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sevenzip", default=shutil.which("7z"))
    args = parser.parse_args()
    if not args.sevenzip:
        parser.error("Pass --sevenzip pointing to an installed 7z executable")
    generate(args.sevenzip)
