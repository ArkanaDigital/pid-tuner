//! Remove API keys from any text that may reach the UI or a log.

/// Replace every secret (and its last 8 characters, which providers sometimes
/// echo) with `****`.
pub fn scrub(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_string();
    for s in secrets {
        let s = s.trim();
        if s.len() < 6 {
            continue;
        }
        out = out.replace(s, "****");
        if s.len() >= 12 {
            let tail = &s[s.len() - 8..];
            out = out.replace(tail, "****");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::scrub;
    #[test]
    fn scrubs_full_key_and_tail() {
        let key = "sk-ant-api03-ABCDEFGHIJKLMNOP";
        let t = format!("invalid x-api-key: {key} (…MNOP)");
        let s = scrub(&t, &[key]);
        assert!(!s.contains("ABCDEFGH") && !s.contains("IJKLMNOP"), "{s}");
        assert_eq!(scrub("nothing here", &[key]), "nothing here");
        assert_eq!(scrub("tiny", &["ab"]), "tiny");
    }
}
