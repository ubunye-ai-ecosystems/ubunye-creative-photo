//! Community and project links: the Help menu, the About dialog, the start screen and the title
//! bar all open these. Modified by Ubunye AI Ecosystems, 2026: the URLs are Ubunye Creative
//! Photo's (`crate::ubunye`); the upstream project is linked as a credit.

use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The community link (upstream's Discord button and menu item now open it).
pub const DISCORD: &str = crate::ubunye::COMMUNITY;
/// Upstream's "ArtCraft Website" slot: the upstream PhotoCraft project, as a credit.
pub const ARTCRAFT_WEBSITE: &str = crate::ubunye::UPSTREAM;
pub const APP_PAGE: &str = crate::ubunye::PRODUCT_PAGE;
pub const GITHUB: &str = crate::ubunye::SOURCE;
pub const ISSUES: &str = crate::ubunye::ISSUES;
/// Upstream's tutorials page; ours is the product page.
pub const TUTORIALS: &str = crate::ubunye::PRODUCT_PAGE;

/// Help-menu link commands: (id, url). Labels live in `menus::UI_COMMANDS`.
pub const COMMANDS: &[(&str, &str)] =
    &[("help.discord", DISCORD), ("help.website", APP_PAGE), ("help.artcraftWebsite", ARTCRAFT_WEBSITE), ("help.github", GITHUB), ("help.reportIssue", ISSUES)];

pub fn url_for(id: &str) -> Option<&'static str> {
    static URLS: std::sync::OnceLock<std::collections::HashMap<&'static str, &'static str>> = std::sync::OnceLock::new();
    URLS.get_or_init(|| {
        let mut urls = std::collections::HashMap::new();
        for &(id, url) in COMMANDS {
            urls.entry(id).or_insert(url);
        }
        urls
    })
    .get(id)
    .copied()
}

/// Open `url` in the system browser (a new tab on the web) and note it in the status bar. Prefers
/// the platform `open_url` service (reliable on Windows/macOS/Linux); falls back to `ctx.open_url`.
pub fn open(app: &mut PhotocraftApp, ctx: &egui::Context, url: &str) -> Value {
    let opened = app.services.open_url.as_ref().map(|f| f(url).is_ok()).unwrap_or(false);
    if !opened {
        ctx.open_url(egui::OpenUrl::new_tab(url));
    }
    let template = "Opened {url}";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("url", url)]);
    json!({"url": url})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_open_through_the_platform_service() {
        // Issue #14: links (Help menu, Discord button, start-page links) must open in the browser.
        // They route through the `open_url` service rather than the unreliable `ctx.open_url`.
        use std::sync::{Arc, Mutex};
        let opened: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let rec = opened.clone();
        let services = crate::Services {
            open_url: Some(Box::new(move |u: &str| {
                rec.lock().unwrap().push(u.to_string());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let ctx = app.ctx().clone();
        // Every Help-menu link id reaches the service with its URL.
        for (id, url) in COMMANDS {
            crate::menus::invoke(&mut app, &ctx, id, json!({})).unwrap();
            assert_eq!(opened.lock().unwrap().last().map(String::as_str), Some(*url), "{id}");
        }
        // The direct open() helper (Discord button / start-page links) also uses it.
        open(&mut app, &ctx, DISCORD);
        assert_eq!(opened.lock().unwrap().last().map(String::as_str), Some(DISCORD));
    }
}
