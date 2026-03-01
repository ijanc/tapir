use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct SessionIndex {
    pub version: u32,
    pub entries: Vec<SessionEntry>,
    #[serde(rename = "originalPath")]
    pub original_path: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "fullPath")]
    pub full_path: String,
    #[serde(rename = "firstPrompt")]
    pub first_prompt: String,
    pub summary: String,
    #[serde(rename = "messageCount")]
    pub message_count: u32,
    pub created: String,
    pub modified: String,
    #[serde(rename = "gitBranch")]
    pub git_branch: String,
    #[serde(rename = "projectPath")]
    pub project_path: String,
}

fn index_path(session_dir: &Path) -> PathBuf {
    session_dir.join("sessions-index.json")
}

pub fn load_index(session_dir: &Path) -> SessionIndex {
    let path = index_path(session_dir);
    let Ok(text) = fs::read_to_string(&path) else {
        return empty_index("");
    };
    serde_json::from_str(&text).unwrap_or_else(|_| empty_index(""))
}

pub fn save_index(session_dir: &Path, index: &SessionIndex) {
    let path = index_path(session_dir);
    let Ok(json) = serde_json::to_string_pretty(index) else {
        return;
    };
    let _ = fs::write(path, json);
}

fn empty_index(project_path: &str) -> SessionIndex {
    SessionIndex {
        version: 1,
        entries: Vec::new(),
        original_path: project_path.to_string(),
    }
}

pub fn create_entry(session_dir: &Path, working_dir: &Path) -> SessionEntry {
    let id = gen_uuid();
    let full_path = session_dir.join(format!("{id}.jsonl"));
    let now = iso_now();
    let branch = git_branch(working_dir);

    SessionEntry {
        session_id: id,
        full_path: full_path.to_string_lossy().to_string(),
        first_prompt: "No prompt".to_string(),
        summary: String::new(),
        message_count: 0,
        created: now.clone(),
        modified: now,
        git_branch: branch,
        project_path: working_dir.to_string_lossy().to_string(),
    }
}

pub fn update_entry(session_dir: &Path, entry: &SessionEntry) {
    let mut index = load_index(session_dir);
    if let Some(e) = index
        .entries
        .iter_mut()
        .find(|e| e.session_id == entry.session_id)
    {
        *e = entry.clone();
    } else {
        index.entries.push(entry.clone());
    }
    if index.original_path.is_empty() {
        index.original_path = entry.project_path.clone();
    }
    save_index(session_dir, &index);
}

pub fn latest_entry(session_dir: &Path) -> Option<SessionEntry> {
    let index = load_index(session_dir);
    index
        .entries
        .into_iter()
        .max_by(|a, b| a.modified.cmp(&b.modified))
}

pub fn session_path(entry: &SessionEntry) -> PathBuf {
    PathBuf::from(&entry.full_path)
}

fn gen_uuid() -> String {
    let mut buf = [0u8; 16];
    if let Ok(mut f) = fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut buf);
    }
    // Set version 4 and variant bits
    buf[6] = (buf[6] & 0x0f) | 0x40;
    buf[8] = (buf[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-\
         {:02x}{:02x}-\
         {:02x}{:02x}-\
         {:02x}{:02x}-\
         {:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        buf[0],
        buf[1],
        buf[2],
        buf[3],
        buf[4],
        buf[5],
        buf[6],
        buf[7],
        buf[8],
        buf[9],
        buf[10],
        buf[11],
        buf[12],
        buf[13],
        buf[14],
        buf[15],
    )
}

pub fn iso_now() -> String {
    unsafe {
        let mut t: libc::time_t = 0;
        libc::time(&mut t);
        let mut tm: libc::tm = std::mem::zeroed();
        libc::gmtime_r(&t, &mut tm);
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
        )
    }
}

fn git_branch(working_dir: &Path) -> String {
    let output = std::process::Command::new("git")
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("HEAD")
        .current_dir(working_dir)
        .output();
    match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        }
        _ => String::new(),
    }
}
