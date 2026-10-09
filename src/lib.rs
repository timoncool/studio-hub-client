//! Studio Hub client for timoncool's desktop studios: anonymous telemetry and in-app notices (the top strip stack,
//! popups, news) from the hub, reached directly or through the RU proxy, with the last feed kept for offline starts.
//!
//! A studio builds a [`Hub`] at start, calls [`Hub::spawn`] inside its tokio runtime, merges [`Hub::router`] into
//! its axum app and counts what users do with [`Hub::count`]. The studio's frontend draws the notices in its own
//! style from `GET /v1/hub/state` and reports what it showed, clicked or closed.
//!
//! Telemetry follows the usual open-source rules: nothing is counted or sent before the start screen has shown its
//! checkbox; `DO_NOT_TRACK=1` or `STUDIO_TELEMETRY=0` turns it off; the install id is random, created only while
//! telemetry is on and resettable; a report carries counts, never content, paths or the IP.

pub mod feed;
mod routes;
pub mod state;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use serde::Serialize;

pub use feed::{Audience, Content, Feed, FeedItem};
use state::{CachedFeed, Seen, State};

/// The hub's own address and the RU proxy in front of it.
pub const DIRECT_URL: &str = "https://studio-hub.timoncool.workers.dev";
pub const PROXY_URL: &str = "https://hub.neuro-cartel.com";

const FEED_EVERY: Duration = Duration::from_secs(6 * 3600);
const REPORT_FIRST: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const REPORT_DAYS: i64 = 14;

/// The card as the studio describes it; values are bucketed so no machine can be told apart by them.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Gpu {
    /// nvidia | amd | intel | apple | other | none
    pub vendor: String,
    /// "<=8" | "12" | "16" | "24+" | "?"
    pub vram_gb: String,
    /// cuda | vulkan | metal | cpu ...
    pub backend: String,
}

impl Gpu {
    pub fn vram_bucket(bytes: u64) -> String {
        let gb = bytes as f64 / 1_073_741_824.0;
        match gb {
            g if g <= 0.0 => "?",
            g if g <= 9.0 => "<=8",
            g if g <= 13.0 => "12",
            g if g <= 20.0 => "16",
            _ => "24+",
        }
        .to_string()
    }
}

pub struct HubConfig {
    /// The program id registered in the hub: yue2, minimax, ace, dub...
    pub app: String,
    /// x.y.z of this build.
    pub version: String,
    /// The studio's data folder; the hub keeps `studio-hub.json` and its image cache there.
    pub data_dir: PathBuf,
    /// The studio's HTTP client, so the hub goes through the proxy the user set.
    pub http: reqwest::Client,
    /// "windows 11", "ubuntu 24.04"... as the studio reads it.
    pub os_label: String,
    pub gpu: Gpu,
    /// The window language, two letters.
    pub ui_lang: String,
    /// Hub addresses in order of preference; empty = the hub and its RU proxy. STUDIO_HUB_URL replaces them.
    pub urls: Vec<String>,
}

#[derive(Clone)]
pub struct Hub {
    inner: Arc<Inner>,
}

struct Inner {
    config: HubConfig,
    urls: Vec<String>,
    test: bool,
    disabled_by_env: bool,
    path: PathBuf,
    state: Mutex<State>,
    ui_lang: Mutex<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Report {
    pub schema: u32,
    pub install: String,
    pub app: String,
    pub version: String,
    pub day: String,
    pub os: String,
    pub ui_lang: String,
    pub gpu: Gpu,
    pub counts: std::collections::BTreeMap<String, u64>,
    pub models: Vec<String>,
    pub notices: std::collections::BTreeMap<String, state::NoticeCounts>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NoticeEvent {
    Shown,
    Clicked,
    Dismissed,
}

fn env_on(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

fn env_off(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"))
}

fn platform() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "macos",
        _ => "linux",
    }
}

fn now_secs() -> i64 {
    Utc::now().timestamp()
}

fn today() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

impl Hub {
    pub fn new(config: HubConfig) -> Result<Hub, String> {
        let urls = match std::env::var("STUDIO_HUB_URL") {
            Ok(url) if !url.trim().is_empty() => vec![url.trim().trim_end_matches('/').to_string()],
            _ if !config.urls.is_empty() => config.urls.iter().map(|u| u.trim_end_matches('/').to_string()).collect(),
            _ => vec![DIRECT_URL.to_string(), PROXY_URL.to_string()],
        };
        let path = config.data_dir.join("studio-hub.json");
        let mut state = State::load(&path)?;
        let disabled_by_env = env_on("DO_NOT_TRACK") || env_off("STUDIO_TELEMETRY");
        if disabled_by_env {
            state.install = None;
            state.days.clear();
        }
        let ui_lang = Mutex::new(config.ui_lang.clone());
        let hub = Hub {
            inner: Arc::new(Inner { config, urls, test: env_on("STUDIO_HUB_TEST"), disabled_by_env, path, state: Mutex::new(state), ui_lang }),
        };
        hub.persist(&hub.lock());
        Ok(hub)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn persist(&self, state: &State) {
        if let Err(e) = state.save(&self.inner.path) {
            tracing::warn!("studio hub: the state was not saved: {e}");
        }
    }

    fn change(&self, f: impl FnOnce(&mut State)) {
        let mut state = self.lock();
        f(&mut state);
        self.persist(&state);
    }

    /// Telemetry counts and reports only when the user has seen the choice, kept it on, and nothing in the
    /// environment turned it off.
    pub fn telemetry_on(&self) -> bool {
        let state = self.lock();
        !self.inner.disabled_by_env && state.acknowledged && state.telemetry.unwrap_or(true)
    }

    pub fn set_ui_lang(&self, lang: &str) {
        *self.inner.ui_lang.lock().unwrap_or_else(|p| p.into_inner()) = lang.chars().take(2).collect::<String>().to_ascii_lowercase();
    }

    fn ui_lang(&self) -> String {
        self.inner.ui_lang.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// The start screen showed the checkbox with this value; from now on its choice holds.
    pub fn acknowledge(&self, enabled: bool) {
        self.change(|s| {
            s.acknowledged = true;
            s.telemetry = Some(enabled);
            Self::fit_install(s, enabled);
        });
    }

    pub fn set_telemetry(&self, enabled: bool) {
        self.change(|s| {
            s.telemetry = Some(enabled);
            Self::fit_install(s, enabled);
        });
    }

    fn fit_install(s: &mut State, enabled: bool) {
        if enabled {
            s.install.get_or_insert_with(|| uuid::Uuid::new_v4().to_string());
        } else {
            s.install = None;
            s.days.clear();
        }
    }

    /// A new random id: from now on this install counts as a new one.
    pub fn reset_install(&self) {
        self.change(|s| {
            let on = s.telemetry.unwrap_or(true) && s.acknowledged;
            s.install = None;
            s.days.clear();
            Self::fit_install(s, on);
        });
    }

    /// Adds `n` to a counter of today (names `[a-z][a-z0-9_]{0,31}`, at most 40 a day). Nothing while telemetry is off.
    pub fn count(&self, name: &str, n: u64) {
        let valid = name.len() <= 32
            && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !valid {
            tracing::warn!("studio hub: counter name {name:?} is not [a-z][a-z0-9_]{{0,31}}, not counted");
            return;
        }
        if !self.telemetry_on() {
            return;
        }
        self.change(|s| {
            let day = s.days.entry(today()).or_default();
            if day.counts.len() >= 40 && !day.counts.contains_key(name) {
                tracing::warn!("studio hub: 40 counters today already, {name} not counted");
                return;
            }
            *day.counts.entry(name.to_string()).or_default() += n;
            day.dirty = true;
        });
    }

    /// The models used today (short names, at most 20).
    pub fn used_model(&self, model: &str) {
        if !self.telemetry_on() || model.is_empty() || model.len() > 40 {
            return;
        }
        self.change(|s| {
            let day = s.days.entry(today()).or_default();
            if day.models.len() < 20 && day.models.insert(model.to_string()) {
                day.dirty = true;
            }
        });
    }

    /// The frontend showed, clicked or closed a notice: remembered always (so it is not shown twice), counted only
    /// with telemetry on.
    pub fn notice(&self, id: &str, event: NoticeEvent) {
        let counting = self.telemetry_on();
        self.change(|s| {
            let at = now_secs();
            let seen = s.seen.entry(id.to_string()).or_insert_with(Seen::default);
            match event {
                NoticeEvent::Shown => {
                    seen.shown_at.get_or_insert(at);
                }
                NoticeEvent::Clicked => seen.clicked_at = Some(at),
                NoticeEvent::Dismissed => seen.dismissed_at = Some(at),
            }
            if counting {
                let day = s.days.entry(today()).or_default();
                if day.notices.len() < 50 || day.notices.contains_key(id) {
                    let c = day.notices.entry(id.to_string()).or_default();
                    match event {
                        NoticeEvent::Shown => c.shown += 1,
                        NoticeEvent::Clicked => c.clicked += 1,
                        NoticeEvent::Dismissed => c.dismissed += 1,
                    }
                    day.dirty = true;
                }
            }
        });
    }

    /// The notices this studio should consider now, newest feed first, with what the user already did with them.
    pub fn items(&self) -> Vec<(FeedItem, Seen)> {
        let state = self.lock();
        let Some(cached) = &state.feed else { return Vec::new() };
        let now = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        let lang = self.ui_lang();
        let who = Audience { version: &self.inner.config.version, platform: platform(), lang: &lang, test: self.inner.test, now: &now };
        let mut items: Vec<(FeedItem, Seen)> = cached
            .feed
            .items
            .iter()
            .filter(|item| feed::fits(item, &who))
            .map(|item| (item.clone(), state.seen.get(&item.id).cloned().unwrap_or_default()))
            .collect();
        items.sort_by(|a, b| b.0.priority.cmp(&a.0.priority));
        items
    }

    fn ordered_urls(&self) -> Vec<String> {
        let last = self.lock().last_source.clone();
        let mut urls = self.inner.urls.clone();
        if let Some(last) = last {
            if let Some(i) = urls.iter().position(|u| *u == last) {
                let first = urls.remove(i);
                urls.insert(0, first);
            }
        }
        urls
    }

    /// Fetches the feed: the address that answered last time first, then the others; a 304 keeps the cache.
    /// When no address answers the studio keeps showing what it has.
    pub async fn refresh(&self) -> Result<(), String> {
        let app = self.inner.config.app.clone();
        let etag = self.lock().feed.as_ref().and_then(|f| f.etag.clone());
        let mut errors = Vec::new();
        for url in self.ordered_urls() {
            let mut request = self.inner.config.http.get(format!("{url}/v1/feed/{app}")).timeout(REQUEST_TIMEOUT);
            if let Some(tag) = &etag {
                request = request.header(reqwest::header::IF_NONE_MATCH, tag);
            }
            let attempt = async {
                let response = request.send().await.map_err(|e| e.to_string())?;
                if response.status() == reqwest::StatusCode::NOT_MODIFIED {
                    return Ok(None);
                }
                if !response.status().is_success() {
                    return Err(format!("HTTP {}", response.status()));
                }
                let tag = response.headers().get(reqwest::header::ETAG).and_then(|v| v.to_str().ok()).map(str::to_string);
                let feed: Feed = response.json().await.map_err(|e| format!("not a feed: {e}"))?;
                Ok(Some((tag, feed)))
            };
            match tokio::time::timeout(REQUEST_TIMEOUT + CONNECT_TIMEOUT, attempt).await {
                Ok(Ok(fresh)) => {
                    self.change(|s| {
                        match fresh {
                            Some((tag, feed)) => s.feed = Some(CachedFeed { etag: tag, feed, source: url.clone(), fetched_at: now_secs() }),
                            None => {
                                if let Some(f) = &mut s.feed {
                                    f.fetched_at = now_secs();
                                    f.source = url.clone();
                                }
                            }
                        }
                        s.last_source = Some(url.clone());
                        s.last_error = None;
                    });
                    return Ok(());
                }
                Ok(Err(e)) => errors.push(format!("{url}: {e}")),
                Err(_) => errors.push(format!("{url}: timed out")),
            }
        }
        let joined = errors.join("; ");
        tracing::warn!("studio hub: no address answered, the last feed stays: {joined}");
        self.change(|s| s.last_error = Some(joined.clone()));
        Err(joined)
    }

    /// The report for one day as it would leave now (the Settings page shows it before anything is sent).
    pub fn report_for(&self, day: &str) -> Option<Report> {
        let state = self.lock();
        let install = state.install.clone()?;
        let d = state.days.get(day)?;
        Some(Report {
            schema: 1,
            install,
            app: self.inner.config.app.clone(),
            version: self.inner.config.version.clone(),
            day: day.to_string(),
            os: self.inner.config.os_label.chars().filter(|c| c.is_ascii_alphanumeric() || " ._+-".contains(*c)).take(40).collect(),
            ui_lang: self.ui_lang(),
            gpu: self.inner.config.gpu.clone(),
            counts: d.counts.clone(),
            models: d.models.iter().cloned().collect(),
            notices: d.notices.clone(),
        })
    }

    /// Sends every changed day of the last two weeks; a day the hub refuses or that fails to leave stays for the next turn.
    pub async fn send_reports(&self) {
        if !self.telemetry_on() {
            return;
        }
        let oldest = (Utc::now() - chrono::Duration::days(REPORT_DAYS)).format("%Y-%m-%d").to_string();
        let days: Vec<String> = {
            let mut s = self.lock();
            s.prune(&today(), &oldest);
            s.days.iter().filter(|(_, d)| d.dirty).map(|(day, _)| day.clone()).collect()
        };
        for day in days {
            let Some(report) = self.report_for(&day) else { continue };
            let mut sent = false;
            for url in self.ordered_urls() {
                let request = self.inner.config.http.post(format!("{url}/v1/report")).timeout(REQUEST_TIMEOUT).json(&report);
                match request.send().await {
                    Ok(r) if r.status().is_success() => {
                        sent = true;
                        break;
                    }
                    Ok(r) if r.status().is_client_error() && r.status() != reqwest::StatusCode::TOO_MANY_REQUESTS => {
                        tracing::warn!("studio hub: the report of {day} was refused ({}): {}", r.status(), r.text().await.unwrap_or_default());
                        break;
                    }
                    Ok(r) => tracing::warn!("studio hub: {url} answered {} to the report", r.status()),
                    Err(e) => tracing::warn!("studio hub: {url}: the report did not leave: {e}"),
                }
            }
            if sent {
                self.change(|s| {
                    if let Some(d) = s.days.get_mut(&day) {
                        d.dirty = false;
                    }
                });
            }
        }
    }

    /// Starts the background turns inside the current tokio runtime: the feed now if the cache is older than six
    /// hours and then every six hours; reports a minute after start and then every six hours.
    pub fn spawn(&self) {
        let feed = self.clone();
        tokio::spawn(async move {
            loop {
                let stale = feed.lock().feed.as_ref().is_none_or(|f| now_secs() - f.fetched_at >= FEED_EVERY.as_secs() as i64);
                if stale {
                    let _ = feed.refresh().await;
                }
                tokio::time::sleep(Duration::from_secs(1800)).await;
            }
        });
        let reports = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(REPORT_FIRST).await;
            loop {
                reports.send_reports().await;
                tokio::time::sleep(FEED_EVERY).await;
            }
        });
    }

    /// The axum routes the studio's frontend talks to, under `/v1/hub`.
    pub fn router<S: Clone + Send + Sync + 'static>(&self) -> axum::Router<S> {
        routes::router(self.clone())
    }

    fn status(&self) -> serde_json::Value {
        let state = self.lock();
        serde_json::json!({
            "telemetry": {
                "enabled": !self.inner.disabled_by_env && state.telemetry.unwrap_or(true),
                "acknowledged": state.acknowledged,
                "disabledByEnv": self.inner.disabled_by_env,
                "install": state.install,
            },
            "source": state.feed.as_ref().map(|f| f.source.clone()),
            "fetchedAt": state.feed.as_ref().map(|f| f.fetched_at),
            "lastError": state.last_error,
            "test": self.inner.test,
        })
    }
}

#[cfg(test)]
mod tests;
