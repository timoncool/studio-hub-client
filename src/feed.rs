//! What the hub sends (schema 1) and the rules a client applies to it: a notice reaches this studio only while it
//! is current, fits its version, platform and window language, and is not a test notice outside test mode.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Feed {
    pub schema: u32,
    pub app: String,
    pub generated: String,
    pub items: Vec<FeedItem>,
    /// The variants of running experiments, apart from `items` so a client that predates experiments shows none of
    /// them instead of all at once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<FeedItem>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Versions {
    pub min: Option<String>,
    pub max: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Ad {
    pub label: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub erid: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Button {
    pub label: String,
    /// url | dismiss | open
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// news | settings | models | update, for action "open"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// primary | secondary
    pub style: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Content {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub buttons: Vec<Button>,
}

/// When and how often the notice is shown (studio-hub/src/shared/types.ts `Rules`, evaluated as in its rules.ts).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Rules {
    pub delay_s: u32,
    pub after_sessions: u32,
    pub views: Option<Vec<String>>,
    /// once | session | interval
    pub frequency: String,
    pub interval_h: u32,
    pub max_shows: Option<u32>,
    /// never | snooze
    pub after_dismiss: String,
    pub snooze_h: u32,
    /// all | new | returning
    pub audience: String,
    pub new_days: u32,
}

/// What a client remembers about one notice. Unix seconds.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct NoticeMemory {
    pub shows: u32,
    pub last_shown: Option<i64>,
    /// The launch of the last impression: a notice shown in this launch stays until closed.
    pub shown_session: Option<u32>,
    pub clicked_at: Option<i64>,
    pub dismissed_at: Option<i64>,
}

/// Whether the rules let the notice show in this launch; delay and views apply when the window draws it.
pub fn eligible(rules: &Rules, seen: &NoticeMemory, sessions: u32, first_seen: i64, now: i64) -> bool {
    const HOUR: i64 = 3600;
    if sessions < rules.after_sessions {
        return false;
    }
    let age = now - first_seen;
    let border = rules.new_days as i64 * 24 * HOUR;
    if (rules.audience == "new" && age > border) || (rules.audience == "returning" && age <= border) {
        return false;
    }
    if let Some(closed) = seen.dismissed_at {
        if rules.after_dismiss != "snooze" || now - closed < rules.snooze_h as i64 * HOUR {
            return false;
        }
    }
    if seen.shown_session == Some(sessions) {
        return true;
    }
    if rules.max_shows.is_some_and(|max| seen.shows >= max) {
        return false;
    }
    if rules.frequency == "once" && seen.shows > 0 {
        return false;
    }
    if rules.frequency == "interval" && seen.last_shown.is_some_and(|at| now - at < rules.interval_h as i64 * HOUR) {
        return false;
    }
    true
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct FeedItem {
    pub id: String,
    /// news | bar | popup
    pub kind: String,
    pub priority: i32,
    pub from: Option<String>,
    pub until: Option<String>,
    pub versions: Option<Versions>,
    pub os: Option<Vec<String>>,
    pub langs: Option<Vec<String>>,
    pub test: bool,
    pub ad: Option<Ad>,
    /// sunset | orchid | lime | graphite, or null: the colour follows the place in the bar stack.
    pub theme: Option<String>,
    pub dismissible: bool,
    pub rules: Rules,
    pub image: Option<String>,
    pub date: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub content: BTreeMap<String, Content>,
    /// One variant of an A/B test: each install sees exactly one variant of an experiment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment: Option<Experiment>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Experiment {
    pub id: String,
    /// The variant's share against the other variants' weights.
    pub weight: u32,
}

/// A bucket in `0..10000`, the same for the same seed and experiment every time and unrelated between experiments.
/// FNV-1a over `seed:experiment`, then MurmurHash3's finaliser so nearby inputs spread evenly; the hub's embed
/// (`embed/notices.ts`) computes the same.
pub fn bucket(seed: &str, experiment: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for byte in seed.bytes().chain(std::iter::once(b':')).chain(experiment.bytes()) {
        h ^= byte as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h % 10_000
}

/// The variants this install sees: one per experiment, picked by weight from the experiment's bucket. Every variant
/// in the feed takes part in the draw, so an install keeps its variant whatever else it fits.
pub fn chosen_variants<'a>(variants: &'a [FeedItem], seed: &str) -> Vec<&'a FeedItem> {
    let mut experiments: Vec<&str> = variants.iter().filter_map(|v| v.experiment.as_ref().map(|e| e.id.as_str())).collect();
    experiments.sort_unstable();
    experiments.dedup();
    experiments
        .into_iter()
        .filter_map(|id| {
            let mut arms: Vec<&FeedItem> = variants.iter().filter(|v| v.experiment.as_ref().is_some_and(|e| e.id == id && e.weight > 0)).collect();
            arms.sort_by_key(|v| v.id.clone());
            let total: u64 = arms.iter().map(|v| v.experiment.as_ref().map_or(0, |e| e.weight as u64)).sum();
            if total == 0 {
                return None;
            }
            let mut point = bucket(seed, id) as u64 * total / 10_000;
            arms.into_iter().find(|v| {
                let weight = v.experiment.as_ref().map_or(0, |e| e.weight as u64);
                if point < weight {
                    true
                } else {
                    point -= weight;
                    false
                }
            })
        })
        .collect()
}

impl FeedItem {
    /// The text in the window language, else English, else the first language the notice has.
    pub fn content_for(&self, lang: &str) -> Option<&Content> {
        self.content.get(lang).or_else(|| self.content.get("en")).or_else(|| self.content.values().next())
    }
}

/// Where and when a client asks: the rules of `fits` read nothing else.
#[derive(Clone, Debug)]
pub struct Audience<'a> {
    pub version: &'a str,
    /// windows | macos | linux
    pub platform: &'a str,
    pub lang: &'a str,
    pub test: bool,
    /// RFC 3339 UTC, compared as text like the hub writes it
    pub now: &'a str,
}

/// Major.minor.patch compared as numbers; a missing part counts as 0.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |v: &str| -> [u64; 3] {
        let mut out = [0; 3];
        for (slot, part) in out.iter_mut().zip(v.split('.')) {
            *slot = part.trim().parse().unwrap_or(0);
        }
        out
    };
    parts(a).cmp(&parts(b))
}

pub fn fits(item: &FeedItem, who: &Audience) -> bool {
    if item.test && !who.test {
        return false;
    }
    if item.from.as_deref().is_some_and(|from| from > who.now) || item.until.as_deref().is_some_and(|until| until <= who.now) {
        return false;
    }
    if item.os.as_ref().is_some_and(|os| !os.iter().any(|o| o == who.platform)) {
        return false;
    }
    if item.langs.as_ref().is_some_and(|langs| !langs.iter().any(|l| l == who.lang)) {
        return false;
    }
    if let Some(v) = &item.versions {
        if v.min.as_deref().is_some_and(|min| compare_versions(who.version, min).is_lt())
            || v.max.as_deref().is_some_and(|max| compare_versions(who.version, max).is_gt())
        {
            return false;
        }
    }
    item.content_for(who.lang).is_some_and(|c| !c.title.is_empty() || !c.body.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(extra: serde_json::Value) -> FeedItem {
        let mut base = serde_json::json!({
            "id": "n", "kind": "bar", "priority": 0, "from": null, "until": null, "versions": null, "os": null, "langs": null,
            "test": false, "ad": null, "theme": null, "dismissible": true, "image": null, "date": null,
            "rules": {"delay_s": 0, "after_sessions": 1, "views": null, "frequency": "session", "interval_h": 24, "max_shows": null,
                      "after_dismiss": "never", "snooze_h": 72, "audience": "all", "new_days": 7},
            "tags": [], "content": {"en": {"title": "", "body": "Hello", "buttons": []}}
        });
        for (k, v) in extra.as_object().unwrap() {
            base[k] = v.clone();
        }
        serde_json::from_value(base).unwrap()
    }

    fn who() -> Audience<'static> {
        Audience { version: "3.5.0", platform: "windows", lang: "ru", test: false, now: "2026-10-09T12:00:00.000Z" }
    }

    #[test]
    fn a_notice_reaches_only_the_studios_it_names() {
        assert!(fits(&item(serde_json::json!({})), &who()));
        assert!(!fits(&item(serde_json::json!({"test": true})), &who()));
        assert!(fits(&item(serde_json::json!({"test": true})), &Audience { test: true, ..who() }));
        assert!(!fits(&item(serde_json::json!({"from": "2026-10-10T00:00:00.000Z"})), &who()));
        assert!(!fits(&item(serde_json::json!({"until": "2026-10-09T11:00:00.000Z"})), &who()));
        assert!(!fits(&item(serde_json::json!({"os": ["web"]})), &who()));
        assert!(fits(&item(serde_json::json!({"os": ["windows", "linux"]})), &who()));
        assert!(!fits(&item(serde_json::json!({"langs": ["en"]})), &who()));
        assert!(!fits(&item(serde_json::json!({"versions": {"min": "3.6.0", "max": null}})), &who()));
        assert!(!fits(&item(serde_json::json!({"versions": {"min": null, "max": "3.4.9"}})), &who()));
        assert!(fits(&item(serde_json::json!({"versions": {"min": "3.5.0", "max": "3.10.0"}})), &who()));
    }

    #[test]
    fn the_rules_match_the_hubs_evaluator() {
        let h = 3600;
        let now = 1_000 * h;
        let mut rules = item(serde_json::json!({})).rules;
        let old = now - 30 * 24 * h;
        rules.frequency = "once".into();
        assert!(eligible(&rules, &NoticeMemory::default(), 3, old, now));
        let shown = NoticeMemory { shows: 1, last_shown: Some(now - h), shown_session: Some(2), ..Default::default() };
        assert!(!eligible(&rules, &shown, 3, old, now), "once ever");
        assert!(eligible(&rules, &NoticeMemory { shown_session: Some(3), ..shown.clone() }, 3, old, now), "stays this launch");
        rules.frequency = "interval".into();
        assert!(!eligible(&rules, &shown, 3, old, now), "not within a day");
        assert!(eligible(&rules, &NoticeMemory { last_shown: Some(now - 25 * h), ..shown.clone() }, 3, old, now));
        rules.frequency = "session".into();
        rules.max_shows = Some(1);
        assert!(!eligible(&rules, &shown, 3, old, now), "the cap");
        rules.max_shows = None;
        let closed = NoticeMemory { dismissed_at: Some(now - 10 * h), ..shown.clone() };
        assert!(!eligible(&rules, &closed, 3, old, now));
        rules.after_dismiss = "snooze".into();
        assert!(!eligible(&rules, &closed, 3, old, now), "snoozed for 72 h");
        assert!(eligible(&rules, &NoticeMemory { dismissed_at: Some(now - 80 * h), ..shown.clone() }, 3, old, now));
        rules.after_sessions = 5;
        assert!(!eligible(&rules, &NoticeMemory::default(), 3, old, now));
        rules.after_sessions = 1;
        rules.audience = "new".into();
        assert!(!eligible(&rules, &NoticeMemory::default(), 3, old, now));
        assert!(eligible(&rules, &NoticeMemory::default(), 1, now - 2 * 24 * h, now));
    }

    #[test]
    fn an_install_keeps_one_variant_and_the_weights_hold() {
        assert_eq!(bucket("seed", "donate"), bucket("seed", "donate"));
        assert_eq!(bucket("0f7d1c3e-4b8a-4f2e-9d6c-1a2b3c4d5e6f", "donate-text"), 4774);
        assert_eq!(bucket("a", "b"), 9259);
        let arm = |id: &str, weight: u32| item(serde_json::json!({"id": id, "experiment": {"id": "donate", "weight": weight}}));
        let variants = vec![arm("donate-b", 1), arm("donate-a", 1), arm("donate-c", 2)];
        let mut seen = std::collections::BTreeMap::<String, u32>::new();
        for n in 0..8000 {
            let chosen = chosen_variants(&variants, &format!("install-{n}"));
            assert_eq!(chosen.len(), 1);
            *seen.entry(chosen[0].id.clone()).or_default() += 1;
        }
        for (id, share) in [("donate-a", 2000.0), ("donate-b", 2000.0), ("donate-c", 4000.0)] {
            let got = seen[id] as f64;
            assert!((got - share).abs() < share * 0.08, "{id}: {got} of 8000");
        }
        let other = item(serde_json::json!({"id": "other", "experiment": {"id": "popup-style", "weight": 1}}));
        assert_eq!(chosen_variants(&[variants[0].clone(), other], "x").len(), 2, "one variant per experiment");
    }

    #[test]
    fn the_text_falls_back_to_english_then_any_language() {
        let n = item(serde_json::json!({"content": {"es": {"title": "Hola"}, "en": {"title": "Hi"}}}));
        assert_eq!(n.content_for("ru").unwrap().title, "Hi");
        let n = item(serde_json::json!({"content": {"es": {"title": "Hola"}}}));
        assert_eq!(n.content_for("ru").unwrap().title, "Hola");
        assert!(compare_versions("3.10.0", "3.9.9").is_gt());
    }
}
