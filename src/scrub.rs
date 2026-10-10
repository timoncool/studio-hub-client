//! A failure reason made safe to send: what failed and why, without anything about the person. The text is
//! cut down here, before it is stored or sent, so nothing personal ever leaves the computer: quoted text (song
//! titles, lyrics, file names), paths (a home folder carries the user's name), links, mail and network
//! addresses, ids and long numbers are replaced by placeholders, and the rest is shortened.

use std::sync::OnceLock;

use regex::Regex;

/// The longest reason kept, in characters.
pub const MAX_REASON: usize = 200;

struct Rule {
    pattern: Regex,
    with: &'static str,
}

fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let rule = |pattern: &str, with: &'static str| Rule { pattern: Regex::new(pattern).expect("a scrub pattern compiles"), with };
        vec![
            rule(r#""[^"]*"|'[^'\n]*'|«[^»]*»|“[^”]*”|„[^“”]*[“”]"#, "\"…\""),
            rule(r"(?i)\b[a-z][a-z0-9+.-]*://\S+", "<url>"),
            rule(r"(?i)\b[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}\b", "<email>"),
            rule(r#"(?i)\b[a-z]:[\\/][^"'<>|:;,\n]*"#, "<path>"),
            rule(r#"\\\\[^"'<>|:;,\n]+"#, "<path>"),
            rule(r#"(^|[\s(=])~?/[^\s"'<>|:;,]+/[^"'<>|:;,\n]*"#, "$1<path>"),
            rule(r"\b\d{1,3}(?:\.\d{1,3}){3}(?::\d+)?\b", "<ip>"),
            rule(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b", "<id>"),
            rule(r"(?i)\b[0-9a-f]{12,}\b", "<id>"),
            rule(r"\b\d{6,}\b", "<n>"),
        ]
    })
}

/// The reason with everything personal replaced, on one line, at most [`MAX_REASON`] characters.
pub fn reason(text: &str) -> String {
    let mut out: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    for rule in rules() {
        out = rule.pattern.replace_all(&out, rule.with).into_owned();
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() <= MAX_REASON {
        return out;
    }
    let mut cut: String = out.chars().take(MAX_REASON - 1).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::reason;

    #[test]
    fn nothing_personal_leaves() {
        assert_eq!(reason(r#"open C:\Users\Ivan Petrov\Music\my song.flac: access denied"#), "open <path>: access denied");
        assert_eq!(reason("read /home/ivan/my music/x.wav: no such file"), "read <path>: no such file");
        assert_eq!(reason("song \"Мой дом\" could not be saved"), "song \"…\" could not be saved");
        assert_eq!(reason("«Привет» не найдено"), "\"…\" не найдено");
        assert_eq!(reason("GET https://example.com/a?token=1 refused, mail ivan@example.com, peer 192.168.1.20:8080"), "GET <url> refused, mail <email>, peer <ip>");
        assert_eq!(reason("job 0199e2a4-7b1c-7d3e-9f00-123456789abc failed at frame 1234567"), "job <id> failed at frame <n>");
        assert_eq!(reason(r"\\server\share\my file.wav: missing"), "<path>: missing");
    }

    #[test]
    fn the_cause_stays_readable_and_short() {
        assert_eq!(reason("CUDA error: out of memory\n  at stage 2/4"), "CUDA error: out of memory at stage 2/4");
        assert_eq!(reason("the song would last 640 seconds; songs go up to 600"), "the song would last 640 seconds; songs go up to 600");
        let long = reason(&"x".repeat(500));
        assert_eq!(long.chars().count(), 200);
        assert!(long.ends_with('…'));
    }
}
