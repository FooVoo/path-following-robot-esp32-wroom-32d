//! UI handlers — serve embedded HTML pages.

use std::sync::LazyLock;

use axum::{extract::Path, response::Html};

const SHARED_CSS: &str = include_str!("../templates/_shared.css");
const SHARED_JS:  &str = include_str!("../templates/_shared.js");

/// Fleet overview page — built once at startup, served as a `&'static str`.
static FLEET_HTML: LazyLock<String> = LazyLock::new(|| {
    include_str!("../templates/fleet.html")
        .replace("@@SHARED_CSS@@", SHARED_CSS)
        .replace("@@SHARED_JS@@",  SHARED_JS)
});

/// Robot detail page template — shared CSS/JS substituted at startup;
/// `{ROBOT_ID}` is substituted per-request after HTML-escaping.
static ROBOT_TEMPLATE: LazyLock<String> = LazyLock::new(|| {
    include_str!("../templates/robot.html")
        .replace("@@SHARED_CSS@@", SHARED_CSS)
        .replace("@@SHARED_JS@@",  SHARED_JS)
});

/// Escape characters with special meaning in HTML.
///
/// Applied to the robot ID before it is injected into `<title>`, `<h1>`, and
/// the inline `const IP = '...'` JS variable to prevent reflected XSS.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&'  => out.push_str("&amp;"),
            '<'  => out.push_str("&lt;"),
            '>'  => out.push_str("&gt;"),
            '"'  => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _    => out.push(ch),
        }
    }
    out
}

pub async fn fleet_ui() -> Html<&'static str> {
    Html(FLEET_HTML.as_str())
}

pub async fn robot_ui(Path(id): Path<String>) -> Html<String> {
    Html(ROBOT_TEMPLATE.replace("{ROBOT_ID}", &html_escape(&id)))
}
