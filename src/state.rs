//! What a studio keeps in `<data>/studio-hub.json`: the install id (only while telemetry is on), the user's choice,
//! the last feed, which notices were shown, clicked or closed, and per-day counters waiting to be reported.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::feed::Feed;

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
pub struct Seen {
    pub shown_at: Option<i64>,
    pub clicked_at: Option<i64>,
    pub dismissed_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct NoticeCounts {
    pub shown: u32,
    pub clicked: u32,
    pub dismissed: u32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Day {
    pub counts: BTreeMap<String, u64>,
    pub models: BTreeSet<String>,
    pub notices: BTreeMap<String, NoticeCounts>,
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
    pub seen: BTreeMap<String, Seen>,
    pub days: BTreeMap<String, Day>,
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
