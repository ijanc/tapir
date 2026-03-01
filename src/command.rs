use crate::config::Config;
use crate::display::ToolOutputLog;
use crate::error::Result;
use crate::readline::Editor;
use crate::session;
use crate::tool;
use crate::types::{Content, Message, Role};
use crate::util::{floor_char_boundary, truncate};

use super::agent::Session;

/// What happened after reading one line of user input.
pub enum InputResult {
    /// A user message was added — proceed to the LLM.
    Ready,
    /// A command was handled — prompt again.
    Continue,
    /// The user wants to quit.
    Quit,
    /// /new was requested — restart the session.
    New,
}

// ----------------------------------------------------------
// Slash-command dispatch
// ----------------------------------------------------------

fn handle_command(
    line: &str,
    config: &mut Config,
    session: &mut Session,
    at_startup: bool,
) -> InputResult {
    let (cmd, arg) = match line.split_once(' ') {
        Some((c, a)) => (c, a.trim()),
        None => (line, ""),
    };

    if let Some(name) = cmd.strip_prefix("/skill:") {
        return handle_skill_command(name, arg, config, session);
    }

    match cmd {
        "/help" => {
            print_help();
            InputResult::Continue
        }
        "/quit" | "/exit" => InputResult::Quit,
        "/new" => {
            if at_startup {
                eprintln!("* already a new session");
                InputResult::Continue
            } else {
                InputResult::New
            }
        }
        "/resume" => {
            if !at_startup {
                eprintln!(
                    "* /resume only works before the \
                     first prompt"
                );
                return InputResult::Continue;
            }
            match try_resume(config, &mut session.entry) {
                Some((file, msgs, _pct)) => {
                    session.file = file;
                    session.messages = msgs;
                    eprintln!(
                        "session: {} (resumed, {} msgs)",
                        session.file.display(),
                        session.messages.len(),
                    );
                    InputResult::Ready
                }
                None => {
                    eprintln!(
                        "* no sessions found for this \
                         directory"
                    );
                    InputResult::Continue
                }
            }
        }
        "/name" => {
            if arg.is_empty() {
                if session.entry.summary.is_empty() {
                    eprintln!("* no name set");
                } else {
                    eprintln!("* {}", session.entry.summary);
                }
            } else {
                session.entry.summary = arg.to_string();
                session::update_entry(&config.session_dir, &session.entry);
                eprintln!("* name: {arg}");
            }
            InputResult::Continue
        }
        "/session" => {
            print_session_info(config, session);
            InputResult::Continue
        }
        "/model" => {
            if arg.is_empty() {
                print_models(config);
            } else {
                switch_model(config, arg);
            }
            InputResult::Continue
        }
        "/hotkeys" => {
            print_hotkeys();
            InputResult::Continue
        }
        "/skills" => {
            if config.skills.is_empty() {
                eprintln!("* no skills loaded");
            } else {
                for skill in &config.skills {
                    let desc = if skill.description.len() > 60 {
                        let end = floor_char_boundary(&skill.description, 57);
                        format!("{}...", &skill.description[..end])
                    } else {
                        skill.description.clone()
                    };
                    eprintln!("  {:20} {}", skill.name, desc,);
                }
            }
            InputResult::Continue
        }
        _ => {
            eprintln!("* unknown command: {cmd}");
            print_help();
            InputResult::Continue
        }
    }
}

fn handle_skill_command(
    name: &str,
    arg: &str,
    config: &Config,
    session: &mut Session,
) -> InputResult {
    let skill = match config.skills.iter().find(|s| s.name == name) {
        Some(s) => s,
        None => {
            eprintln!("* unknown skill: {name}");
            eprintln!("* use /skills to list available skills");
            return InputResult::Continue;
        }
    };

    let content = match std::fs::read_to_string(&skill.path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("* error reading {}: {e}", skill.path.display());
            return InputResult::Continue;
        }
    };

    let body = crate::skill::skill_body(&content);
    let mut text =
        format!("<skill name=\"{}\">\n{}\n</skill>", skill.name, body,);
    if !arg.is_empty() {
        text.push_str(&format!("\n\nUser: {arg}"));
    }

    if session.entry.first_prompt == "No prompt" {
        session.entry.first_prompt = format!("/skill:{name}");
    }
    add_user_message(session, &text);
    InputResult::Ready
}

fn print_help() {
    eprintln!("  /resume          Resume last session");
    eprintln!("  /new             Start a new session");
    eprintln!("  /model [name]    Show or switch model");
    eprintln!("  /name <name>     Set session display name");
    eprintln!("  /session         Show session info");
    eprintln!("  /quit, /exit     Quit tapir");
    eprintln!("  /help            Show this help");
    eprintln!();
    eprintln!("  !cmd             Run cmd, send output to LLM");
    eprintln!("  !!cmd            Run cmd, don't send to LLM");
    eprintln!();
    eprintln!("  /hotkeys         Show keyboard shortcuts");
    eprintln!("  /skills          List available skills");
    eprintln!("  /skill:name      Load and execute a skill");
}

fn print_hotkeys() {
    eprintln!("  Navigation:");
    eprintln!("    Left/Right       Move cursor");
    eprintln!("    Ctrl+Left/Right  Move by word");
    eprintln!("    Ctrl-A / Home    Beginning of line");
    eprintln!("    Ctrl-E / End     End of line");
    eprintln!("    Up / Ctrl-P      Previous history");
    eprintln!("    Down / Ctrl-N    Next history");
    eprintln!();
    eprintln!("  Editing:");
    eprintln!("    Backspace        Delete char before cursor");
    eprintln!("    Delete           Delete char at cursor");
    eprintln!("    Ctrl-U           Clear entire line");
    eprintln!("    Ctrl-K           Delete to end of line");
    eprintln!("    Ctrl-W           Delete word backward");
    eprintln!("    Ctrl-G           Open external editor");
    eprintln!("    Tab              Complete @path");
    eprintln!();
    eprintln!("  Control:");
    eprintln!("    Enter            Submit input");
    eprintln!("    Ctrl-C           Cancel current line");
    eprintln!("    Ctrl-D           Quit (on empty line)");
    eprintln!("    Ctrl-O           Toggle tool output");
}

fn print_session_info(config: &Config, session: &Session) {
    eprintln!("  path:     {}", session.file.display());
    eprintln!("  id:       {}", session.entry.session_id);
    if !session.entry.summary.is_empty() {
        eprintln!("  name:     {}", session.entry.summary);
    }
    eprintln!("  model:    {}", config.model);
    eprintln!("  messages: {}", session.messages.len());
    if let Some(pct) = session.token_pct {
        eprintln!("  context:  {pct}%");
    }
    let (in_cost, out_cost) = match &config.model_info {
        Some(m) => (m.input_cost_per_m, m.output_cost_per_m),
        None => (3.0, 15.0),
    };
    let cost_in = session.total_input_tokens as f64 / 1_000_000.0 * in_cost;
    let cost_out = session.total_output_tokens as f64 / 1_000_000.0 * out_cost;
    let cost = cost_in + cost_out;
    eprintln!(
        "  tokens:   {} in / {} out",
        session.total_input_tokens, session.total_output_tokens,
    );
    eprintln!("  cost:     ${cost:.4}");
    eprintln!("  created:  {}", session.entry.created);
    eprintln!("  modified: {}", session.entry.modified);
    if !session.entry.git_branch.is_empty() {
        eprintln!("  branch:   {}", session.entry.git_branch);
    }
}

fn print_models(config: &Config) {
    eprintln!("  current: {}", config.model);
    if config.models.is_empty() {
        eprintln!(
            "  (no models in config, set any model \
             ID with /model <id>)"
        );
        return;
    }
    eprintln!();
    let mut names: Vec<&String> = config.models.keys().collect();
    names.sort();
    for name in names {
        let info = &config.models[name];
        let marker = if *name == config.model { " *" } else { "" };
        eprintln!(
            "  {name}{marker}  \
             ${}/{}  ctx={}K  out={}K",
            info.input_cost_per_m,
            info.output_cost_per_m,
            info.context / 1000,
            info.max_output / 1000,
        );
        if !info.notes.is_empty() {
            eprintln!("    {}", info.notes);
        }
    }
}

fn switch_model(config: &mut Config, name: &str) {
    config.model = name.to_string();
    config.model_info = config.models.get(name).cloned();
    eprintln!("* model: {name}");
    if config.model_info.is_none() && !config.models.is_empty() {
        eprintln!("  (not in config, pricing unknown)");
    }
}

// ----------------------------------------------------------
// Shell escape helpers
// ----------------------------------------------------------

enum ShellInput {
    Text(String),
    Send(String),
    Discard(String),
}

fn classify_input(line: &str) -> ShellInput {
    if let Some(cmd) = line.strip_prefix("!!") {
        ShellInput::Discard(cmd.to_string())
    } else if let Some(cmd) = line.strip_prefix('!') {
        ShellInput::Send(cmd.to_string())
    } else {
        ShellInput::Text(line.to_string())
    }
}

fn run_shell(working_dir: &std::path::Path, cmd: &str) -> String {
    tool::run_bash(working_dir, cmd, 30)
        .unwrap_or_else(|e| format!("error: {e}"))
}

// ----------------------------------------------------------
// Read one line of user input, handling commands & shell
// escapes. Returns InputResult to tell the caller what to
// do next.
// ----------------------------------------------------------

pub fn read_input(
    editor: &mut Editor,
    config: &mut Config,
    session: &mut Session,
    tool_log: &mut ToolOutputLog,
    at_startup: bool,
) -> Result<InputResult> {
    loop {
        eprintln!();
        let prompt = match session.token_pct {
            Some(p) => format!("{p}% \x1b[1m>\x1b[0m "),
            None => "\x1b[1m>\x1b[0m ".to_string(),
        };
        let line = match editor.readline(&prompt, Some(tool_log))? {
            Some(line) if !line.is_empty() => line,
            _ => return Ok(InputResult::Quit),
        };

        if line == "?" {
            print_help();
            continue;
        }

        // Slash commands
        if line.starts_with('/') {
            match handle_command(&line, config, session, at_startup) {
                InputResult::Continue => continue,
                other => return Ok(other),
            }
        }

        // Shell escapes
        match classify_input(&line) {
            ShellInput::Discard(cmd) => {
                let out = run_shell(&config.working_dir, &cmd);
                eprintln!("{out}");
                continue;
            }
            ShellInput::Send(cmd) => {
                let out = run_shell(&config.working_dir, &cmd);
                eprintln!("{out}");
                let text = format!("Shell command: {cmd}\nOutput:\n{out}");
                add_user_message(session, &text);
                if session.entry.first_prompt == "No prompt" {
                    session.entry.first_prompt = truncate(&cmd, 100);
                }
                return Ok(InputResult::Ready);
            }
            ShellInput::Text(text) => {
                if session.entry.first_prompt == "No prompt" {
                    session.entry.first_prompt = truncate(&text, 100);
                }
                add_user_message(session, &text);
                return Ok(InputResult::Ready);
            }
        }
    }
}

fn add_user_message(session: &mut Session, text: &str) {
    session.push_message(Message {
        role: Role::User,
        content: Content::Text(text.to_string()),
    });
}

fn try_resume(
    config: &Config,
    entry: &mut session::SessionEntry,
) -> Option<(std::path::PathBuf, Vec<Message>, Option<u32>)> {
    let latest = session::latest_entry(&config.session_dir)?;
    let path = session::session_path(&latest);
    let msgs = super::agent::load_session(&path).ok()?;
    if msgs.is_empty() {
        return None;
    }
    let pct = super::agent::load_token_pct(&path);
    *entry = latest;
    Some((path, msgs, pct))
}
