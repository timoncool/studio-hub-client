//! What a studio keeps in `<data>/studio-hub.json`: the install id (only while telemetry is on), the user's choice,
//! the last feed, which notices were shown, clicked or closed, and per-day counters waiting to be reported.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::feed::{Feed, NoticeMemory};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct CachedFeed {
    pub etag: Option<String>,
    pub feed: Feed,
    /// The hub address that answered.
    pub source: String,
    /// Unix seconds.
    pub fetched_at: i64,
}


#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct NoticeCounts {
    pub shown: u32,
    pub clicked: u32,
    pub dismissed: u32,
    /// Clicks by what was clicked: `b0`, `b1`... for a notice's buttons in order, `link` for a link in its text.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub buttons: BTreeMap<String, u32>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModelSet {
    pub set: Option<String>,
    /// Sorted, so the same parts are one set however they were listed.
    pub components: Vec<String>,
    /// Pieces of work done on it that day: people switch sets, and each set counts its own.
    #[serde(default)]
    pub count: u64,
}

/// Seconds of result made and of work spent on one kind of work with one model set in a day.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SetSpeed {
    pub kind: String,
    pub set: Option<String>,
    pub media_s: u64,
    pub work_s: u64,
}

/// An answer in the issue of a reported problem, as the hub hands it over.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProblemAnswer {
    pub id: u64,
    pub at: String,
    pub by: String,
    pub excerpt: String,
    pub url: String,
}

/// A problem reported from this studio: the issue the hub made of it and what was answered there.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct FiledProblem {
    /// What the hub answers about the issue by; never shown.
    pub token: String,
    pub number: u64,
    pub url: String,
    /// Unix seconds.
    pub filed_at: i64,
    #[serde(default)]
    pub closed: bool,
    #[serde(default)]
    pub closed_at: Option<i64>,
    /// The hub no longer knows the report.
    #[serde(default)]
    pub gone: bool,
    #[serde(default)]
    pub checked_at: i64,
    #[serde(default)]
    pub answers: Vec<ProblemAnswer>,
    /// The answers the person was shown, by id.
    #[serde(default)]
    pub seen: Vec<u64>,
    #[serde(default)]
    pub closed_seen: bool,
}

/// How one route of the studio's own API was used in a day: every call, and the calls answered with an error.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct RouteUse {
    pub calls: u64,
    #[serde(default)]
    pub errors: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Day {
    pub counts: BTreeMap<String, u64>,
    pub models: BTreeSet<String>,
    pub notices: BTreeMap<String, NoticeCounts>,
    /// The model sets the day's work ran on: a ready-made set's name, or none for one put together by hand, and its
    /// parts either way.
    #[serde(default)]
    pub model_sets: Vec<ModelSet>,
    /// Seconds of result and of work by kind of work and model set (`Hub::worked_on`).
    #[serde(default)]
    pub speed: Vec<SetSpeed>,
    /// Failures by what failed and why, the why already scrubbed: `kind` and reason joined by a tab.
    #[serde(default)]
    pub failures: BTreeMap<String, u32>,
    /// The studio's API by route template (`POST /api/songs/{id}/stems`): which features are used, never with what.
    #[serde(default)]
    pub routes: BTreeMap<String, RouteUse>,
    /// Changed since the hub last accepted this day.
    pub dirty: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct State {
    pub install: Option<String>,
    /// None until the user answers; the checkbox on the start screen starts checked.
    pub telemetry: Option<bool>,
    /// The checkbox has been on screen: before that nothing is counted or sent.
    pub acknowledged: bool,
    pub feed: Option<CachedFeed>,
    pub last_source: Option<String>,
    pub last_error: Option<String>,
    /// Launches so far, counted when the studio starts.
    #[serde(default)]
    pub sessions: u32,
    /// Unix seconds of the first launch with the hub client (for new or returning installs).
    #[serde(default)]
    pub first_seen: i64,
    /// The last popup, for the global cap of one a day across all popups.
    #[serde(default)]
    pub last_popup_at: Option<i64>,
    pub seen: BTreeMap<String, NoticeMemory>,
    /// Picks this install's variant of each experiment; random, kept on this computer and never sent, so it holds
    /// with telemetry off as well.
    #[serde(default)]
    pub bucket_seed: Option<String>,
    pub days: BTreeMap<String, Day>,
    /// The journey's facts, kept on this computer whatever the telemetry choice and never reported: counters and
    /// route templates over the install's life, the days with a launch, the starts of this and the last launch.
    #[serde(default)]
    pub totals: BTreeMap<String, u64>,
    #[serde(default)]
    pub routes_used: BTreeSet<String>,
    #[serde(default)]
    pub active_days: u32,
    #[serde(default)]
    pub last_active_day: Option<String>,
    #[serde(default)]
    pub launch_at: Option<i64>,
    #[serde(default)]
    pub previous_launch: Option<i64>,
    #[serde(default)]
    pub asks: crate::feed::AskMemory,
    /// Problems reported from this studio, oldest first; kept whatever the telemetry choice.
    #[serde(default)]
    pub problems: Vec<FiledProblem>,
}

impl State {
    pub fn load(path: &Path) -> Result<State, String> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, body).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Days older than the hub accepts are dropped; clean days older than a day are dropped too.
    pub fn prune(&mut self, today: &str, oldest: &str) {
        self.days.retain(|day, d| day.as_str() >= oldest && (d.dirty || day.as_str() >= today));
    }
}
