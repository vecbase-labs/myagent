use std::collections::VecDeque;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::Result;
use tokio::fs;

const MAX_ENTRY_LENGTH: usize = 500;
const INDENTATION_SPACES: usize = 2;

/// List directory entries with BFS traversal, depth control, pagination, and type indicators.
/// Matches Codex list_dir behavior.
pub async fn execute(
    dir_path: &str,
    depth: usize,
    offset: usize,
    limit: usize,
    work_dir: &str,
) -> Result<String> {
    let depth = if depth == 0 { 2 } else { depth };
    let offset = if offset == 0 { 1 } else { offset };
    let limit = if limit == 0 { 25 } else { limit };

    let path = if Path::new(dir_path).is_absolute() {
        PathBuf::from(dir_path)
    } else {
        Path::new(work_dir).join(dir_path)
    };

    if !path.is_dir() {
        return Err(anyhow::anyhow!("{} is not a directory", path.display()));
    }

    let mut entries = Vec::new();
    collect_entries(&path, Path::new(""), depth, &mut entries).await?;

    if entries.is_empty() {
        return Ok("(empty directory)".to_string());
    }

    entries.sort_unstable_by(|a, b| a.sort_key.cmp(&b.sort_key));

    let start_index = offset - 1;
    if start_index >= entries.len() {
        return Err(anyhow::anyhow!("offset exceeds directory entry count"));
    }

    let remaining = entries.len() - start_index;
    let capped_limit = limit.min(remaining);
    let end_index = start_index + capped_limit;
    let selected = &entries[start_index..end_index];

    let mut output = Vec::with_capacity(selected.len() + 2);
    output.push(format!("Absolute path: {}", path.display()));

    for entry in selected {
        output.push(format_entry_line(entry));
    }

    if end_index < entries.len() {
        output.push(format!("More than {capped_limit} entries found"));
    }

    Ok(output.join("\n"))
}

struct DirEntry {
    sort_key: String,
    display_name: String,
    depth: usize,
    kind: DirEntryKind,
}

#[derive(Clone, Copy, PartialEq)]
enum DirEntryKind {
    Directory,
    File,
    Symlink,
    Other,
}

async fn collect_entries(
    dir_path: &Path,
    relative_prefix: &Path,
    depth: usize,
    entries: &mut Vec<DirEntry>,
) -> Result<()> {
    let mut queue = VecDeque::new();
    queue.push_back((dir_path.to_path_buf(), relative_prefix.to_path_buf(), depth));

    while let Some((current_dir, prefix, remaining_depth)) = queue.pop_front() {
        let mut read_dir = fs::read_dir(&current_dir).await
            .map_err(|e| anyhow::anyhow!("failed to read directory: {e}"))?;

        let mut dir_entries = Vec::new();

        while let Some(entry) = read_dir.next_entry().await
            .map_err(|e| anyhow::anyhow!("failed to read directory: {e}"))? {
            let file_type = entry.file_type().await
                .map_err(|e| anyhow::anyhow!("failed to inspect entry: {e}"))?;

            let file_name = entry.file_name();
            let relative_path = if prefix.as_os_str().is_empty() {
                PathBuf::from(&file_name)
            } else {
                prefix.join(&file_name)
            };

            let display_name = truncate_name(&file_name);
            let display_depth = prefix.components().count();
            let sort_key = normalize_path(&relative_path);
            let kind = classify(&file_type);

            dir_entries.push((
                entry.path(),
                relative_path,
                kind,
                DirEntry { sort_key, display_name, depth: display_depth, kind },
            ));
        }

        dir_entries.sort_unstable_by(|a, b| a.3.sort_key.cmp(&b.3.sort_key));

        for (entry_path, relative_path, kind, dir_entry) in dir_entries {
            if kind == DirEntryKind::Directory && remaining_depth > 1 {
                queue.push_back((entry_path, relative_path, remaining_depth - 1));
            }
            entries.push(dir_entry);
        }
    }

    Ok(())
}

fn normalize_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    let normalized = s.replace('\\', "/");
    if normalized.len() > MAX_ENTRY_LENGTH {
        take_at_char_boundary(&normalized, MAX_ENTRY_LENGTH)
    } else {
        normalized
    }
}

fn truncate_name(name: &OsStr) -> String {
    let s = name.to_string_lossy();
    if s.len() > MAX_ENTRY_LENGTH {
        take_at_char_boundary(&s, MAX_ENTRY_LENGTH)
    } else {
        s.to_string()
    }
}

fn take_at_char_boundary(s: &str, max: usize) -> String {
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

fn format_entry_line(entry: &DirEntry) -> String {
    let indent = " ".repeat(entry.depth * INDENTATION_SPACES);
    let mut name = entry.display_name.clone();
    match entry.kind {
        DirEntryKind::Directory => name.push('/'),
        DirEntryKind::Symlink => name.push('@'),
        DirEntryKind::Other => name.push('?'),
        DirEntryKind::File => {}
    }
    format!("{indent}{name}")
}

fn classify(ft: &std::fs::FileType) -> DirEntryKind {
    if ft.is_symlink() {
        DirEntryKind::Symlink
    } else if ft.is_dir() {
        DirEntryKind::Directory
    } else if ft.is_file() {
        DirEntryKind::File
    } else {
        DirEntryKind::Other
    }
}
