use std::path::Path;

use anyhow::{bail, Result};

/// Patch hunk types matching Codex's apply_patch format.
enum PatchHunk {
    AddFile { path: String, contents: String },
    DeleteFile { path: String },
    UpdateFile {
        path: String,
        move_to: Option<String>,
        chunks: Vec<UpdateChunk>,
    },
}

struct UpdateChunk {
    context: Option<String>,
    old_lines: Vec<String>,
    new_lines: Vec<String>,
}

/// Parse and apply a patch in Codex's custom format.
pub async fn execute(input: &str, work_dir: &str) -> Result<String> {
    let hunks = parse_patch(input)?;
    apply_hunks(&hunks, work_dir)
}

// --- Parser (state machine) ---

fn parse_patch(input: &str) -> Result<Vec<PatchHunk>> {
    let lines: Vec<&str> = input.lines().collect();
    let mut i = 0;
    let mut hunks = Vec::new();

    // Skip to "*** Begin Patch"
    while i < lines.len() && lines[i].trim() != "*** Begin Patch" {
        i += 1;
    }
    if i >= lines.len() {
        bail!("Missing '*** Begin Patch' header");
    }
    i += 1;

    while i < lines.len() {
        let line = lines[i];

        if line.trim() == "*** End Patch" {
            break;
        } else if let Some(path) = line.strip_prefix("*** Add File: ") {
            i += 1;
            let mut contents = String::new();
            while i < lines.len() && lines[i].starts_with('+') {
                if !contents.is_empty() {
                    contents.push('\n');
                }
                contents.push_str(&lines[i][1..]);
                i += 1;
            }
            hunks.push(PatchHunk::AddFile {
                path: path.trim().to_string(),
                contents,
            });
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            hunks.push(PatchHunk::DeleteFile {
                path: path.trim().to_string(),
            });
            i += 1;
        } else if let Some(path) = line.strip_prefix("*** Update File: ") {
            let path = path.trim().to_string();
            i += 1;

            // Optional move
            let mut move_to = None;
            if i < lines.len() {
                if let Some(dest) = lines[i].strip_prefix("*** Move to: ") {
                    move_to = Some(dest.trim().to_string());
                    i += 1;
                }
            }

            // Parse chunks
            let mut chunks = Vec::new();
            while i < lines.len()
                && !lines[i].starts_with("*** ")
            {
                if lines[i].starts_with("@@") {
                    let ctx = lines[i]
                        .strip_prefix("@@ ")
                        .or_else(|| lines[i].strip_prefix("@@"))
                        .unwrap_or("")
                        .to_string();
                    let context = if ctx.is_empty() { None } else { Some(ctx) };
                    i += 1;

                    let mut old_lines = Vec::new();
                    let mut new_lines = Vec::new();

                    while i < lines.len()
                        && !lines[i].starts_with("@@")
                        && !lines[i].starts_with("*** ")
                    {
                        let l = lines[i];
                        if let Some(rest) = l.strip_prefix('-') {
                            old_lines.push(rest.to_string());
                        } else if let Some(rest) = l.strip_prefix('+') {
                            new_lines.push(rest.to_string());
                        } else if let Some(rest) = l.strip_prefix(' ') {
                            old_lines.push(rest.to_string());
                            new_lines.push(rest.to_string());
                        } else if l == "*** End of File" {
                            i += 1;
                            break;
                        } else {
                            // Context line without prefix (treat as context)
                            old_lines.push(l.to_string());
                            new_lines.push(l.to_string());
                        }
                        i += 1;
                    }

                    chunks.push(UpdateChunk {
                        context,
                        old_lines,
                        new_lines,
                    });
                } else {
                    i += 1;
                }
            }

            hunks.push(PatchHunk::UpdateFile {
                path,
                move_to,
                chunks,
            });
        } else {
            i += 1;
        }
    }

    if hunks.is_empty() {
        bail!("No valid hunks found in patch");
    }
    Ok(hunks)
}

// --- Applier ---

fn apply_hunks(hunks: &[PatchHunk], work_dir: &str) -> Result<String> {
    let mut summary = Vec::new();

    for hunk in hunks {
        match hunk {
            PatchHunk::AddFile { path, contents } => {
                let full = resolve_path(work_dir, path);
                if let Some(parent) = Path::new(&full).parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&full, contents)?;
                summary.push(format!("Created {path}"));
            }
            PatchHunk::DeleteFile { path } => {
                let full = resolve_path(work_dir, path);
                std::fs::remove_file(&full)
                    .map_err(|e| anyhow::anyhow!("Failed to delete {path}: {e}"))?;
                summary.push(format!("Deleted {path}"));
            }
            PatchHunk::UpdateFile {
                path,
                move_to,
                chunks,
            } => {
                let full = resolve_path(work_dir, path);
                let content = std::fs::read_to_string(&full)
                    .map_err(|e| anyhow::anyhow!("Failed to read {path}: {e}"))?;

                let mut file_lines: Vec<String> =
                    content.lines().map(|l| l.to_string()).collect();

                // Compute all replacements
                let mut replacements: Vec<(usize, usize, Vec<String>)> = Vec::new();
                let mut search_start = 0;

                for chunk in chunks {
                    // Find where to apply this chunk
                    let start = if let Some(ctx) = &chunk.context {
                        seek_context(&file_lines, ctx, search_start)?
                    } else {
                        search_start
                    };

                    let match_start =
                        find_old_lines(&file_lines, &chunk.old_lines, start)?;

                    replacements.push((
                        match_start,
                        chunk.old_lines.len(),
                        chunk.new_lines.clone(),
                    ));
                    search_start = match_start + chunk.old_lines.len();
                }

                // Apply in reverse order
                replacements.sort_by(|a, b| b.0.cmp(&a.0));
                for (start, old_len, new_lines) in &replacements {
                    let end = (*start + old_len).min(file_lines.len());
                    file_lines.splice(*start..end, new_lines.iter().cloned());
                }

                let new_content = file_lines.join("\n");
                let new_content = if content.ends_with('\n') {
                    format!("{new_content}\n")
                } else {
                    new_content
                };

                if let Some(dest) = move_to {
                    let dest_full = resolve_path(work_dir, dest);
                    if let Some(parent) = Path::new(&dest_full).parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&dest_full, &new_content)?;
                    std::fs::remove_file(&full)?;
                    summary.push(format!("Moved {path} → {dest}"));
                } else {
                    std::fs::write(&full, &new_content)?;
                    summary.push(format!(
                        "Updated {path} ({} chunks applied)",
                        chunks.len()
                    ));
                }
            }
        }
    }

    Ok(summary.join("\n"))
}

// --- Fuzzy matching (3-level fallback like Codex) ---

fn seek_context(lines: &[String], context: &str, start: usize) -> Result<usize> {
    // Level 1: exact match
    for i in start..lines.len() {
        if lines[i] == context {
            return Ok(i);
        }
    }
    // Level 2: trim end
    for i in start..lines.len() {
        if lines[i].trim_end() == context.trim_end() {
            return Ok(i);
        }
    }
    // Level 3: trim both
    for i in start..lines.len() {
        if lines[i].trim() == context.trim() {
            return Ok(i);
        }
    }
    bail!(
        "Could not find context line: '{context}' (searched from line {})",
        start + 1
    )
}

fn find_old_lines(
    file_lines: &[String],
    old_lines: &[String],
    start: usize,
) -> Result<usize> {
    if old_lines.is_empty() {
        return Ok(start);
    }

    let len = old_lines.len();
    // Level 1: exact
    for i in start..=file_lines.len().saturating_sub(len) {
        if file_lines[i..i + len]
            .iter()
            .zip(old_lines)
            .all(|(a, b)| a == b)
        {
            return Ok(i);
        }
    }
    // Level 2: trim end
    for i in start..=file_lines.len().saturating_sub(len) {
        if file_lines[i..i + len]
            .iter()
            .zip(old_lines)
            .all(|(a, b)| a.trim_end() == b.trim_end())
        {
            return Ok(i);
        }
    }
    // Level 3: trim both
    for i in start..=file_lines.len().saturating_sub(len) {
        if file_lines[i..i + len]
            .iter()
            .zip(old_lines)
            .all(|(a, b)| a.trim() == b.trim())
        {
            return Ok(i);
        }
    }

    let preview: Vec<_> = old_lines.iter().take(3).collect();
    bail!(
        "Could not match old lines starting from line {}: {:?}",
        start + 1,
        preview
    )
}

fn resolve_path(work_dir: &str, path: &str) -> String {
    if Path::new(path).is_absolute() {
        path.to_string()
    } else {
        format!("{work_dir}/{path}")
    }
}
