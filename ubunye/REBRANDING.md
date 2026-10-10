# Rebranding PhotoCraft → Ubunye Creative Photo

Status: **inventory only.** Branding changes start after the brand kit is approved (Milestone 4).

## Obligations
- The ArtCraft name, wordmark and logos (`docs/brand/`) are proprietary. Upstream's `docs/brand/LICENSE-brand.txt` requires them to be **removed** from any distributed modified version.
- We may say "Based on PhotoCraft by the ArtCraft team" in plain text. That credit appears in About, the README and the website.
- LICENSE-MIT, LICENSE-APACHE, NOTICE and ATTRIBUTION stay unchanged. Our notice is added alongside them, not in place of them.

## Target identity (from `brand.toml`)
| Item | Upstream | Ubunye |
|---|---|---|
| Product name | PhotoCraft | Ubunye Creative Photo |
| App / bundle / Flatpak id | `ai.storyteller.photocraft` | `org.ubunye.creative.photo` |
| Vendor / manufacturer | Learning Machines LLC | Ubunye AI Ecosystems |
| Windows MSI UpgradeCode | upstream GUID | `B5F26DFD-A4A7-4D46-8DDF-79537F0A829B` |
| Env var prefix | `PHOTOCRAFT_` | `UBUNYE_PHOTO_` (keep reading `PHOTOCRAFT_` as a fallback) |
| Links | getartcraft.com, discord.gg/artcraft, storytold | thabangvision.com/studio, ubunye-ai-ecosystems |
| AppImage update info | `gh-releases-zsync\|storytold\|photocraft\|…` | downloads.thabangvision.com zsync |

## Touchpoints (from the Phase-1 inventory)
- "PhotoCraft": more than 12,000 occurrences, including 16 locale files
- `apps/photocraft/src/main.rs:54` app id; `packaging/macos/Info.plist.in` (UTTypes `.document`, `.ora`, `.pdn`, `.qoi`); WiX UpgradeCode, Manufacturer, ProgIDs `Photocraft.*`; MIME `application/x-photocraft`
- icons in `assets/app-icon/` (kitsune); ArtCraft logos in `docs/brand/`
- 56 `PHOTOCRAFT_*` env vars; nfpm vendor and homepage; AppImage update info (`packaging/linux/package.sh:128`)

## How (keeps upstream merges cheap)
1. **Brand layer.** One module (`crates/ui-egui/src/brand.rs` or a new `ubunye-brand` crate) exposes the name, ids, links and logos, generated from `ubunye/brand.toml` by `build.rs`. Upstream call sites get a one-line indirection. Offer that hook upstream.
2. **Locale overlay.** Load `ubunye/i18n/<lang>.*` after upstream catalogs to override product-name strings, instead of editing upstream locale files.
3. **Packaging overlay.** `ubunye/packaging/` holds templated Info.plist, WiX, nfpm and Flatpak files rendered from `brand.toml`. Our release workflow uses these; upstream `packaging/` stays untouched.
4. **Brand-leak gate.** CI greps for `ArtCraft|getartcraft|ai\.storyteller|Learning Machines|storytold` outside the allow-list (LICENSE*, NOTICE, ATTRIBUTION*, UPSTREAM*, credits). It is informational until M4, then blocking.
