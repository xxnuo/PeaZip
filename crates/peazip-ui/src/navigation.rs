use peazip_core::Entry;
use std::collections::{BTreeMap, BTreeSet};

const HISTORY_LIMIT: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserState {
    history: Vec<String>,
    position: usize,
}

impl Default for BrowserState {
    fn default() -> Self {
        Self {
            history: vec![String::new()],
            position: 0,
        }
    }
}

impl BrowserState {
    pub(crate) fn current(&self) -> &str {
        &self.history[self.position]
    }

    pub(crate) fn can_back(&self) -> bool {
        self.position > 0
    }

    pub(crate) fn can_forward(&self) -> bool {
        self.position + 1 < self.history.len()
    }

    pub(crate) fn back(&mut self) -> bool {
        if !self.can_back() {
            return false;
        }
        self.position -= 1;
        true
    }

    pub(crate) fn forward(&mut self) -> bool {
        if !self.can_forward() {
            return false;
        }
        self.position += 1;
        true
    }

    pub(crate) fn up(&mut self) -> bool {
        if self.current().is_empty() {
            return false;
        }
        let parent = self
            .current()
            .trim_end_matches('/')
            .rsplit_once('/')
            .map_or_else(String::new, |(parent, _)| format!("{parent}/"));
        self.visit(parent)
    }

    pub(crate) fn root(&mut self) -> bool {
        self.visit(String::new())
    }

    pub(crate) fn enter(&mut self, path: &str, entries: &[Entry]) -> Result<(), String> {
        let directory =
            canonical_directory(path).ok_or_else(|| "Invalid archive directory".to_owned())?;
        if !directory.is_empty()
            && !entries.iter().any(|entry| {
                (entry.is_dir
                    && entry.path.trim_end_matches('/') == directory.trim_end_matches('/'))
                    || (entry.path.starts_with(&directory) && entry.path.len() > directory.len())
            })
        {
            return Err("Archive directory not found".into());
        }
        self.visit(directory);
        Ok(())
    }

    fn visit(&mut self, directory: String) -> bool {
        if self.current() == directory {
            return false;
        }
        self.history.truncate(self.position + 1);
        self.history.push(directory);
        if self.history.len() > HISTORY_LIMIT {
            self.history.remove(0);
        }
        self.position = self.history.len() - 1;
        true
    }
}

fn canonical_directory(path: &str) -> Option<String> {
    if path.is_empty() || path == "/" {
        return Some(String::new());
    }
    let path = path.strip_suffix('/').unwrap_or(path);
    if path.contains(['\\', ':'])
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return None;
    }
    Some(format!("{path}/"))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Sort {
    #[default]
    NameAsc,
    NameDesc,
    SizeAsc,
    SizeDesc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserRow {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) is_dir: bool,
    pub(crate) size: u64,
    pub(crate) explicit: bool,
}

pub(crate) fn rows(entries: &[Entry], directory: &str, flat: bool, sort: Sort) -> Vec<BrowserRow> {
    let mut output = if flat {
        entries
            .iter()
            .map(|entry| BrowserRow {
                path: entry.path.clone(),
                name: entry.path.clone(),
                is_dir: entry.is_dir,
                size: entry.size,
                explicit: true,
            })
            .collect::<Vec<_>>()
    } else {
        let Some(directory) = canonical_directory(directory) else {
            return Vec::new();
        };
        let mut children: BTreeMap<String, BrowserRow> = BTreeMap::new();
        for entry in entries {
            let Some(tail) = entry
                .path
                .strip_prefix(&directory)
                .filter(|tail| !tail.is_empty())
            else {
                continue;
            };
            let row = if let Some(at) = tail.find('/') {
                let explicit = entry.is_dir && at + 1 == tail.len();
                BrowserRow {
                    path: format!("{directory}{}", &tail[..=at]),
                    name: tail[..at].into(),
                    is_dir: true,
                    size: if explicit { entry.size } else { 0 },
                    explicit,
                }
            } else {
                BrowserRow {
                    path: entry.path.clone(),
                    name: tail.into(),
                    is_dir: entry.is_dir,
                    size: entry.size,
                    explicit: true,
                }
            };
            if row.explicit {
                children.insert(row.path.clone(), row);
            } else {
                children.entry(row.path.clone()).or_insert(row);
            }
        }
        children.into_values().collect()
    };
    output.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| match sort {
                Sort::NameAsc => a.name.cmp(&b.name),
                Sort::NameDesc => b.name.cmp(&a.name),
                Sort::SizeAsc => a.size.cmp(&b.size),
                Sort::SizeDesc => b.size.cmp(&a.size),
            })
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.path.cmp(&b.path))
    });
    output
}

pub(crate) fn selection_paths(row: &BrowserRow, entries: &[Entry]) -> Vec<String> {
    if row.explicit {
        return vec![row.path.clone()];
    }
    let mut seen = BTreeSet::new();
    entries
        .iter()
        .filter(|entry| entry.path.starts_with(&row.path) && entry.path.len() > row.path.len())
        .filter(|entry| seen.insert(entry.path.as_str()))
        .map(|entry| entry.path.clone())
        .collect()
}

fn covered(path: &str, selected: &BTreeSet<String>) -> bool {
    selected.contains(path)
        || path
            .match_indices('/')
            .any(|(at, _)| selected.contains(&path[..=at]))
}

fn contains(row: &BrowserRow, path: &str) -> bool {
    path == row.path || (row.is_dir && path.starts_with(&row.path))
}

fn atomic_entries(entries: &[Entry]) -> Vec<&Entry> {
    use std::ops::Bound::{Excluded, Unbounded};
    let paths: BTreeSet<_> = entries.iter().map(|entry| entry.path.as_str()).collect();
    entries
        .iter()
        .filter(|entry| {
            !entry.is_dir
                || !paths
                    .range::<str, _>((Excluded(entry.path.as_str()), Unbounded))
                    .next()
                    .is_some_and(|path| path.starts_with(&entry.path))
        })
        .collect()
}

pub(crate) fn selection_state(
    row: &BrowserRow,
    entries: &[Entry],
    selected: &BTreeSet<String>,
) -> (bool, bool) {
    if selected.is_empty() {
        return (false, false);
    }
    if !row.is_dir {
        return (covered(&row.path, selected), false);
    }
    let mut count = 0;
    let mut checked = 0;
    for entry in atomic_entries(entries)
        .into_iter()
        .filter(|entry| contains(row, &entry.path))
    {
        count += 1;
        checked += usize::from(covered(&entry.path, selected));
    }
    (
        count > 0 && count == checked,
        checked > 0 && checked < count,
    )
}

pub(crate) fn set_selected(
    row: &BrowserRow,
    checked: bool,
    entries: &[Entry],
    selected: &mut BTreeSet<String>,
) {
    if checked {
        for path in selection_paths(row, entries) {
            selected.retain(|existing| {
                existing != &path && !(path.ends_with('/') && existing.starts_with(&path))
            });
            if !covered(&path, selected) {
                selected.insert(path);
            }
        }
    } else {
        // Only files and empty explicit directories cannot reinclude an unchecked descendant.
        *selected = atomic_entries(entries)
            .into_iter()
            .filter(|entry| covered(&entry.path, selected) && !contains(row, &entry.path))
            .map(|entry| entry.path.clone())
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, size: u64) -> Entry {
        Entry {
            path: path.into(),
            is_dir: path.ends_with('/'),
            size,
            compressed_size: size,
        }
    }

    fn paths(rows: &[BrowserRow]) -> Vec<&str> {
        rows.iter().map(|row| row.path.as_str()).collect()
    }

    #[test]
    fn navigation_parent_history_branching_and_archive_reset() {
        let entries = [entry("docs/deep/file", 1), entry("other/", 0)];
        let mut browser = BrowserState::default();
        assert_eq!(browser.current(), "");
        assert!(!browser.back());
        assert!(!browser.forward());
        assert!(!browser.up());
        assert!(!browser.root());
        browser.enter("docs/deep", &entries).unwrap();
        assert_eq!(browser.current(), "docs/deep/");
        browser.enter("docs/deep/", &entries).unwrap();
        assert_eq!(browser.history.len(), 2);
        assert!(browser.up());
        assert_eq!(browser.current(), "docs/");
        assert!(browser.back());
        assert_eq!(browser.current(), "docs/deep/");
        assert!(browser.forward());
        assert_eq!(browser.current(), "docs/");
        assert!(browser.back());
        browser.enter("other/", &entries).unwrap();
        assert!(!browser.can_forward());
        assert!(browser.root());
        assert_eq!(browser.current(), "");
        assert!(browser.back());
        assert_eq!(browser.current(), "other/");
        browser.enter("/", &entries).unwrap();
        assert_eq!(browser.current(), "");
        browser = BrowserState::default();
        assert!(!browser.can_back());
        assert!(!browser.can_forward());
    }

    #[test]
    fn refuses_traversal_files_and_unknown_directories_without_mutating_history() {
        let entries = [entry("docs/file", 1), entry("docs-other/", 0)];
        let mut browser = BrowserState::default();
        browser.enter("docs", &entries).unwrap();
        let history = browser.history.clone();
        for path in [
            "..",
            "../docs",
            "docs/../",
            "./docs",
            "/docs",
            "docs//",
            "docs/./",
            "docs\\",
            "C:/docs",
            "docs\0",
            "missing",
            "docs/file",
        ] {
            assert!(browser.enter(path, &entries).is_err(), "{path:?}");
            assert_eq!(browser.history, history);
            assert_eq!(browser.current(), "docs/");
        }
    }

    #[test]
    fn history_never_exceeds_256_locations() {
        let entries: Vec<_> = (0..300).map(|i| entry(&format!("dir-{i}/"), 0)).collect();
        let mut browser = BrowserState::default();
        for entry in &entries {
            browser.enter(&entry.path, &entries).unwrap();
        }
        assert_eq!(browser.history.len(), 256);
        let mut count = 0;
        while browser.back() {
            count += 1;
        }
        assert_eq!(count, 255);
        assert_eq!(browser.current(), "dir-44/");
        while browser.forward() {}
        assert_eq!(browser.current(), "dir-299/");
        assert!(browser.root());
        assert_eq!(browser.history.len(), 256);
    }

    #[test]
    fn hierarchy_derives_deep_implicit_directories_and_preserves_empty_explicit_ones() {
        let entries = [
            entry("docs/deep/中文.txt", 7),
            entry("docs/other", 2),
            entry("empty/", 0),
            entry("root", 3),
        ];
        let root = rows(&entries, "", false, Sort::NameAsc);
        assert_eq!(paths(&root), ["docs/", "empty/", "root"]);
        assert_eq!(
            root[0],
            BrowserRow {
                path: "docs/".into(),
                name: "docs".into(),
                is_dir: true,
                size: 0,
                explicit: false
            }
        );
        assert!(root[1].explicit);
        assert_eq!(
            paths(&rows(&entries, "docs/", false, Sort::NameAsc)),
            ["docs/deep/", "docs/other"]
        );
        assert_eq!(
            rows(&entries, "docs/deep/", false, Sort::NameAsc)[0].name,
            "中文.txt"
        );
        assert!(rows(&entries, "empty/", false, Sort::NameAsc).is_empty());
        assert_eq!(rows(&entries, "/", false, Sort::NameAsc), root);
    }

    #[test]
    fn explicit_directory_wins_over_implicit_duplicates_in_either_order() {
        let entries = [entry("docs/a", 2), entry("docs/b", 3), entry("docs/", 0)];
        let original = entries.clone();
        for entries in [entries.to_vec(), entries.iter().cloned().rev().collect()] {
            let root = rows(&entries, "", false, Sort::NameAsc);
            assert_eq!(root.len(), 1);
            assert!(root[0].explicit);
            assert_eq!(root[0].size, 0);
            assert_eq!(paths(&root), ["docs/"]);
        }
        assert_eq!(entries, original);
    }

    #[test]
    fn flat_mode_keeps_every_actual_entry_and_displays_full_paths() {
        let entries = [
            entry("docs/deep/中文.txt", 7),
            entry("empty/", 0),
            entry("root", 3),
        ];
        let flat = rows(&entries, "docs/", true, Sort::NameAsc);
        assert_eq!(paths(&flat), ["empty/", "docs/deep/中文.txt", "root"]);
        assert!(flat.iter().all(|row| row.explicit && row.name == row.path));
        assert_eq!(flat.len(), entries.len());
    }

    #[test]
    fn all_sorts_keep_folders_first_and_size_ties_have_stable_names() {
        let entries = [
            entry("b", 5),
            entry("a", 5),
            entry("c", 1),
            entry("z/", 0),
            entry("d/", 0),
        ];
        for (sort, expected) in [
            (Sort::NameAsc, ["d/", "z/", "a", "b", "c"]),
            (Sort::NameDesc, ["z/", "d/", "c", "b", "a"]),
            (Sort::SizeAsc, ["d/", "z/", "c", "a", "b"]),
            (Sort::SizeDesc, ["d/", "z/", "a", "b", "c"]),
        ] {
            assert_eq!(paths(&rows(&entries, "", false, sort)), expected);
        }
        let reversed: Vec<_> = entries.iter().cloned().rev().collect();
        assert_eq!(
            rows(&entries, "", false, Sort::SizeDesc),
            rows(&reversed, "", false, Sort::SizeDesc)
        );
    }

    #[test]
    fn implicit_selection_maps_only_actual_descendants_in_archive_order() {
        let entries = [
            entry("docs/deep/a", 3),
            entry("docs/empty/", 0),
            entry("docs/b", 2),
            entry("docs-other/file", 4),
        ];
        let root = rows(&entries, "", false, Sort::NameAsc);
        let docs = root.iter().find(|row| row.path == "docs/").unwrap();
        assert_eq!(
            selection_paths(docs, &entries),
            ["docs/deep/a", "docs/empty/", "docs/b"]
        );
        let mut duplicate = entries.to_vec();
        duplicate.push(entries[0].clone());
        assert_eq!(
            selection_paths(docs, &duplicate),
            selection_paths(docs, &entries)
        );
        let child = rows(&entries, "docs/", false, Sort::NameAsc)
            .into_iter()
            .find(|row| row.path == "docs/empty/")
            .unwrap();
        assert_eq!(selection_paths(&child, &entries), ["docs/empty/"]);
        let leaf = rows(&entries, "docs/deep/", false, Sort::NameAsc).remove(0);
        assert_eq!(selection_paths(&leaf, &entries), ["docs/deep/a"]);
    }

    #[test]
    fn unchecking_child_of_explicit_parent_changes_actual_extraction() {
        use peazip_core::{
            Limits, MemoryEntry, create_zip, extract_selected_archive, list_archive,
        };
        let input: Vec<_> = [
            ("docs/", ""),
            ("docs/private", "secret"),
            ("docs/public", "public"),
        ]
        .into_iter()
        .map(|(path, data)| MemoryEntry {
            path: path.into(),
            is_dir: path.ends_with('/'),
            data: data.as_bytes().to_vec(),
        })
        .collect();
        let archive = create_zip(&input, Limits::default()).unwrap();
        let entries = list_archive(&archive, Limits::default()).unwrap();
        let original = entries.clone();
        let parent = rows(&entries, "", false, Sort::NameAsc).remove(0);
        let children = rows(&entries, "docs/", false, Sort::NameAsc);
        let private = children.iter().find(|row| row.name == "private").unwrap();
        let public = children.iter().find(|row| row.name == "public").unwrap();
        let mut selected = BTreeSet::new();
        set_selected(&parent, true, &entries, &mut selected);
        assert_eq!(selected, BTreeSet::from(["docs/".into()]));
        assert_eq!(selection_state(private, &entries, &selected), (true, false));
        assert_eq!(selection_state(public, &entries, &selected), (true, false));
        selected.insert("docs/private".into());
        set_selected(&parent, true, &entries, &mut selected);
        assert_eq!(selected, BTreeSet::from(["docs/".into()]));
        set_selected(private, false, &entries, &mut selected);
        assert_eq!(selected, BTreeSet::from(["docs/public".into()]));
        assert_eq!(
            selection_state(private, &entries, &selected),
            (false, false)
        );
        assert_eq!(selection_state(public, &entries, &selected), (true, false));
        assert_eq!(selection_state(&parent, &entries, &selected), (false, true));
        let paths: Vec<_> = selected.iter().cloned().collect();
        let output = extract_selected_archive(&archive, &paths, Limits::default()).unwrap();
        assert_eq!(output, vec![input[2].clone()]);
        #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
        {
            let sandbox = tempfile::tempdir().unwrap();
            let destination = sandbox.path().join("output");
            assert_eq!(
                peazip_native::extract_archive_to_directory(
                    &archive,
                    &paths,
                    &destination,
                    Limits::default(),
                )
                .unwrap(),
                1
            );
            assert_eq!(
                std::fs::read(destination.join("docs/public")).unwrap(),
                b"public"
            );
            assert!(!destination.join("docs/private").exists());
        }
        set_selected(private, true, &entries, &mut selected);
        assert_eq!(selection_state(&parent, &entries, &selected), (true, false));
        assert_eq!(entries, original);
    }

    #[test]
    fn implicit_selection_compacts_explicit_grandchildren_and_excludes_subtrees() {
        let entries = [
            entry("docs/deep/public", 1),
            entry("docs/deep/private/secret", 2),
            entry("docs/deep/private/", 0),
            entry("docs/deep/", 0),
            entry("docs/empty/", 0),
            entry("other/empty/", 0),
            entry("docs-other/file", 3),
        ];
        let root = rows(&entries, "", false, Sort::NameAsc);
        let docs = root.iter().find(|row| row.path == "docs/").unwrap();
        assert!(!docs.explicit);
        let private = rows(&entries, "docs/deep/", false, Sort::NameAsc)
            .into_iter()
            .find(|row| row.path == "docs/deep/private/")
            .unwrap();
        let mut selected = BTreeSet::from(["other/empty/".into(), "docs-other/file".into()]);
        set_selected(docs, true, &entries, &mut selected);
        assert_eq!(
            selected,
            BTreeSet::from([
                "docs/deep/".into(),
                "docs/empty/".into(),
                "other/empty/".into(),
                "docs-other/file".into(),
            ])
        );
        assert_eq!(selection_state(docs, &entries, &selected), (true, false));
        assert_eq!(
            selection_state(&private, &entries, &selected),
            (true, false)
        );
        set_selected(&private, false, &entries, &mut selected);
        assert_eq!(
            selected,
            BTreeSet::from([
                "docs/deep/public".into(),
                "docs/empty/".into(),
                "other/empty/".into(),
                "docs-other/file".into(),
            ])
        );
        assert_eq!(selection_state(docs, &entries, &selected), (false, true));
        assert_eq!(
            selection_state(&private, &entries, &selected),
            (false, false)
        );
        set_selected(docs, false, &entries, &mut selected);
        assert_eq!(
            selected,
            BTreeSet::from(["other/empty/".into(), "docs-other/file".into()])
        );
    }

    #[test]
    fn immediate_rows_do_not_expand_deep_archives() {
        let entries: Vec<_> = (0..128)
            .map(|i| entry(&format!("{i:03}/{}file", "a/".repeat(500)), 1))
            .collect();
        let original = entries.clone();
        let root = rows(&entries, "", false, Sort::NameAsc);
        assert_eq!(root.len(), entries.len());
        assert!(
            root.iter()
                .all(|row| row.is_dir && !row.explicit && row.path.len() == 4)
        );
        assert_eq!(rows(&entries, "000/", false, Sort::NameAsc).len(), 1);
        assert_eq!(entries, original);
    }
}
