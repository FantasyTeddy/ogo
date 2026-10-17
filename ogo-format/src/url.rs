//! URL helpers for the web API.

/// Decodes `%XX` escapes, e.g. a file name in a URL path; `None` if an escape is broken or the
/// result isn't UTF-8.
pub fn percent_decode(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, after)) = rest.split_first() {
        if byte == b'%' {
            let hex = after
                .get(..2)
                .filter(|hex| hex.iter().all(u8::is_ascii_hexdigit))?;
            bytes.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            rest = &after[2..];
        } else {
            bytes.push(byte);
            rest = after;
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_escapes() {
        assert_eq!(percent_decode("Tetris.gb").unwrap(), "Tetris.gb");
        assert_eq!(
            percent_decode("Tetris%20(JUE)%20%5B!%5D.gb").unwrap(),
            "Tetris (JUE) [!].gb"
        );
        assert_eq!(percent_decode("Pok%C3%A9mon").unwrap(), "Pokémon");
        assert_eq!(percent_decode("a+b").unwrap(), "a+b");
        assert_eq!(percent_decode("%2e%2E").unwrap(), "..");
    }

    #[test]
    fn rejects_broken_escapes() {
        assert_eq!(percent_decode("%"), None);
        assert_eq!(percent_decode("%4"), None);
        assert_eq!(percent_decode("%zz"), None);
        assert_eq!(percent_decode("%+1"), None);
        assert_eq!(percent_decode("%C3"), None);
    }
}
