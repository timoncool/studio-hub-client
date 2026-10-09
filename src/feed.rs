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
    /// once | until_dismissed
    pub show: String,
    pub image: Option<String>,
    pub date: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub content: BTreeMap<String, Content>,
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
            "test": false, "ad": null, "theme": null, "dismissible": true, "show": "until_dismissed", "image": null, "date": null,
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
    fn the_text_falls_back_to_english_then_any_language() {
        let n = item(serde_json::json!({"content": {"es": {"title": "Hola"}, "en": {"title": "Hi"}}}));
        assert_eq!(n.content_for("ru").unwrap().title, "Hi");
        let n = item(serde_json::json!({"content": {"es": {"title": "Hola"}}}));
        assert_eq!(n.content_for("ru").unwrap().title, "Hola");
        assert!(compare_versions("3.10.0", "3.9.9").is_gt());
    }
}
