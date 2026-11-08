import tempfile
import unittest
from pathlib import Path

from archive_licenses import archive_licenses


class ArchiveLicensesTest(unittest.TestCase):
    def test_resolved_codec_graph_keeps_both_versions_and_excludes_unrelated_packages(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            packages = []
            for package_id, name, version in [
                ('core', 'peazip-core', '0.1'), ('regex', 'regex', '1'),
                ('tar', 'tar', '1'), ('flate', 'flate2', '1'),
                ('codec1', 'codec', '1'), ('codec2', 'codec', '2'),
                ('unrelated', 'unrelated', '1'),
            ]:
                path = root / package_id
                path.mkdir()
                (path / 'LICENSE').write_text(package_id)
                packages.append(dict(id=package_id, name=name, version=version,
                                     manifest_path=str(path / 'Cargo.toml')))
            dependencies = {'core': ['tar', 'flate'], 'regex': ['codec1'],
                            'tar': ['codec1'], 'flate': ['codec2'],
                            'codec1': [], 'codec2': [], 'unrelated': []}
            metadata = dict(packages=packages, resolve=dict(nodes=[
                dict(id=key, dependencies=values) for key, values in dependencies.items()]))
            files = dict(archive_licenses(metadata))
            self.assertEqual(set(files), {'peazip-core-0.1-LICENSE', 'regex-1-LICENSE',
                                         'tar-1-LICENSE', 'flate2-1-LICENSE',
                                         'codec-1-LICENSE', 'codec-2-LICENSE'})
            self.assertEqual(files['codec-2-LICENSE'].read_text(), 'codec2')
            self.assertEqual(set(dict(archive_licenses(metadata, roots=('flate2',)))),
                             {'flate2-1-LICENSE', 'codec-2-LICENSE'})


if __name__ == '__main__':
    unittest.main()
