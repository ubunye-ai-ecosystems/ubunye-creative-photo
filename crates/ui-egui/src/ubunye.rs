//! Ubunye Photo identity: the one place the shell reads its product name, ids and links
//! from. Values mirror `ubunye/brand.toml`; `python -I ubunye/brand/apply.py --check` fails when
//! they drift. This file is Ubunye's own (not upstream PhotoCraft's), so upstream merges never
//! touch it.

/// ThabangVision collaboration: the client of the service on this computer and its window.
pub mod kopano;
pub mod kopano_panel;
#[cfg(test)]
mod tests_kopano;

/// The product name shown in the UI, installers and package metadata.
pub const NAME: &str = "Ubunye Photo";
/// The short form, where a short name is needed.
pub const SHORT_NAME: &str = "Photo";
/// The parent brand (Ubunye Creative Studio, by Ubunye AI Ecosystems, distributed by ThabangVision).
pub const SUITE: &str = "Ubunye Creative Studio";
/// App id: macOS bundle id, Linux desktop id, Flatpak id, Wayland app id.
pub const APP_ID: &str = "org.ubunye.creative.photo";
/// Vendor, manufacturer and package maintainer.
pub const VENDOR: &str = "Ubunye AI Ecosystems";
/// Product page.
pub const PRODUCT_PAGE: &str = "https://www.thabangvision.com/tools/photo";
/// Source repository.
pub const SOURCE: &str = "https://github.com/ubunye-ai-ecosystems/ubunye-creative-photo";
/// Where to report problems.
pub const ISSUES: &str = "https://github.com/ubunye-ai-ecosystems/ubunye-creative-photo/issues";
/// Community discussion (replaces upstream's Discord server).
pub const COMMUNITY: &str = "https://github.com/ubunye-ai-ecosystems/ubunye-creative-photo/discussions";
/// The upstream project this distribution is based on (credit link).
pub const UPSTREAM: &str = "https://github.com/storytold/photocraft";
/// Licence of the code.
pub const LICENSE: &str = "MIT OR Apache-2.0";

/// The About tab's credit block: the product and its parent brand, the plain-text upstream credit
/// (a link to the upstream repository) and the licence. Upstream's About text, version line and
/// third-party credits stay.
pub fn about_credit(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.label(crate::i18n::fmt(tl!("{name}, part of {suite}."), &[("name", NAME), ("suite", SUITE)]));
    let credit = ui.link(egui::RichText::new(tl!("Based on PhotoCraft by the ArtCraft team")).color(t.accent)).on_hover_text(UPSTREAM);
    if credit.clicked() {
        crate::links::open(app, ui.ctx(), UPSTREAM);
    }
    ui.label(tl!("License: MIT OR Apache-2.0"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_ubunye() {
        assert_eq!(NAME, "Ubunye Photo");
        assert_eq!(APP_ID, "org.ubunye.creative.photo");
        for url in [PRODUCT_PAGE, SOURCE, ISSUES, COMMUNITY] {
            assert!(!url.contains("getartcraft") && !url.contains("discord") && !url.contains("storytold"), "{url}");
        }
    }

    #[test]
    fn the_credit_and_licence_are_translated() {
        for lang in crate::i18n::Lang::all().filter(|l| l.complete_menus()) {
            for s in [
                "{name}, part of {suite}.",
                "Based on PhotoCraft by the ArtCraft team",
                "License: MIT OR Apache-2.0",
                "Join the Community",
                "Upstream PhotoCraft Project",
            ] {
                assert!(crate::i18n::has(lang, s), "{}: {s}", lang.code());
            }
        }
    }
}
