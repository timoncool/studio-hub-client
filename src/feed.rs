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
    /// The limits of the program's journey for the requests (`class: ask`).
    #[serde(default)]
    pub journey: Journey,
}

/// How often a program may ask (studio-hub/src/shared/rules.ts, the same defaults).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Journey {
    pub ask_gap_h: u32,
    pub ask_gap_sessions: u32,
    pub quiet_after: u32,
    pub quiet_days: u32,
    pub holdout_pct: u32,
}

impl Default for Journey {
    fn default() -> Self {
        Journey { ask_gap_h: 72, ask_gap_sessions: 2, quiet_after: 2, quiet_days: 30, holdout_pct: 10 }
    }
}

/// What the client knows of this install for the journey's conditions; local, never reported.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    /// Every `*_done` over the install's life.
    pub done: u64,
    /// `*_done` in this launch.
    pub session_done: u64,
    /// Days (UTC) with a launch, today included.
    pub active_days: u32,
    /// The start of the launch before this one, Unix seconds.
    pub previous_launch: Option<i64>,
    /// The last task of this launch, when it failed: its kind and its scrubbed reason.
    pub last_failure: Option<(String, String)>,
    /// A task is running now.
    pub running: bool,
    /// Counters over the install's life.
    pub counts: BTreeMap<String, u64>,
    /// Route templates the install has called.
    pub routes: std::collections::BTreeSet<String>,
    /// The author's programs on this computer.
    pub installed: Vec<String>,
}

/// What the client remembers of its requests (`class: ask`), across launches.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct AskMemory {
    pub last_ask: Option<i64>,
    pub last_ask_session: Option<u32>,
    pub ask_dismissals: u32,
    pub last_ask_dismissed: Option<i64>,
}

/// Whether a step's conditions hold; they hold in every launch, the one that showed it too, so a step leaves the
/// journey as soon as one fails (max_done, after_failure, not_installed). A condition without its fact fails.
pub fn step_ok(rules: &Rules, facts: &Facts, seen: &BTreeMap<String, NoticeMemory>, first_seen: i64, now: i64) -> bool {
    const DAY: i64 = 24 * 3600;
    let at_least = |min: Option<u32>, have: u64| min.is_none_or(|min| have >= min as u64);
    if !at_least(rules.min_done, facts.done) || rules.max_done.is_some_and(|max| facts.done > max as u64) {
        return false;
    }
    if !at_least(rules.min_active_days, facts.active_days as u64) {
        return false;
    }
    if rules.min_age_days.is_some_and(|days| now - first_seen < days as i64 * DAY) {
        return false;
    }
    if rules.after_success && (facts.session_done == 0 || facts.last_failure.is_some()) {
        return false;
    }
    if let Some(want) = &rules.after_failure {
        let (kind, reason) = want.split_once(':').map_or((want.as_str(), ""), |(k, r)| (k, r));
        if !facts.last_failure.as_ref().is_some_and(|(k, r)| k == kind && r.starts_with(reason)) {
            return false;
        }
    }
    if rules.after_event.as_ref().is_some_and(|event| facts.counts.get(event).copied().unwrap_or(0) == 0) {
        return false;
    }
    if let Some(days) = rules.absent_days {
        if !facts.previous_launch.is_some_and(|previous| now - previous >= days as i64 * DAY) {
            return false;
        }
    }
    if rules.unused_route.as_ref().is_some_and(|route| facts.routes.contains(route)) {
        return false;
    }
    if rules.not_installed.as_ref().is_some_and(|apps| apps.iter().any(|app| facts.installed.contains(app))) {
        return false;
    }
    if rules.shown_before.as_ref().is_some_and(|ids| ids.iter().any(|id| seen.get(id).is_none_or(|s| s.shows == 0))) {
        return false;
    }
    true
}

/// Whether the journey lets a request (`class: ask`) show in this launch.
pub fn ask_ok(journey: &Journey, seed: &str, facts: &Facts, asks: &AskMemory, shown_now: bool, sessions: u32, now: i64) -> bool {
    const HOUR: i64 = 3600;
    if in_holdout(journey, seed) {
        return false;
    }
    if shown_now {
        return true;
    }
    if facts.running || facts.last_failure.is_some() {
        return false;
    }
    if asks.last_ask.is_some_and(|at| now - at < journey.ask_gap_h as i64 * HOUR) {
        return false;
    }
    if asks.last_ask_session.is_some_and(|at| sessions.saturating_sub(at) < journey.ask_gap_sessions) {
        return false;
    }
    if asks.ask_dismissals >= journey.quiet_after && asks.last_ask_dismissed.is_some_and(|at| now - at < journey.quiet_days as i64 * 24 * HOUR) {
        return false;
    }
    true
}

/// The install is in the journey's control group: it sees the help, never a request.
pub fn in_holdout(journey: &Journey, seed: &str) -> bool {
    bucket(seed, "journey") < journey.holdout_pct * 100
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
    /// never | keep: whether a notice clicked once shows again in a later launch.
    #[serde(default = "keep")]
    pub after_click: String,
    /// The journey's conditions of a step (studio-hub rules.ts): none set — a notice outside the journey.
    #[serde(default)]
    pub min_done: Option<u32>,
    #[serde(default)]
    pub max_done: Option<u32>,
    #[serde(default)]
    pub min_active_days: Option<u32>,
    #[serde(default)]
    pub min_age_days: Option<u32>,
    #[serde(default)]
    pub after_success: bool,
    /// `kind` or `kind:start of the reason`.
    #[serde(default)]
    pub after_failure: Option<String>,
    #[serde(default)]
    pub after_event: Option<String>,
    #[serde(default)]
    pub absent_days: Option<u32>,
    #[serde(default)]
    pub unused_route: Option<String>,
    #[serde(default)]
    pub not_installed: Option<Vec<String>>,
    #[serde(default)]
    pub shown_before: Option<Vec<String>>,
    /// help | ask | news; none — outside the journey. The journey's limits and control group hold for `ask`.
    #[serde(default)]
    pub class: Option<String>,
}

fn keep() -> String {
    "keep".to_string()
}

/// How long a client remembers a notice that left the feed, seconds: a notice paused and back keeps its shows, clicks
/// and closings.
pub const GONE_KEEPS: i64 = 90 * 24 * 3600;

/// What a client remembers about one notice. Unix seconds.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct NoticeMemory {
    pub shows: u32,
    pub last_shown: Option<i64>,
    /// The launch of the last impression: a notice shown in this launch stays until closed.
    pub shown_session: Option<u32>,
    pub clicked_at: Option<i64>,
    pub dismissed_at: Option<i64>,
    /// When the notice was first missing from the feed; None while it is in the feed.
    #[serde(default)]
    pub gone_at: Option<i64>,
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
    if rules.after_click == "never" && seen.clicked_at.is_some() {
        return false;
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

    fn rules_with(extra: serde_json::Value) -> Rules {
        let mut base = serde_json::json!({"delay_s": 0, "after_sessions": 1, "views": null, "frequency": "session", "interval_h": 24,
            "max_shows": null, "after_dismiss": "never", "snooze_h": 72, "audience": "all", "new_days": 7});
        for (k, v) in extra.as_object().unwrap() {
            base[k] = v.clone();
        }
        serde_json::from_value(base).unwrap()
    }

    /// A rule set of an older hub (no after_click, no journey) reads as before; never keeps a clicked notice away in
    /// later launches and on screen in the launch that showed it.
    #[test]
    fn a_clicked_notice_with_after_click_never_does_not_come_back() {
        let now = 2_000_000_000;
        let old = rules_with(serde_json::json!({}));
        assert_eq!((old.after_click.as_str(), old.class.clone(), old.min_done), ("keep", None, None));
        let clicked = NoticeMemory { shows: 1, last_shown: Some(now - 10), shown_session: Some(2), clicked_at: Some(now - 5), ..Default::default() };
        assert!(eligible(&old, &clicked, 3, 0, now), "keep: back in the next launch");
        let never = rules_with(serde_json::json!({"after_click": "never"}));
        assert!(!eligible(&never, &clicked, 3, 0, now), "never: not in the next launch");
        assert!(eligible(&never, &clicked, 2, 0, now), "never: stays in the launch that showed it");
    }

    #[test]
    fn a_step_needs_every_condition_it_names() {
        let now = 2_000_000_000;
        let day = 24 * 3600;
        let seen = BTreeMap::new();
        let facts = Facts { done: 3, session_done: 1, active_days: 2, previous_launch: Some(now - 10 * day), ..Default::default() };
        let ok = |extra: serde_json::Value, facts: &Facts| step_ok(&rules_with(extra), facts, &seen, now - 20 * day, now);
        assert!(ok(serde_json::json!({}), &facts), "no condition");
        assert!(ok(serde_json::json!({"min_done": 3, "max_done": 5, "min_active_days": 2, "min_age_days": 14}), &facts));
        assert!(!ok(serde_json::json!({"min_done": 4}), &facts));
        assert!(!ok(serde_json::json!({"max_done": 0}), &facts), "help ends after the first work");
        assert!(!ok(serde_json::json!({"min_age_days": 30}), &facts));
        assert!(ok(serde_json::json!({"after_success": true}), &facts));
        let failed = Facts { last_failure: Some(("render".into(), "TTS: out of memory".into())), ..facts.clone() };
        assert!(!ok(serde_json::json!({"after_success": true}), &failed));
        assert!(ok(serde_json::json!({"after_failure": "render"}), &failed));
        assert!(ok(serde_json::json!({"after_failure": "render:TTS"}), &failed));
        assert!(!ok(serde_json::json!({"after_failure": "render:setup"}), &failed));
        assert!(!ok(serde_json::json!({"after_failure": "render"}), &facts), "no failure in this launch");
        assert!(!ok(serde_json::json!({"after_event": "setup_ready"}), &facts), "no such counter yet");
        let counted = Facts { counts: [("setup_ready".to_string(), 1)].into(), ..facts.clone() };
        assert!(ok(serde_json::json!({"after_event": "setup_ready"}), &counted));
        assert!(ok(serde_json::json!({"absent_days": 7}), &facts));
        assert!(!ok(serde_json::json!({"absent_days": 14}), &facts));
        assert!(!ok(serde_json::json!({"absent_days": 1}), &Facts { previous_launch: None, ..facts.clone() }), "first launch");
        let routed = Facts { routes: ["POST /api/karaoke".to_string()].into(), installed: vec!["speech".into()], ..facts.clone() };
        assert!(!ok(serde_json::json!({"unused_route": "POST /api/karaoke"}), &routed));
        assert!(ok(serde_json::json!({"unused_route": "POST /api/stems"}), &routed));
        assert!(!ok(serde_json::json!({"not_installed": ["speech", "dub"]}), &routed));
        assert!(ok(serde_json::json!({"not_installed": ["dub"]}), &routed));
        let shown: BTreeMap<String, NoticeMemory> = [("star-yue2".to_string(), NoticeMemory { shows: 1, ..Default::default() })].into();
        assert!(step_ok(&rules_with(serde_json::json!({"shown_before": ["star-yue2"]})), &facts, &shown, 0, now));
        assert!(!step_ok(&rules_with(serde_json::json!({"shown_before": ["star-yue2", "promo"]})), &facts, &shown, 0, now));
    }

    #[test]
    fn a_request_waits_its_gap_and_goes_quiet_after_two_closings() {
        let now = 2_000_000_000;
        let h = 3600;
        let journey = Journey::default();
        let seed = (0..).map(|i| format!("seed-{i}")).find(|s| !in_holdout(&journey, s)).unwrap();
        let held = (0..).map(|i| format!("seed-{i}")).find(|s| in_holdout(&journey, s)).unwrap();
        let facts = Facts::default();
        let none = AskMemory::default();
        assert!(ask_ok(&journey, &seed, &facts, &none, false, 5, now));
        assert!(!ask_ok(&journey, &held, &facts, &none, true, 5, now), "the control group never sees a request");
        assert!(!ask_ok(&journey, &seed, &Facts { running: true, ..Facts::default() }, &none, false, 5, now));
        assert!(!ask_ok(&journey, &seed, &Facts { last_failure: Some(("render".into(), "x".into())), ..Facts::default() }, &none, false, 5, now));
        let recent = AskMemory { last_ask: Some(now - 10 * h), last_ask_session: Some(1), ..none.clone() };
        assert!(!ask_ok(&journey, &seed, &facts, &recent, false, 5, now), "72 h between requests");
        assert!(ask_ok(&journey, &seed, &facts, &recent, true, 5, now), "the one shown in this launch stays");
        let one_launch = AskMemory { last_ask: Some(now - 80 * h), last_ask_session: Some(4), ..none.clone() };
        assert!(!ask_ok(&journey, &seed, &facts, &one_launch, false, 5, now), "two launches between requests");
        let quiet = AskMemory { last_ask: Some(now - 80 * h), last_ask_session: Some(1), ask_dismissals: 2, last_ask_dismissed: Some(now - 5 * 24 * h) };
        assert!(!ask_ok(&journey, &seed, &facts, &quiet, false, 5, now), "30 quiet days after two closings");
        assert!(ask_ok(&journey, &seed, &facts, &AskMemory { last_ask_dismissed: Some(now - 31 * 24 * h), ..quiet }, false, 5, now));
    }

    /// The hub's own values (studio-hub/test/rules.test.ts): the client and the hub answer the same.
    #[test]
    fn the_journey_answers_as_the_hub_does() {
        let h = 3600;
        let day = 24 * h;
        let now = 1000 * h;
        assert_eq!(bucket("0f7d1c3e-4b8a-4f2e-9d6c-1a2b3c4d5e6f", "journey"), 3051);
        assert_eq!(bucket("install-1", "journey"), 364);
        assert_eq!(bucket("install-2", "journey"), 1479);
        let journey = Journey::default();
        assert!(in_holdout(&journey, "install-1"));
        assert!(!in_holdout(&journey, "install-2"));
        assert!(!in_holdout(&Journey { holdout_pct: 0, ..Journey::default() }, "install-1"));

        let first_seen = now - 30 * day;
        let none = BTreeMap::new();
        let ok = |extra: serde_json::Value, facts: &Facts| step_ok(&rules_with(extra), facts, &none, first_seen, now);
        let f = Facts::default();
        assert!(!ok(serde_json::json!({"min_done": 1}), &f));
        assert!(ok(serde_json::json!({"min_done": 1}), &Facts { done: 1, ..f.clone() }));
        assert!(!ok(serde_json::json!({"max_done": 0}), &Facts { done: 1, ..f.clone() }));
        assert!(!ok(serde_json::json!({"min_active_days": 3}), &Facts { active_days: 2, ..f.clone() }));
        assert!(ok(serde_json::json!({"min_active_days": 3}), &Facts { active_days: 3, ..f.clone() }));
        assert!(!ok(serde_json::json!({"min_age_days": 31}), &f));
        assert!(ok(serde_json::json!({"min_age_days": 30}), &f));
        assert!(!ok(serde_json::json!({"after_success": true}), &Facts { done: 5, ..f.clone() }));
        assert!(ok(serde_json::json!({"after_success": true}), &Facts { done: 5, session_done: 1, ..f.clone() }));
        let oom = Facts { last_failure: Some(("song".into(), "CUDA error: out of memory".into())), ..f.clone() };
        assert!(!ok(serde_json::json!({"after_success": true}), &Facts { session_done: 1, ..oom.clone() }));
        assert!(ok(serde_json::json!({"after_failure": "song"}), &oom));
        assert!(ok(serde_json::json!({"after_failure": "song:CUDA error"}), &oom));
        assert!(!ok(serde_json::json!({"after_failure": "song:Vulkan"}), &oom));
        assert!(!ok(serde_json::json!({"after_failure": "cover"}), &oom));
        assert!(!ok(serde_json::json!({"after_failure": "song"}), &f));
        assert!(!ok(serde_json::json!({"absent_days": 14}), &f));
        assert!(!ok(serde_json::json!({"absent_days": 14}), &Facts { previous_launch: Some(now - 13 * day), ..f.clone() }));
        assert!(ok(serde_json::json!({"absent_days": 14}), &Facts { previous_launch: Some(now - 14 * day), ..f.clone() }));

        let seed = "install-2";
        let ask = |asks: AskMemory, facts: &Facts, shown_now: bool| ask_ok(&journey, seed, facts, &asks, shown_now, 3, now);
        let a = AskMemory::default();
        assert!(!ask(AskMemory { last_ask: Some(now - 71 * h), ..a.clone() }, &f, false));
        assert!(!ask(AskMemory { last_ask: Some(now - 72 * h), last_ask_session: Some(2), ..a.clone() }, &f, false));
        assert!(ask(AskMemory { last_ask: Some(now - 72 * h), last_ask_session: Some(1), ..a.clone() }, &f, false));
        assert!(!ask(AskMemory { ask_dismissals: 2, last_ask_dismissed: Some(now - 29 * day), ..a.clone() }, &f, false));
        assert!(ask(AskMemory { ask_dismissals: 2, last_ask_dismissed: Some(now - 30 * day), ..a.clone() }, &f, false));
        assert!(ask(AskMemory { ask_dismissals: 1, last_ask_dismissed: Some(now - day), ..a.clone() }, &f, false));
        assert!(ask(a.clone(), &Facts { running: true, ..f.clone() }, true));
        assert!(!ask_ok(&journey, "install-1", &f, &a, true, 3, now));
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
