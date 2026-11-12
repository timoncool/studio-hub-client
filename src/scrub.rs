//! A failure reason made safe to send: what failed and why, without anything about the person. The text is
//! cut down here, before it is stored or sent, so nothing personal ever leaves the computer: quoted text (song
//! titles, lyrics, file names), paths (a home folder carries the user's name), links, mail and network
//! addresses, ids and long numbers are replaced by placeholders, and the rest is shortened.

use std::sync::OnceLock;

use regex::Regex;

/// The longest reason kept, in UTF-16 units, as the hub measures.
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

/// The reason with everything personal replaced, on one line, at most [`MAX_REASON`] UTF-16 units.
pub fn reason(text: &str) -> String {
    let mut out: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    for rule in rules() {
        out = rule.pattern.replace_all(&out, rule.with).into_owned();
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.encode_utf16().count() <= MAX_REASON {
        return out;
    }
    let mut cut = String::new();
    let mut units = 0;
    for c in out.chars() {
        units += c.len_utf16();
        if units > MAX_REASON - 1 {
            break;
        }
        cut.push(c);
    }
    cut.push('…');
    cut
}

/// A log made safe to publish: every line cleaned as a reason is, the lines kept apart, and the tail of at most
/// `max_chars` characters, cut at a line's start.
pub fn lines(text: &str, max_chars: usize) -> String {
    let cleaned: Vec<String> = text
        .lines()
        .map(|line| {
            let mut out: String = line.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
            for rule in rules() {
                out = rule.pattern.replace_all(&out, rule.with).into_owned();
            }
            out.trim_end().to_string()
        })
        .collect();
    let mut kept: Vec<&str> = Vec::new();
    let mut room = max_chars;
    for line in cleaned.iter().rev() {
        let need = line.chars().count() + 1;
        if need > room {
            break;
        }
        room -= need;
        kept.push(line);
    }
    kept.reverse();
    kept.join("\n").trim_matches('\n').to_string()
}

#[cfg(test)]
mod tests {
    use super::{lines, reason};

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
    fn a_log_keeps_its_lines_and_its_tail_without_anything_personal() {
        let log = "[Load] C:\\Users\\Ivan\\models\\dit.gguf\n[DiT] step 3/8\nCUDA error: out of memory";
        assert_eq!(lines(log, 60_000), "[Load] <path>\n[DiT] step 3/8\nCUDA error: out of memory");
        assert_eq!(lines(log, 45), "[DiT] step 3/8\nCUDA error: out of memory", "the tail, cut at a line's start");
        assert_eq!(lines("", 100), "");
    }

    #[test]
    fn the_cause_stays_readable_and_short() {
        assert_eq!(reason("CUDA error: out of memory\n  at stage 2/4"), "CUDA error: out of memory at stage 2/4");
        assert_eq!(reason("the song would last 640 seconds; songs go up to 600"), "the song would last 640 seconds; songs go up to 600");
        let long = reason(&"x".repeat(500));
        assert_eq!(long.chars().count(), 200);
        assert!(long.ends_with('…'));
        assert_eq!(reason(&"🎵".repeat(150)).encode_utf16().count(), 199);
    }
}
