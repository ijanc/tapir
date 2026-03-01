use std::fmt::Write as FmtWrite;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use crate::api;
use crate::config::Config;
use crate::display::ToolOutputLog;
use crate::error::Result;
use crate::readline::Editor;
use crate::session;
use crate::signal;
use crate::sse::{Delta, SseEvent};
use crate::stream;
use crate::tool;
use crate::types::{
    Content, ContentBlock, Message, Request, Role, StopReason, SystemBlock,
};
use crate::util::truncate;

const COMPACT_THRESHOLD: u32 = 160_000;
const KEEP_RECENT_TOKENS: u32 = 40_000;

/// Mutable state shared across the session, passed to
/// command handlers to avoid excessive parameters.
pub(crate) struct Session {
    pub(crate) entry: session::SessionEntry,
    pub(crate) file: PathBuf,
    pub(crate) messages: Vec<Message>,
    pub(crate) token_pct: Option<u32>,
    pub(crate) total_input_tokens: u64,
    pub(crate) total_output_tokens: u64,
}

impl Session {
    pub(crate) fn push_message(&mut self, msg: Message) {
        save_message(&self.file, &msg);
        self.messages.push(msg);
    }
}

// ----------------------------------------------------------
// Main entry point
// ----------------------------------------------------------

use crate::command::{self, InputResult};

pub fn run(config: &mut Config) -> Result<()> {
    fs::create_dir_all(&config.session_dir)?;

    let tools = tool::definitions();
    let mut editor = Editor::new()?;

    // Outer loop: each iteration is one full session.
    // /new restarts this loop.
    loop {
        let entry =
            session::create_entry(&config.session_dir, &config.working_dir);
        let file = session::session_path(&entry);

        let mut session = Session {
            entry,
            file,
            messages: Vec::new(),
            token_pct: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
        };

        if !config.context_files.is_empty() {
            eprintln!("context:");
            for path in &config.context_files {
                eprintln!(
                    "  - {}",
                    crate::context::display_path(path, &config.working_dir,)
                );
            }
            eprintln!();
        }
        if !config.skills.is_empty() {
            eprintln!("skills:");
            let limit = 5;
            for skill in config.skills.iter().take(limit) {
                eprintln!("  - {}", skill.name);
            }
            let total = config.skills.len();
            if total > limit {
                eprintln!("  ({total} total, use /skills to see all)");
            }
            eprintln!();
        }
        eprintln!("cwd:     {}", config.working_dir.display());
        eprintln!("session: {}", session.entry.session_id);
        eprintln!();

        // Initial input (supports /resume, /help, etc.)
        let mut empty_log = ToolOutputLog::new();
        match command::read_input(
            &mut editor,
            config,
            &mut session,
            &mut empty_log,
            true,
        )? {
            InputResult::Quit => {
                eprintln!("bye");
                return Ok(());
            }
            InputResult::New => continue,
            InputResult::Continue => unreachable!(),
            InputResult::Ready => {}
        }

        // Persist new entry in index (or updated after
        // resume)
        session.entry.message_count = session.messages.len() as u32;
        session::update_entry(&config.session_dir, &session.entry);

        // Conversation loop for this session
        if run_session(config, &tools, &mut editor, &mut session)? {
            // /new was requested — loop to create fresh
            // session
            continue;
        }
        // Normal exit
        return Ok(());
    }
}

/// Drive a single session until quit or /new.
/// Returns `true` if /new was requested.
fn run_session(
    config: &mut Config,
    tools: &[crate::types::ToolDef],
    editor: &mut Editor,
    session: &mut Session,
) -> Result<bool> {
    let mut last_input_tokens: u32 = 0;
    let mut tool_log = ToolOutputLog::new();

    loop {
        tool_log.clear();
        if last_input_tokens > COMPACT_THRESHOLD {
            compact(config, &mut session.messages, last_input_tokens)?;
        }

        let thinking = if config.thinking_budget > 0 {
            Some(crate::types::ThinkingConfig {
                kind: "enabled",
                budget_tokens: config.thinking_budget,
            })
        } else {
            None
        };

        config.ensure_full_prompt();
        let request = Request {
            model: &config.model,
            max_tokens: config.max_tokens,
            thinking,
            system: vec![SystemBlock::cached_text(config.full_prompt())],
            messages: &session.messages,
            tools,
            stream: true,
        };

        let result = stream::stream_response(config, &request)?;

        // Accumulate usage
        let u = &result.usage;
        last_input_tokens = u.input_tokens;
        session.total_input_tokens += u.input_tokens as u64;
        session.total_output_tokens += u.output_tokens as u64;
        let context_window = config
            .model_info
            .as_ref()
            .map(|m| m.context)
            .unwrap_or(200_000);
        let pct =
            (u.input_tokens as f64 / context_window as f64 * 100.0) as u32;
        session.token_pct = Some(pct);
        save_token_pct(&session.file, pct);
        eprint!(
            "\n* tokens: in={} out={} ({pct}%)",
            u.input_tokens, u.output_tokens
        );
        if u.cache_creation_input_tokens > 0 {
            eprint!(" cache_write={}", u.cache_creation_input_tokens);
        }
        if u.cache_read_input_tokens > 0 {
            eprint!(" cache_read={}", u.cache_read_input_tokens);
        }
        eprintln!();

        // Handle empty interrupted response
        if result.interrupted && result.content.is_empty() {
            // skip to prompt
        } else {
            let mut tool_calls: Vec<(String, String, serde_json::Value)> =
                Vec::new();

            for block in &result.content {
                if let ContentBlock::ToolUse { id, name, input } = block {
                    tool_calls.push((id.clone(), name.clone(), input.clone()));
                }
            }

            session.push_message(Message {
                role: Role::Assistant,
                content: Content::Blocks(result.content),
            });

            if !result.interrupted && result.stop_reason == StopReason::ToolUse
            {
                signal::clear();
                let results: Vec<ContentBlock> = std::thread::scope(|s| {
                    let handles: Vec<_> = tool_calls
                        .iter()
                        .map(|(id, name, input)| {
                            let wd = &config.working_dir;
                            s.spawn(move || {
                                if signal::is_interrupted() {
                                    return ContentBlock::ToolResult {
                                        tool_use_id: id.clone(),
                                        content: "(cancelled)".to_string(),
                                        is_error: Some(true),
                                    };
                                }
                                let output = tool::execute(wd, name, input);
                                let (content, is_error) = match output {
                                    Ok(out) => {
                                        let display = truncate(&out, 50_000);
                                        (display, None)
                                    }
                                    Err(e) => {
                                        let msg = e.to_string();
                                        eprintln!("* error: {msg}");
                                        (msg, Some(true))
                                    }
                                };
                                ContentBlock::ToolResult {
                                    tool_use_id: id.clone(),
                                    content,
                                    is_error,
                                }
                            })
                        })
                        .collect();
                    handles.into_iter().map(|h| h.join().unwrap()).collect()
                });
                if signal::is_interrupted() {
                    eprintln!("* tools interrupted");
                    signal::clear();
                }

                // Print collapsed tool output
                for ((_, name, input), result_block) in
                    tool_calls.iter().zip(results.iter())
                {
                    if let ContentBlock::ToolResult {
                        content, is_error, ..
                    } = result_block
                    {
                        if *is_error == Some(true) || content.is_empty() {
                            continue;
                        }
                        let header = stream::tool_call_header(name, input);
                        tool_log.push(header, content.clone());
                        tool_log.print_last();
                    }
                }

                session.push_message(Message {
                    role: Role::User,
                    content: Content::Blocks(results),
                });
                continue;
            }

            if result.stop_reason == StopReason::MaxTokens {
                eprintln!(
                    "* warning: response truncated \
                     (max_tokens reached)"
                );
                break;
            }
        }

        // Update index
        session.entry.message_count = session.messages.len() as u32;
        session.entry.modified = session::iso_now();
        session::update_entry(&config.session_dir, &session.entry);

        // Read next user input
        match command::read_input(
            editor,
            config,
            session,
            &mut tool_log,
            false,
        )? {
            InputResult::Ready => {}
            InputResult::Continue => unreachable!(),
            InputResult::Quit => {
                eprintln!("bye");
                return Ok(false);
            }
            InputResult::New => {
                eprintln!("* starting new session");
                return Ok(true);
            }
        }
    }

    Ok(false)
}

pub(crate) fn load_session(path: &std::path::Path) -> Result<Vec<Message>> {
    let content = fs::read_to_string(path)?;
    let mut messages = Vec::new();
    for line in content.lines() {
        if line.is_empty() {
            continue;
        }
        let msg: Message = serde_json::from_str(line)?;
        messages.push(msg);
    }
    Ok(messages)
}

fn meta_path(session: &std::path::Path) -> std::path::PathBuf {
    let mut p = session.as_os_str().to_owned();
    p.push(".meta");
    std::path::PathBuf::from(p)
}

pub(crate) fn load_token_pct(session: &std::path::Path) -> Option<u32> {
    fs::read_to_string(meta_path(session))
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn save_token_pct(session: &std::path::Path, pct: u32) {
    let _ = fs::write(meta_path(session), pct.to_string());
}

fn save_message(path: &std::path::Path, msg: &Message) {
    let json = match serde_json::to_string(msg) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("* warning: failed to serialize message: {e}");
            return;
        }
    };
    let mut file = match OpenOptions::new().create(true).append(true).open(path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("* warning: cannot open session file: {e}");
            return;
        }
    };
    if let Err(e) = writeln!(file, "{json}") {
        eprintln!("* warning: failed to write message: {e}");
    }
}

fn compact(
    config: &Config,
    messages: &mut Vec<Message>,
    input_tokens: u32,
) -> Result<()> {
    let cut = find_cut_point(messages, input_tokens);
    if cut == 0 {
        return Ok(());
    }

    eprintln!("* compacting ({cut} messages → summary)...");

    let old = &messages[..cut];
    let conversation = serialize_for_summary(old);
    let summary = generate_summary(config, &conversation)?;

    let kept = messages.split_off(cut);
    messages.clear();
    messages.push(Message {
        role: Role::User,
        content: Content::Text(format!("<context>\n{summary}\n</context>")),
    });
    messages.push(Message {
        role: Role::Assistant,
        content: Content::Text(
            "Understood, continuing from where we left off.".to_string(),
        ),
    });
    messages.extend(kept);

    eprintln!("* compacted: {} messages remaining", messages.len());
    Ok(())
}

fn find_cut_point(messages: &[Message], input_tokens: u32) -> usize {
    if messages.len() < 6 {
        return 0;
    }

    let keep_ratio = KEEP_RECENT_TOKENS as f64 / input_tokens as f64;
    let keep_count = (messages.len() as f64 * keep_ratio).ceil() as usize;
    let keep_count = keep_count.max(4);

    if keep_count >= messages.len() {
        return 0;
    }

    let target = messages.len() - keep_count;

    // Adjust forward to a user text message (turn boundary)
    for (i, msg) in messages.iter().enumerate().skip(target) {
        if msg.role == Role::User && matches!(&msg.content, Content::Text(_)) {
            return i;
        }
    }

    0
}

fn serialize_for_summary(messages: &[Message]) -> String {
    let mut out = String::new();
    for msg in messages {
        let role = if msg.role == Role::User {
            "User"
        } else {
            "Assistant"
        };
        match &msg.content {
            Content::Text(t) => {
                let _ = writeln!(out, "[{role}]: {t}");
            }
            Content::Blocks(blocks) => {
                for block in blocks {
                    match block {
                        ContentBlock::Thinking { .. } => {}
                        ContentBlock::Text { text } => {
                            let _ = writeln!(out, "[{role}]: {text}");
                        }
                        ContentBlock::ToolUse { name, input, .. } => {
                            let _ =
                                writeln!(out, "[Tool call]: {name}({input})");
                        }
                        ContentBlock::ToolResult {
                            content, is_error, ..
                        } => {
                            let tag = if *is_error == Some(true) {
                                "Tool error"
                            } else {
                                "Tool result"
                            };
                            let display = truncate(content, 2000);
                            let _ = writeln!(out, "[{tag}]: {display}");
                        }
                    }
                }
            }
        }
    }
    out
}

fn generate_summary(config: &Config, conversation: &str) -> Result<String> {
    let msgs = [Message {
        role: Role::User,
        content: Content::Text(conversation.to_string()),
    }];
    let request = Request {
        model: &config.model,
        max_tokens: 2048,
        thinking: None,
        system: vec![SystemBlock::text(
            "Summarize this coding session. Capture:\n\
             1. The user's goal\n\
             2. What was accomplished (files read, created, \
             modified)\n\
             3. Key decisions and reasoning\n\
             4. Current state and next steps\n\n\
             Be concise. Preserve critical context needed \
             to continue the work.",
        )],
        messages: &msgs,
        tools: &[],
        stream: true,
    };

    let mut reader = api::send_stream(config, &request)?;
    let mut summary = String::new();

    loop {
        match reader.next_event()? {
            Some(SseEvent::ContentBlockDelta {
                delta: Delta::Text(s),
                ..
            }) => {
                summary.push_str(&s);
            }
            Some(SseEvent::MessageStop) | None => break,
            _ => {}
        }
    }

    Ok(summary)
}
