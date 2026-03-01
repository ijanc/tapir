pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = floor_char_boundary(s, max);
        format!("{}...\n(truncated, {} bytes total)", &s[..end], s.len())
    }
}

/// Keep the first `max_lines` lines and `max_bytes` bytes.
/// Returns the (possibly truncated) string and whether
/// truncation occurred.
pub fn truncate_head(
    s: &str,
    max_lines: usize,
    max_bytes: usize,
) -> (String, bool) {
    if s.len() <= max_bytes {
        let line_count = s.lines().count();
        if line_count <= max_lines {
            return (s.to_string(), false);
        }
    }

    let mut end = 0;
    for (lines, line) in s.split_inclusive('\n').enumerate() {
        if lines >= max_lines || end + line.len() > max_bytes {
            break;
        }
        end += line.len();
    }

    let end = floor_char_boundary(s, end);
    let mut out = s[..end].to_string();
    if !out.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&format!(
        "... ({} lines, {} bytes total)",
        s.lines().count(),
        s.len()
    ));
    (out, true)
}

/// Keep the last `max_lines` lines and `max_bytes` bytes.
/// Returns the (possibly truncated) string and whether
/// truncation occurred.
pub fn truncate_tail(
    s: &str,
    max_lines: usize,
    max_bytes: usize,
) -> (String, bool) {
    if s.len() <= max_bytes {
        let line_count = s.lines().count();
        if line_count <= max_lines {
            return (s.to_string(), false);
        }
    }

    // Walk backwards from end to find start position
    let lines: Vec<&str> = s.lines().collect();
    let total_lines = lines.len();
    let keep = total_lines.min(max_lines);
    let start_line = total_lines - keep;

    // Find byte offset of start_line
    let mut byte_offset = 0;
    for line in &lines[..start_line] {
        byte_offset += line.len() + 1; // +1 for '\n'
    }

    // Also enforce max_bytes from the end
    let byte_start = if s.len() > max_bytes {
        s.len() - max_bytes
    } else {
        0
    };
    let start = byte_offset.max(byte_start);
    let start = ceil_char_boundary(s, start);

    let mut out =
        format!("... ({total_lines} lines, {} bytes total)\n", s.len());
    out.push_str(&s[start..]);
    (out, true)
}

/// Truncate a single line to `max_chars` characters.
pub fn truncate_line(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{truncated}...")
    }
}

/// Find the smallest byte index >= `i` that is a valid
/// char boundary. Counterpart to `floor_char_boundary`.
pub fn ceil_char_boundary(s: &str, i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    let mut pos = i;
    while pos < s.len() && !s.is_char_boundary(pos) {
        pos += 1;
    }
    pos
}

/// Find the largest byte index <= `i` that is a valid
/// char boundary. Equivalent to the nightly
/// `str::floor_char_boundary`.
pub fn floor_char_boundary(s: &str, i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    let mut pos = i;
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

/// Normalize a string for fuzzy matching: collapse
/// whitespace, replace smart quotes and unicode dashes.
pub fn normalize_for_match(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_ws = false;
    for ch in s.chars() {
        let mapped = match ch {
            // Smart single quotes → ASCII
            '\u{2018}' | '\u{2019}' => '\'',
            // Smart double quotes → ASCII
            '\u{201C}' | '\u{201D}' => '"',
            // En-dash, em-dash → hyphen
            '\u{2013}' | '\u{2014}' => '-',
            other => other,
        };
        if mapped.is_whitespace() {
            if !prev_ws && !out.is_empty() {
                out.push(' ');
            }
            prev_ws = true;
        } else {
            out.push(mapped);
            prev_ws = false;
        }
    }
    // Trim trailing space from whitespace collapse
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Generate a unified-style diff for a single-region edit.
/// Shows the edit location with 3 lines of context.
pub fn edit_diff(
    path: &str,
    full_old: &str,
    old_text: &str,
    new_text: &str,
) -> String {
    let ctx = 3;
    let old_lines: Vec<&str> = full_old.lines().collect();

    // Find where old_text starts in the file
    let byte_start = match full_old.find(old_text) {
        Some(pos) => pos,
        None => return String::new(),
    };

    let prefix = &full_old[..byte_start];
    let start_line = if byte_start == 0 {
        0
    } else if prefix.ends_with('\n') {
        prefix.lines().count()
    } else {
        prefix.lines().count().saturating_sub(1)
    };
    let old_line_count = old_text.lines().count().max(1);
    let end_line = (start_line + old_line_count).min(old_lines.len());

    let new_lines: Vec<&str> = new_text.lines().collect();

    let ctx_start = start_line.saturating_sub(ctx);
    let ctx_end = (end_line + ctx).min(old_lines.len());

    let mut out = String::new();
    out.push_str(&format!("--- {path}\n+++ {path}\n"));
    out.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        ctx_start + 1,
        ctx_end - ctx_start,
        ctx_start + 1,
        (start_line - ctx_start) + new_lines.len() + (ctx_end - end_line),
    ));

    // Leading context
    for line in &old_lines[ctx_start..start_line] {
        out.push_str(&format!(" {line}\n"));
    }
    // Removed lines
    for line in &old_lines[start_line..end_line] {
        out.push_str(&format!("-{line}\n"));
    }
    // Added lines
    for line in &new_lines {
        out.push_str(&format!("+{line}\n"));
    }
    // Trailing context
    for line in &old_lines[end_line..ctx_end] {
        out.push_str(&format!(" {line}\n"));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_long() {
        let result = truncate("hello world", 5);
        assert!(result.starts_with("hello"));
        assert!(result.contains("truncated"));
    }

    #[test]
    fn test_floor_char_boundary_ascii() {
        assert_eq!(floor_char_boundary("hello", 3), 3);
    }

    #[test]
    fn test_floor_char_boundary_multibyte() {
        // "café": c(1) a(1) f(1) é(2) = 5 bytes
        // byte 4 is mid-char (inside 'é'), should snap back to 3
        let s = "café";
        assert_eq!(floor_char_boundary(s, 4), 3);
        // byte 3 is a valid boundary (before 'é')
        assert_eq!(floor_char_boundary(s, 3), 3);
    }

    #[test]
    fn test_floor_char_boundary_beyond() {
        assert_eq!(floor_char_boundary("hi", 100), 2);
    }

    #[test]
    fn test_truncate_head_no_truncation() {
        let (out, truncated) = truncate_head("a\nb\nc\n", 10, 1000);
        assert_eq!(out, "a\nb\nc\n");
        assert!(!truncated);
    }

    #[test]
    fn test_truncate_head_by_lines() {
        let input = "line1\nline2\nline3\nline4\nline5\n";
        let (out, truncated) = truncate_head(input, 2, 100_000);
        assert!(truncated);
        assert!(out.starts_with("line1\nline2\n"));
        assert!(out.contains("5 lines"));
    }

    #[test]
    fn test_truncate_head_by_bytes() {
        let input = "abcdefghij\nklmnop\n";
        let (out, truncated) = truncate_head(input, 1000, 11);
        assert!(truncated);
        assert!(out.starts_with("abcdefghij\n"));
    }

    #[test]
    fn test_truncate_tail_no_truncation() {
        let (out, truncated) = truncate_tail("a\nb\n", 10, 1000);
        assert_eq!(out, "a\nb\n");
        assert!(!truncated);
    }

    #[test]
    fn test_truncate_tail_by_lines() {
        let input = "line1\nline2\nline3\nline4\nline5";
        let (out, truncated) = truncate_tail(input, 2, 100_000);
        assert!(truncated);
        assert!(out.contains("line4"));
        assert!(out.contains("line5"));
        assert!(!out.contains("line3"));
    }

    #[test]
    fn test_truncate_line_short() {
        assert_eq!(truncate_line("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_line_long() {
        let input = "a".repeat(20);
        let out = truncate_line(&input, 10);
        assert_eq!(out, format!("{}...", "a".repeat(10)));
    }

    #[test]
    fn test_ceil_char_boundary_ascii() {
        assert_eq!(ceil_char_boundary("hello", 3), 3);
    }

    #[test]
    fn test_ceil_char_boundary_multibyte() {
        let s = "café";
        // byte 4 is mid-char (inside 'é'), should snap to 5
        assert_eq!(ceil_char_boundary(s, 4), 5);
    }

    #[test]
    fn test_ceil_char_boundary_beyond() {
        assert_eq!(ceil_char_boundary("hi", 100), 2);
    }

    #[test]
    fn test_normalize_for_match_smart_quotes() {
        let input = "\u{201C}hello\u{201D} \u{2018}world\u{2019}";
        assert_eq!(normalize_for_match(input), "\"hello\" 'world'");
    }

    #[test]
    fn test_normalize_for_match_dashes() {
        let input = "a\u{2013}b\u{2014}c";
        assert_eq!(normalize_for_match(input), "a-b-c");
    }

    #[test]
    fn test_normalize_for_match_whitespace() {
        let input = "  hello   world  ";
        assert_eq!(normalize_for_match(input), "hello world");
    }

    #[test]
    fn test_edit_diff_basic() {
        let file = "line1\nline2\nline3\nline4\nline5\n";
        let diff = edit_diff("test.rs", file, "line3", "line3a");
        assert!(diff.contains("-line3"));
        assert!(diff.contains("+line3a"));
        assert!(diff.contains("--- test.rs"));
    }
}
