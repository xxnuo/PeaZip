from pathlib import Path


def archive_licenses(metadata):
    for name in ("sevenz-turbo", "lzma-turbo", "crc-turbo"):
        package = next(p for p in metadata["packages"] if p["name"] == name)
        root = Path(package["manifest_path"]).parent
        for pattern in ("LICENSE*", "NOTICE*", "CONTRIBUTORS*"):
            for source in sorted(root.glob(pattern)):
                if source.is_file():
                    yield f"{name}-{package['version']}-{source.name}", source
