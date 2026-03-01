use std::io::{self, Write};

use crate::config::Config;
use crate::error::Result;
use crate::sse::{BlockStart, Delta, SseEvent};
use crate::timer::ThinkingTimer;
use crate::types::{ContentBlock, Request, StopReason, Usage};
use crate::{api, signal};

pub struct StreamResult {
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    pub usage: Usage,
    pub interrupted: bool,
}

enum BlockState {
    /// Not inside any content block.
    Idle,
    /// Accumulating a thinking block.
    Thinking { thinking: String, signature: String },
    /// Accumulating a text block.
    Text { buf: String, at_line_start: bool, first_line: bool },
    /// Accumulating a tool-use block.
    ToolUse {
        id: String,
        name: String,
        json: String,
    },
}

pub fn stream_response(
    config: &Config,
    request: &Request<'_>,
) -> Result<StreamResult> {
    signal::clear();
    let timer = ThinkingTimer::start();
    let mut timer = Some(timer);

    let mut reader = api::send_stream(config, request)?;

    let mut content: Vec<ContentBlock> = Vec::new();
    let mut usage = Usage::default();
    let mut stop_reason = StopReason::EndTurn;
    let mut interrupted = false;
    let mut block = BlockState::Idle;

    let mut stdout = io::stdout();

    loop {
        let event = reader.next_event()?;

        let event = match event {
            Some(e) => e,
            None => {
                if let Some(t) = timer.take() {
                    t.stop();
                }
                if signal::is_interrupted() {
                    interrupted = true;
                    if let BlockState::Text {
                        ref buf,
                        at_line_start,
                        ..
                    } = block
                        && !buf.is_empty()
                    {
                        if !at_line_start {
                            let _ = writeln!(stdout);
                        }
                        content.push(ContentBlock::Text { text: buf.clone() });
                    }
                    eprintln!("\n* interrupted");
                }
                break;
            }
        };

        match event {
            SseEvent::MessageStart {
                input_tokens,
                cache_creation,
                cache_read,
            } => {
                usage.input_tokens = input_tokens;
                usage.cache_creation_input_tokens = cache_creation;
                usage.cache_read_input_tokens = cache_read;
            }
            SseEvent::ContentBlockStart { block: start, .. } => {
                if let Some(t) = timer.take() {
                    t.stop();
                }
                block = match start {
                    BlockStart::Thinking => BlockState::Thinking {
                        thinking: String::new(),
                        signature: String::new(),
                    },
                    BlockStart::Text => BlockState::Text {
                        buf: String::new(),
                        at_line_start: true,
                        first_line: true,
                    },
                    BlockStart::ToolUse { id, name } => BlockState::ToolUse {
                        id,
                        name,
                        json: String::new(),
                    },
                };
            }
            SseEvent::ContentBlockDelta { delta, .. } => {
                if let Some(t) = timer.take() {
                    t.stop();
                }
                match (&mut block, delta) {
                    (
                        BlockState::Thinking { thinking, .. },
                        Delta::Thinking(s),
                    ) => {
                        thinking.push_str(&s);
                    }
                    (
                        BlockState::Thinking { signature, .. },
                        Delta::Signature(s),
                    ) => {
                        signature.push_str(&s);
                    }
                    (
                        BlockState::Text {
                            buf,
                            at_line_start,
                            first_line,
                        },
                        Delta::Text(s),
                    ) => {
                        buf.push_str(&s);
                        for ch in s.chars() {
                            if *at_line_start {
                                let prefix = if *first_line {
                                    "< "
                                } else {
                                    "  "
                                };
                                let _ = write!(stdout, "{prefix}");
                                *at_line_start = false;
                            }
                            let _ = write!(stdout, "{ch}");
                            if ch == '\n' {
                                *at_line_start = true;
                                *first_line = false;
                            }
                        }
                        let _ = stdout.flush();
                    }
                    (BlockState::ToolUse { json, .. }, Delta::InputJson(s)) => {
                        json.push_str(&s);
                    }
                    _ => {}
                }
            }
            SseEvent::ContentBlockStop { .. } => {
                match std::mem::replace(&mut block, BlockState::Idle) {
                    BlockState::Thinking {
                        thinking,
                        signature,
                    } => {
                        let tokens = thinking.len() / 4;
                        eprintln!("* thinking (~{tokens} tokens)");
                        content.push(ContentBlock::Thinking {
                            thinking,
                            signature,
                        });
                    }
                    BlockState::Text {
                        buf, at_line_start, ..
                    } => {
                        if !at_line_start {
                            let _ = writeln!(stdout);
                        }
                        content.push(ContentBlock::Text { text: buf });
                    }
                    BlockState::ToolUse { id, name, json } => {
                        let input: serde_json::Value =
                            serde_json::from_str(&json).unwrap_or(
                                serde_json::Value::Object(Default::default()),
                            );
                        print_tool_call(&name, &input);
                        content.push(ContentBlock::ToolUse { id, name, input });
                    }
                    BlockState::Idle => {}
                }
            }
            SseEvent::MessageDelta {
                stop_reason: reason,
                output_tokens,
            } => {
                stop_reason = reason;
                usage.output_tokens = output_tokens;
            }
            SseEvent::MessageStop => break,
            SseEvent::Ping => {}
        }
    }

    Ok(StreamResult {
        content,
        stop_reason,
        usage,
        interrupted,
    })
}

pub(crate) fn tool_call_header(
    name: &str,
    input: &serde_json::Value,
) -> String {
    match name {
        "read_file" => {
            let path = input["path"].as_str().unwrap_or("?");
            format!("read: {path}")
        }
        "write_file" => {
            let path = input["path"].as_str().unwrap_or("?");
            format!("write: {path}")
        }
        "edit_file" => {
            let path = input["path"].as_str().unwrap_or("?");
            format!("edit: {path}")
        }
        "bash" => {
            let cmd = input["command"].as_str().unwrap_or("?");
            format!("bash: {cmd}")
        }
        _ => name.to_string(),
    }
}

fn print_tool_call(name: &str, input: &serde_json::Value) {
    let header = tool_call_header(name, input);
    eprintln!("* {header}");
    if name == "edit_file" {
        if let Some(old) = input["old_string"].as_str() {
            for line in old.lines() {
                eprintln!("- {line}");
            }
        }
        if let Some(new) = input["new_string"].as_str() {
            for line in new.lines() {
                eprintln!("+ {line}");
            }
        }
    }
}
