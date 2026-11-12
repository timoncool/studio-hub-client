//! A problem a person reports from the studio without a GitHub account: the hub files it as an issue in the
//! program's repository, and what is answered there comes back to the studio. Reporting is the person's own act,
//! so it works with telemetry off; the install id goes along only while telemetry is on.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::feed::{Button, Content, FeedItem, Rules};
use crate::state::{FiledProblem, ProblemAnswer};
use crate::{now_secs, scrub, Hub};

/// The hub's limits, checked here first so a report it would refuse never leaves.
const MAX_WHAT: usize = 4000;
const MIN_WHAT: usize = 10;
const MAX_STEPS: usize = 4000;
const MAX_EXPECTED: usize = 2000;
const MAX_SET: usize = 80;
const MAX_LOG: usize = 60_000;
/// Reports remembered for their answers; the oldest go first.
const MAX_PROBLEMS: usize = 50;
const MAX_ANSWERS: usize = 50;
/// An open report is asked about hourly, a closed one daily for a month: it may be reopened or answered once more.
const CHECK_OPEN: i64 = 3600;
const CHECK_CLOSED: i64 = 24 * 3600;
const CLOSED_FOR: i64 = 30 * 24 * 3600;
const FILE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// Above every notice of the hub: an answer to the person's own report.
const ANSWER_PRIORITY: i32 = 1000;
/// Characters of a comment shown in the answer's line.
const ANSWER_LINE: usize = 140;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProblemKind {
    Bug,
    Idea,
    Question,
}

/// What the person wrote and what the studio adds; the program, version, system, language and card come from the
/// hub's own configuration.
#[derive(Clone, Debug)]
pub struct ProblemReport {
    pub kind: ProblemKind,
    pub what: String,
    pub steps: Option<String>,
    pub expected: Option<String>,
    /// The model set in use.
    pub models: Option<String>,
    /// The tail of the engine log; cleaned of paths, addresses and ids here before it leaves.
    pub log: Option<String>,
}

/// The issue the hub made of a report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Filed {
    pub number: u64,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportError {
    /// A field the hub refuses, named as the hub names it.
    Field(String),
    /// Too many reports from this computer; seconds until the next one is taken.
    Rate { retry_after_s: u64 },
    /// The hub does not take reports now: no GitHub application behind it, or switched off.
    NotConfigured,
    /// The hub reached its ceiling of issues for the day.
    Busy,
    /// No hub address answered, or GitHub did not take the issue.
    Unreachable(String),
}

impl std::fmt::Display for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReportError::Field(field) => write!(f, "the hub refused the report: {field}"),
            ReportError::Rate { retry_after_s } => write!(f, "too many reports from this computer, the next one in {retry_after_s} s"),
            ReportError::NotConfigured => write!(f, "the hub does not take reports now"),
            ReportError::Busy => write!(f, "the hub took its day's worth of reports"),
            ReportError::Unreachable(why) => write!(f, "the report did not reach the hub: {why}"),
        }
    }
}

/// A report of this computer and where it stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProblemStatus {
    pub number: u64,
    pub url: String,
    pub closed: bool,
    /// Unix seconds.
    pub filed_at: i64,
}

/// Something new in a report's issue the person has not been shown: a comment, or its closing (no author then).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Answer {
    pub number: u64,
    pub url: String,
    pub at: Option<String>,
    pub by: Option<String>,
    pub excerpt: Option<String>,
    pub closed: bool,
}

#[derive(Deserialize)]
struct FiledBody {
    number: u64,
    url: String,
    token: String,
}

#[derive(Deserialize)]
struct RefusedBody {
    #[serde(default)]
    error: String,
    #[serde(default)]
    retry_after_s: Option<u64>,
}

#[derive(Deserialize)]
struct IssueBody {
    state: String,
    #[serde(default)]
    answers: Vec<ProblemAnswer>,
}

/// The id of the notice that shows an answer to report `number`, and the number back from such an id.
fn answer_id(number: u64) -> String {
    format!("answer-{number}")
}

pub(crate) fn answer_number(id: &str) -> Option<u64> {
    id.strip_prefix("answer-")?.parse().ok()
}

/// The answer's line in the window's languages: what happened to the report, then the start of the comment.
fn answer_content(answer: &Answer) -> BTreeMap<String, Content> {
    let n = answer.number;
    let lines: [(&str, String, &str); 5] = if answer.excerpt.is_some() {
        [
            ("en", format!("An answer to your report #{n}"), "Open"),
            ("ru", format!("Ответ на ваше сообщение № {n}"), "Открыть"),
            ("zh", format!("您的反馈 #{n} 有新回复"), "查看"),
            ("ja", format!("ご報告 #{n} に返信があります"), "開く"),
            ("ko", format!("보내신 문제 #{n}에 답변이 달렸습니다"), "열기"),
        ]
    } else {
        [
            ("en", format!("Your report #{n} is closed"), "Open"),
            ("ru", format!("Ваше сообщение № {n} закрыто"), "Открыть"),
            ("zh", format!("您的反馈 #{n} 已关闭"), "查看"),
            ("ja", format!("ご報告 #{n} はクローズされました"), "開く"),
            ("ko", format!("보내신 문제 #{n}이(가) 종료되었습니다"), "열기"),
        ]
    };
    let excerpt: Option<String> = answer.excerpt.as_deref().map(|excerpt| {
        let line = excerpt.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.chars().count() > ANSWER_LINE { format!("{}…", line.chars().take(ANSWER_LINE).collect::<String>()) } else { line }
    });
    lines
        .into_iter()
        .map(|(lang, headline, open)| {
            let body = match excerpt.as_deref().filter(|excerpt| !excerpt.is_empty()) {
                Some(excerpt) => format!("{headline}: {excerpt}"),
                None => headline,
            };
            let button = Button { label: open.into(), action: "url".into(), url: Some(answer.url.clone()), target: None, style: "primary".into() };
            (lang.to_string(), Content { title: String::new(), body, buttons: vec![button] })
        })
        .collect()
}

fn text(value: &str, max: usize) -> String {
    let clean: String = value.chars().filter(|c| *c == '\n' || !c.is_control()).collect();
    clean.trim().chars().take(max).collect()
}

fn optional(value: &Option<String>, max: usize) -> Option<String> {
    value.as_deref().map(|value| text(value, max)).filter(|value| !value.is_empty())
}

impl Hub {
    /// The body of `POST /v1/issue` as it would leave now: what the window shows before anything is sent.
    pub fn problem_body(&self, report: &ProblemReport) -> Result<serde_json::Value, ReportError> {
        let what = text(&report.what, MAX_WHAT);
        if what.chars().count() < MIN_WHAT {
            return Err(ReportError::Field(format!("what: at least {MIN_WHAT} characters")));
        }
        let config = &self.inner.config;
        let mut body = serde_json::json!({
            "schema": 1,
            "app": config.app,
            "version": config.version,
            "os": config.os_label.chars().filter(|c| c.is_ascii_alphanumeric() || " ._+-".contains(*c)).take(40).collect::<String>(),
            "ui_lang": self.ui_lang(),
            "gpu": config.gpu,
            "kind": report.kind,
            "what": what,
        });
        let fields = body.as_object_mut().expect("an object");
        for (name, value) in [("steps", optional(&report.steps, MAX_STEPS)), ("expected", optional(&report.expected, MAX_EXPECTED)), ("models", optional(&report.models, MAX_SET))] {
            if let Some(value) = value {
                fields.insert(name.into(), value.into());
            }
        }
        if let Some(log) = report.log.as_deref().map(|log| scrub::lines(log, MAX_LOG)).filter(|log| !log.is_empty()) {
            fields.insert("log".into(), log.into());
        }
        let state = self.lock();
        if self.telemetry_on_with(&state) {
            if let Some(install) = &state.install {
                fields.insert("install".into(), install.clone().into());
            }
            drop(state);
            if let Some(machine) = self.inner.machine.get_or_init(crate::machine::id) {
                fields.insert("machine".into(), machine.clone().into());
            }
        }
        Ok(body)
    }

    /// Files the report through the hub and remembers the issue, so its answers reach this studio.
    pub async fn report_problem(&self, report: ProblemReport) -> Result<Filed, ReportError> {
        let body = self.problem_body(&report)?;
        let mut errors = Vec::new();
        for url in self.ordered_urls() {
            let request = self.inner.config.http.post(format!("{url}/v1/issue")).timeout(FILE_TIMEOUT).json(&body);
            let response = match request.send().await {
                Ok(response) => response,
                Err(e) => {
                    errors.push(format!("{url}: {e}"));
                    continue;
                }
            };
            let status = response.status();
            if status.is_success() {
                let filed: FiledBody = response.json().await.map_err(|e| ReportError::Unreachable(format!("{url}: not an answer of the hub: {e}")))?;
                self.change(|s| {
                    s.problems.retain(|known| known.token != filed.token);
                    s.problems.push(FiledProblem { token: filed.token.clone(), number: filed.number, url: filed.url.clone(), filed_at: now_secs(), ..FiledProblem::default() });
                    let extra = s.problems.len().saturating_sub(MAX_PROBLEMS);
                    s.problems.drain(..extra);
                });
                return Ok(Filed { number: filed.number, url: filed.url });
            }
            let refused: RefusedBody = response.json().await.unwrap_or(RefusedBody { error: String::new(), retry_after_s: None });
            match status.as_u16() {
                400 | 413 | 422 => return Err(ReportError::Field(if refused.error.is_empty() { format!("HTTP {status}") } else { refused.error })),
                429 => return Err(ReportError::Rate { retry_after_s: refused.retry_after_s.unwrap_or(3600) }),
                503 if refused.error == "not_configured" => return Err(ReportError::NotConfigured),
                503 if refused.error == "busy" => return Err(ReportError::Busy),
                503 if refused.error == "github" => return Err(ReportError::Unreachable("GitHub did not take the issue".into())),
                // an answer that is not the hub's own (a proxy's 502, a page of the network): the next address is tried
                _ => errors.push(format!("{url}: HTTP {status}")),
            }
        }
        Err(ReportError::Unreachable(errors.join("; ")))
    }

    /// The reports of this computer, newest first.
    pub fn problems(&self) -> Vec<ProblemStatus> {
        let state = self.lock();
        state.problems.iter().rev().filter(|p| !p.gone).map(|p| ProblemStatus { number: p.number, url: p.url.clone(), closed: p.closed, filed_at: p.filed_at }).collect()
    }

    /// What was answered in the reports' issues and not shown yet: every new comment, and a closing with no comment
    /// beside it.
    pub fn problem_answers(&self) -> Vec<Answer> {
        let state = self.lock();
        let mut out = Vec::new();
        for p in state.problems.iter().filter(|p| !p.gone) {
            let before = out.len();
            for a in p.answers.iter().filter(|a| !p.seen.contains(&a.id)) {
                out.push(Answer { number: p.number, url: a.url.clone(), at: Some(a.at.clone()), by: Some(a.by.clone()), excerpt: Some(a.excerpt.clone()), closed: p.closed });
            }
            if p.closed && !p.closed_seen && out.len() == before {
                out.push(Answer { number: p.number, url: p.url.clone(), at: None, by: None, excerpt: None, closed: true });
            }
        }
        out
    }

    /// The answers as notices of the window's own kind, one bar a report with its newest answer: they stand in
    /// [`Hub::items`] above the hub's notices, and closing or clicking one marks the report's answers seen.
    pub(crate) fn answer_items(&self) -> Vec<FeedItem> {
        let mut newest: BTreeMap<u64, Answer> = BTreeMap::new();
        for answer in self.problem_answers() {
            newest.insert(answer.number, answer);
        }
        newest
            .into_values()
            .map(|answer| FeedItem {
                id: answer_id(answer.number),
                kind: "bar".into(),
                priority: ANSWER_PRIORITY,
                from: None,
                until: None,
                versions: None,
                os: None,
                langs: None,
                test: false,
                ad: None,
                theme: None,
                dismissible: true,
                rules: Rules {
                    delay_s: 0,
                    after_sessions: 1,
                    views: None,
                    frequency: "session".into(),
                    interval_h: 24,
                    max_shows: None,
                    after_dismiss: "never".into(),
                    snooze_h: 72,
                    audience: "all".into(),
                    new_days: 7,
                    after_click: "never".into(),
                    min_done: None,
                    max_done: None,
                    min_active_days: None,
                    min_age_days: None,
                    after_success: false,
                    after_failure: None,
                    after_event: None,
                    absent_days: None,
                    unused_route: None,
                    not_installed: None,
                    shown_before: None,
                    class: None,
                },
                image: None,
                date: None,
                tags: Vec::new(),
                content: answer_content(&answer),
                experiment: None,
            })
            .collect()
    }

    /// The person was shown what [`Hub::problem_answers`] gave for this report.
    pub fn answers_seen(&self, number: u64) {
        self.change(|s| {
            for p in s.problems.iter_mut().filter(|p| p.number == number) {
                p.seen = p.answers.iter().map(|a| a.id).collect();
                p.closed_seen = p.closed;
            }
        });
    }

    /// Asks the hub about the reports that are due; the feed's turn calls it, and the studio may when its window opens.
    pub async fn check_problems(&self) {
        let now = now_secs();
        let due: Vec<String> = {
            let state = self.lock();
            state
                .problems
                .iter()
                .filter(|p| !p.gone)
                .filter(|p| if p.closed { now - p.closed_at.unwrap_or(p.filed_at) < CLOSED_FOR && now - p.checked_at >= CHECK_CLOSED } else { now - p.checked_at >= CHECK_OPEN })
                .map(|p| p.token.clone())
                .collect()
        };
        for token in due {
            for url in self.ordered_urls() {
                let request = self.inner.config.http.get(format!("{url}/v1/issue/{token}")).timeout(crate::REQUEST_TIMEOUT);
                let Ok(response) = request.send().await else { continue };
                if response.status() == reqwest::StatusCode::NOT_FOUND {
                    self.change(|s| s.problems.iter_mut().filter(|p| p.token == token).for_each(|p| p.gone = true));
                    break;
                }
                if !response.status().is_success() {
                    continue;
                }
                let Ok(issue) = response.json::<IssueBody>().await else { continue };
                self.change(|s| {
                    for p in s.problems.iter_mut().filter(|p| p.token == token) {
                        let closed = issue.state == "closed";
                        if closed && !p.closed {
                            p.closed_at = Some(now);
                        }
                        if !closed {
                            p.closed_seen = false;
                            p.closed_at = None;
                        }
                        p.closed = closed;
                        p.checked_at = now;
                        let mut answers = issue.answers.clone();
                        let extra = answers.len().saturating_sub(MAX_ANSWERS);
                        answers.drain(..extra);
                        p.seen.retain(|id| answers.iter().any(|a| a.id == *id));
                        p.answers = answers;
                    }
                });
                break;
            }
        }
    }
}
