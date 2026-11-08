from pathlib import Path


def archive_licenses(metadata):
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    pending = [package["id"] for package in packages.values()
               if package["name"] in ("peazip-core", "regex")]
    selected = set()
    while pending:
        package_id = pending.pop()
        if package_id not in selected:
            selected.add(package_id)
            pending.extend(nodes[package_id]["dependencies"])
    for package in sorted((packages[key] for key in selected),
                          key=lambda package: (package["name"], package["version"], package["id"])):
        root = Path(package["manifest_path"]).parent
        for pattern in ("LICENSE*", "NOTICE*", "CONTRIBUTORS*"):
            for source in sorted(root.glob(pattern)):
                if source.is_file():
                    yield f"{package['name']}-{package['version']}-{source.name}", source
