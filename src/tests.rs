use std::sync::{Arc, Mutex};

use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};

use super::*;

#[derive(Clone, Default)]
struct Mock {
    reports: Arc<Mutex<Vec<serde_json::Value>>>,
    feed_calls: Arc<Mutex<u32>>,
}

const ETAG: &str = "\"abc\"";

fn feed() -> serde_json::Value {
    serde_json::json!({"schema": 1, "app": "yue2", "generated": "2026-10-09T00:00:00.000Z", "items": [
        {"id": "low", "kind": "bar", "priority": 1, "from": null, "until": null, "versions": null, "os": null, "langs": null, "test": false,
         "ad": null, "theme": null, "dismissible": true, "rules": {"delay_s": 0, "after_sessions": 1, "views": null, "frequency": "session", "interval_h": 24, "max_shows": null, "after_dismiss": "never", "snooze_h": 72, "audience": "all", "new_days": 7}, "image": null, "date": null, "tags": [],
         "content": {"en": {"title": "", "body": "Low", "buttons": []}}},
        {"id": "high", "kind": "bar", "priority": 9, "from": null, "until": null, "versions": null, "os": null, "langs": null, "test": false,
         "ad": null, "theme": "lime", "dismissible": true, "rules": {"delay_s": 0, "after_sessions": 1, "views": null, "frequency": "session", "interval_h": 24, "max_shows": null, "after_dismiss": "never", "snooze_h": 72, "audience": "all", "new_days": 7}, "image": null, "date": null, "tags": [],
         "content": {"en": {"title": "", "body": "High", "buttons": []}}},
        {"id": "test-only", "kind": "popup", "priority": 5, "from": null, "until": null, "versions": null, "os": null, "langs": null, "test": true,
         "ad": null, "theme": null, "dismissible": true, "rules": {"delay_s": 20, "after_sessions": 1, "views": null, "frequency": "once", "interval_h": 24, "max_shows": null, "after_dismiss": "never", "snooze_h": 72, "audience": "all", "new_days": 7}, "image": null, "date": null, "tags": [],
         "content": {"en": {"title": "T", "body": "", "buttons": []}}}
    ]})
}

async fn serve(mock: Mock) -> String {
    let app = Router::new()
        .route(
            "/v1/feed/yue2",
            get({
                let mock = mock.clone();
                move |headers: HeaderMap| {
                    let mock = mock.clone();
                    async move {
                        *mock.feed_calls.lock().unwrap() += 1;
                        if headers.get("if-none-match").and_then(|v| v.to_str().ok()) == Some(ETAG) {
                            return (StatusCode::NOT_MODIFIED, [("etag", ETAG)], Json(serde_json::Value::Null));
                        }
                        (StatusCode::OK, [("etag", ETAG)], Json(feed()))
                    }
                }
            }),
        )
        .route(
            "/v1/report",
            post({
                let mock = mock.clone();
                move |Json(body): Json<serde_json::Value>| {
                    let mock = mock.clone();
                    async move {
                        mock.reports.lock().unwrap().push(body);
                        Json(serde_json::json!({"ok": true}))
                    }
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// An address nothing listens on: the first in the cascade, so the client has to move on.
async fn dead() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}

fn make_hub(dir: &std::path::Path, urls: Vec<String>) -> Hub {
    Hub::new(HubConfig {
        app: "yue2".into(),
        version: "3.5.0".into(),
        data_dir: dir.to_path_buf(),
        http: reqwest::Client::new(),
        os_label: "windows 11".into(),
        gpu: Gpu { vendor: "nvidia".into(), vram_gb: Gpu::vram_bucket(24 * 1_073_741_824), backend: "cuda".into() },
        ui_lang: "ru".into(),
        urls,
    })
    .unwrap()
}

#[tokio::test]
async fn the_feed_comes_through_the_cascade_and_revalidates_with_its_etag() {
    let dir = tempfile::tempdir().unwrap();
    let mock = Mock::default();
    let live = serve(mock.clone()).await;
    let hub = make_hub(dir.path(), vec![dead().await, live.clone()]);
    hub.refresh().await.unwrap();
    let ids: Vec<String> = hub.items().into_iter().map(|(i, _, _)| i.id).collect();
    assert_eq!(ids, ["high", "low"], "priority order, the test notice left out");
    assert_eq!(hub.lock().last_source.as_deref(), Some(live.as_str()));

    hub.refresh().await.unwrap();
    assert_eq!(*mock.feed_calls.lock().unwrap(), 2);
    assert_eq!(hub.items().len(), 2, "a 304 keeps the cached feed");

    let again = make_hub(dir.path(), vec![dead().await]);
    assert!(again.refresh().await.is_err());
    assert_eq!(again.items().len(), 2, "offline: the last feed stays");
}

#[tokio::test]
async fn nothing_is_counted_before_the_choice_and_the_id_lives_only_while_telemetry_is_on() {
    let dir = tempfile::tempdir().unwrap();
    let hub = make_hub(dir.path(), vec![dead().await]);
    hub.count("songs", 1);
    assert!(hub.lock().days.is_empty(), "the checkbox was not on screen yet");
    assert!(hub.lock().install.is_none());

    hub.acknowledge(true);
    let id = hub.lock().install.clone().unwrap();
    assert_eq!(uuid::Uuid::parse_str(&id).unwrap().get_version_num(), 4);
    hub.count("songs", 2);
    hub.count("Bad Name", 1);
    hub.used_model("yue2-q8");
    let report = hub.report_for(&today()).unwrap();
    assert_eq!(report.counts.get("songs"), Some(&2));
    assert_eq!(report.counts.len(), 1);
    assert_eq!(report.models, ["yue2-q8"]);
    assert_eq!(report.gpu.vram_gb, "24+");

    hub.reset_install();
    assert_ne!(hub.lock().install.as_deref(), Some(id.as_str()), "a new id");
    assert!(hub.lock().days.is_empty(), "the old id's counts go with it");

    hub.set_telemetry(false);
    assert!(hub.lock().install.is_none());
    hub.count("songs", 1);
    assert!(hub.lock().days.is_empty());

    hub.notice("high", NoticeEvent::Dismissed);
    assert!(hub.lock().seen.get("high").unwrap().dismissed_at.is_some(), "closing is remembered even with telemetry off");
}

#[tokio::test]
async fn a_report_leaves_once_and_carries_counts_and_notice_results() {
    let dir = tempfile::tempdir().unwrap();
    let mock = Mock::default();
    let hub = make_hub(dir.path(), vec![dead().await, serve(mock.clone()).await]);
    hub.acknowledge(true);
    hub.count("songs", 3);
    hub.notice("high", NoticeEvent::Shown);
    hub.notice("high", NoticeEvent::Clicked);
    hub.send_reports().await;
    hub.send_reports().await;
    let reports = mock.reports.lock().unwrap().clone();
    assert_eq!(reports.len(), 1, "a clean day is not sent again");
    let r = &reports[0];
    assert_eq!(r["schema"], 1);
    assert_eq!(r["app"], "yue2");
    assert_eq!(r["counts"]["songs"], 3);
    assert_eq!(r["notices"]["high"]["clicked"], 1);
    assert!(r.get("path").is_none() && r.get("ip").is_none());

    hub.count("songs", 1);
    hub.send_reports().await;
    assert_eq!(mock.reports.lock().unwrap().len(), 2, "a changed day goes again (the hub replaces it)");
}

#[tokio::test]
async fn the_state_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let first = make_hub(dir.path(), vec![dead().await]);
    first.acknowledge(true);
    first.notice("high", NoticeEvent::Shown);
    let id = first.lock().install.clone();
    let second = make_hub(dir.path(), vec![dead().await]);
    assert_eq!(second.lock().install, id);
    assert!(second.lock().seen.contains_key("high"));
    assert!(second.telemetry_on());
    assert_eq!(second.lock().sessions, 2, "each start is a launch");
}

#[test]
fn a_card_reports_the_largest_class_it_holds() {
    let gib = |g: f64| (g * 1_073_741_824.0) as u64;
    assert_eq!(Gpu::vram_bucket(gib(11.99)), "12");
    assert_eq!(Gpu::vram_bucket(gib(10.0)), "<=8");
    assert_eq!(Gpu::vram_bucket(gib(20.0)), "16");
    assert_eq!(Gpu::vram_bucket(gib(23.99)), "24+");
    assert_eq!(Gpu::vram_bucket(0), "?");
}

#[tokio::test]
async fn only_two_ascii_letters_become_the_report_language() {
    let dir = tempfile::tempdir().unwrap();
    let hub = make_hub(dir.path(), vec![dead().await]);
    hub.set_ui_lang("pt-BR");
    assert_eq!(hub.ui_lang(), "pt");
    hub.set_ui_lang("ру");
    hub.set_ui_lang("x");
    assert_eq!(hub.ui_lang(), "pt");
}

#[tokio::test]
async fn a_model_set_is_one_set_whatever_order_its_parts_came_in_and_a_failure_keeps_no_personal_text() {
    let dir = tempfile::tempdir().unwrap();
    let hub = make_hub(dir.path(), vec![dead().await]);
    hub.acknowledge(true);
    let parts = |list: &[&str]| list.iter().map(|part| part.to_string()).collect::<Vec<_>>();
    hub.used_models(None, &parts(&["lm-4b-q8", "dit-xl-turbo-q6", "vae-standard-bf16"]), 2);
    hub.used_models(None, &parts(&["vae-standard-bf16", "lm-4b-q8", "dit-xl-turbo-q6", "lm-4b-q8"]), 1);
    hub.used_models(Some("quality-q8"), &parts(&["lm-q8", "dit-q8"]), 4);
    hub.used_models(None, &[], 1);
    hub.failed("song", r"open C:\Users\Ivan\Music\x.flac: CUDA error: out of memory");
    hub.failed("song", r"open D:\other\y.flac: CUDA error: out of memory");
    hub.failed("Bad Kind", "ignored");
    hub.failed("Song", "ignored: the hub takes lowercase kinds only");
    hub.used_model("a model name far longer than the forty characters the hub takes");
    let report = hub.report_for(&today()).unwrap();
    assert!(report.models.is_empty(), "{:?}", report.models);
    assert_eq!(report.model_sets.len(), 2, "{:?}", report.model_sets);
    assert!(report.model_sets.iter().any(|set| set.set.is_none() && set.components == ["dit-xl-turbo-q6", "lm-4b-q8", "vae-standard-bf16"] && set.count == 3));
    assert!(report.model_sets.iter().any(|set| set.set.as_deref() == Some("quality-q8") && set.components == ["dit-q8", "lm-q8"] && set.count == 4));
    assert_eq!(report.failures.len(), 1);
    assert_eq!((report.failures[0].kind.as_str(), report.failures[0].reason.as_str(), report.failures[0].count), ("song", "open <path>: CUDA error: out of memory", 2));
    let sent = serde_json::to_value(&report).unwrap();
    assert!(!sent.to_string().contains("Ivan"));
}

#[tokio::test]
async fn a_click_says_which_button_or_link() {
    let dir = tempfile::tempdir().unwrap();
    let hub = make_hub(dir.path(), vec![dead().await]);
    hub.acknowledge(true);
    hub.notice_on("promo", NoticeEvent::Clicked, Some("b1"));
    hub.notice_on("promo", NoticeEvent::Clicked, Some("b1"));
    hub.notice_on("promo", NoticeEvent::Clicked, Some("link"));
    hub.notice_on("promo", NoticeEvent::Clicked, Some("not a button"));
    let report = hub.report_for(&today()).unwrap();
    let promo = report.notices.get("promo").unwrap();
    assert_eq!(promo.clicked, 4);
    assert_eq!(promo.buttons.get("b1"), Some(&2));
    assert_eq!(promo.buttons.get("link"), Some(&1));
    assert_eq!(promo.buttons.len(), 2);
}

#[tokio::test]
async fn a_closing_studio_sends_its_day() {
    let dir = tempfile::tempdir().unwrap();
    let mock = Mock::default();
    let hub = make_hub(dir.path(), vec![serve(mock.clone()).await]);
    hub.acknowledge(true);
    hub.count("songs", 1);
    hub.flush(std::time::Duration::from_secs(5)).await;
    assert_eq!(mock.reports.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn every_call_to_the_studios_api_is_counted_by_its_route_and_nothing_else() {
    use tower::ServiceExt;
    let dir = tempfile::tempdir().unwrap();
    let hub = make_hub(dir.path(), vec![dead().await]);
    let app: Router = hub.track(
        Router::new()
            .route("/api/songs/{id}/stems", post(|| async { "ok" }))
            .route("/api/songs", get(|| async { "ok" }))
            .route("/api/broken", post(|| async { StatusCode::INTERNAL_SERVER_ERROR }))
            .route("/api/jobs/{id}", get(|| async { StatusCode::NOT_FOUND }))
            .route("/mcp", post(|| async { "ok" }))
            .route("/files/{*rest}", get(|| async { "file" }))
            .merge(hub.router()),
    );
    let call = |method: &str, uri: &str| axum::http::Request::builder().method(method).uri(uri).body(axum::body::Body::empty()).unwrap();
    app.clone().oneshot(call("GET", "/api/songs")).await.unwrap();
    hub.acknowledge(true);
    for uri in ["/api/songs/a1/stems", "/api/songs/b2/stems"] {
        app.clone().oneshot(call("POST", uri)).await.unwrap();
    }
    app.clone().oneshot(call("GET", "/api/songs")).await.unwrap();
    app.clone().oneshot(call("POST", "/api/broken")).await.unwrap();
    app.clone().oneshot(call("GET", "/files/a/b.js")).await.unwrap();
    app.clone().oneshot(call("GET", "/v1/hub/state")).await.unwrap();
    app.clone().oneshot(call("GET", "/nowhere")).await.unwrap();
    app.clone().oneshot(call("GET", "/api/jobs/gone")).await.unwrap();
    app.clone().oneshot(call("POST", "/mcp")).await.unwrap();
    let routes = hub.report_for(&today()).unwrap().routes;
    let seen: Vec<(&str, u64, u64)> = routes.iter().map(|(k, v)| (k.as_str(), v.calls, v.errors)).collect();
    assert_eq!(
        seen,
        [("GET /api/jobs/{id}", 1, 0), ("GET /api/songs", 1, 0), ("POST /api/broken", 1, 1), ("POST /api/songs/{id}/stems", 2, 0)],
        "templates, not ids; the call before the choice, the MCP transport and the hub's own routes are not counted; a 404 is no error"
    );
    assert!(!hub.lock().days[&today()].dirty, "calls alone do not make a day to report");

    hub.send_reports().await;
    let again = make_hub(dir.path(), vec![dead().await]);
    assert_eq!(again.lock().days[&today()].routes.len(), 4, "the counts reached the disk with the report turn, sent or not");
}

#[tokio::test]
async fn an_install_sees_one_variant_of_an_experiment_and_old_feeds_still_parse() {
    let dir = tempfile::tempdir().unwrap();
    let hub = make_hub(dir.path(), vec![dead().await]);
    let mut body = feed();
    let arm = |id: &str| {
        let mut v = body["items"][0].clone();
        v["id"] = serde_json::json!(id);
        v["experiment"] = serde_json::json!({"id": "donate", "weight": 1});
        v
    };
    body["variants"] = serde_json::json!([arm("donate-a"), arm("donate-b")]);
    let parsed: Feed = serde_json::from_value(body).unwrap();
    hub.change(|s| s.feed = Some(CachedFeed { etag: None, feed: parsed, source: "x".into(), fetched_at: now_secs() }));
    let ids: Vec<String> = hub.items().into_iter().map(|(i, _, _)| i.id).collect();
    let variants: Vec<&String> = ids.iter().filter(|id| id.starts_with("donate-")).collect();
    assert_eq!(variants.len(), 1, "{ids:?}");
    let first = variants[0].clone();
    let again = make_hub(dir.path(), vec![dead().await]);
    assert!(again.items().iter().any(|(i, _, _)| i.id == first), "the same variant after a restart");

    let old: Feed = serde_json::from_value(feed()).unwrap();
    assert!(old.variants.is_empty());
}
