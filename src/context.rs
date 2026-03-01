use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub struct SystemPrompt {
    pub prompt: String,
    pub context_files: Vec<PathBuf>,
}

/// Load the full system prompt from files and defaults.
///
/// Assembly order:
/// 1. Base prompt (SYSTEM.md or default)
/// 2. APPEND_SYSTEM.md files
/// 3. Working directory line
/// 4. Context files (AGENTS.md/CLAUDE.md)
pub fn load_system_prompt(working_dir: &Path) -> SystemPrompt {
    let home = env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    let home_dir = home.join(".tapir").join("agent");
    load_system_prompt_with_home(&home_dir, working_dir)
}

fn load_system_prompt_with_home(
    home_dir: &Path,
    working_dir: &Path,
) -> SystemPrompt {
    let mut prompt = load_base_prompt(home_dir, working_dir);

    prompt
        .push_str(&format!("\n\nWorking directory: {}", working_dir.display()));

    let (context, context_files) = find_context_files(home_dir, working_dir);
    if !context.is_empty() {
        prompt.push_str("\n\n---\n\n");
        prompt.push_str(&context);
    }

    SystemPrompt {
        prompt,
        context_files,
    }
}

const DEFAULT_PROMPT: &str = "\
You are a coding assistant. You help users with software \
engineering tasks including solving bugs, adding features, \
refactoring code, and explaining code.\n\n\
# Tools\n\n\
You have seven tools:\n\
- read_file: Read file contents with line numbers. \
Supports offset (1-indexed) and limit parameters for \
reading specific sections of large files.\n\
- write_file: Write content to a file (create or overwrite)\n\
- edit_file: Replace a unique string in a file. Supports \
fuzzy matching for whitespace and unicode variations \
(smart quotes, dashes) when exact match fails.\n\
- bash: Run a shell command\n\
- ls: List directory contents\n\
- find: Find files by glob pattern (uses fd)\n\
- grep: Search file contents by regex (uses ripgrep)\n\n\
All file paths are sandboxed to the working directory. \
Paths outside it will be rejected.\n\n\
# Guidelines\n\n\
- Read files before modifying them. Understand existing code \
before suggesting changes.\n\
- Use edit_file for targeted changes to existing files. \
Use write_file only for new files or complete rewrites.\n\
- Use ls, find, and grep to explore the codebase before \
making changes. Prefer these over bash for file discovery \
and search.\n\
- Do not create files unless necessary. Prefer editing \
existing files to creating new ones.\n\
- Keep changes minimal and focused. Only make changes that \
are directly requested or clearly necessary.\n\
- Do not add features, refactor code, or make improvements \
beyond what was asked.\n\
- Do not add error handling, comments, or type annotations \
to code you did not change.\n\
- Run tests after making changes when a test command is \
available.\n\n\
# Executing actions with care\n\n\
Consider the reversibility of your actions. You can freely \
read files and run non-destructive commands. But for \
actions that are hard to reverse or could be destructive, \
explain what you intend to do and why before proceeding.\n\n\
Examples of risky actions:\n\
- Deleting files or directories\n\
- Overwriting files with significant content\n\
- Running commands that modify system state\n\
- Git operations like force-push, reset --hard, or \
branch deletion\n\n\
When you encounter unexpected state (unfamiliar files, \
uncommitted changes, lock files), investigate before \
overwriting or deleting.\n\n\
# Security\n\n\
Be careful not to introduce security vulnerabilities such \
as command injection, XSS, SQL injection, and other common \
vulnerabilities. Prioritize writing safe, secure, and \
correct code.\n\n\
# Style\n\n\
- Be concise. Explain what you are doing briefly.\n\
- When referencing code, include file paths to help the \
user navigate.\n\
- Do not give time estimates for tasks.";

/// Load the base system prompt.
///
/// Priority:
/// 1. `working_dir/.tapir/SYSTEM.md` (project)
/// 2. `home_dir/SYSTEM.md` (global)
/// 3. Hardcoded default
///
/// Then append APPEND_SYSTEM.md from both locations.
fn load_base_prompt(home_dir: &Path, working_dir: &Path) -> String {
    let project_tapir = working_dir.join(".tapir");

    // Base: project > global > default
    let mut prompt = read_optional_file(&project_tapir.join("SYSTEM.md"))
        .or_else(|| read_optional_file(&home_dir.join("SYSTEM.md")))
        .unwrap_or_else(|| DEFAULT_PROMPT.to_string());

    // Append files
    for path in [
        project_tapir.join("APPEND_SYSTEM.md"),
        home_dir.join("APPEND_SYSTEM.md"),
    ] {
        if let Some(s) = read_optional_file(&path) {
            prompt.push_str("\n\n");
            prompt.push_str(&s);
        }
    }

    prompt
}

/// Discover and concatenate AGENTS.md/CLAUDE.md files.
///
/// Search order (root-first):
/// 1. `home_dir` (global ~/.tapir/agent/)
/// 2. Each ancestor of `working_dir` from root down
/// 3. `working_dir` itself
///
/// In each directory, prefer AGENTS.md over CLAUDE.md.
fn find_context_files(
    home_dir: &Path,
    working_dir: &Path,
) -> (String, Vec<PathBuf>) {
    let mut parts = Vec::new();
    let mut paths = Vec::new();

    // Global (AGENTS.md preferred, CLAUDE.md fallback)
    if let Some((s, path)) = read_context_in(home_dir) {
        parts.push(s);
        paths.push(path);
    }

    // Walk ancestors root-first (skip working_dir itself
    // and home_dir which is handled above as global)
    let ancestors: Vec<&Path> = working_dir.ancestors().skip(1).collect();
    for dir in ancestors.into_iter().rev() {
        if dir == home_dir {
            continue;
        }
        if let Some((s, path)) = read_context_in(dir) {
            parts.push(s);
            paths.push(path);
        }
    }

    // Working dir (skip if same as home_dir, already handled)
    if working_dir == home_dir {
        return (parts.join("\n\n"), paths);
    }
    if let Some((s, path)) = read_context_in(working_dir) {
        parts.push(s);
        paths.push(path);
    }

    (parts.join("\n\n"), paths)
}

/// Read AGENTS.md (preferred) or CLAUDE.md from a directory.
fn read_context_in(dir: &Path) -> Option<(String, PathBuf)> {
    let agents = dir.join("AGENTS.md");
    if let Some(s) = read_optional_file(&agents) {
        return Some((s, agents));
    }
    let claude = dir.join("CLAUDE.md");
    if let Some(s) = read_optional_file(&claude) {
        return Some((s, claude));
    }
    None
}

/// Format a path for display: `~` for home, `./` for working dir.
pub fn display_path(path: &Path, working_dir: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(working_dir) {
        return format!("./{}", rel.display());
    }
    if let Ok(home) = env::var("HOME")
        && let Ok(rel) = path.strip_prefix(&home)
    {
        return format!("~/{}", rel.display());
    }
    path.display().to_string()
}

/// Read a file if it exists. Returns `None` for missing
/// files, empty files, and read errors (logged to stderr).
fn read_optional_file(path: &Path) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(s) if s.trim().is_empty() => None,
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            eprintln!("warning: {}: {e}", path.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn read_optional_file_returns_content() {
        let dir = tempdir("ctx_read");
        let f = dir.join("test.md");
        fs::write(&f, "hello").unwrap();
        assert_eq!(read_optional_file(&f), Some("hello".into()));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_optional_file_returns_none_for_missing() {
        let path = std::env::temp_dir().join("tapir_no_such_file.md");
        assert_eq!(read_optional_file(&path), None);
    }

    #[test]
    fn read_optional_file_returns_none_for_empty() {
        let dir = tempdir("ctx_empty");
        let f = dir.join("empty.md");
        fs::write(&f, "  \n  ").unwrap();
        assert_eq!(read_optional_file(&f), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn find_context_files_from_nested_dirs() {
        let root = tempdir("ctx_find");
        let child = root.join("a").join("b");
        fs::create_dir_all(&child).unwrap();

        // AGENTS.md in root and child
        fs::write(root.join("AGENTS.md"), "root-ctx").unwrap();
        fs::write(child.join("AGENTS.md"), "child-ctx").unwrap();

        let (result, paths) = find_context_files(&root, &child);
        assert!(result.contains("root-ctx"));
        assert!(result.contains("child-ctx"));
        // root before child
        let root_pos = result.find("root-ctx").unwrap();
        let child_pos = result.find("child-ctx").unwrap();
        assert!(root_pos < child_pos);
        assert_eq!(paths.len(), 2);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_context_files_prefers_agents_over_claude() {
        let dir = tempdir("ctx_prefer");
        fs::write(dir.join("AGENTS.md"), "agents").unwrap();
        fs::write(dir.join("CLAUDE.md"), "claude").unwrap();

        let (result, paths) = find_context_files(&dir, &dir);
        assert!(result.contains("agents"));
        assert!(!result.contains("claude"));
        assert_eq!(paths, vec![dir.join("AGENTS.md")]);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn find_context_files_falls_back_to_claude() {
        let dir = tempdir("ctx_claude");
        fs::write(dir.join("CLAUDE.md"), "claude-content").unwrap();

        let (result, paths) = find_context_files(&dir, &dir);
        assert!(result.contains("claude-content"));
        assert_eq!(paths, vec![dir.join("CLAUDE.md")]);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_base_prompt_uses_default_when_no_files() {
        let dir = tempdir("ctx_default");
        let home = tempdir("ctx_default_home");
        let result = load_base_prompt(&home, &dir);
        assert!(result.contains("coding assistant"));
        fs::remove_dir_all(&dir).unwrap();
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn load_base_prompt_project_system_replaces_default() {
        let dir = tempdir("ctx_sys_proj");
        let tapir = dir.join(".tapir");
        fs::create_dir_all(&tapir).unwrap();
        fs::write(tapir.join("SYSTEM.md"), "custom prompt").unwrap();
        let home = tempdir("ctx_sys_proj_home");

        let result = load_base_prompt(&home, &dir);
        assert_eq!(result, "custom prompt");
        assert!(!result.contains("coding assistant"));

        fs::remove_dir_all(&dir).unwrap();
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn load_base_prompt_global_system_replaces_default() {
        let dir = tempdir("ctx_sys_glob");
        let home = tempdir("ctx_sys_glob_home");
        fs::write(home.join("SYSTEM.md"), "global prompt").unwrap();

        let result = load_base_prompt(&home, &dir);
        assert_eq!(result, "global prompt");

        fs::remove_dir_all(&dir).unwrap();
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn load_base_prompt_project_overrides_global() {
        let dir = tempdir("ctx_sys_over");
        let tapir = dir.join(".tapir");
        fs::create_dir_all(&tapir).unwrap();
        fs::write(tapir.join("SYSTEM.md"), "project").unwrap();
        let home = tempdir("ctx_sys_over_home");
        fs::write(home.join("SYSTEM.md"), "global").unwrap();

        let result = load_base_prompt(&home, &dir);
        assert_eq!(result, "project");

        fs::remove_dir_all(&dir).unwrap();
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn load_base_prompt_appends_append_files() {
        let dir = tempdir("ctx_append");
        let tapir = dir.join(".tapir");
        fs::create_dir_all(&tapir).unwrap();
        fs::write(tapir.join("APPEND_SYSTEM.md"), "project-extra").unwrap();
        let home = tempdir("ctx_append_home");
        fs::write(home.join("APPEND_SYSTEM.md"), "global-extra").unwrap();

        let result = load_base_prompt(&home, &dir);
        assert!(result.contains("coding assistant"));
        assert!(result.contains("project-extra"));
        assert!(result.contains("global-extra"));

        fs::remove_dir_all(&dir).unwrap();
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn load_system_prompt_assembles_all_parts() {
        let root = tempdir("ctx_full");
        let project = root.join("project");
        fs::create_dir_all(&project).unwrap();

        // Context file in root
        fs::write(root.join("AGENTS.md"), "root-instructions").unwrap();

        // Context file in project
        fs::write(project.join("AGENTS.md"), "project-instructions").unwrap();

        // Custom system prompt
        let tapir = project.join(".tapir");
        fs::create_dir_all(&tapir).unwrap();
        fs::write(tapir.join("SYSTEM.md"), "custom base").unwrap();
        fs::write(tapir.join("APPEND_SYSTEM.md"), "extra").unwrap();

        let home = tempdir("ctx_full_home");
        let sp = load_system_prompt_with_home(&home, &project);

        // Base prompt replaced
        assert!(sp.prompt.starts_with("custom base"));
        // Append applied
        assert!(sp.prompt.contains("extra"));
        // Context files present, in order
        let root_pos = sp.prompt.find("root-instructions").unwrap();
        let proj_pos = sp.prompt.find("project-instructions").unwrap();
        assert!(root_pos < proj_pos);
        // Working dir injected
        assert!(sp.prompt.contains("Working directory:"));
        // Paths tracked
        assert_eq!(sp.context_files.len(), 2);

        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&home).unwrap();
    }

    fn tempdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("tapir_{name}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }
}
