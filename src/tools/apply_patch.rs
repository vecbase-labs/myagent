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
    is_end_of_file: bool,
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
                    let mut is_end_of_file = false;

                    while i < lines.len()
                        && !lines[i].starts_with("@@")
                        && !lines[i].starts_with("*** ")
                    {
                        let l = lines[i];
                        if l.trim() == "*** End of File" {
                            is_end_of_file = true;
                            i += 1;
                            break;
                        } else if let Some(rest) = l.strip_prefix('-') {
                            old_lines.push(rest.to_string());
                        } else if let Some(rest) = l.strip_prefix('+') {
                            new_lines.push(rest.to_string());
                        } else if let Some(rest) = l.strip_prefix(' ') {
                            old_lines.push(rest.to_string());
                            new_lines.push(rest.to_string());
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
                        is_end_of_file,
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

// --- Applier (matches Codex logic) ---

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

                // Split by \n (not .lines()) to match Codex behavior
                let mut file_lines: Vec<String> =
                    content.split('\n').map(String::from).collect();

                // Drop trailing empty element from final newline
                if file_lines.last().is_some_and(String::is_empty) {
                    file_lines.pop();
                }

                let replacements = compute_replacements(&file_lines, path, chunks)?;
                let mut new_lines = apply_replacements(file_lines, &replacements);

                // Ensure trailing newline
                if !new_lines.last().is_some_and(String::is_empty) {
                    new_lines.push(String::new());
                }
                let new_content = new_lines.join("\n");

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

/// Compute replacements matching Codex's compute_replacements logic.
fn compute_replacements(
    original_lines: &[String],
    path: &str,
    chunks: &[UpdateChunk],
) -> Result<Vec<(usize, usize, Vec<String>)>> {
    let mut replacements: Vec<(usize, usize, Vec<String>)> = Vec::new();
    let mut line_index: usize = 0;

    for chunk in chunks {
        // Use context line to narrow search position
        if let Some(ctx_line) = &chunk.context {
            if let Some(idx) = seek_sequence(
                original_lines,
                &[ctx_line.clone()],
                line_index,
                false,
            ) {
                line_index = idx + 1;
            } else {
                bail!(
                    "Failed to find context '{}' in {}",
                    ctx_line,
                    path
                );
            }
        }

        if chunk.old_lines.is_empty() {
            // Pure addition — insert at end
            let insertion_idx = if original_lines.last().is_some_and(String::is_empty) {
                original_lines.len() - 1
            } else {
                original_lines.len()
            };
            replacements.push((insertion_idx, 0, chunk.new_lines.clone()));
            continue;
        }

        // Try to find old_lines in the file
        let mut pattern: &[String] = &chunk.old_lines;
        let mut found = seek_sequence(original_lines, pattern, line_index, chunk.is_end_of_file);

        let mut new_slice: &[String] = &chunk.new_lines;

        // Retry without trailing empty line (Codex eof handling)
        if found.is_none() && pattern.last().is_some_and(String::is_empty) {
            pattern = &pattern[..pattern.len() - 1];
            if new_slice.last().is_some_and(String::is_empty) {
                new_slice = &new_slice[..new_slice.len() - 1];
            }
            found = seek_sequence(original_lines, pattern, line_index, chunk.is_end_of_file);
        }

        if let Some(start_idx) = found {
            replacements.push((start_idx, pattern.len(), new_slice.to_vec()));
            line_index = start_idx + pattern.len();
        } else {
            let preview: Vec<_> = chunk.old_lines.iter().take(3).collect();
            bail!(
                "Failed to find expected lines in {}:\n{:?}",
                path,
                preview
            );
        }
    }

    replacements.sort_by(|(a, _, _), (b, _, _)| a.cmp(b));
    Ok(replacements)
}

/// Apply replacements in reverse order to avoid index shifting.
fn apply_replacements(
    mut lines: Vec<String>,
    replacements: &[(usize, usize, Vec<String>)],
) -> Vec<String> {
    for (start_idx, old_len, new_segment) in replacements.iter().rev() {
        let start_idx = *start_idx;
        let old_len = *old_len;

        // Remove old lines
        for _ in 0..old_len {
            if start_idx < lines.len() {
                lines.remove(start_idx);
            }
        }

        // Insert new lines
        for (offset, new_line) in new_segment.iter().enumerate() {
            lines.insert(start_idx + offset, new_line.clone());
        }
    }

    lines
}

// --- seek_sequence: 4-level fuzzy matching (matches Codex) ---

/// Find `pattern` lines within `lines` starting at or after `start`.
/// When `eof` is true, first tries matching from end-of-file.
/// 4 levels: exact → trim_end → trim → Unicode normalization.
fn seek_sequence(
    lines: &[String],
    pattern: &[String],
    start: usize,
    eof: bool,
) -> Option<usize> {
    if pattern.is_empty() {
        return Some(start);
    }
    if pattern.len() > lines.len() {
        return None;
    }

    let search_start = if eof && lines.len() >= pattern.len() {
        lines.len() - pattern.len()
    } else {
        start
    };

    // Level 1: exact match
    for i in search_start..=lines.len().saturating_sub(pattern.len()) {
        if lines[i..i + pattern.len()] == *pattern {
            return Some(i);
        }
    }
    // Level 2: trim end
    for i in search_start..=lines.len().saturating_sub(pattern.len()) {
        let mut ok = true;
        for (p_idx, pat) in pattern.iter().enumerate() {
            if lines[i + p_idx].trim_end() != pat.trim_end() {
                ok = false;
                break;
            }
        }
        if ok {
            return Some(i);
        }
    }
    // Level 3: trim both
    for i in search_start..=lines.len().saturating_sub(pattern.len()) {
        let mut ok = true;
        for (p_idx, pat) in pattern.iter().enumerate() {
            if lines[i + p_idx].trim() != pat.trim() {
                ok = false;
                break;
            }
        }
        if ok {
            return Some(i);
        }
    }
    // Level 4: Unicode normalization (dashes, quotes, spaces)
    fn normalise(s: &str) -> String {
        s.trim()
            .chars()
            .map(|c| match c {
                // Various dash / hyphen code-points → ASCII '-'
                '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
                | '\u{2212}' => '-',
                // Fancy single quotes → '\''
                '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
                // Fancy double quotes → '"'
                '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
                // Non-breaking space and other odd spaces → normal space
                '\u{00A0}' | '\u{2002}' | '\u{2003}' | '\u{2004}' | '\u{2005}' | '\u{2006}'
                | '\u{2007}' | '\u{2008}' | '\u{2009}' | '\u{200A}' | '\u{202F}' | '\u{205F}'
                | '\u{3000}' => ' ',
                other => other,
            })
            .collect::<String>()
    }

    for i in search_start..=lines.len().saturating_sub(pattern.len()) {
        let mut ok = true;
        for (p_idx, pat) in pattern.iter().enumerate() {
            if normalise(&lines[i + p_idx]) != normalise(pat) {
                ok = false;
                break;
            }
        }
        if ok {
            return Some(i);
        }
    }

    None
}

fn resolve_path(work_dir: &str, path: &str) -> String {
    if Path::new(path).is_absolute() {
        path.to_string()
    } else {
        Path::new(work_dir).join(path).to_string_lossy().to_string()
    }
}
