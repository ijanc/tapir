use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::signal;
use crate::types::{CacheControl, ToolDef};
use crate::util::{
    edit_diff, normalize_for_match, truncate_head, truncate_line, truncate_tail,
};

pub fn safe_path(working_dir: &Path, path: &str) -> Result<PathBuf> {
    let candidate = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        working_dir.join(path)
    };

    let canonical = candidate.canonicalize().map_err(|e| {
        Error::Security(format!("cannot resolve path {path}: {e}"))
    })?;

    let working_canonical = working_dir.canonicalize().map_err(|e| {
        Error::Security(format!("cannot resolve working dir: {e}"))
    })?;

    if !canonical.starts_with(&working_canonical) {
        return Err(Error::Security(format!(
            "path {path} is outside working directory"
        )));
    }

    Ok(canonical)
}

pub fn safe_path_for_write(working_dir: &Path, path: &str) -> Result<PathBuf> {
    let candidate = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        working_dir.join(path)
    };

    let parent = candidate.parent().ok_or_else(|| {
        Error::Security(format!("no parent directory for {path}"))
    })?;

    let parent_canonical = parent.canonicalize().map_err(|e| {
        Error::Security(format!("cannot resolve parent of {path}: {e}"))
    })?;

    let working_canonical = working_dir.canonicalize().map_err(|e| {
        Error::Security(format!("cannot resolve working dir: {e}"))
    })?;

    if !parent_canonical.starts_with(&working_canonical) {
        return Err(Error::Security(format!(
            "path {path} is outside working directory"
        )));
    }

    let filename = candidate
        .file_name()
        .ok_or_else(|| Error::Security(format!("no filename in {path}")))?;

    Ok(parent_canonical.join(filename))
}

pub fn definitions() -> Vec<ToolDef> {
    let mut tools = vec![
        ToolDef {
            name: "read_file".to_string(),
            description: "Read the contents of a file. \
                 Supports offset and limit for \
                 partial reads."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description":
                            "Path to the file to read"
                    },
                    "offset": {
                        "type": "integer",
                        "description":
                            "Line number to start from \
                             (1-indexed, default: 1)"
                    },
                    "limit": {
                        "type": "integer",
                        "description":
                            "Maximum number of lines to \
                             read"
                    }
                },
                "required": ["path"]
            }),
            cache_control: None,
        },
        ToolDef {
            name: "write_file".to_string(),
            description: "Write content to a file, creating it \
                 if it doesn't exist"
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description":
                            "Path to the file to write"
                    },
                    "content": {
                        "type": "string",
                        "description":
                            "Content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }),
            cache_control: None,
        },
        ToolDef {
            name: "edit_file".to_string(),
            description: "Edit a file by replacing a string \
                 match with new content. The old_string \
                 must appear exactly once in the file. \
                 Supports fuzzy matching for whitespace \
                 and unicode variations."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description":
                            "Path to the file to edit"
                    },
                    "old_string": {
                        "type": "string",
                        "description":
                            "String to find \
                             (must be unique in file)"
                    },
                    "new_string": {
                        "type": "string",
                        "description":
                            "String to replace it with"
                    }
                },
                "required": [
                    "path",
                    "old_string",
                    "new_string"
                ]
            }),
            cache_control: None,
        },
        ToolDef {
            name: "bash".to_string(),
            description: "Run a shell command".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description":
                            "Shell command to execute"
                    },
                    "timeout": {
                        "type": "integer",
                        "description":
                            "Timeout in seconds (default: 120)"
                    }
                },
                "required": ["command"]
            }),
            cache_control: None,
        },
        ToolDef {
            name: "ls".to_string(),
            description: "List directory contents, sorted \
                 alphabetically. Directories have a \
                 trailing /."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description":
                            "Directory to list \
                             (default: working directory)"
                    }
                }
            }),
            cache_control: None,
        },
        ToolDef {
            name: "find".to_string(),
            description: "Find files matching a glob pattern \
                 using fd. Returns up to 1000 results."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description":
                            "Glob pattern to search for \
                             (e.g. \"*.rs\", \"test_*\")"
                    },
                    "path": {
                        "type": "string",
                        "description":
                            "Directory to search in \
                             (default: working directory)"
                    }
                },
                "required": ["pattern"]
            }),
            cache_control: None,
        },
        ToolDef {
            name: "grep".to_string(),
            description: "Search file contents using ripgrep. \
                 Returns matching lines with file paths \
                 and line numbers."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description":
                            "Regex pattern to search for"
                    },
                    "path": {
                        "type": "string",
                        "description":
                            "File or directory to search \
                             (default: working directory)"
                    },
                    "context": {
                        "type": "integer",
                        "description":
                            "Lines of context around \
                             matches (default: 2)"
                    }
                },
                "required": ["pattern"]
            }),
            cache_control: None,
        },
    ];

    // Tag last tool with cache_control for prompt
    // caching (tools + system prompt cached together)
    if let Some(last) = tools.last_mut() {
        last.cache_control = Some(CacheControl::ephemeral());
    }

    tools
}

const READ_MAX_LINES: usize = 2000;
const READ_MAX_BYTES: usize = 50_000;
const BASH_MAX_LINES: usize = 1000;
const BASH_MAX_BYTES: usize = 30_000;
const LS_MAX_ENTRIES: usize = 500;
const LS_MAX_BYTES: usize = 30_000;
const GREP_LINE_MAX_CHARS: usize = 500;

pub fn execute(
    working_dir: &Path,
    name: &str,
    input: &serde_json::Value,
) -> Result<String> {
    match name {
        "read_file" => exec_read_file(working_dir, name, input),
        "write_file" => exec_write_file(working_dir, name, input),
        "edit_file" => exec_edit_file(working_dir, name, input),
        "bash" => exec_bash(working_dir, name, input),
        "ls" => exec_ls(working_dir, name, input),
        "find" => exec_find(working_dir, name, input),
        "grep" => exec_grep(working_dir, name, input),
        _ => Err(Error::Tool {
            name: name.to_string(),
            message: "unknown tool".to_string(),
        }),
    }
}

fn exec_read_file(
    working_dir: &Path,
    name: &str,
    input: &serde_json::Value,
) -> Result<String> {
    let path = input["path"].as_str().ok_or_else(|| Error::Tool {
        name: name.to_string(),
        message: "missing path".to_string(),
    })?;
    let offset = input["offset"].as_u64().map(|v| v as usize);
    let limit = input["limit"].as_u64().map(|v| v as usize);

    let resolved = safe_path(working_dir, path)?;
    let content = fs::read_to_string(&resolved)?;
    let total_lines = content.lines().count();

    // Apply offset/limit
    let start = offset.map(|o| o.saturating_sub(1)).unwrap_or(0);
    let lines: Vec<&str> = content.lines().collect();

    let selected: Vec<&str> = if let Some(lim) = limit {
        lines.iter().skip(start).take(lim).copied().collect()
    } else {
        lines.iter().skip(start).copied().collect()
    };

    // Number lines
    let mut numbered = String::new();
    for (i, line) in selected.iter().enumerate() {
        let line_num = start + i + 1;
        numbered.push_str(&format!("{line_num}\t{line}\n"));
    }

    // Apply truncation
    let (output, truncated) =
        truncate_head(&numbered, READ_MAX_LINES, READ_MAX_BYTES);

    if truncated {
        Ok(format!(
            "{output}\n\
             Hint: use offset/limit to read specific \
             sections (file has {total_lines} lines)"
        ))
    } else {
        Ok(output)
    }
}

fn exec_write_file(
    working_dir: &Path,
    name: &str,
    input: &serde_json::Value,
) -> Result<String> {
    let path = input["path"].as_str().ok_or_else(|| Error::Tool {
        name: name.to_string(),
        message: "missing path".to_string(),
    })?;
    let content = input["content"].as_str().ok_or_else(|| Error::Tool {
        name: name.to_string(),
        message: "missing content".to_string(),
    })?;
    let resolved = safe_path_for_write(working_dir, path)?;
    if let Some(parent) = resolved.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&resolved, content)?;
    Ok(format!("Wrote {} bytes to {}", content.len(), path))
}

fn exec_edit_file(
    working_dir: &Path,
    name: &str,
    input: &serde_json::Value,
) -> Result<String> {
    let path = input["path"].as_str().ok_or_else(|| Error::Tool {
        name: name.to_string(),
        message: "missing path".to_string(),
    })?;
    let old = input["old_string"].as_str().ok_or_else(|| Error::Tool {
        name: name.to_string(),
        message: "missing old_string".to_string(),
    })?;
    let new = input["new_string"].as_str().ok_or_else(|| Error::Tool {
        name: name.to_string(),
        message: "missing new_string".to_string(),
    })?;
    let resolved = safe_path(working_dir, path)?;
    let content = fs::read_to_string(&resolved)?;

    // Try exact match first
    let count = content.matches(old).count();
    if count == 1 {
        let updated = content.replacen(old, new, 1);
        fs::write(&resolved, &updated)?;
        let diff = edit_diff(path, &content, old, new);
        return Ok(format!("Edited {path}\n{diff}"));
    }
    if count > 1 {
        return Err(Error::Tool {
            name: name.to_string(),
            message: format!(
                "old_string appears {count} times in \
                 {path} (must be unique)"
            ),
        });
    }

    // Exact match failed — try fuzzy match
    match fuzzy_replace(&content, old, new) {
        Some(updated) => {
            // Find where the fuzzy match was to generate diff
            let norm_content = normalize_for_match(&content);
            let norm_old = normalize_for_match(old);
            let norm_pos = norm_content.find(&norm_old).unwrap_or(0);
            // Map back to find approximate original region
            let orig_pos = map_norm_offset_to_original(&content, norm_pos);
            // Use the original region for diff context
            let old_end = (orig_pos + old.len()).min(content.len());
            let orig_old = &content[orig_pos..old_end];
            let diff = edit_diff(path, &content, orig_old, new);
            fs::write(&resolved, &updated)?;
            Ok(format!("Edited {path} (fuzzy match)\n{diff}"))
        }
        None => Err(Error::Tool {
            name: name.to_string(),
            message: format!("old_string not found in {path}"),
        }),
    }
}

/// Try to replace `old` with `new` in `content` using
/// normalized (fuzzy) matching. Returns the updated
/// content if exactly one normalized match is found.
fn fuzzy_replace(content: &str, old: &str, new: &str) -> Option<String> {
    let norm_content = normalize_for_match(content);
    let norm_old = normalize_for_match(old);

    if norm_old.is_empty() {
        return None;
    }

    // Must match exactly once in normalized space
    let matches: Vec<usize> = norm_content
        .match_indices(&norm_old)
        .map(|(i, _)| i)
        .collect();
    if matches.len() != 1 {
        return None;
    }

    let norm_pos = matches[0];
    let orig_start = map_norm_offset_to_original(content, norm_pos);
    let orig_end =
        map_norm_offset_to_original(content, norm_pos + norm_old.len());

    let mut result = String::with_capacity(content.len());
    result.push_str(&content[..orig_start]);
    result.push_str(new);
    result.push_str(&content[orig_end..]);
    Some(result)
}

/// Map a byte offset in normalized text back to the
/// corresponding offset in the original text.
fn map_norm_offset_to_original(original: &str, norm_offset: usize) -> usize {
    let mut norm_pos = 0;
    let mut prev_ws = false;
    let mut orig_pos = 0;
    let mut started = false;

    for ch in original.chars() {
        if norm_pos >= norm_offset {
            break;
        }

        let mapped = match ch {
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201C}' | '\u{201D}' => '"',
            '\u{2013}' | '\u{2014}' => '-',
            other => other,
        };

        if mapped.is_whitespace() {
            if !prev_ws && started {
                norm_pos += 1; // single space in normalized
            }
            prev_ws = true;
        } else {
            norm_pos += mapped.len_utf8();
            prev_ws = false;
            started = true;
        }

        orig_pos += ch.len_utf8();
    }

    orig_pos
}

fn exec_bash(
    working_dir: &Path,
    name: &str,
    input: &serde_json::Value,
) -> Result<String> {
    let command = input["command"].as_str().ok_or_else(|| Error::Tool {
        name: name.to_string(),
        message: "missing command".to_string(),
    })?;
    let timeout_secs = input["timeout"].as_u64().unwrap_or(120).clamp(1, 600);
    let output = run_bash(working_dir, command, timeout_secs)?;

    let (truncated, was_truncated) =
        truncate_tail(&output, BASH_MAX_LINES, BASH_MAX_BYTES);
    if was_truncated {
        Ok(truncated)
    } else {
        Ok(output)
    }
}

fn exec_ls(
    working_dir: &Path,
    name: &str,
    input: &serde_json::Value,
) -> Result<String> {
    let dir = if let Some(p) = input["path"].as_str() {
        safe_path(working_dir, p)?
    } else {
        working_dir.to_path_buf()
    };

    let mut entries: Vec<String> = Vec::new();
    let read_dir = fs::read_dir(&dir).map_err(|e| Error::Tool {
        name: name.to_string(),
        message: format!("cannot read directory {}: {e}", dir.display()),
    })?;

    for entry in read_dir {
        let entry = entry.map_err(|e| Error::Tool {
            name: name.to_string(),
            message: format!("error reading entry: {e}"),
        })?;
        let name_str = entry.file_name().to_string_lossy().to_string();
        let file_type = entry.file_type().map_err(|e| Error::Tool {
            name: name.to_string(),
            message: format!("cannot get file type: {e}"),
        })?;
        if file_type.is_dir() {
            entries.push(format!("{name_str}/"));
        } else {
            entries.push(name_str);
        }
    }

    // Sort case-insensitively
    entries.sort_by_key(|a| a.to_lowercase());

    // Limit entries
    let total = entries.len();
    let mut output = String::new();
    let mut bytes = 0;

    for (count, entry) in entries.iter().enumerate() {
        if count >= LS_MAX_ENTRIES || bytes + entry.len() + 1 > LS_MAX_BYTES {
            output.push_str(&format!(
                "\n... ({total} entries total, showing {count})"
            ));
            break;
        }
        output.push_str(entry);
        output.push('\n');
        bytes += entry.len() + 1;
    }

    if output.is_empty() {
        output.push_str("(empty directory)");
    }

    Ok(output)
}

fn exec_find(
    working_dir: &Path,
    name: &str,
    input: &serde_json::Value,
) -> Result<String> {
    let pattern = input["pattern"].as_str().ok_or_else(|| Error::Tool {
        name: name.to_string(),
        message: "missing pattern".to_string(),
    })?;

    let search_dir = if let Some(p) = input["path"].as_str() {
        safe_path(working_dir, p)?
    } else {
        working_dir.to_path_buf()
    };

    let result = Command::new("fd")
        .arg("--glob")
        .arg(pattern)
        .arg("--max-results")
        .arg("1000")
        .current_dir(&search_dir)
        .output();

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !output.status.success() && !stderr.is_empty() {
                return Ok(format!("stderr: {stderr}"));
            }
            if stdout.is_empty() {
                return Ok("No files found matching pattern.".to_string());
            }
            let (out, _) =
                truncate_head(&stdout, READ_MAX_LINES, READ_MAX_BYTES);
            Ok(out)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok("Error: fd not found. Install it: \
                 https://github.com/sharkdp/fd"
                .to_string())
        }
        Err(e) => Err(Error::Tool {
            name: name.to_string(),
            message: format!("failed to run fd: {e}"),
        }),
    }
}

fn exec_grep(
    working_dir: &Path,
    name: &str,
    input: &serde_json::Value,
) -> Result<String> {
    let pattern = input["pattern"].as_str().ok_or_else(|| Error::Tool {
        name: name.to_string(),
        message: "missing pattern".to_string(),
    })?;

    let ctx = input["context"].as_u64().unwrap_or(2);

    let search_path = if let Some(p) = input["path"].as_str() {
        safe_path(working_dir, p)?
    } else {
        working_dir.to_path_buf()
    };

    let result = Command::new("rg")
        .arg("--json")
        .arg("--max-count")
        .arg("100")
        .arg("--context")
        .arg(ctx.to_string())
        .arg(pattern)
        .arg(&search_path)
        .current_dir(working_dir)
        .output();

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.is_empty() {
                return Ok("No matches found.".to_string());
            }
            Ok(format_rg_json(&stdout, working_dir))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok("Error: rg (ripgrep) not found. Install it: \
                 https://github.com/BurntSushi/ripgrep"
                .to_string())
        }
        Err(e) => Err(Error::Tool {
            name: name.to_string(),
            message: format!("failed to run rg: {e}"),
        }),
    }
}

/// Parse ripgrep JSON output into a compact, readable
/// format: `path\n  line_num:text`
fn format_rg_json(json_output: &str, working_dir: &Path) -> String {
    let mut output = String::new();
    let mut current_path: Option<String> = None;
    let wd = working_dir.to_string_lossy();

    for line in json_output.lines() {
        let Ok(obj) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        let msg_type = obj["type"].as_str().unwrap_or("");

        match msg_type {
            "match" | "context" => {
                let data = &obj["data"];
                let path_text = data["path"]["text"].as_str().unwrap_or("");
                let line_number = data["line_number"].as_u64().unwrap_or(0);
                let text = data["lines"]["text"]
                    .as_str()
                    .unwrap_or("")
                    .trim_end_matches('\n');
                let text = truncate_line(text, GREP_LINE_MAX_CHARS);

                // Strip working dir prefix for display
                let display_path = path_text
                    .strip_prefix(&*wd)
                    .map_or(path_text, |p| p.strip_prefix('/').unwrap_or(p));

                // Print path header on change
                let path_changed =
                    current_path.as_deref() != Some(display_path);
                if path_changed {
                    if current_path.is_some() {
                        output.push('\n');
                    }
                    output.push_str(display_path);
                    output.push('\n');
                    current_path = Some(display_path.to_string());
                }

                let sep = if msg_type == "match" { ':' } else { '-' };
                output.push_str(&format!("  {line_number}{sep}{text}\n"));
            }
            _ => {}
        }
    }

    if output.is_empty() {
        return "No matches found.".to_string();
    }

    // Apply truncation
    let (out, _) = truncate_head(&output, READ_MAX_LINES, READ_MAX_BYTES);
    out
}

#[cfg(target_os = "openbsd")]
pub fn shell_command() -> Command {
    Command::new("ksh")
}

#[cfg(not(target_os = "openbsd"))]
pub fn shell_command() -> Command {
    Command::new("bash")
}

fn format_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut result = String::new();
    if !stdout.is_empty() {
        result.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str("stderr: ");
        result.push_str(&stderr);
    }
    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&format!("exit code: {code}"));
    }
    if result.is_empty() {
        result.push_str("(no output, exit code 0)");
    }
    result
}

pub fn run_bash(
    working_dir: &Path,
    command: &str,
    timeout_secs: u64,
) -> Result<String> {
    let child = shell_command()
        .arg("-c")
        .arg(command)
        .current_dir(working_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let pid = child.id();
    let (tx, rx) = mpsc::channel();
    let timeout = Duration::from_secs(timeout_secs);

    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });

    let start = std::time::Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(result) => return Ok(format_output(&result?)),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if signal::is_interrupted() {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGKILL);
                    }
                    let _ = rx.recv();
                    return Err(Error::Tool {
                        name: "bash".to_string(),
                        message: "(cancelled)".to_string(),
                    });
                }
                if start.elapsed() >= timeout {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGKILL);
                    }
                    match rx.recv_timeout(Duration::from_secs(5)) {
                        Ok(Ok(output)) => {
                            let mut text = format_output(&output);
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(&format!(
                                "(timed out after \
                                 {timeout_secs}s)"
                            ));
                            return Ok(text);
                        }
                        _ => {
                            return Ok(format!(
                                "(timed out after \
                                 {timeout_secs}s)"
                            ));
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(Error::Tool {
                    name: "bash".to_string(),
                    message: "command thread panicked".to_string(),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    /// Serializes bash tests that depend on the global
    /// `INTERRUPTED` flag to avoid races between tests.
    static SIGNAL_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_safe_path_within_dir() {
        let dir = std::env::temp_dir().join("tapir_test");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("test.txt");
        fs::write(&file, "hello").unwrap();

        let result = safe_path(&dir, "test.txt");
        assert!(result.is_ok());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_safe_path_rejects_traversal() {
        let dir = std::env::temp_dir().join("tapir_test2");
        fs::create_dir_all(&dir).unwrap();

        let result = safe_path(&dir, "../../../etc/passwd");
        assert!(result.is_err());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_safe_path_for_write_new_file() {
        let dir = std::env::temp_dir().join("tapir_test3");
        fs::create_dir_all(&dir).unwrap();

        let result = safe_path_for_write(&dir, "new_file.txt");
        assert!(result.is_ok());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_safe_path_for_write_rejects_traversal() {
        let dir = std::env::temp_dir().join("tapir_test4");
        fs::create_dir_all(&dir).unwrap();

        let result = safe_path_for_write(&dir, "../../etc/evil.txt");
        assert!(result.is_err());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_bash_timeout() {
        use crate::signal;
        let _lock = SIGNAL_LOCK.lock().unwrap();
        signal::clear();
        let dir = std::env::temp_dir();
        let result = execute(
            &dir,
            "bash",
            &serde_json::json!({
                "command": "sleep 60",
                "timeout": 1
            }),
        );
        let output = result.unwrap();
        assert!(
            output.contains("timed out after 1s"),
            "expected timeout message, got: {output}",
        );
    }

    #[test]
    fn test_bash_no_timeout() {
        let dir = std::env::temp_dir();
        let result = execute(
            &dir,
            "bash",
            &serde_json::json!({
                "command": "echo hello",
                "timeout": 10
            }),
        );
        let output = result.unwrap();
        assert!(output.contains("hello"));
    }

    #[test]
    fn test_bash_default_timeout() {
        let dir = std::env::temp_dir();
        let result =
            execute(&dir, "bash", &serde_json::json!({ "command": "echo ok" }));
        assert!(result.unwrap().contains("ok"));
    }

    #[test]
    fn test_bash_interrupted() {
        use crate::signal;

        let _lock = SIGNAL_LOCK.lock().unwrap();
        let dir = std::env::temp_dir();
        signal::set();
        let start = std::time::Instant::now();
        let result = run_bash(&dir, "sleep 60", 120);
        signal::clear();

        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(5),
            "took too long: {elapsed:?}",
        );
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("(cancelled)"),
            "expected cancellation, got: {msg}",
        );
    }

    #[test]
    fn test_read_file_with_offset_limit() {
        let dir = std::env::temp_dir().join("tapir_read_offset");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("test.txt");
        fs::write(&file, "line1\nline2\nline3\nline4\nline5\n").unwrap();

        let result = execute(
            &dir,
            "read_file",
            &serde_json::json!({
                "path": "test.txt",
                "offset": 2,
                "limit": 2
            }),
        );
        let output = result.unwrap();
        assert!(output.contains("2\tline2"));
        assert!(output.contains("3\tline3"));
        assert!(!output.contains("1\tline1"));
        assert!(!output.contains("4\tline4"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_read_file_numbered() {
        let dir = std::env::temp_dir().join("tapir_read_numbered");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("test.txt");
        fs::write(&file, "alpha\nbeta\n").unwrap();

        let result = execute(
            &dir,
            "read_file",
            &serde_json::json!({ "path": "test.txt" }),
        );
        let output = result.unwrap();
        assert!(output.contains("1\talpha"));
        assert!(output.contains("2\tbeta"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_edit_file_with_diff() {
        let dir = std::env::temp_dir().join("tapir_edit_diff");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("test.txt");
        fs::write(&file, "line1\nline2\nline3\n").unwrap();

        let result = execute(
            &dir,
            "edit_file",
            &serde_json::json!({
                "path": "test.txt",
                "old_string": "line2",
                "new_string": "modified"
            }),
        );
        let output = result.unwrap();
        assert!(output.contains("Edited test.txt"));
        assert!(output.contains("-line2"));
        assert!(output.contains("+modified"));

        let content = fs::read_to_string(&file).unwrap();
        assert!(content.contains("modified"));
        assert!(!content.contains("line2"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_edit_file_fuzzy_match() {
        let dir = std::env::temp_dir().join("tapir_edit_fuzzy");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("test.txt");
        // File has smart quotes
        fs::write(&file, "hello \u{201C}world\u{201D} test\n").unwrap();

        let result = execute(
            &dir,
            "edit_file",
            &serde_json::json!({
                "path": "test.txt",
                "old_string": "\"world\"",
                "new_string": "\"earth\""
            }),
        );
        let output = result.unwrap();
        assert!(
            output.contains("fuzzy"),
            "expected fuzzy match notice, got: {output}"
        );

        let content = fs::read_to_string(&file).unwrap();
        assert!(content.contains("\"earth\""));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_ls_basic() {
        let dir = std::env::temp_dir().join("tapir_ls");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), "").unwrap();
        fs::write(dir.join("b.txt"), "").unwrap();
        fs::create_dir_all(dir.join("subdir")).unwrap();

        let result = execute(&dir, "ls", &serde_json::json!({}));
        let output = result.unwrap();
        assert!(output.contains("a.txt"));
        assert!(output.contains("b.txt"));
        assert!(output.contains("subdir/"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_fuzzy_replace_whitespace() {
        let content = "hello   world";
        let result = fuzzy_replace(content, "hello world", "goodbye");
        assert_eq!(result, Some("goodbye".to_string()));
    }

    #[test]
    fn test_fuzzy_replace_no_match() {
        let content = "hello world";
        let result = fuzzy_replace(content, "xyz", "abc");
        assert_eq!(result, None);
    }

    #[test]
    fn test_fuzzy_replace_multiple_matches() {
        let content = "hello hello";
        let result = fuzzy_replace(content, "hello", "bye");
        assert_eq!(result, None);
    }

    #[test]
    fn test_format_rg_json() {
        let json = r#"{"type":"match","data":{"path":{"text":"/tmp/test.rs"},"lines":{"text":"fn main() {\n"},"line_number":1}}"#;
        let wd = Path::new("/tmp");
        let result = format_rg_json(json, wd);
        assert!(result.contains("test.rs"));
        assert!(result.contains("1:fn main()"));
    }
}
