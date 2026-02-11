use std::collections::VecDeque;
use std::path::Path;

use anyhow::Result;

const MAX_ENTRIES: usize = 500;
const MAX_NAME_LENGTH: usize = 500;

/// List directory entries with BFS traversal, depth control, and type indicators.
pub async fn execute(dir_path: &str, depth: usize, work_dir: &str) -> Result<String> {
    let depth = if depth == 0 { 2 } else { depth };

    let path = if Path::new(dir_path).is_absolute() {
        dir_path.to_string()
    } else {
        format!("{work_dir}/{dir_path}")
    };

    let root = Path::new(&path);
    if !root.is_dir() {
        return Err(anyhow::anyhow!("{path} is not a directory"));
    }

    let mut entries: Vec<(usize, String, &str)> = Vec::new(); // (depth, name, suffix)
    let mut queue: VecDeque<(std::path::PathBuf, usize)> = VecDeque::new();
    queue.push_back((root.to_path_buf(), 0));

    while let Some((dir, current_depth)) = queue.pop_front() {
        if entries.len() >= MAX_ENTRIES {
            break;
        }

        let mut children: Vec<std::fs::DirEntry> = match std::fs::read_dir(&dir) {
            Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
            Err(_) => continue,
        };
        children.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

        for child in children {
            if entries.len() >= MAX_ENTRIES {
                break;
            }

            let name = child.file_name().to_string_lossy().to_string();
            let display_name = if name.len() > MAX_NAME_LENGTH {
                format!("{}...", &name[..MAX_NAME_LENGTH])
            } else {
                name
            };

            let ft = child.file_type().unwrap_or_else(|_| {
                // fallback: treat as file
                std::fs::metadata(child.path())
                    .map(|m| m.file_type())
                    .unwrap_or_else(|_| child.file_type().unwrap())
            });

            let suffix = if ft.is_dir() {
                "/"
            } else if ft.is_symlink() {
                "@"
            } else {
                ""
            };

            entries.push((current_depth, display_name, suffix));

            if ft.is_dir() && current_depth + 1 < depth {
                queue.push_back((child.path(), current_depth + 1));
            }
        }
    }

    if entries.is_empty() {
        return Ok("(empty directory)".to_string());
    }

    let mut output = Vec::with_capacity(entries.len());
    for (d, name, suffix) in &entries {
        let indent = "  ".repeat(*d);
        output.push(format!("{indent}{name}{suffix}"));
    }

    if entries.len() >= MAX_ENTRIES {
        output.push(format!("\n... (truncated at {MAX_ENTRIES} entries)"));
    }

    Ok(output.join("\n"))
}
