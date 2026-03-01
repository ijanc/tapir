use std::io::{self, Write};

const COLLAPSED_LINES: usize = 3;
const INDENT: &str = "    ";

/// One tool call's output for display purposes.
pub(crate) struct ToolOutput {
    header: String,
    output: String,
    expanded: bool,
}

impl ToolOutput {
    fn print(&self) {
        let mut stderr = io::stderr();
        let _ = writeln!(stderr, "{INDENT}\x1b[2m⎿\x1b[0m");

        let lines: Vec<&str> = self.output.lines().collect();
        if lines.is_empty() {
            return;
        }

        if self.expanded || lines.len() <= COLLAPSED_LINES {
            for line in &lines {
                let _ = writeln!(stderr, "{INDENT} {line}");
            }
        } else {
            for line in &lines[..COLLAPSED_LINES] {
                let _ = writeln!(stderr, "{INDENT} {line}");
            }
            let remaining = lines.len() - COLLAPSED_LINES;
            let _ = writeln!(
                stderr,
                "{INDENT} \x1b[2m\u{2026} +{remaining} lines \
                 (ctrl+o to expand)\x1b[0m"
            );
        }
    }
}

/// Stores recent tool outputs for the current turn.
pub(crate) struct ToolOutputLog {
    entries: Vec<ToolOutput>,
}

impl ToolOutputLog {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub(crate) fn push(&mut self, header: String, output: String) {
        self.entries.push(ToolOutput {
            header,
            output,
            expanded: false,
        });
    }

    /// Print the most recently added entry (collapsed).
    pub(crate) fn print_last(&self) {
        if let Some(entry) = self.entries.last() {
            entry.print();
        }
    }

    /// Toggle the last entry and re-print it.
    pub(crate) fn toggle_last(&mut self) {
        if let Some(entry) = self.entries.last_mut() {
            entry.expanded = !entry.expanded;
            // Re-print header + output in new state
            eprintln!("* {}", entry.header);
            entry.print();
        }
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
}
