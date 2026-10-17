//! One disk traversal contract for counts, bounded previews and full imports.
use super::{ScanItem, ScanResult, SkippedFile, VIDEO_EXT};
use crate::{
    code::{parse_file_info, Code},
    video_filter::VideoFilter,
};
use serde::Serialize;
use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
};
use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;

pub const SUMMARY_ENTRY_LIMIT: usize = 5000;
const ERROR_LIMIT: usize = 100;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanIssue {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub total: usize,
    pub skipped_total: usize,
    pub visited: usize,
    pub errors: Vec<ScanIssue>,
    pub errors_total: usize,
    pub cancelled: bool,
    pub truncated: bool,
    pub complete: bool,
}

impl ScanSummary {
    fn error(&mut self, path: &Path, message: impl ToString) {
        self.errors_total += 1;
        if self.errors.len() < ERROR_LIMIT {
            self.errors.push(ScanIssue {
                path: path.into(),
                message: message.to_string(),
            });
        }
    }
}

fn absolute_normalized(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in std::path::absolute(path)
        .unwrap_or_else(|_| path.into())
        .components()
    {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component),
        }
    }
    normalized
}

fn walk(
    roots: &[PathBuf],
    exclude: Option<&Path>,
    filter: &VideoFilter,
    cancel: &CancellationToken,
    entry_limit: Option<usize>,
    item_limit: Option<usize>,
    on_video: &mut dyn FnMut(&Path, &Path, u64),
    on_skip: &mut dyn FnMut(SkippedFile),
) -> ScanSummary {
    let mut summary = ScanSummary::default();
    let excluded = exclude.map(|path| {
        path.canonicalize()
            .unwrap_or_else(|_| absolute_normalized(path))
    });
    // Directory identities are represented by canonical paths, preserving input
    // order while pruning overlapping roots/aliases before descending again.
    let mut seen_dirs = HashSet::new();
    'roots: for root in roots {
        if cancel.is_cancelled() {
            summary.cancelled = true;
            break;
        }
        let canonical = match root.canonicalize() {
            Ok(path) => path,
            Err(error) => {
                summary.error(root, error);
                continue;
            }
        };
        // 来源可以是目录,也可以是单个视频文件(只扫这一个文件,不扫它所在的目录)
        if !canonical.is_dir() && !canonical.is_file() {
            summary.error(root, "扫描来源不是可访问的目录或文件");
            continue;
        }
        let mut entries = WalkDir::new(&canonical).into_iter();
        loop {
            if cancel.is_cancelled() {
                summary.cancelled = true;
                break 'roots;
            }
            if entry_limit.is_some_and(|limit| summary.visited >= limit) {
                summary.truncated = true;
                break 'roots;
            }
            let Some(entry) = entries.next() else { break };
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    summary.visited += 1;
                    summary.error(error.path().unwrap_or(root), &error);
                    continue;
                }
            };
            let path = entry.path();
            if excluded
                .as_ref()
                .is_some_and(|excluded| path.starts_with(excluded))
            {
                if entry.file_type().is_dir() {
                    entries.skip_current_dir();
                }
                continue;
            }
            if entry.file_type().is_dir() && !seen_dirs.insert(path.to_path_buf()) {
                entries.skip_current_dir();
                continue;
            }
            summary.visited += 1;
            if !entry.file_type().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy();
            if name.starts_with("._") {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !VIDEO_EXT.contains(&ext.as_str()) {
                continue;
            }
            // 文件根:显示原路径本身,番号按文件名解析
            let (relative, display) = if path == canonical {
                (Path::new(entry.file_name()), root.clone())
            } else {
                let relative = path.strip_prefix(&canonical).unwrap_or(path);
                (relative, root.join(relative))
            };
            let size = match std::fs::metadata(path) {
                Ok(metadata) => metadata.len(),
                Err(error) => {
                    summary.error(&display, error);
                    continue;
                }
            };
            if let Some(reason) = filter.skip(&name, Some(size)) {
                summary.skipped_total += 1;
                on_skip(SkippedFile {
                    path: display,
                    size,
                    reason,
                });
                continue;
            }
            if item_limit.is_some_and(|limit| summary.total >= limit) {
                summary.truncated = true;
                break 'roots;
            }
            summary.total += 1;
            on_video(&display, relative, size);
        }
    }
    summary.cancelled |= cancel.is_cancelled();
    summary.complete = !summary.cancelled && !summary.truncated && summary.errors_total == 0;
    summary
}

/// Bounded counts only: no number parsing, media rows or skipped-file list.
pub fn scan_summary(
    roots: &[PathBuf],
    exclude: Option<&Path>,
    filter: &VideoFilter,
    cancel: &CancellationToken,
) -> ScanSummary {
    walk(
        roots,
        exclude,
        filter,
        cancel,
        Some(SUMMARY_ENTRY_LIMIT),
        None,
        &mut |_, _, _| {},
        &mut |_| {},
    )
}

fn collect(
    roots: &[PathBuf],
    exclude: Option<&Path>,
    filter: &VideoFilter,
    cancel: &CancellationToken,
    entry_limit: Option<usize>,
    item_limit: Option<usize>,
    on_item: &mut dyn FnMut(&ScanItem),
) -> ScanResult {
    let mut result = ScanResult::default();
    result.summary = walk(
        roots,
        exclude,
        filter,
        cancel,
        entry_limit,
        item_limit,
        &mut |path, relative, size| {
            let info = parse_file_info(&relative.to_string_lossy());
            let code = Code::from_info(&info);
            let item = ScanItem {
                id: result.items.len() as u32,
                path: path.into(),
                size,
                info,
                code,
            };
            on_item(&item);
            result.items.push(item);
        },
        &mut |skipped| result.skipped.push(skipped),
    );
    result.unrecognized = result
        .items
        .iter()
        .filter(|item| item.code.is_none())
        .count();
    result.codes = result
        .items
        .iter()
        .filter_map(|item| item.code.as_ref().map(|code| &code.id))
        .collect::<HashSet<_>>()
        .len();
    result
}

pub fn scan_preview(
    roots: &[PathBuf],
    exclude: Option<&Path>,
    filter: &VideoFilter,
    cancel: &CancellationToken,
    limit: usize,
) -> ScanResult {
    collect(
        roots,
        exclude,
        filter,
        cancel,
        Some(SUMMARY_ENTRY_LIMIT),
        Some(limit),
        &mut |_| {},
    )
}

/// Full imports use the same cancellation/error/pruning rules without limits.
pub fn scan(
    roots: &[PathBuf],
    exclude: Option<&Path>,
    filter: &VideoFilter,
    cancel: &CancellationToken,
    on_item: &mut dyn FnMut(&ScanItem),
) -> ScanResult {
    collect(roots, exclude, filter, cancel, None, None, on_item)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_video_file_root_yields_only_that_file() {
        let root = tempfile::tempdir().unwrap();
        let downloads = root.path().join("Downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        for name in ["ABC-123.mp4", "ABC-124.mp4", "ABC-123.chs.srt", "notes.txt"] { std::fs::write(downloads.join(name), b"v").unwrap(); }
        let file = downloads.join("ABC-123.mp4");
        let r = scan(&[file.clone()], None, &VideoFilter::default(), &CancellationToken::new(), &mut |_| {});
        assert!(r.summary.complete, "{:?}", r.summary.errors);
        assert_eq!(r.items.iter().map(|i| i.path.clone()).collect::<Vec<_>>(), vec![file.clone()], "siblings in the folder are not scanned");
        assert_eq!(r.items[0].code.as_ref().map(|c| c.id.as_str()), Some("ABC-123"));
        // A non-video file root yields nothing and is not an error; a file inside the library is excluded.
        let text = scan(&[downloads.join("notes.txt")], None, &VideoFilter::default(), &CancellationToken::new(), &mut |_| {});
        assert!(text.items.is_empty() && text.summary.complete);
        let excluded = scan(&[file.clone()], Some(&downloads), &VideoFilter::default(), &CancellationToken::new(), &mut |_| {});
        assert!(excluded.items.is_empty());
    }

    #[test]
    fn excluded_library_is_pruned_before_descending() {
        let root = tempfile::tempdir().unwrap();
        let library = root.path().join("library");
        for i in 0..100 {
            let dir = library.join(i.to_string());
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("ABC-123.mp4"), b"v").unwrap();
        }
        std::fs::write(root.path().join("ABC-456.mp4"), b"v").unwrap();
        let summary = scan_summary(
            &[root.path().into()],
            Some(&library),
            &VideoFilter::default(),
            &CancellationToken::new(),
        );
        assert_eq!(summary.total, 1);
        assert_eq!(
            summary.visited, 2,
            "the root and source video only; library descendants are never visited"
        );
        assert!(summary.complete);
        println!(
            "pruning: 203 disk entries -> {} visited entries",
            summary.visited
        );
    }
    #[test]
    fn summary_and_preview_stop_early_but_full_import_is_complete() {
        let root = tempfile::tempdir().unwrap();
        for i in 0..SUMMARY_ENTRY_LIMIT + 10 {
            std::fs::write(root.path().join(format!("ABC-{i:05}.mp4")), b"v").unwrap();
        }
        let token = CancellationToken::new();
        let filter = VideoFilter::default();
        let roots = [root.path().into()];
        let summary = scan_summary(&roots, None, &filter, &token);
        assert_eq!(summary.visited, SUMMARY_ENTRY_LIMIT);
        assert!(summary.truncated && !summary.complete);
        let preview = scan_preview(&roots, None, &filter, &token, 7);
        assert_eq!(preview.items.len(), 7);
        assert_eq!(preview.summary.total, 7);
        assert_eq!(
            preview.summary.visited, 9,
            "stop at the first extra video, instead of scanning all then truncating"
        );
        assert!(preview.summary.truncated);
        let full = scan(&roots, None, &filter, &token, &mut |_| {});
        assert_eq!(full.items.len(), SUMMARY_ENTRY_LIMIT + 10);
        assert!(full.summary.complete);
        println!(
            "preview: {} visited entries instead of {} full entries",
            preview.summary.visited, full.summary.visited
        );
    }
    #[test]
    fn cancellation_keeps_partial_results_and_does_not_visit_more_entries() {
        let root = tempfile::tempdir().unwrap();
        for i in 0..30 {
            std::fs::write(root.path().join(format!("ABC-{i:03}.mp4")), b"v").unwrap();
        }
        let token = CancellationToken::new();
        let result = scan(
            &[root.path().into()],
            None,
            &VideoFilter::default(),
            &token,
            &mut |_| token.cancel(),
        );
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.summary.visited, 2);
        assert!(result.summary.cancelled && !result.summary.complete);
        let empty = scan_summary(&[root.path().into()], None, &VideoFilter::default(), &token);
        assert_eq!(empty.visited, 0);
        assert!(empty.cancelled && !empty.complete);
    }
    #[cfg(unix)]
    #[test]
    fn aliases_and_overlapping_roots_preserve_paths_without_double_counting() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let library = source.join("library");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(source.join("ABC-123.mp4"), b"v").unwrap();
        std::fs::write(library.join("ABC-456.mp4"), b"v").unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        let result = scan(
            &[alias.clone(), source.clone()],
            Some(&library),
            &VideoFilter::default(),
            &CancellationToken::new(),
            &mut |_| {},
        );
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].path, alias.join("ABC-123.mp4"));
        assert_eq!(result.summary.visited, 2);
        assert!(result.summary.complete);
        let excluded = scan_summary(
            &[library.clone()],
            Some(&alias.join("library")),
            &VideoFilter::default(),
            &CancellationToken::new(),
        );
        assert_eq!(excluded.visited, 0);
        assert_eq!(excluded.total, 0);
    }
    #[test]
    fn mixed_available_and_missing_roots_report_partial_error() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("ABC-123.mp4"), b"v").unwrap();
        let result = scan_summary(
            &[root.path().into(), root.path().join("missing")],
            None,
            &VideoFilter::default(),
            &CancellationToken::new(),
        );
        assert_eq!(result.total, 1);
        assert_eq!(result.errors_total, 1);
        assert!(!result.complete);
        assert!(!result.cancelled && !result.truncated);
    }
}
