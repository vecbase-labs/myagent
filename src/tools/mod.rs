pub mod apply_patch;
pub mod grep_files;
pub mod list_dir;
pub mod read_file;
pub mod shell;

use anyhow::Result;
use serde_json::{json, Value};

use crate::ai::ToolDef;
use shell::Shell;

/// Whether a tool supports parallel execution (read lock).
/// Tools that return `false` take an exclusive write lock.
pub fn supports_parallel(name: &str) -> bool {
    matches!(name, "shell" | "read_file" | "list_dir" | "grep_files")
}

/// Build all tool definitions for the AI loop.
pub fn build_tool_definitions(shell: &Shell) -> Vec<ToolDef> {
    let shell_name = shell.shell_type.name();
    let shell_desc = match shell.shell_type {
        shell::ShellType::PowerShell => format!(
            "Execute a {shell_name} command and return its output. \
            This is a Windows system using PowerShell. Use PowerShell syntax \
            (e.g. Get-ChildItem instead of ls, Get-Content instead of cat). \
            Use this to run shell commands, including invoking AI coding agents."
        ),
        shell::ShellType::Cmd => format!(
            "Execute a {shell_name} command and return its output. \
            This is a Windows system using cmd.exe. Use Windows CMD syntax. \
            Use this to run shell commands, including invoking AI coding agents."
        ),
        _ => format!(
            "Execute a {shell_name} command and return its output. \
            Use this to run shell commands, including invoking AI coding agents \
            like `claude`, `codex`, or `gemini`. Always prefer specific commands \
            over interactive shells."
        ),
    };

    vec![
        ToolDef {
            name: "shell".to_string(),
            description: shell_desc,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": format!("The {shell_name} command to execute")
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "description": "Timeout in milliseconds (default: 120000)"
                    }
                },
                "required": ["command"]
            }),
        },
        ToolDef {
            name: "read_file".to_string(),
            description: "Read a file with 1-indexed line numbers. \
                Returns lines formatted as L{number}: {content}."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Absolute or relative file path"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "1-indexed line number to start from (default: 1)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of lines to return (default: 2000)"
                    }
                },
                "required": ["file_path"]
            }),
        },
        ToolDef {
            name: "list_dir".to_string(),
            description: "List directory entries recursively with type indicators. \
                Directories end with /, symlinks with @."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "dir_path": {
                        "type": "string",
                        "description": "Absolute or relative directory path"
                    },
                    "depth": {
                        "type": "integer",
                        "description": "Maximum traversal depth (default: 2)"
                    }
                },
                "required": ["dir_path"]
            }),
        },
        ToolDef {
            name: "grep_files".to_string(),
            description: "Search for files whose contents match a regex pattern. \
                Returns file paths sorted by modification time."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Regular expression pattern to search for"
                    },
                    "include": {
                        "type": "string",
                        "description": "Optional glob filter (e.g. \"*.rs\", \"*.py\")"
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory or file to search in (default: workspace)"
                    }
                },
                "required": ["pattern"]
            }),
        },
        ToolDef {
            name: "apply_patch".to_string(),
            description: "Apply file changes using a patch format. Supports creating, \
                deleting, updating, and moving files. Use this format:\n\
                *** Begin Patch\n\
                *** Add File: path\n\
                +new line\n\
                *** Delete File: path\n\
                *** Update File: path\n\
                @@ context line to locate\n\
                -old line\n\
                +new line\n\
                *** End Patch"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "patch": {
                        "type": "string",
                        "description": "The patch content in the format described above"
                    }
                },
                "required": ["patch"]
            }),
        },
    ]
}

/// Execute a tool by name and return the result as a string.
pub async fn execute_tool(
    name: &str,
    input: &Value,
    work_dir: &str,
    detected_shell: &Shell,
) -> Result<String> {
    match name {
        "shell" => {
            let command = input["command"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("shell tool requires 'command' string"))?;
            let timeout_ms = input["timeout_ms"].as_u64().unwrap_or(120_000);
            shell::execute(detected_shell, command, timeout_ms, work_dir).await
        }
        "read_file" => {
            let file_path = input["file_path"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("read_file requires 'file_path' string"))?;
            let offset = input["offset"].as_u64().unwrap_or(1) as usize;
            let limit = input["limit"].as_u64().unwrap_or(2000) as usize;
            read_file::execute(file_path, offset, limit, work_dir).await
        }
        "list_dir" => {
            let dir_path = input["dir_path"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("list_dir requires 'dir_path' string"))?;
            let depth = input["depth"].as_u64().unwrap_or(2) as usize;
            list_dir::execute(dir_path, depth, work_dir).await
        }
        "grep_files" => {
            let pattern = input["pattern"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("grep_files requires 'pattern' string"))?;
            let include = input["include"].as_str();
            let path = input["path"].as_str();
            let limit = input["limit"].as_u64().unwrap_or(100) as usize;
            grep_files::execute(pattern, include, path, limit, work_dir).await
        }
        "apply_patch" => {
            let patch = input["patch"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("apply_patch requires 'patch' string"))?;
            apply_patch::execute(patch, work_dir).await
        }
        _ => Err(anyhow::anyhow!("Unknown tool: {name}")),
    }
}
