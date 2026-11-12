//! A problem a person reports from the studio without a GitHub account: the hub files it as an issue in the
//! program's repository and the studio shows its number and link. Reporting is the person's own act, so it works
//! with telemetry off; the install id goes along only while telemetry is on.

use serde::{Deserialize, Serialize};

use crate::{scrub, Hub};

/// The hub's limits, checked here first so a report it would refuse never leaves.
const MAX_WHAT: usize = 4000;
const MIN_WHAT: usize = 10;
const MAX_STEPS: usize = 4000;
const MAX_EXPECTED: usize = 2000;
const MAX_SET: usize = 80;
const MAX_LOG: usize = 60_000;
const FILE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Deserialize)]
struct RefusedBody {
    #[serde(default)]
    error: String,
    #[serde(default)]
    retry_after_s: Option<u64>,
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

    /// Files the report through the hub: the issue's number and link, or why it was not filed.
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
                return response.json().await.map_err(|e| ReportError::Unreachable(format!("{url}: not an answer of the hub: {e}")));
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
}
