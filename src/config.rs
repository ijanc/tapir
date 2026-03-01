use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};

#[derive(Default, Deserialize)]
struct FileConfig {
    api_key: Option<String>,
    model: Option<String>,
    max_tokens: Option<u32>,
    thinking_budget: Option<u32>,
    api_url: Option<String>,
    #[serde(default, rename = "_models")]
    models: HashMap<String, ModelInfo>,
    #[serde(default)]
    skills: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[allow(dead_code)]
pub struct ModelInfo {
    pub context: u32,
    pub max_output: u32,
    pub input_cost_per_m: f64,
    pub output_cost_per_m: f64,
    pub extended_thinking: bool,
    #[serde(default)]
    pub notes: String,
}

pub struct Config {
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub thinking_budget: u32,
    pub api_url: String,
    pub working_dir: PathBuf,
    pub session_dir: PathBuf,
    pub system_prompt: String,
    pub context_files: Vec<PathBuf>,
    pub model_info: Option<ModelInfo>,
    pub models: HashMap<String, ModelInfo>,
    pub skills: Vec<crate::skill::Skill>,
    /// Cached full prompt (system_prompt + skills).
    /// Built lazily on first API call.
    pub full_prompt: Option<String>,
}

impl Config {
    pub fn load(config_path: Option<&str>) -> Result<Self> {
        let home = env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let tapir_dir = PathBuf::from(&home).join(".tapir");
        let file_cfg = match config_path {
            Some(p) => load_file_config_from(Path::new(p)),
            None => load_file_config(&tapir_dir),
        };

        let api_key = env::var("ANTHROPIC_API_KEY")
            .ok()
            .or(file_cfg.api_key)
            .ok_or(Error::NoApiKey)?;

        let model = env::var("TAPIR_MODEL")
            .ok()
            .or(file_cfg.model)
            .unwrap_or_else(|| "claude-opus-4-6".into());

        let max_tokens = env::var("TAPIR_MAX_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok())
            .or(file_cfg.max_tokens)
            .unwrap_or(16384);

        let thinking_budget = env::var("TAPIR_THINKING_BUDGET")
            .ok()
            .and_then(|v| v.parse().ok())
            .or(file_cfg.thinking_budget)
            .unwrap_or(0);

        let api_url = env::var("TAPIR_API_URL")
            .ok()
            .or(file_cfg.api_url)
            .unwrap_or_else(|| "https://api.anthropic.com/v1/messages".into());

        let working_dir = env::current_dir()?;
        let encoded = encode_path(&working_dir);
        let session_dir = tapir_dir.join("sessions").join(&encoded);

        let sp = crate::context::load_system_prompt(&working_dir);

        let skills =
            crate::skill::discover_skills(&working_dir, &file_cfg.skills);

        let model_info = file_cfg.models.get(&model).cloned();
        let models = file_cfg.models;

        Ok(Config {
            api_key,
            model,
            max_tokens,
            thinking_budget,
            api_url,
            working_dir,
            session_dir,
            system_prompt: sp.prompt,
            context_files: sp.context_files,
            model_info,
            models,
            skills,
            full_prompt: None,
        })
    }

    /// Build and cache the full system prompt (base + skills)
    /// if not already built.
    pub fn ensure_full_prompt(&mut self) {
        if self.full_prompt.is_some() {
            return;
        }
        let skill_prompt = crate::skill::format_skills(&self.skills);
        self.full_prompt = Some(if skill_prompt.is_empty() {
            self.system_prompt.clone()
        } else {
            format!("{}\n\n{}", self.system_prompt, skill_prompt,)
        });
    }

    /// Return the full system prompt. Panics if
    /// `ensure_full_prompt()` has not been called.
    pub fn full_prompt(&self) -> &str {
        self.full_prompt.as_deref().unwrap_or(&self.system_prompt)
    }
}

fn load_file_config(tapir_dir: &Path) -> FileConfig {
    load_file_config_from(&tapir_dir.join("config.json"))
}

fn load_file_config_from(path: &Path) -> FileConfig {
    let Ok(text) = std::fs::read_to_string(path) else {
        return FileConfig::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("warning: {}: {e}", path.display());
        FileConfig::default()
    })
}

/// Encode a path for use as a directory name.
/// `/home/ijanc/src/tapir` → `-home-ijanc-src-tapir`
fn encode_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    s.replace('/', "-")
}
