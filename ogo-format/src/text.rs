//! Fitting text into a number of characters, for the monospace fonts on the device.

/// Shortens `text` to `max_chars`, ending in "..." if it was cut.
pub fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max_chars.saturating_sub(3)).collect();
    format!("{}...", kept.trim_end())
}

/// Word-wraps `text` into at most `max_lines` lines of `max_chars`. Words longer than a line are
/// cut, and the last line ends in "..." if text was left over.
pub fn wrap(text: &str, max_chars: usize, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let mut word = word.to_owned();
        loop {
            let needed = if current.is_empty() { word.chars().count() } else { current.chars().count() + 1 + word.chars().count() };
            if needed <= max_chars {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(&word);
                break;
            }
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                continue;
            }
            // A single word longer than a line: split it.
            let head: String = word.chars().take(max_chars).collect();
            word = word.chars().skip(max_chars).collect();
            lines.push(head);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        let last = lines.last_mut().expect("max_lines > 0");
        *last = truncate(&format!("{last}..."), max_chars);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_keeps_short_text() {
        assert_eq!(truncate("Snake", 5), "Snake");
        assert_eq!(truncate("", 3), "");
    }

    #[test]
    fn truncate_cuts_with_dots() {
        assert_eq!(truncate("Falling Blocks", 10), "Falling...");
        assert_eq!(truncate("ab cd", 4), "a...");
        assert_eq!(truncate("äöüäöü", 5), "äö...");
    }

    #[test]
    fn wrap_breaks_at_words() {
        assert_eq!(wrap("The quick brown fox", 10, 3), ["The quick", "brown fox"]);
        assert_eq!(wrap("  spaced   out  ", 20, 2), ["spaced out"]);
        assert!(wrap("", 10, 2).is_empty());
    }

    #[test]
    fn wrap_splits_long_words() {
        assert_eq!(wrap("abcdefghij", 4, 5), ["abcd", "efgh", "ij"]);
    }

    #[test]
    fn wrap_marks_leftover_text() {
        let lines = wrap("one two three four five six", 9, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].ends_with("..."), "{lines:?}");
        assert!(lines.iter().all(|line| line.chars().count() <= 9), "{lines:?}");
    }
}
