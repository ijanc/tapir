use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

/// Parsed YAML frontmatter fields.
#[cfg_attr(not(test), allow(dead_code))]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
    body_start: usize,
}

/// Parse YAML frontmatter from a SKILL.md file's content.
fn parse_frontmatter(content: &str) -> Option<Frontmatter> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return None;
    }
    let after_open = &content[3..];
    let after_open = after_open
        .strip_prefix('\n')
        .or_else(|| after_open.strip_prefix("\r\n"))?;

    // Handle empty frontmatter (closing --- on next line)
    let (close, yaml_block) = if after_open.starts_with("---") {
        (0, "")
    } else {
        let pos = after_open.find("\n---")?;
        (pos + 1, &after_open[..pos])
    };
    let dash_start = content.len() - after_open.len() + close;
    let body_start = dash_start + 3;
    // Skip newline after closing ---
    let body_start = if content.as_bytes().get(body_start) == Some(&b'\n') {
        body_start + 1
    } else if content.as_bytes().get(body_start) == Some(&b'\r')
        && content.as_bytes().get(body_start + 1) == Some(&b'\n')
    {
        body_start + 2
    } else {
        body_start
    };

    let mut name = None;
    let mut description = None;
    for line in yaml_block.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "name" => name = Some(value.to_string()),
                "description" => description = Some(value.to_string()),
                _ => {}
            }
        }
    }

    Some(Frontmatter {
        name,
        description,
        body_start,
    })
}

fn validate_name(name: &str) -> std::result::Result<(), String> {
    if name.is_empty() || name.len() > 64 {
        return Err(format!("name must be 1-64 chars, got {}", name.len()));
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err("name must not start or end with hyphen".into());
    }
    if name.contains("--") {
        return Err("name must not contain consecutive hyphens".into());
    }
    for ch in name.chars() {
        if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() && ch != '-' {
            return Err(format!(
                "invalid character '{ch}' \
                 (lowercase a-z, 0-9, hyphens only)"
            ));
        }
    }
    Ok(())
}

fn load_skill_file(path: &Path) -> Option<Skill> {
    let content = fs::read_to_string(path).ok()?;
    let fm = parse_frontmatter(&content)?;

    let name = fm.name?;
    let description = fm.description.filter(|d| !d.is_empty())?;

    if let Err(e) = validate_name(&name) {
        eprintln!("warning: {}: {e}", path.display());
    }

    if path.file_name().map(|f| f == "SKILL.md").unwrap_or(false)
        && let Some(parent_name) = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
        && parent_name != name
    {
        eprintln!(
            "warning: {}: name '{}' doesn't \
             match directory '{}'",
            path.display(),
            name,
            parent_name,
        );
    }

    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    Some(Skill {
        name,
        description,
        path,
    })
}

fn load_from_dir(dir: &Path) -> Vec<Skill> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut skills = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            let skill_md = path.join("SKILL.md");
            if skill_md.is_file()
                && let Some(s) = load_skill_file(&skill_md)
            {
                skills.push(s);
            }
        } else if path.is_file()
            && path.extension().is_some_and(|e| e == "md")
            && let Some(s) = load_skill_file(&path)
        {
            skills.push(s);
        }
    }

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// Discover skills from an ordered list of directories.
/// First occurrence of a name wins; duplicates warn.
fn discover_skills_from_dirs(dirs: &[PathBuf]) -> Vec<Skill> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();

    for dir in dirs {
        for skill in load_from_dir(dir) {
            if seen.contains(&skill.name) {
                eprintln!(
                    "warning: duplicate skill '{}' \
                     in {}, skipping",
                    skill.name,
                    dir.display(),
                );
                continue;
            }
            seen.insert(skill.name.clone());
            result.push(skill);
        }
    }

    result
}

/// Find the git repo root by walking up from `dir`.
fn git_root(dir: &Path) -> Option<PathBuf> {
    let mut current = dir;
    loop {
        if current.join(".git").exists() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

/// Main entry point: discover all skills from standard
/// locations plus config paths.
pub fn discover_skills(
    working_dir: &Path,
    config_paths: &[String],
) -> Vec<Skill> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let home = PathBuf::from(&home);
    let boundary = git_root(working_dir);

    let mut dirs = Vec::new();

    // 1. Global
    dirs.push(home.join(".tapir").join("agent").join("skills"));
    dirs.push(home.join(".agents").join("skills"));

    // 2. Ancestors (root-first), up to git root or fs root
    let ancestors: Vec<&Path> = working_dir.ancestors().skip(1).collect();
    for dir in ancestors.into_iter().rev() {
        if let Some(ref root) = boundary
            && !dir.starts_with(root)
        {
            continue;
        }
        dirs.push(dir.join(".agents").join("skills"));
    }

    // 3. Project
    dirs.push(working_dir.join(".tapir").join("skills"));
    dirs.push(working_dir.join(".agents").join("skills"));

    // 4. Config paths
    for path_str in config_paths {
        let p = if path_str.starts_with('~') {
            home.join(path_str.trim_start_matches("~/"))
        } else {
            PathBuf::from(path_str)
        };
        dirs.push(p);
    }

    discover_skills_from_dirs(&dirs)
}

pub fn format_skills(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out = String::from("<available-skills>\n");
    for skill in skills {
        out.push_str(&format!(
            "<skill name=\"{}\" path=\"{}\">\n\
             {}\n</skill>\n",
            skill.name,
            skill.path.display(),
            skill.description,
        ));
    }
    out.push_str("</available-skills>");
    out
}

/// Extract the body of a SKILL.md after the frontmatter.
pub fn skill_body(content: &str) -> &str {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content;
    }
    let after_open = match trimmed[3..]
        .strip_prefix('\n')
        .or_else(|| trimmed[3..].strip_prefix("\r\n"))
    {
        Some(s) => s,
        None => return content,
    };
    match after_open.find("\n---") {
        Some(close) => {
            let rest = &after_open[close + 4..];
            rest.strip_prefix('\n')
                .or_else(|| rest.strip_prefix("\r\n"))
                .unwrap_or(rest)
        }
        None => content,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_frontmatter() {
        let content = "---\nname: my-skill\n\
                        description: A test skill\n---\n\
                        # Body";
        let fm = parse_frontmatter(content).unwrap();
        assert_eq!(fm.name.as_deref(), Some("my-skill"));
        assert_eq!(fm.description.as_deref(), Some("A test skill"));
        assert_eq!(&content[fm.body_start..].trim_start(), &"# Body");
    }

    #[test]
    fn parse_frontmatter_with_optional_fields() {
        let content = "---\nname: pdf-tools\n\
                        description: PDF processing\n\
                        license: MIT\n\
                        compatibility: Requires poppler\n\
                        ---\nBody";
        let fm = parse_frontmatter(content).unwrap();
        assert_eq!(fm.name.as_deref(), Some("pdf-tools"));
        assert_eq!(fm.description.as_deref(), Some("PDF processing"),);
    }

    #[test]
    fn parse_frontmatter_missing_fences() {
        assert!(parse_frontmatter("no fences here").is_none());
    }

    #[test]
    fn parse_frontmatter_empty() {
        let fm = parse_frontmatter("---\n---\n").unwrap();
        assert!(fm.name.is_none());
        assert!(fm.description.is_none());
    }

    #[test]
    fn validate_name_valid() {
        assert!(validate_name("pdf-tools").is_ok());
        assert!(validate_name("a").is_ok());
        assert!(validate_name("my-skill-123").is_ok());
    }

    #[test]
    fn validate_name_empty() {
        assert!(validate_name("").is_err());
    }

    #[test]
    fn validate_name_too_long() {
        let long = "a".repeat(65);
        assert!(validate_name(&long).is_err());
    }

    #[test]
    fn validate_name_uppercase() {
        assert!(validate_name("My-Skill").is_err());
    }

    #[test]
    fn validate_name_leading_hyphen() {
        assert!(validate_name("-skill").is_err());
    }

    #[test]
    fn validate_name_trailing_hyphen() {
        assert!(validate_name("skill-").is_err());
    }

    #[test]
    fn validate_name_consecutive_hyphens() {
        assert!(validate_name("my--skill").is_err());
    }

    #[test]
    fn validate_name_invalid_chars() {
        assert!(validate_name("my_skill").is_err());
        assert!(validate_name("my skill").is_err());
    }

    #[test]
    fn skill_body_strips_frontmatter() {
        let content = "---\nname: x\n---\n# Body\ntext";
        assert_eq!(skill_body(content), "# Body\ntext");
    }

    #[test]
    fn skill_body_no_frontmatter() {
        assert_eq!(skill_body("just text"), "just text");
    }

    fn tempdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tapir_skill_{name}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn load_from_dir_subdirectory_skill() {
        let dir = tempdir("subdir");
        let skill_dir = dir.join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\n\
             description: A test skill\n---\n# Body",
        )
        .unwrap();

        let skills = load_from_dir(&dir);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "my-skill");
        assert_eq!(skills[0].description, "A test skill");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_from_dir_direct_md() {
        let dir = tempdir("direct");
        std::fs::write(
            dir.join("quick-tool.md"),
            "---\nname: quick-tool\n\
             description: A quick tool\n---\nContent",
        )
        .unwrap();

        let skills = load_from_dir(&dir);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "quick-tool");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_from_dir_skips_missing_description() {
        let dir = tempdir("nodesc");
        let skill_dir = dir.join("no-desc");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: no-desc\n---\nBody",
        )
        .unwrap();

        let skills = load_from_dir(&dir);
        assert!(skills.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_from_dir_warns_name_mismatch() {
        let dir = tempdir("mismatch");
        let skill_dir = dir.join("wrong-name");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: right-name\n\
             description: Test\n---\nBody",
        )
        .unwrap();

        let skills = load_from_dir(&dir);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "right-name");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_from_dir_empty() {
        let dir = tempdir("empty_dir");
        let skills = load_from_dir(&dir);
        assert!(skills.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_from_dir_nonexistent() {
        let dir = PathBuf::from("/tmp/tapir_skill_nonexistent");
        let skills = load_from_dir(&dir);
        assert!(skills.is_empty());
    }

    #[test]
    fn format_skills_empty() {
        assert_eq!(format_skills(&[]), "");
    }

    #[test]
    fn format_skills_produces_xml() {
        let skills = vec![Skill {
            name: "test-skill".into(),
            description: "Does testing".into(),
            path: PathBuf::from("/tmp/test/SKILL.md"),
        }];
        let xml = format_skills(&skills);
        assert!(xml.contains("<available-skills>"));
        assert!(xml.contains("</available-skills>"));
        assert!(xml.contains(
            r#"<skill name="test-skill" path="/tmp/test/SKILL.md">"#
        ));
        assert!(xml.contains("Does testing"));
        assert!(xml.contains("</skill>"));
    }

    #[test]
    fn discover_deduplicates_by_name() {
        let dir1 = tempdir("dedup1");
        let dir2 = tempdir("dedup2");

        let s1 = dir1.join("my-skill");
        std::fs::create_dir_all(&s1).unwrap();
        std::fs::write(
            s1.join("SKILL.md"),
            "---\nname: my-skill\n\
             description: First\n---\nBody",
        )
        .unwrap();

        let s2 = dir2.join("my-skill");
        std::fs::create_dir_all(&s2).unwrap();
        std::fs::write(
            s2.join("SKILL.md"),
            "---\nname: my-skill\n\
             description: Second\n---\nBody",
        )
        .unwrap();

        let skills = discover_skills_from_dirs(&[dir1.clone(), dir2.clone()]);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].description, "First");

        std::fs::remove_dir_all(&dir1).unwrap();
        std::fs::remove_dir_all(&dir2).unwrap();
    }

    #[test]
    fn discover_merges_multiple_dirs() {
        let dir1 = tempdir("merge1");
        let dir2 = tempdir("merge2");

        let s1 = dir1.join("skill-a");
        std::fs::create_dir_all(&s1).unwrap();
        std::fs::write(
            s1.join("SKILL.md"),
            "---\nname: skill-a\n\
             description: A\n---\n",
        )
        .unwrap();

        let s2 = dir2.join("skill-b");
        std::fs::create_dir_all(&s2).unwrap();
        std::fs::write(
            s2.join("SKILL.md"),
            "---\nname: skill-b\n\
             description: B\n---\n",
        )
        .unwrap();

        let skills = discover_skills_from_dirs(&[dir1.clone(), dir2.clone()]);
        assert_eq!(skills.len(), 2);

        std::fs::remove_dir_all(&dir1).unwrap();
        std::fs::remove_dir_all(&dir2).unwrap();
    }
}
