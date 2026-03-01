use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::display::ToolOutputLog;

const HISTORY_SIZE: usize = 100;

pub struct Editor {
    history: Vec<String>,
    history_path: PathBuf,
    orig_termios: libc::termios,
    working_dir: PathBuf,
}

impl Editor {
    pub fn new() -> io::Result<Self> {
        let orig = unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut t) != 0 {
                return Err(io::Error::last_os_error());
            }
            t
        };

        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let history_path = PathBuf::from(home).join(".tapir/history");

        let history = load_history(&history_path);
        let working_dir =
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        Ok(Editor {
            history,
            history_path,
            orig_termios: orig,
            working_dir,
        })
    }

    pub fn readline(
        &mut self,
        prompt: &str,
        tool_log: Option<&mut ToolOutputLog>,
    ) -> io::Result<Option<String>> {
        self.enable_raw()?;
        let result = self.read_line_raw(prompt, tool_log);
        self.disable_raw()?;
        // Move to next line after input
        println!();
        io::stdout().flush()?;
        result
    }

    fn read_line_raw(
        &mut self,
        prompt: &str,
        mut tool_log: Option<&mut ToolOutputLog>,
    ) -> io::Result<Option<String>> {
        let mut buf: Vec<u8> = Vec::new();
        let mut cursor: usize = 0;
        let mut hist_idx: usize = self.history.len();
        let mut saved_line = String::new();

        self.print_line(prompt, &buf, cursor)?;

        let mut stdin = io::stdin().lock();
        let mut byte = [0u8; 1];

        loop {
            if stdin.read(&mut byte)? == 0 {
                if buf.is_empty() {
                    return Ok(None);
                }
                break;
            }

            match byte[0] {
                // Ctrl-D
                4 if buf.is_empty() => return Ok(None),
                // Ctrl-C
                3 => {
                    buf.clear();
                    cursor = 0;
                    print!("\r\n");
                    self.print_line(prompt, &buf, cursor)?;
                }
                // Enter
                b'\r' | b'\n' => break,
                // Backspace
                127 | 8 => {
                    if cursor > 0 {
                        cursor -= 1;
                        buf.remove(cursor);
                        self.print_line(prompt, &buf, cursor)?;
                    }
                }
                // Escape sequence
                27 => {
                    let mut seq = [0u8; 2];
                    if stdin.read(&mut seq[0..1])? == 0 {
                        continue;
                    }
                    if seq[0] != b'[' {
                        continue;
                    }
                    if stdin.read(&mut seq[1..2])? == 0 {
                        continue;
                    }
                    match seq[1] {
                        // Up arrow
                        b'A' => {
                            if hist_idx > 0 {
                                if hist_idx == self.history.len() {
                                    saved_line = String::from_utf8_lossy(&buf)
                                        .to_string();
                                }
                                hist_idx -= 1;
                                buf =
                                    self.history[hist_idx].as_bytes().to_vec();
                                cursor = buf.len();
                                self.print_line(prompt, &buf, cursor)?;
                            }
                        }
                        // Down arrow
                        b'B' => {
                            if hist_idx < self.history.len() {
                                hist_idx += 1;
                                if hist_idx == self.history.len() {
                                    buf = saved_line.as_bytes().to_vec();
                                } else {
                                    buf = self.history[hist_idx]
                                        .as_bytes()
                                        .to_vec();
                                }
                                cursor = buf.len();
                                self.print_line(prompt, &buf, cursor)?;
                            }
                        }
                        // Right arrow
                        b'C' => {
                            if cursor < buf.len() {
                                cursor += 1;
                                self.print_line(prompt, &buf, cursor)?;
                            }
                        }
                        // Left arrow
                        b'D' => {
                            if cursor > 0 {
                                cursor -= 1;
                                self.print_line(prompt, &buf, cursor)?;
                            }
                        }
                        // Extended sequences: ESC [ 1 ; <mod> <key>
                        b'1' => {
                            let mut ext = [0u8; 3];
                            // Read ";", modifier, direction
                            let _ = stdin.read(&mut ext[0..1]);
                            let _ = stdin.read(&mut ext[1..2]);
                            let _ = stdin.read(&mut ext[2..3]);
                            if ext[0] == b';' && ext[1] == b'5' {
                                match ext[2] {
                                    // Ctrl+Right: word forward
                                    b'C' => {
                                        while cursor < buf.len()
                                            && buf[cursor] == b' '
                                        {
                                            cursor += 1;
                                        }
                                        while cursor < buf.len()
                                            && buf[cursor] != b' '
                                        {
                                            cursor += 1;
                                        }
                                        self.print_line(prompt, &buf, cursor)?;
                                    }
                                    // Ctrl+Left: word backward
                                    b'D' => {
                                        while cursor > 0
                                            && buf[cursor - 1] == b' '
                                        {
                                            cursor -= 1;
                                        }
                                        while cursor > 0
                                            && buf[cursor - 1] != b' '
                                        {
                                            cursor -= 1;
                                        }
                                        self.print_line(prompt, &buf, cursor)?;
                                    }
                                    _ => {}
                                }
                            }
                        }
                        // Delete key (ESC [ 3 ~)
                        b'3' => {
                            let mut tilde = [0u8; 1];
                            let _ = stdin.read(&mut tilde);
                            if cursor < buf.len() {
                                buf.remove(cursor);
                                self.print_line(prompt, &buf, cursor)?;
                            }
                        }
                        // Home (ESC [ H)
                        b'H' => {
                            cursor = 0;
                            self.print_line(prompt, &buf, cursor)?;
                        }
                        // End (ESC [ F)
                        b'F' => {
                            cursor = buf.len();
                            self.print_line(prompt, &buf, cursor)?;
                        }
                        _ => {}
                    }
                }
                // Ctrl-P (history prev)
                16 => {
                    if hist_idx > 0 {
                        if hist_idx == self.history.len() {
                            saved_line =
                                String::from_utf8_lossy(&buf).to_string();
                        }
                        hist_idx -= 1;
                        buf = self.history[hist_idx].as_bytes().to_vec();
                        cursor = buf.len();
                        self.print_line(prompt, &buf, cursor)?;
                    }
                }
                // Ctrl-N (history next)
                14 => {
                    if hist_idx < self.history.len() {
                        hist_idx += 1;
                        if hist_idx == self.history.len() {
                            buf = saved_line.as_bytes().to_vec();
                        } else {
                            buf = self.history[hist_idx].as_bytes().to_vec();
                        }
                        cursor = buf.len();
                        self.print_line(prompt, &buf, cursor)?;
                    }
                }
                // Ctrl-O (toggle tool output)
                15 => {
                    if let Some(ref mut log) = tool_log {
                        print!("\r\n");
                        log.toggle_last();
                        self.print_line(prompt, &buf, cursor)?;
                    }
                }
                // Ctrl-A (home)
                1 => {
                    cursor = 0;
                    self.print_line(prompt, &buf, cursor)?;
                }
                // Ctrl-E (end)
                5 => {
                    cursor = buf.len();
                    self.print_line(prompt, &buf, cursor)?;
                }
                // Ctrl-U (kill line)
                21 => {
                    buf.clear();
                    cursor = 0;
                    self.print_line(prompt, &buf, cursor)?;
                }
                // Ctrl-K (kill to end of line)
                11 => {
                    buf.truncate(cursor);
                    self.print_line(prompt, &buf, cursor)?;
                }
                // Ctrl-W (kill word back)
                23 => {
                    while cursor > 0 && buf[cursor - 1] == b' ' {
                        cursor -= 1;
                        buf.remove(cursor);
                    }
                    while cursor > 0 && buf[cursor - 1] != b' ' {
                        cursor -= 1;
                        buf.remove(cursor);
                    }
                    self.print_line(prompt, &buf, cursor)?;
                }
                // Ctrl-G (open external editor)
                7 => {
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(edited) = self.open_editor(&text)? {
                        buf = edited.into_bytes();
                        cursor = buf.len();
                    }
                    self.print_line(prompt, &buf, cursor)?;
                }
                // Tab — complete @path
                b'\t' => {
                    if let Some((at_pos, completions)) =
                        self.find_completions(&buf, cursor)
                    {
                        self.apply_completion(
                            prompt,
                            &mut buf,
                            &mut cursor,
                            at_pos,
                            &completions,
                        )?;
                    }
                }
                // Printable
                c if c >= 32 => {
                    buf.insert(cursor, c);
                    cursor += 1;
                    self.print_line(prompt, &buf, cursor)?;
                }
                _ => {}
            }
        }

        let line = String::from_utf8_lossy(&buf).to_string();
        if !line.is_empty() {
            self.add_history(&line);
        }
        Ok(Some(line))
    }

    fn print_line(
        &self,
        prompt: &str,
        buf: &[u8],
        cursor: usize,
    ) -> io::Result<()> {
        let s = String::from_utf8_lossy(buf);
        let mut out = io::stdout();
        // Clear line, print prompt + buffer, position
        // cursor
        write!(out, "\r\x1b[K{prompt}{s}")?;
        let back = buf.len() - cursor;
        if back > 0 {
            write!(out, "\x1b[{back}D")?;
        }
        out.flush()
    }

    fn add_history(&mut self, line: &str) {
        // Don't add duplicates of the last entry
        if self.history.last().map(|s| s.as_str()) == Some(line) {
            return;
        }
        self.history.push(line.to_string());
        if self.history.len() > HISTORY_SIZE {
            self.history.remove(0);
        }
        append_history(&self.history_path, line);
    }

    // -------------------------------------------------
    // Tab completion for @path
    // -------------------------------------------------

    /// Scan backward from cursor to find `@`, then
    /// collect matching filesystem entries.
    fn find_completions(
        &self,
        buf: &[u8],
        cursor: usize,
    ) -> Option<(usize, Vec<String>)> {
        // Find the @ before cursor
        let text = &buf[..cursor];
        let at_pos = text.iter().rposition(|&b| b == b'@')?;

        let partial = std::str::from_utf8(&text[at_pos + 1..]).ok()?;

        let (dir, prefix) = split_path_prefix(&self.working_dir, partial);

        let entries = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => return Some((at_pos, Vec::new())),
        };

        let mut matches: Vec<String> = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // Skip hidden files unless prefix starts with .
            if name.starts_with('.') && !prefix.starts_with('.') {
                continue;
            }
            if name.starts_with(prefix) {
                let is_dir = entry.file_type().is_ok_and(|ft| ft.is_dir());
                // Build the full relative path from the
                // original partial's directory part
                let dir_part = if let Some(i) = partial.rfind('/') {
                    &partial[..=i]
                } else {
                    ""
                };
                let mut path = format!("{dir_part}{name}");
                if is_dir {
                    path.push('/');
                }
                matches.push(path);
            }
        }
        matches.sort();
        Some((at_pos, matches))
    }

    fn apply_completion(
        &self,
        prompt: &str,
        buf: &mut Vec<u8>,
        cursor: &mut usize,
        at_pos: usize,
        completions: &[String],
    ) -> io::Result<()> {
        match completions.len() {
            0 => {} // no matches — beep or ignore
            1 => {
                // Single match: replace partial with it
                let replacement = &completions[0];
                // Remove from after @ to cursor
                buf.drain(at_pos + 1..*cursor);
                let bytes = replacement.as_bytes();
                for (i, &b) in bytes.iter().enumerate() {
                    buf.insert(at_pos + 1 + i, b);
                }
                *cursor = at_pos + 1 + bytes.len();
                self.print_line(prompt, buf, *cursor)?;
            }
            _ => {
                // Multiple: complete common prefix, show
                // options
                let common = common_prefix(completions);
                let current_partial =
                    std::str::from_utf8(&buf[at_pos + 1..*cursor])
                        .unwrap_or("");

                if common.len() > current_partial.len() {
                    buf.drain(at_pos + 1..*cursor);
                    let bytes = common.as_bytes();
                    for (i, &b) in bytes.iter().enumerate() {
                        buf.insert(at_pos + 1 + i, b);
                    }
                    *cursor = at_pos + 1 + bytes.len();
                }

                // Show candidates below the prompt
                let mut out = io::stdout();
                write!(out, "\r\n")?;
                for c in completions {
                    write!(out, "  {c}\r\n")?;
                }
                self.print_line(prompt, buf, *cursor)?;
            }
        }
        Ok(())
    }

    fn open_editor(&self, text: &str) -> io::Result<Option<String>> {
        let editor = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| "vi".into());

        let tmp = std::env::temp_dir().join(".tapir-edit.md");
        fs::write(&tmp, text)?;

        self.disable_raw()?;
        print!("\r\n");
        io::stdout().flush()?;

        let status = Command::new(&editor)
            .arg(&tmp)
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status();

        self.enable_raw()?;

        match status {
            Ok(s) if s.success() => {
                let content = fs::read_to_string(&tmp)?;
                let _ = fs::remove_file(&tmp);
                let trimmed = content
                    .trim_end_matches('\n')
                    .trim_end_matches('\r')
                    .to_string();
                Ok(Some(trimmed))
            }
            _ => {
                let _ = fs::remove_file(&tmp);
                Ok(None)
            }
        }
    }

    fn enable_raw(&self) -> io::Result<()> {
        unsafe {
            let mut raw = self.orig_termios;
            raw.c_iflag &= !(libc::BRKINT
                | libc::ICRNL
                | libc::INPCK
                | libc::ISTRIP
                | libc::IXON);
            raw.c_oflag &= !libc::OPOST;
            raw.c_cflag |= libc::CS8;
            raw.c_lflag &=
                !(libc::ECHO | libc::ICANON | libc::IEXTEN | libc::ISIG);
            raw.c_cc[libc::VMIN] = 1;
            raw.c_cc[libc::VTIME] = 0;
            if libc::tcsetattr(0, libc::TCSAFLUSH, &raw) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    fn disable_raw(&self) -> io::Result<()> {
        unsafe {
            if libc::tcsetattr(0, libc::TCSAFLUSH, &self.orig_termios) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        let _ = self.disable_raw();
    }
}

/// Split a partial path into (directory_to_list,
/// filename_prefix). E.g. "src/ma" → ("<wd>/src", "ma"),
/// "" → ("<wd>", "").
fn split_path_prefix<'a>(
    working_dir: &Path,
    partial: &'a str,
) -> (PathBuf, &'a str) {
    if let Some(i) = partial.rfind('/') {
        let dir_part = &partial[..i];
        let prefix = &partial[i + 1..];
        let dir = if dir_part.is_empty() {
            working_dir.to_path_buf()
        } else {
            working_dir.join(dir_part)
        };
        (dir, prefix)
    } else {
        (working_dir.to_path_buf(), partial)
    }
}

fn common_prefix(items: &[String]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let first = &items[0];
    let mut len = first.len();
    for item in &items[1..] {
        len = len.min(item.len());
        for (i, (a, b)) in first.bytes().zip(item.bytes()).enumerate() {
            if a != b {
                len = len.min(i);
                break;
            }
        }
    }
    first[..len].to_string()
}

fn load_history(path: &PathBuf) -> Vec<String> {
    let Ok(file) = fs::File::open(path) else {
        return Vec::new();
    };
    let lines: Vec<String> = io::BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() > HISTORY_SIZE {
        lines[lines.len() - HISTORY_SIZE..].to_vec()
    } else {
        lines
    }
}

fn append_history(path: &PathBuf, line: &str) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}
