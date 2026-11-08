use peazip_native::{
    CreateOptions, DirectoryLimits, DirectorySnapshot, Error, Limits, create_archive_from_paths,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn open(path: &Path) -> DirectorySnapshot {
    DirectorySnapshot::open(path, DirectoryLimits::default()).unwrap()
}

fn options() -> [CreateOptions; 3] {
    [
        CreateOptions::Tar,
        CreateOptions::TarGzip { level: 1 },
        CreateOptions::TarGzip { level: 9 },
    ]
}

fn id(snapshot: &DirectorySnapshot, name: &str) -> usize {
    snapshot
        .entries()
        .iter()
        .find(|entry| entry.name == name)
        .unwrap()
        .id
}

fn tree(path: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, relative: &Path, result: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for child in fs::read_dir(root.join(relative)).unwrap() {
            let child = child.unwrap();
            let name = relative.join(child.file_name());
            let kind = child.file_type().unwrap();
            assert!(!kind.is_symlink());
            result.insert(
                name.clone(),
                if kind.is_dir() {
                    None
                } else {
                    Some(fs::read(child.path()).unwrap())
                },
            );
            if kind.is_dir() {
                visit(root, &name, result);
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(path, Path::new(""), &mut result);
    result
}

#[test]
fn paths_and_snapshot_archives_are_independently_readable_with_unicode_long_paths_and_metadata() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let long_dir = "目录".repeat(30);
    fs::create_dir_all(source.join("tree").join(&long_dir)).unwrap();
    fs::create_dir(source.join("tree/empty")).unwrap();
    fs::write(
        source.join("tree").join(&long_dir).join("payload.bin"),
        [0, 127, 255],
    )
    .unwrap();
    fs::write(source.join("tree/zero"), []).unwrap();
    fs::write(source.join("loose.bin"), b"loose").unwrap();
    let expected = tree(&source);
    let snapshot = open(&source);
    let ids = [id(&snapshot, "tree"), id(&snapshot, "loose.bin")];
    let paths = [source.join("tree"), source.join("loose.bin")];
    let python_available = Command::new("uv").arg("--version").output().is_ok();
    for (at, options) in options().into_iter().enumerate() {
        for from_snapshot in [false, true] {
            let archive = root
                .path()
                .join(format!("archive-{at}-{from_snapshot}.tar"));
            let listed = if from_snapshot {
                snapshot
                    .create_archive(&ids, &archive, Limits::default(), options)
                    .unwrap()
            } else {
                create_archive_from_paths(&paths, &archive, Limits::default(), options).unwrap()
            };
            assert_eq!(listed.len(), 6);
            let bytes = fs::read(&archive).unwrap();
            assert_eq!(
                peazip_core::detect_format(&bytes).unwrap(),
                options.format()
            );
            assert_eq!(
                peazip_core::list_archive(&bytes, Limits::default()).unwrap(),
                listed
            );
            let destination = root.path().join(format!("unpacked-{at}-{from_snapshot}"));
            fs::create_dir(&destination).unwrap();
            let result = Command::new("tar")
                .arg("-xf")
                .arg(&archive)
                .arg("-C")
                .arg(&destination)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "system tar: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(tree(&destination), expected);
            if python_available {
                let result = Command::new("uv").args(["run", "python", "-c", r#"
import gzip, io, pathlib, sys, tarfile
data = pathlib.Path(sys.argv[1]).read_bytes()
if data.startswith(b'\x1f\x8b'):
    assert data[3] == 0 and data[4:8] == bytes(4)
    data = gzip.decompress(data)
assert len(data) % 512 == 0 and data[-1024:] == bytes(1024)
with tarfile.open(fileobj=io.BytesIO(data), mode='r:') as archive:
    members = archive.getmembers()
    assert len(members) == 6 and any(len(m.name.encode()) > 100 for m in members)
    for member in members:
        assert member.uid == member.gid == member.mtime == 0
        assert member.mode == (0o755 if member.isdir() else 0o644)
        if member.isfile():
            actual = archive.extractfile(member).read()
            expected = b'\x00\x7f\xff' if member.name.endswith('/payload.bin') else b'loose' if member.name == 'loose.bin' else b''
            assert actual == expected, member.name
print('PASS independent Python tarfile/gzip metadata, long names and every payload byte')
"#]).arg(&archive).output().unwrap();
                assert!(
                    result.status.success(),
                    "Python: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
                eprint!("{}", String::from_utf8_lossy(&result.stdout));
            }
            assert_eq!(tree(&source), expected);
        }
    }
}

#[test]
fn invalid_options_and_source_limits_never_publish_or_leave_staging() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let outputs = root.path().join("outputs");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&outputs).unwrap();
    fs::write(source.join("payload"), b"12345").unwrap();
    let snapshot = open(&source);
    let ids = [id(&snapshot, "payload")];
    let paths = [source.join("payload")];
    for level in [0, 10, 255] {
        let invalid = CreateOptions::TarGzip { level };
        assert!(matches!(
            snapshot.create_archive(
                &[usize::MAX],
                &outputs.join("invalid"),
                Limits::default(),
                invalid
            ),
            Err(Error::InvalidCreateOptions(_))
        ));
        assert!(matches!(
            create_archive_from_paths(
                &[source.join("missing")],
                &outputs.join("invalid"),
                Limits::default(),
                invalid
            ),
            Err(Error::InvalidCreateOptions(_))
        ));
    }
    for options in options() {
        for limits in [
            Limits {
                max_archive_bytes: 1,
                ..Limits::default()
            },
            Limits {
                max_entries: 0,
                ..Limits::default()
            },
            Limits {
                max_path_bytes: 1,
                ..Limits::default()
            },
            Limits {
                max_entry_bytes: 4,
                ..Limits::default()
            },
            Limits {
                max_total_bytes: 4,
                ..Limits::default()
            },
        ] {
            assert!(
                create_archive_from_paths(&paths, &outputs.join("rejected"), limits, options)
                    .is_err()
            );
            assert!(
                snapshot
                    .create_archive(&ids, &outputs.join("rejected"), limits, options)
                    .is_err()
            );
            assert!(tree(&outputs).is_empty());
        }
    }
    assert_eq!(fs::read(source.join("payload")).unwrap(), b"12345");
}

#[test]
fn gzip_ratio_errors_do_not_publish_a_fallback_format_or_leave_temporary_files() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let outputs = root.path().join("outputs");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&outputs).unwrap();
    fs::write(source.join("payload"), vec![0; 64 * 1024]).unwrap();
    let snapshot = open(&source);
    let limits = Limits {
        max_compression_ratio: 1,
        ..Limits::default()
    };
    for level in [1, 6, 9] {
        let options = CreateOptions::TarGzip { level };
        assert!(matches!(
            snapshot.create_archive(
                &[id(&snapshot, "payload")],
                &outputs.join("out"),
                limits,
                options
            ),
            Err(Error::Limit("compression ratio"))
        ));
        assert!(matches!(
            create_archive_from_paths(
                &[source.join("payload")],
                &outputs.join("out"),
                limits,
                options
            ),
            Err(Error::Limit("compression ratio"))
        ));
        assert!(tree(&outputs).is_empty());
    }
}

#[test]
fn existing_targets_and_missing_parents_are_preserved_for_all_new_formats() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let outputs = root.path().join("outputs");
    fs::create_dir(&source).unwrap();
    fs::create_dir_all(outputs.join("directory")).unwrap();
    fs::write(source.join("file"), b"source").unwrap();
    fs::write(outputs.join("existing"), b"sentinel").unwrap();
    fs::write(outputs.join("directory/child"), b"retain").unwrap();
    let before = tree(&outputs);
    let snapshot = open(&source);
    for options in options() {
        for target in [
            outputs.join("existing"),
            outputs.join("directory"),
            outputs.join("missing/child"),
        ] {
            assert!(
                snapshot
                    .create_archive(
                        &[id(&snapshot, "file")],
                        &target,
                        Limits::default(),
                        options
                    )
                    .is_err()
            );
            assert!(
                create_archive_from_paths(
                    &[source.join("file")],
                    &target,
                    Limits::default(),
                    options
                )
                .is_err()
            );
            assert_eq!(tree(&outputs), before);
        }
    }
}

#[cfg(unix)]
#[test]
fn held_snapshot_paths_and_nested_link_rejection_are_shared_by_tar_and_gzip_writers() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let chosen = root.path().join("chosen");
    let moved = root.path().join("moved");
    let outside = root.path().join("outside");
    let outputs = root.path().join("outputs");
    fs::create_dir_all(chosen.join("tree")).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::create_dir(&outputs).unwrap();
    fs::write(chosen.join("tree/file"), b"inside").unwrap();
    fs::write(outside.join("secret"), b"outside").unwrap();
    let snapshot = open(&chosen);
    let ids = [id(&snapshot, "tree")];
    fs::rename(&chosen, &moved).unwrap();
    symlink(&outside, &chosen).unwrap();
    for (at, options) in options().into_iter().enumerate() {
        let target = outputs.join(format!("good-{at}"));
        snapshot
            .create_archive(&ids, &target, Limits::default(), options)
            .unwrap();
        let entries =
            peazip_core::extract_archive(&fs::read(target).unwrap(), Limits::default()).unwrap();
        assert_eq!(
            entries.iter().find(|entry| !entry.is_dir).unwrap().data,
            b"inside"
        );
    }
    symlink(outside.join("secret"), moved.join("tree/link")).unwrap();
    for options in options() {
        assert!(matches!(
            snapshot.create_archive(&ids, &outputs.join("bad"), Limits::default(), options),
            Err(Error::UnsupportedEntry(_))
        ));
        let destination = outputs.join("link");
        symlink(outside.join("secret"), &destination).unwrap();
        assert!(
            snapshot
                .create_archive(&ids, &destination, Limits::default(), options)
                .is_err()
        );
        assert_eq!(fs::read_link(&destination).unwrap(), outside.join("secret"));
        fs::remove_file(destination).unwrap();
    }
    assert_eq!(fs::read_dir(&outputs).unwrap().count(), 3);
    assert_eq!(fs::read(outside.join("secret")).unwrap(), b"outside");
}
