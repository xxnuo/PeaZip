use peazip_core::{ArchiveFormat, CreateOptions, Limits, SevenZipCompression};
use peazip_native::{DirectoryLimits, DirectorySnapshot, create_archive_from_paths};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn files(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, path: &Path, result: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for child in fs::read_dir(root.join(path)).unwrap() {
            let child = child.unwrap();
            let relative = path.join(child.file_name());
            assert!(!child.file_type().unwrap().is_symlink());
            let dir = child.file_type().unwrap().is_dir();
            result.insert(
                relative.clone(),
                if dir {
                    None
                } else {
                    Some(fs::read(child.path()).unwrap())
                },
            );
            if dir {
                visit(root, &relative, result);
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, Path::new(""), &mut result);
    result
}

#[test]
fn path_and_snapshot_creation_publish_real_stored_and_lzma2_archives_without_overwriting() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fs::create_dir_all(source.join("目录/empty")).unwrap();
    fs::write(source.join("输入你好.txt"), "native 7z 中文\n").unwrap();
    fs::write(
        source.join("目录").join(format!("{}.bin", "长".repeat(40))),
        [0, 127, 255, 1],
    )
    .unwrap();
    fs::write(source.join("zero"), []).unwrap();
    let expected = files(root.path());
    let snapshot = DirectorySnapshot::open(root.path(), DirectoryLimits::default()).unwrap();
    let id = snapshot
        .entries()
        .iter()
        .find(|entry| entry.name == "source")
        .unwrap()
        .id;
    let independent = Command::new("7z").arg("i").output().is_ok();
    for (index, method) in [
        SevenZipCompression::Stored,
        SevenZipCompression::Lzma2 { level: 1 },
        SevenZipCompression::Lzma2 { level: 9 },
    ]
    .into_iter()
    .enumerate()
    {
        for from_snapshot in [false, true] {
            let archive = root
                .path()
                .join(format!("output-{index}-{from_snapshot}.7z"));
            let options = CreateOptions::SevenZip(method);
            let entries = if from_snapshot {
                snapshot
                    .create_archive(&[id], &archive, Limits::default(), options)
                    .unwrap()
            } else {
                create_archive_from_paths(
                    std::slice::from_ref(&source),
                    &archive,
                    Limits::default(),
                    options,
                )
                .unwrap()
            };
            let bytes = fs::read(&archive).unwrap();
            assert_eq!(
                peazip_core::detect_format(&bytes).unwrap(),
                ArchiveFormat::SevenZip
            );
            assert_eq!(
                peazip_core::list_archive(&bytes, Limits::default()).unwrap(),
                entries
            );
            let extracted = root.path().join(format!("decoded-{index}-{from_snapshot}"));
            if independent {
                let result = Command::new("7z")
                    .arg("x")
                    .arg("-y")
                    .arg(format!("-o{}", extracted.display()))
                    .arg(&archive)
                    .output()
                    .unwrap();
                assert!(
                    result.status.success(),
                    "independent 7z: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
                assert_eq!(files(&extracted), expected);
                let listed = Command::new("7z")
                    .args(["l", "-slt"])
                    .arg(&archive)
                    .output()
                    .unwrap();
                assert!(listed.status.success());
                let listed = String::from_utf8_lossy(&listed.stdout);
                assert!(
                    listed.contains(match method {
                        SevenZipCompression::Stored => "Method = Copy",
                        _ => "Method = LZMA2",
                    }),
                    "{listed}"
                );
            }
            assert!(
                create_archive_from_paths(
                    std::slice::from_ref(&source),
                    &archive,
                    Limits::default(),
                    options
                )
                .is_err()
            );
            assert_eq!(fs::read(&archive).unwrap(), bytes);
        }
    }
    assert_eq!(
        files(&source),
        expected
            .into_iter()
            .filter_map(|(path, data)| path
                .strip_prefix("source")
                .ok()
                .filter(|path| !path.as_os_str().is_empty())
                .map(|path| (path.to_owned(), data)))
            .collect()
    );
}
