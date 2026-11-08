use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::ops::Bound::{Included, Unbounded};
use zip::read::Config;
use zip::unstable::read::ZipIterable;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

mod gzip;
mod sevenzip;
mod tar;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    Tar,
    TarGzip,
    SevenZip,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExtractPaths {
    #[default]
    Full,
    FilesOnly,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExtractOptions {
    pub paths: ExtractPaths,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ZipCompression {
    #[default]
    Automatic,
    Stored,
    Deflate {
        level: u8,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ZipOptions {
    pub compression: ZipCompression,
}

impl ZipOptions {
    pub fn validate(&self) -> Result<(), Error> {
        if let ZipCompression::Deflate { level } = self.compression
            && !(1..=9).contains(&level)
        {
            return Err(Error::InvalidZipOptions(
                "Deflate level must be between 1 and 9",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateOptions {
    Zip(ZipOptions),
    Tar,
    TarGzip { level: u8 },
}

impl Default for CreateOptions {
    fn default() -> Self {
        Self::Zip(ZipOptions::default())
    }
}

impl CreateOptions {
    pub fn validate(&self) -> Result<(), Error> {
        match self {
            Self::Zip(options) => options.validate(),
            Self::Tar => Ok(()),
            Self::TarGzip { level } if (1..=9).contains(level) => Ok(()),
            Self::TarGzip { .. } => Err(Error::InvalidCreateOptions(
                "Gzip level must be between 1 and 9",
            )),
        }
    }

    pub fn format(&self) -> ArchiveFormat {
        match self {
            Self::Zip(_) => ArchiveFormat::Zip,
            Self::Tar => ArchiveFormat::Tar,
            Self::TarGzip { .. } => ArchiveFormat::TarGzip,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_archive_bytes: u64,
    pub max_entries: usize,
    pub max_path_bytes: usize,
    pub max_entry_bytes: u64,
    pub max_total_bytes: u64,
    pub max_compression_ratio: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_archive_bytes: 32 * 1024 * 1024,
            max_entries: 4096,
            max_path_bytes: 1024,
            max_entry_bytes: 16 * 1024 * 1024,
            max_total_bytes: 64 * 1024 * 1024,
            max_compression_ratio: 200,
        }
    }
}

#[derive(Debug)]
pub enum Error {
    NoFilesSelected,
    InvalidZipOptions(&'static str),
    InvalidCreateOptions(&'static str),
    Limit(&'static str),
    UnsafePath(String),
    DuplicatePath(String),
    UnsupportedEntry(String),
    UnknownEntry(String),
    InvalidArchive(zip::result::ZipError),
    InvalidTar(io::Error),
    InvalidGzip(io::Error),
    InvalidSevenZip(io::Error),
    UnsupportedFormat,
    Io(io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoFilesSelected => write!(f, "no files selected for extraction"),
            Self::InvalidZipOptions(message) => write!(f, "invalid ZIP options: {message}"),
            Self::InvalidCreateOptions(message) => write!(f, "invalid creation options: {message}"),
            Self::Limit(kind) => write!(f, "Archive limit exceeded: {kind}"),
            Self::UnsafePath(path) => write!(f, "unsafe archive path: {path:?}"),
            Self::DuplicatePath(path) => {
                write!(f, "duplicate or conflicting archive path: {path:?}")
            }
            Self::UnsupportedEntry(path) => write!(f, "unsupported archive entry: {path:?}"),
            Self::UnknownEntry(path) => write!(f, "archive entry not found: {path:?}"),
            Self::InvalidArchive(e) => write!(f, "invalid ZIP: {e}"),
            Self::InvalidTar(e) => write!(f, "invalid TAR: {e}"),
            Self::InvalidGzip(e) => write!(f, "invalid gzip: {e}"),
            Self::InvalidSevenZip(e) => write!(f, "invalid 7z: {e}"),
            Self::UnsupportedFormat => write!(f, "unsupported archive format"),
            Self::Io(e) => write!(f, "archive I/O error: {e}"),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidArchive(e) => Some(e),
            Self::Io(e) | Self::InvalidTar(e) | Self::InvalidGzip(e) | Self::InvalidSevenZip(e) => {
                Some(e)
            }
            _ => None,
        }
    }
}
impl From<zip::result::ZipError> for Error {
    fn from(e: zip::result::ZipError) -> Self {
        Self::InvalidArchive(e)
    }
}
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    /// Legacy backend accounting: TAR uses logical size; the first solid 7z member holds the block total.
    pub compressed_size: u64,
    /// Physical packed span attributable to this file, including TAR padding; None for shared streams or directories.
    pub packed_size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryEntry {
    pub path: String,
    pub is_dir: bool,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestReport {
    pub entries: usize,
    pub files: usize,
    pub total_bytes: u64,
    pub checksummed_files: usize,
}

fn test_report(entries: &[Entry], checksummed_files: usize) -> TestReport {
    TestReport {
        entries: entries.len(),
        files: entries.iter().filter(|entry| !entry.is_dir).count(),
        total_bytes: entries.iter().map(|entry| entry.size).sum(),
        checksummed_files,
    }
}

fn check(value: u64, max: u64, kind: &'static str) -> Result<(), Error> {
    if value > max {
        Err(Error::Limit(kind))
    } else {
        Ok(())
    }
}

fn path_key(path: &str, is_dir: bool, limits: Limits) -> Result<String, Error> {
    check(
        path.len() as u64,
        limits.max_path_bytes as u64,
        "path bytes",
    )?;
    if is_dir && !path.ends_with('/') {
        return Err(Error::UnsafePath(path.into()));
    }
    let key = if is_dir {
        path.strip_suffix('/').unwrap_or(path)
    } else {
        path
    };
    if key.is_empty()
        || path.contains(['\\', ':', '<', '>', '"', '|', '?', '*'])
        || path.chars().any(char::is_control)
        || key.split('/').any(|part| {
            let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            part.is_empty()
                || part == "."
                || part == ".."
                || part.ends_with(['.', ' '])
                || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || (stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        })
    {
        return Err(Error::UnsafePath(path.into()));
    }
    Ok(key.to_lowercase())
}

fn insert_path(paths: &mut BTreeMap<String, bool>, key: String, dir: bool) -> Result<(), Error> {
    if paths.contains_key(&key)
        || (!dir && {
            let prefix = format!("{key}/");
            paths
                .range::<str, _>((Included(prefix.as_str()), Unbounded))
                .next()
                .is_some_and(|(path, _)| path.starts_with(&prefix))
        })
        || key
            .match_indices('/')
            .any(|(i, _)| paths.get(&key[..i]) == Some(&false))
    {
        return Err(Error::DuplicatePath(key));
    }
    paths.insert(key, dir);
    Ok(())
}

fn sizes(size: u64, compressed: u64, total: &mut u64, limits: Limits) -> Result<(), Error> {
    check(size, limits.max_entry_bytes, "entry bytes")?;
    *total = total.checked_add(size).ok_or(Error::Limit("total bytes"))?;
    check(*total, limits.max_total_bytes, "total bytes")?;
    if size > compressed.saturating_mul(limits.max_compression_ratio) {
        return Err(Error::Limit("compression ratio"));
    }
    Ok(())
}

pub fn detect_format(bytes: &[u8]) -> Result<ArchiveFormat, Error> {
    let format = detect_candidate(bytes)?;
    if format == ArchiveFormat::TarGzip {
        // Detection has no caller-supplied limits; validate gzip/TAR with the defaults.
        let limits = Limits::default();
        tar::list(&gzip::decode(bytes, limits)?, limits)?;
    }
    Ok(format)
}

fn detect_candidate(bytes: &[u8]) -> Result<ArchiveFormat, Error> {
    // Prefer a TAR header at offset zero over a ZIP embedded in its payload or tail.
    let tar = tar::recognizes(bytes);
    if matches!(&tar, Ok(true)) {
        return Ok(ArchiveFormat::Tar);
    }
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(ArchiveFormat::TarGzip);
    }
    if bytes.starts_with(sevenzip::SIGNATURE) {
        return Ok(ArchiveFormat::SevenZip);
    }
    // A bad TAR checksum can be ZIP payload; a complete TAR end marker is not an SFX stub.
    if bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || (!bytes.starts_with(&[0; 1024])
            && ZipIterable::try_new(Cursor::new(bytes), Config::default()).is_ok())
    {
        return Ok(ArchiveFormat::Zip);
    }
    tar?;
    Err(Error::UnsupportedFormat)
}

pub fn list_archive(bytes: &[u8], limits: Limits) -> Result<Vec<Entry>, Error> {
    list_archive_with_format(bytes, limits).map(|(_, entries)| entries)
}

pub fn test_archive(bytes: &[u8], limits: Limits) -> Result<TestReport, Error> {
    check(
        bytes.len() as u64,
        limits.max_archive_bytes,
        "archive bytes",
    )?;
    match detect_candidate(bytes)? {
        ArchiveFormat::Zip => test_zip(bytes, limits),
        ArchiveFormat::Tar => Ok(test_report(&tar::list(bytes, limits)?, 0)),
        ArchiveFormat::TarGzip => {
            let decoded = gzip::decode(bytes, limits)?;
            let entries = tar::list(&decoded, limits)?;
            let files = entries.iter().filter(|entry| !entry.is_dir).count();
            Ok(test_report(&entries, files))
        }
        ArchiveFormat::SevenZip => sevenzip::test(bytes, limits),
    }
}

pub fn list_archive_with_format(
    bytes: &[u8],
    limits: Limits,
) -> Result<(ArchiveFormat, Vec<Entry>), Error> {
    check(
        bytes.len() as u64,
        limits.max_archive_bytes,
        "archive bytes",
    )?;
    let format = detect_candidate(bytes)?;
    let entries = match format {
        ArchiveFormat::Zip => list_zip(bytes, limits),
        ArchiveFormat::Tar => tar::list(bytes, limits),
        ArchiveFormat::TarGzip => {
            let mut entries = tar::list(&gzip::decode(bytes, limits)?, limits)?;
            for entry in &mut entries {
                entry.packed_size = None;
            }
            Ok(entries)
        }
        ArchiveFormat::SevenZip => sevenzip::list(bytes, limits),
    }?;
    Ok((format, entries))
}

pub fn extract_archive(bytes: &[u8], limits: Limits) -> Result<Vec<MemoryEntry>, Error> {
    extract_matching_archive(bytes, None, limits)
}

pub fn extract_selected_archive(
    bytes: &[u8],
    selected: &[String],
    limits: Limits,
) -> Result<Vec<MemoryEntry>, Error> {
    extract_selected_archive_with_options(bytes, selected, limits, ExtractOptions::default())
}

pub fn extract_selected_archive_with_options(
    bytes: &[u8],
    selected: &[String],
    limits: Limits,
    options: ExtractOptions,
) -> Result<Vec<MemoryEntry>, Error> {
    let mut output = extract_matching_archive(bytes, Some(selected), limits)?;
    if options.paths == ExtractPaths::FilesOnly {
        // Preserve source validation, resource checks and selected CRC checks before renaming.
        output.retain(|entry| !entry.is_dir);
        if output.is_empty() {
            return Err(Error::NoFilesSelected);
        }
        let mut paths = BTreeMap::new();
        for entry in &mut output {
            let name = entry.path.rsplit('/').next().unwrap_or("");
            insert_path(&mut paths, path_key(name, false, limits)?, false)?;
            entry.path = name.to_owned();
        }
    }
    Ok(output)
}

fn extract_matching_archive(
    bytes: &[u8],
    selected: Option<&[String]>,
    limits: Limits,
) -> Result<Vec<MemoryEntry>, Error> {
    check(
        bytes.len() as u64,
        limits.max_archive_bytes,
        "archive bytes",
    )?;
    match detect_candidate(bytes)? {
        ArchiveFormat::Zip => extract_matching(bytes, selected, limits),
        ArchiveFormat::Tar => tar::extract(bytes, selected, limits),
        ArchiveFormat::TarGzip => tar::extract(&gzip::decode(bytes, limits)?, selected, limits),
        ArchiveFormat::SevenZip => sevenzip::extract(bytes, selected, limits),
    }
}

pub fn list_zip(bytes: &[u8], limits: Limits) -> Result<Vec<Entry>, Error> {
    check(
        bytes.len() as u64,
        limits.max_archive_bytes,
        "archive bytes",
    )?;
    // Check the count before allocating a complete central-directory index.
    let mut archive = ZipIterable::try_new(Cursor::new(bytes), Config::default())?
        .enforce_header_consistency(true);
    check(
        archive.len() as u64,
        limits.max_entries as u64,
        "entry count",
    )?;
    let mut paths = BTreeMap::new();
    let mut total = 0;
    let mut entries = Vec::new();
    for file in archive.files()? {
        let file = file?;
        let path = file.name()?.into_owned();
        let dir = file.is_dir();
        let key = path_key(&path, dir, limits)?;
        insert_path(&mut paths, key, dir)?;
        let mode = file.unix_mode().unwrap_or(0) & 0o170000;
        if file.encrypted()
            || !matches!(mode, 0 | 0o100000 | 0o040000)
            || (mode == 0o040000 && !dir)
            || (mode == 0o100000 && dir)
            || !matches!(
                file.compression(),
                CompressionMethod::Stored | CompressionMethod::Deflated
            )
            || (dir && file.size() != 0)
        {
            return Err(Error::UnsupportedEntry(path));
        }
        sizes(file.size(), file.compressed_size(), &mut total, limits)?;
        entries.push(Entry {
            path,
            is_dir: dir,
            size: file.size(),
            compressed_size: file.compressed_size(),
            packed_size: (!dir).then_some(file.compressed_size()),
        });
    }
    Ok(entries)
}

pub fn extract_zip(bytes: &[u8], limits: Limits) -> Result<Vec<MemoryEntry>, Error> {
    extract_matching(bytes, None, limits)
}

fn test_zip(bytes: &[u8], limits: Limits) -> Result<TestReport, Error> {
    let entries = list_zip(bytes, limits)?;
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;
    let mut total = 0u64;
    let mut buffer = [0; 8192];
    for (i, entry) in entries.iter().enumerate() {
        let mut file = archive.by_index(i)?.take(entry.size.saturating_add(1));
        let mut actual = 0u64;
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            actual += count as u64;
            check(actual, entry.size, "declared entry size")?;
        }
        if actual != entry.size {
            return Err(Error::Limit("declared entry size"));
        }
        total = total
            .checked_add(actual)
            .ok_or(Error::Limit("total bytes"))?;
        check(total, limits.max_total_bytes, "total bytes")?;
    }
    let files = entries.iter().filter(|entry| !entry.is_dir).count();
    let mut report = test_report(&entries, files);
    report.total_bytes = total;
    Ok(report)
}

pub fn extract_selected_zip(
    bytes: &[u8],
    selected: &[String],
    limits: Limits,
) -> Result<Vec<MemoryEntry>, Error> {
    extract_matching(bytes, Some(selected), limits)
}

fn extract_matching(
    bytes: &[u8],
    selected: Option<&[String]>,
    limits: Limits,
) -> Result<Vec<MemoryEntry>, Error> {
    let entries = list_zip(bytes, limits)?;
    let indices = selected_indices(&entries, selected, limits)?;
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;
    let mut output = Vec::new();
    for i in indices {
        let entry = &entries[i];
        let mut file = archive.by_index(i)?;
        let mut data = Vec::new();
        // One extra byte detects false size declarations without unbounded decompression.
        file.by_ref()
            .take(entry.size.saturating_add(1))
            .read_to_end(&mut data)?;
        if data.len() as u64 != entry.size {
            return Err(Error::Limit("declared entry size"));
        }
        output.push(MemoryEntry {
            path: entry.path.clone(),
            is_dir: entry.is_dir,
            data,
        });
    }
    Ok(output)
}

fn selected_indices(
    entries: &[Entry],
    selected: Option<&[String]>,
    limits: Limits,
) -> Result<Vec<usize>, Error> {
    let selected = selected.map(|paths| paths.iter().map(String::as_str).collect::<BTreeSet<_>>());
    if let Some(selected) = &selected {
        let names: BTreeSet<_> = entries.iter().map(|entry| entry.path.as_str()).collect();
        for path in selected {
            if !names.contains(path) {
                return Err(Error::UnknownEntry((*path).into()));
            }
        }
    }
    let mut indices = Vec::new();
    let mut output_paths = BTreeSet::new();
    for (i, entry) in entries.iter().enumerate() {
        if let Some(selected) = &selected
            && !selected.contains(entry.path.as_str())
            && !entry
                .path
                .match_indices('/')
                .any(|(at, _)| selected.contains(&entry.path[..=at]))
        {
            continue;
        }
        // Implicit parents consume filesystem entries too, even if the archive omits them.
        let end = entry.path.trim_end_matches('/').len();
        for boundary in entry
            .path
            .match_indices('/')
            .map(|(at, _)| at)
            .chain(std::iter::once(end))
        {
            output_paths.insert(entry.path[..boundary].to_owned());
            check(
                output_paths.len() as u64,
                limits.max_entries as u64,
                "output entry count",
            )?;
        }
        indices.push(i);
    }
    Ok(indices)
}

struct BoundedWriter {
    cursor: Cursor<Vec<u8>>,
    max: u64,
}
impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .cursor
            .position()
            .checked_add(bytes.len() as u64)
            .is_none_or(|end| end > self.max)
        {
            return Err(io::Error::other("archive byte limit exceeded"));
        }
        self.cursor.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for BoundedWriter {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.cursor.seek(pos)
    }
}

pub fn create_zip(entries: &[MemoryEntry], limits: Limits) -> Result<Vec<u8>, Error> {
    create_zip_with_options(entries, limits, ZipOptions::default())
}

pub fn validate_memory_entries(entries: &[MemoryEntry], limits: Limits) -> Result<(), Error> {
    check(
        entries.len() as u64,
        limits.max_entries as u64,
        "entry count",
    )?;
    let mut paths = BTreeMap::new();
    let mut total = 0u64;
    for entry in entries {
        insert_path(
            &mut paths,
            path_key(&entry.path, entry.is_dir, limits)?,
            entry.is_dir,
        )?;
        if entry.is_dir && !entry.data.is_empty() {
            return Err(Error::UnsupportedEntry(entry.path.clone()));
        }
        check(
            entry.data.len() as u64,
            limits.max_entry_bytes,
            "entry bytes",
        )?;
        total = total
            .checked_add(entry.data.len() as u64)
            .ok_or(Error::Limit("total bytes"))?;
        check(total, limits.max_total_bytes, "total bytes")?;
    }
    Ok(())
}

pub fn create_archive(
    entries: &[MemoryEntry],
    limits: Limits,
    options: CreateOptions,
) -> Result<Vec<u8>, Error> {
    options.validate()?;
    if let CreateOptions::Zip(zip_options) = options {
        return create_zip_with_options(entries, limits, zip_options);
    }
    validate_memory_entries(entries, limits)?;
    let writer = BoundedWriter {
        cursor: Cursor::new(Vec::new()),
        max: limits.max_archive_bytes,
    };
    let writer = match options {
        CreateOptions::Tar => write_tar(entries, writer)?,
        CreateOptions::TarGzip { level } => {
            let encoder = flate2::GzBuilder::new()
                .mtime(0)
                .write(writer, flate2::Compression::new(u32::from(level)));
            write_tar(entries, encoder)?.finish()?
        }
        CreateOptions::Zip(_) => unreachable!(),
    };
    let bytes = writer.cursor.into_inner();
    list_archive(&bytes, limits)?;
    Ok(bytes)
}

fn write_tar<W: Write>(entries: &[MemoryEntry], writer: W) -> io::Result<W> {
    let mut builder = ::tar::Builder::new(writer);
    for entry in entries {
        let mut header = ::tar::Header::new_gnu();
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_mode(if entry.is_dir { 0o755 } else { 0o644 });
        header.set_entry_type(if entry.is_dir {
            ::tar::EntryType::Directory
        } else {
            ::tar::EntryType::Regular
        });
        header.set_size(entry.data.len() as u64);
        builder.append_data(&mut header, &entry.path, entry.data.as_slice())?;
    }
    builder.into_inner()
}

pub fn create_zip_with_options(
    entries: &[MemoryEntry],
    limits: Limits,
    zip_options: ZipOptions,
) -> Result<Vec<u8>, Error> {
    zip_options.validate()?;
    validate_memory_entries(entries, limits)?;
    let methods: &[CompressionMethod] = match zip_options.compression {
        ZipCompression::Automatic => &[CompressionMethod::Deflated, CompressionMethod::Stored],
        ZipCompression::Stored => &[CompressionMethod::Stored],
        ZipCompression::Deflate { .. } => &[CompressionMethod::Deflated],
    };
    let level = match zip_options.compression {
        ZipCompression::Deflate { level } => Some(i64::from(level)),
        _ => None,
    };
    for &method in methods {
        let mut writer = ZipWriter::new(BoundedWriter {
            cursor: Cursor::new(Vec::new()),
            max: limits.max_archive_bytes,
        });
        let options = SimpleFileOptions::default()
            .compression_method(method)
            .compression_level(level);
        for entry in entries {
            if entry.is_dir {
                writer.add_directory(
                    &entry.path,
                    options
                        .compression_method(CompressionMethod::Stored)
                        .compression_level(None),
                )?;
            } else {
                writer.start_file(&entry.path, options)?;
                writer.write_all(&entry.data)?;
            }
        }
        let bytes = writer.finish()?.cursor.into_inner();
        match list_zip(&bytes, limits) {
            // Retry trusted input without compression so it passes the same read limits.
            Err(Error::Limit("compression ratio"))
                if zip_options.compression == ZipCompression::Automatic
                    && method == CompressionMethod::Deflated => {}
            result => {
                result?;
                return Ok(bytes);
            }
        }
    }
    unreachable!("the final compression method returns its validation result")
}
