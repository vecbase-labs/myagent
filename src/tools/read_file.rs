use std::path::Path;

use anyhow::Result;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};

const MAX_LINE_LENGTH: usize = 500;

/// Read a file with 1-indexed line numbers, offset, and limit.
/// Output format: `L{line_number}: {content}`
pub async fn execute(file_path: &str, offset: usize, limit: usize, work_dir: &str) -> Result<String> {
    let offset = if offset == 0 { 1 } else { offset };
    let limit = if limit == 0 { 2000 } else { limit };

    let path = if Path::new(file_path).is_absolute() {
        Path::new(file_path).to_path_buf()
    } else {
        Path::new(work_dir).join(file_path)
    };

    let file = File::open(&path).await
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", path.display()))?;

    let mut reader = BufReader::new(file);
    let mut collected = Vec::new();
    let mut line_num = 0usize;
    let mut buf = Vec::new();

    loop {
        buf.clear();
        let bytes_read = reader.read_until(b'\n', &mut buf).await
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", path.display()))?;

        if bytes_read == 0 {
            break;
        }

        // Strip trailing newline / CRLF
        if buf.last() == Some(&b'\n') {
            buf.pop();
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
        }

        line_num += 1;

        if line_num < offset {
            continue;
        }
        if collected.len() >= limit {
            break;
        }

        let line = format_line(&buf);
        collected.push(format!("L{line_num}: {line}"));
    }

    if line_num < offset {
        return Err(anyhow::anyhow!(
            "offset {offset} exceeds file length ({line_num} lines)"
        ));
    }

    if collected.is_empty() {
        Ok("(empty file)".to_string())
    } else {
        Ok(collected.join("\n"))
    }
}

fn format_line(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    if s.len() > MAX_LINE_LENGTH {
        // Truncate at char boundary
        let mut end = MAX_LINE_LENGTH;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    } else {
        s.to_string()
    }
}
