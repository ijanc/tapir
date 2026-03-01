use std::io::{self, BufRead};

use serde::Deserialize;

use crate::signal;
use crate::types::StopReason;

// -- Public event types --

#[derive(Debug)]
pub enum SseEvent {
    MessageStart {
        input_tokens: u32,
        cache_creation: u32,
        cache_read: u32,
    },
    ContentBlockStart {
        #[allow(dead_code)]
        index: u32,
        block: BlockStart,
    },
    ContentBlockDelta {
        #[allow(dead_code)]
        index: u32,
        delta: Delta,
    },
    ContentBlockStop {
        #[allow(dead_code)]
        index: u32,
    },
    MessageDelta {
        stop_reason: StopReason,
        output_tokens: u32,
    },
    MessageStop,
    Ping,
}

#[derive(Debug)]
pub enum BlockStart {
    Thinking,
    Text,
    ToolUse { id: String, name: String },
}

#[derive(Debug)]
pub enum Delta {
    Thinking(String),
    Signature(String),
    Text(String),
    InputJson(String),
}

// -- Reader --

pub struct SseReader {
    reader: Box<dyn BufRead>,
}

impl SseReader {
    pub fn new(reader: Box<dyn BufRead>) -> Self {
        Self { reader }
    }

    /// Read the next SSE event.
    ///
    /// Returns `Ok(None)` on stream end or interruption.
    pub fn next_event(&mut self) -> crate::error::Result<Option<SseEvent>> {
        let mut event_type = String::new();
        let mut data = String::new();

        loop {
            if signal::is_interrupted() {
                return Ok(None);
            }

            let mut line = String::new();
            let n = match self.reader.read_line(&mut line) {
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                    if signal::is_interrupted() {
                        return Ok(None);
                    }
                    continue;
                }
                Err(e) => return Err(e.into()),
            };

            if n == 0 {
                // EOF
                return Ok(None);
            }

            let line = line.trim_end_matches('\n').trim_end_matches('\r');

            if line.is_empty() {
                // Blank line = dispatch event
                if data.is_empty() {
                    continue;
                }
                let evt = parse_event(&event_type, &data)?;
                return Ok(Some(evt));
            }

            if let Some(val) = line.strip_prefix("event: ") {
                event_type = val.to_string();
            } else if let Some(val) = line.strip_prefix("data: ") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(val);
            }
            // Ignore other fields (id:, retry:, comments)
        }
    }
}

// -- Internal deserialization types --

#[derive(Deserialize)]
struct RawMessageStart {
    message: RawMessageStartMsg,
}

#[derive(Deserialize)]
struct RawMessageStartMsg {
    usage: RawUsage,
}

#[derive(Deserialize)]
struct RawUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
}

#[derive(Deserialize)]
struct RawContentBlockStart {
    index: u32,
    content_block: RawContentBlock,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum RawContentBlock {
    #[serde(rename = "thinking")]
    Thinking,
    #[serde(rename = "text")]
    Text {
        #[allow(dead_code)]
        text: String,
    },
    #[serde(rename = "tool_use")]
    ToolUse { id: String, name: String },
}

#[derive(Deserialize)]
struct RawContentBlockDelta {
    index: u32,
    delta: RawDelta,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum RawDelta {
    #[serde(rename = "thinking_delta")]
    Thinking { thinking: String },
    #[serde(rename = "signature_delta")]
    Signature { signature: String },
    #[serde(rename = "text_delta")]
    Text { text: String },
    #[serde(rename = "input_json_delta")]
    InputJson { partial_json: String },
}

#[derive(Deserialize)]
struct RawContentBlockStop {
    index: u32,
}

#[derive(Deserialize)]
struct RawMessageDelta {
    delta: RawMessageDeltaInner,
    usage: RawUsage,
}

#[derive(Deserialize)]
struct RawMessageDeltaInner {
    stop_reason: StopReason,
}

// -- Parsing --

fn parse_event(event_type: &str, data: &str) -> crate::error::Result<SseEvent> {
    match event_type {
        "message_start" => {
            let raw: RawMessageStart = serde_json::from_str(data)?;
            let u = raw.message.usage;
            Ok(SseEvent::MessageStart {
                input_tokens: u.input_tokens,
                cache_creation: u.cache_creation_input_tokens,
                cache_read: u.cache_read_input_tokens,
            })
        }
        "content_block_start" => {
            let raw: RawContentBlockStart = serde_json::from_str(data)?;
            let block = match raw.content_block {
                RawContentBlock::Thinking => BlockStart::Thinking,
                RawContentBlock::Text { .. } => BlockStart::Text,
                RawContentBlock::ToolUse { id, name } => {
                    BlockStart::ToolUse { id, name }
                }
            };
            Ok(SseEvent::ContentBlockStart {
                index: raw.index,
                block,
            })
        }
        "content_block_delta" => {
            let raw: RawContentBlockDelta = serde_json::from_str(data)?;
            let delta = match raw.delta {
                RawDelta::Thinking { thinking } => Delta::Thinking(thinking),
                RawDelta::Signature { signature } => {
                    Delta::Signature(signature)
                }
                RawDelta::Text { text } => Delta::Text(text),
                RawDelta::InputJson { partial_json } => {
                    Delta::InputJson(partial_json)
                }
            };
            Ok(SseEvent::ContentBlockDelta {
                index: raw.index,
                delta,
            })
        }
        "content_block_stop" => {
            let raw: RawContentBlockStop = serde_json::from_str(data)?;
            Ok(SseEvent::ContentBlockStop { index: raw.index })
        }
        "message_delta" => {
            let raw: RawMessageDelta = serde_json::from_str(data)?;
            Ok(SseEvent::MessageDelta {
                stop_reason: raw.delta.stop_reason,
                output_tokens: raw.usage.output_tokens,
            })
        }
        "message_stop" => Ok(SseEvent::MessageStop),
        "ping" => Ok(SseEvent::Ping),
        _ => Ok(SseEvent::Ping), // ignore unknown events
    }
}
