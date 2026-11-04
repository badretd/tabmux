//! Directory and ZIP batch ingestion.
//!
//! Batch inputs are containers, not table formats: one directory or one ZIP
//! archive expands into a deterministic, lexicographically ordered list of
//! member files. Supported table formats are read through the normal format
//! registry; unsupported files become `format.unsupported` events rather than
//! disappearing silently.
//!
//! ZIP members are treated as hostile input. Absolute paths, `..` components,
//! backslash separators, entry-count bombs, oversized archives, and members
//! with suspicious compression ratios are rejected before any output is
//! created.

use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use walkdir::WalkDir;

use crate::error::{Error, Result};
use crate::formats::{self, FormatId};
use crate::model::SourceRef;

/// Default maximum number of members in one ZIP archive.
pub const DEFAULT_MAX_ENTRIES: usize = 10_000;
/// Default maximum total uncompressed size of one ZIP archive (512 MiB).
pub const DEFAULT_MAX_UNCOMPRESSED_SIZE: u64 = 512 * 1024 * 1024;
/// Default maximum compression ratio for a member at least [`RATIO_MIN_BYTES`].
pub const DEFAULT_MAX_COMPRESSION_RATIO: u64 = 200;

/// Per-member compression ratios are only checked above this size. Tiny files
/// can have huge ratios without being a denial-of-service risk.
const RATIO_MIN_BYTES: u64 = 1024 * 1024;

/// Resource limits for ZIP archive ingestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveLimits {
    pub max_entries: usize,
    pub max_uncompressed_size: u64,
    pub max_compression_ratio: u64,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            max_entries: DEFAULT_MAX_ENTRIES,
            max_uncompressed_size: DEFAULT_MAX_UNCOMPRESSED_SIZE,
            max_compression_ratio: DEFAULT_MAX_COMPRESSION_RATIO,
        }
    }
}

/// Options for scanning a batch input.
#[derive(Debug, Clone, Default)]
pub struct BatchOptions {
    /// Descend into subdirectories. ZIP members are always traversed.
    pub recursive: bool,
    /// Glob patterns limiting which supported files are converted.
    pub include: Vec<String>,
    /// Glob patterns excluding files from the batch entirely.
    pub exclude: Vec<String>,
    pub limits: ArchiveLimits,
}

/// One supported file discovered in a batch input.
#[derive(Debug, Clone)]
pub struct BatchSource {
    /// Source reference used for diagnostics. ZIP members use
    /// `archive.zip::member/path` as the display path.
    pub source: SourceRef,
    /// Path relative to the batch root. Always safe to join under an output
    /// directory.
    pub relative: PathBuf,
    pub bytes: Vec<u8>,
    pub format: FormatId,
}

/// One file that will not be converted.
#[derive(Debug, Clone)]
pub struct UnsupportedInput {
    pub source: SourceRef,
    pub relative: PathBuf,
    pub reason: String,
}

/// The deterministic result of scanning a directory or ZIP archive.
#[derive(Debug, Clone, Default)]
pub struct BatchScan {
    pub sources: Vec<BatchSource>,
    pub unsupported: Vec<UnsupportedInput>,
}

/// Whether a path should be handled as a batch container.
pub fn is_batch_input(path: &Path) -> bool {
    path.is_dir()
        || path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

/// Scan a directory or ZIP archive into supported sources and skipped files.
pub fn scan(input: &Path, options: &BatchOptions) -> Result<BatchScan> {
    let filters = Filters::new(&options.include, &options.exclude)?;

    if input.is_dir() {
        return scan_directory(input, options, &filters);
    }
    if input
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
    {
        return scan_zip(input, options, &filters);
    }

    Err(Error::usage(format!(
        "batch input must be a directory or .zip archive: {}",
        input.display()
    )))
}

#[derive(Debug)]
struct Filters {
    include: Option<GlobSet>,
    exclude: Option<GlobSet>,
}

impl Filters {
    fn new(include: &[String], exclude: &[String]) -> Result<Self> {
        Ok(Self {
            include: compile_globs(include, "--include")?,
            exclude: compile_globs(exclude, "--exclude")?,
        })
    }

    fn is_excluded(&self, relative: &Path) -> bool {
        self.exclude
            .as_ref()
            .is_some_and(|set| set.is_match(relative_key(relative)))
    }

    fn is_included(&self, relative: &Path) -> bool {
        self.include
            .as_ref()
            .is_none_or(|set| set.is_match(relative_key(relative)))
    }
}

fn compile_globs(patterns: &[String], flag: &str) -> Result<Option<GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }

    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return Err(Error::usage(format!("{flag} contains an empty glob")));
        }
        let glob = Glob::new(pattern)
            .map_err(|error| Error::usage(format!("invalid {flag} glob '{pattern}': {error}")))?;
        builder.add(glob);
    }

    Ok(Some(builder.build().map_err(|error| {
        Error::usage(format!("could not compile {flag} globs: {error}"))
    })?))
}

fn scan_directory(root: &Path, options: &BatchOptions, filters: &Filters) -> Result<BatchScan> {
    let depth = if options.recursive { usize::MAX } else { 1 };
    let mut scan = BatchScan::default();

    for entry in WalkDir::new(root).max_depth(depth).into_iter() {
        let entry = entry.map_err(|error| {
            Error::data(format!("could not traverse '{}': {error}", root.display()))
        })?;

        // `follow_links` is off, so symlinks are not regular files here.
        if !entry.file_type().is_file() {
            continue;
        }

        let Ok(relative) = entry.path().strip_prefix(root) else {
            return Err(Error::data(format!(
                "found a path outside the batch root: {}",
                entry.path().display()
            )));
        };
        if relative.as_os_str().is_empty() || filters.is_excluded(relative) {
            continue;
        }

        let display = entry.path().display().to_string();
        match formats::detect_path(relative) {
            Some(format) => {
                if !filters.is_included(relative) {
                    continue;
                }
                let bytes = std::fs::read(entry.path())
                    .map_err(|error| io_path_error(entry.path(), error))?;
                scan.sources.push(BatchSource {
                    source: SourceRef::new(display, format),
                    relative: relative.to_path_buf(),
                    bytes,
                    format,
                });
            }
            None => scan.unsupported.push(UnsupportedInput {
                source: SourceRef::new(display, FormatId::Unknown),
                relative: relative.to_path_buf(),
                reason: "unsupported input format".to_owned(),
            }),
        }
    }

    sort_scan(&mut scan);
    Ok(scan)
}

fn scan_zip(input: &Path, options: &BatchOptions, filters: &Filters) -> Result<BatchScan> {
    let file = File::open(input).map_err(|error| io_path_error(input, error))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| zip_error(input, error))?;

    if archive.len() > options.limits.max_entries {
        return Err(Error::data(format!(
            "archive '{}' has {} members, exceeding --max-archive-entries {}",
            input.display(),
            archive.len(),
            options.limits.max_entries
        )));
    }

    let mut members = Vec::<ZipMember>::new();
    let mut declared_total = 0u64;

    for index in 0..archive.len() {
        let member = archive
            .by_index(index)
            .map_err(|error| zip_error(input, error))?;
        let name = member
            .name()
            .map_err(|error| zip_error(input, error))?
            .to_string();

        // Reject Windows-style separators on every platform so an archive
        // produced on one OS cannot escape on another.
        if name.contains('\\') {
            return Err(Error::data(format!(
                "archive '{}' member '{name}' contains a backslash path separator; refusing unsafe member",
                input.display()
            )));
        }

        let relative = member.enclosed_name().ok_or_else(|| {
            Error::data(format!(
                "archive '{}' member '{name}' escapes the archive root; refusing unsafe member",
                input.display()
            ))
        })?;
        reject_parent_components(input, &name, &relative)?;

        if member.is_dir() || name.ends_with('/') || relative.as_os_str().is_empty() {
            continue;
        }

        let size = member.size();
        let compressed = member.compressed_size();
        declared_total = declared_total.checked_add(size).ok_or_else(|| {
            Error::data(format!(
                "archive '{}' declares an uncompressed size that overflows",
                input.display()
            ))
        })?;
        if declared_total > options.limits.max_uncompressed_size {
            return Err(Error::data(format!(
                "archive '{}' declares {declared_total} uncompressed bytes, exceeding --max-uncompressed-size {}",
                input.display(),
                options.limits.max_uncompressed_size
            )));
        }

        if size >= RATIO_MIN_BYTES {
            let ratio = size.checked_div(compressed).unwrap_or(u64::MAX);
            if ratio > options.limits.max_compression_ratio {
                return Err(Error::data(format!(
                    "archive '{}' member '{name}' has compression ratio {ratio}, exceeding --max-compression-ratio {}",
                    input.display(),
                    options.limits.max_compression_ratio
                )));
            }
        }

        members.push(ZipMember {
            index,
            relative,
            size,
        });
    }

    members.sort_by_key(|member| relative_key(&member.relative));

    let mut scan = BatchScan::default();
    let mut actual_total = 0u64;
    for member in members {
        if filters.is_excluded(&member.relative) {
            continue;
        }

        let display = format!("{}::{}", input.display(), relative_key(&member.relative));
        let Some(format) = formats::detect_path(&member.relative) else {
            scan.unsupported.push(UnsupportedInput {
                source: SourceRef::new(display, FormatId::Unknown),
                relative: member.relative,
                reason: "unsupported input format".to_owned(),
            });
            continue;
        };
        if !filters.is_included(&member.relative) {
            continue;
        }

        let entry = archive
            .by_index(member.index)
            .map_err(|error| zip_error(input, error))?;
        let mut bytes = Vec::new();
        entry
            .take(member.size)
            .read_to_end(&mut bytes)
            .map_err(|error| zip_error(input, zip::result::ZipError::Io(error)))?;
        if bytes.len() as u64 != member.size {
            return Err(Error::data(format!(
                "archive '{}' member '{display}' ended before its declared size",
                input.display()
            )));
        }

        actual_total = actual_total.saturating_add(bytes.len() as u64);
        if actual_total > options.limits.max_uncompressed_size {
            return Err(Error::data(format!(
                "archive '{}' expands beyond --max-uncompressed-size {}",
                input.display(),
                options.limits.max_uncompressed_size
            )));
        }

        scan.sources.push(BatchSource {
            source: SourceRef::new(display, format),
            relative: member.relative,
            bytes,
            format,
        });
    }

    sort_scan(&mut scan);
    Ok(scan)
}

struct ZipMember {
    index: usize,
    relative: PathBuf,
    size: u64,
}

fn reject_parent_components(input: &Path, name: &str, relative: &Path) -> Result<()> {
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(Error::data(format!(
            "archive '{}' member '{name}' escapes the archive root; refusing unsafe member",
            input.display()
        )));
    }
    Ok(())
}

fn sort_scan(scan: &mut BatchScan) {
    scan.sources
        .sort_by_key(|source| relative_key(&source.relative));
    scan.unsupported
        .sort_by_key(|unsupported| relative_key(&unsupported.relative));
}

fn relative_key(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn zip_error(path: &Path, error: zip::result::ZipError) -> Error {
    match error {
        zip::result::ZipError::Io(error) => Error::Io(error),
        other => Error::data(format!("archive '{}': {other}", path.display())),
    }
}

fn io_path_error(path: &Path, error: std::io::Error) -> Error {
    Error::Io(std::io::Error::new(
        error.kind(),
        format!("{}: {error}", path.display()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_paths_with_parent_components_are_rejected() {
        let input = Path::new("bad.zip");
        let error = reject_parent_components(input, "../escape.csv", Path::new("../escape.csv"))
            .expect_err("parent components must be rejected");
        assert!(error.to_string().contains("escapes the archive root"));

        reject_parent_components(input, "nested/data.csv", Path::new("nested/data.csv"))
            .expect("a nested path is safe");
    }

    #[test]
    fn relative_keys_use_forward_slashes() {
        assert_eq!(
            relative_key(Path::new("nested").join("data.csv").as_path()),
            "nested/data.csv"
        );
    }

    #[test]
    fn invalid_globs_are_usage_errors() {
        let error = Filters::new(&["[".to_owned()], &[]).expect_err("invalid glob");
        assert!(error.to_string().contains("--include"));
        assert_eq!(error.exit_code(), crate::error::ExitCode::Usage);
    }
}
