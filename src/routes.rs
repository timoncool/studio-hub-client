use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::{Hub, NoticeEvent};

pub fn router<S: Clone + Send + Sync + 'static>(hub: Hub) -> Router<S> {
    Router::new()
        .route("/v1/hub/state", get(state))
        .route("/v1/hub/notices/{id}/{event}", post(notice))
        .route("/v1/hub/telemetry", post(telemetry))
        .route("/v1/hub/telemetry/reset", post(reset))
        .route("/v1/hub/telemetry/preview", get(preview))
        .route("/v1/hub/refresh", post(refresh))
        .route("/v1/hub/media/{key}", get(media))
        .with_state(hub)
}

#[derive(Deserialize)]
struct WindowLang {
    lang: Option<String>,
}

/// Everything the window needs: the telemetry choice, where the feed came from, and the notices that fit now,
/// each with what the user already did with it. The window draws them in its own style and passes its language
/// as `?lang=`, which also goes into the report.
async fn state(State(hub): State<Hub>, Query(window): Query<WindowLang>) -> Json<serde_json::Value> {
    if let Some(lang) = window.lang.as_deref() {
        hub.set_ui_lang(lang);
    }
    let mut out = hub.status();
    let items: Vec<serde_json::Value> = hub
        .items()
        .into_iter()
        .map(|(item, seen, eligible)| {
            let mut value = serde_json::to_value(&item).unwrap_or_default();
            value["seen"] = serde_json::to_value(&seen).unwrap_or_default();
            value["eligible"] = json!(eligible);
            value
        })
        .collect();
    out["items"] = json!(items);
    Json(out)
}

async fn notice(State(hub): State<Hub>, Path((id, event)): Path<(String, String)>) -> Response {
    let event = match event.as_str() {
        "shown" => NoticeEvent::Shown,
        "clicked" => NoticeEvent::Clicked,
        "dismissed" => NoticeEvent::Dismissed,
        _ => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "event is shown, clicked or dismissed" }))).into_response(),
    };
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "bad notice id" }))).into_response();
    }
    hub.notice(&id, event);
    Json(json!({ "ok": true })).into_response()
}

#[derive(Deserialize)]
struct Choice {
    enabled: bool,
    /// true from the start screen: the checkbox has been seen.
    #[serde(default)]
    acknowledge: bool,
}

async fn telemetry(State(hub): State<Hub>, Json(choice): Json<Choice>) -> Json<serde_json::Value> {
    if choice.acknowledge {
        hub.acknowledge(choice.enabled);
    } else {
        hub.set_telemetry(choice.enabled);
    }
    Json(hub.status())
}

async fn reset(State(hub): State<Hub>) -> Json<serde_json::Value> {
    hub.reset_install();
    Json(hub.status())
}

/// Exactly what today's report would carry, for "What is sent" in Settings.
async fn preview(State(hub): State<Hub>) -> Json<serde_json::Value> {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    Json(json!({ "enabled": hub.telemetry_on(), "report": hub.report_for(&today) }))
}

async fn refresh(State(hub): State<Hub>) -> Response {
    match hub.refresh().await {
        Ok(()) => Json(hub.status()).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, Json(json!({ "error": e }))).into_response(),
    }
}

/// A notice image, downloaded once from the hub into the studio's data folder.
async fn media(State(hub): State<Hub>, Path(key): Path<String>) -> Response {
    let (stem, ext) = key.split_once('.').unwrap_or((&key, ""));
    let mime = match ext {
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    if stem.len() != 64 || !stem.chars().all(|c| c.is_ascii_hexdigit()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let file = hub.inner.config.data_dir.join("hub-media").join(&key);
    if let Ok(bytes) = tokio::fs::read(&file).await {
        return ([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], bytes).into_response();
    }
    for url in hub.ordered_urls() {
        let Ok(response) = hub.inner.config.http.get(format!("{url}/v1/media/{key}")).timeout(crate::REQUEST_TIMEOUT).send().await else { continue };
        if !response.status().is_success() {
            continue;
        }
        let Ok(bytes) = response.bytes().await else { continue };
        if let Some(dir) = file.parent() {
            if let Err(e) = tokio::fs::create_dir_all(dir).await {
                tracing::warn!("studio hub: {}: {e}", dir.display());
            }
        }
        if let Err(e) = tokio::fs::write(&file, &bytes).await {
            tracing::warn!("studio hub: the image {key} was not cached: {e}");
        }
        return ([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], bytes.to_vec()).into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}
