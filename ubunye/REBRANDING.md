# Rebranding PhotoCraft to Ubunye Photo

Status: **applied** (branch `ubunye/rebrand`, October 2026). `python -I ubunye/brand/apply.py --check`
passes and runs as a blocking job in `.github/workflows/ubunye-ci.yml`.

Ubunye Photo is part of Ubunye Creative Studio, by Ubunye AI Ecosystems, distributed by
ThabangVision. It is based on PhotoCraft by the ArtCraft team (https://github.com/storytold/photocraft),
used under MIT OR Apache-2.0.

## Identity

| Item | Upstream | Ubunye |
|---|---|---|
| Product name | PhotoCraft | Ubunye Photo (`NAME` in `ubunye/brand/apply.py`) |
| Parent brand | ArtCraft | Ubunye Creative Studio |
| App / bundle / Flatpak id | `ai.storyteller.photocraft` | `org.ubunye.creative.photo` |
| Vendor / manufacturer | Learning Machines LLC | Ubunye AI Ecosystems |
| Windows MSI UpgradeCode | `D9660F32-...` | `B5F26DFD-A4A7-4D46-8DDF-79537F0A829B` (studio-tools `brand/identities.toml`) |
| MSI main component GUID | `A6DE41B7-...` | `BE899921-351E-4A2D-AB58-1758C5DCB870` |
| MIME type of `.pcraft` | `application/x-photocraft` | `application/x-ubunye-creative-photo` |
| macOS UTIs | `ai.storyteller.photocraft.*` | `org.ubunye.creative.photo.*` |
| Windows ProgIDs | `Photocraft.*` | `UbunyeCreativePhoto.*` |
| Settings folder | `Photocraft` / `photocraft` | `org.ubunye.creative.photo` on every platform; portable `UbunyeCreativePhotoData` |
| Links | getartcraft.com, discord.gg/artcraft, storytold issues | https://www.thabangvision.com/tools/photo, our repo, issues and discussions |
| AppImage updates | `gh-releases-zsync` on storytold releases | `zsync|https://downloads.thabangvision.com/photo/stable/ubunye-creative-photo-linux-<arch>.AppImage.zsync` |
| Icon | kitsune | `ubunye/brand/assets/icon.svg` |

## What changed

- **Brand module.** `crates/ui-egui/src/ubunye.rs` (Ubunye's own file) holds the name, suite, ids
  and links, plus the About credit block. `links.rs` points its constants at it.
- **UI text, all 16 locales.** Every user-visible "PhotoCraft" in Rust strings (window title, menus,
  macOS app menu, dialogs, tooltips, errors, start screen, MCP server instructions) now reads
  "Ubunye Photo". The catalogs are rewritten by the same rule, so English keys and translations stay
  in step. Upstream's Discord button and menu item became "Join the Community" (our GitHub
  discussions); "ArtCraft Website" became "Upstream PhotoCraft Project" (a credit link). New strings
  have translations in every language (the `OVERLAY` table in `apply.py`).
- **About.** Shows "Ubunye Photo", the version, "Ubunye Photo, part of Ubunye Creative Studio.",
  "Based on PhotoCraft by the ArtCraft team" (plain text, links to the upstream repo) and
  "License: MIT OR Apache-2.0". The contributor and model credits are unchanged.
- **CLI.** `--help` and `--version` name Ubunye Photo and say it is based on PhotoCraft.
- **ArtCraft marks.** The logo files in `docs/brand/` are deleted (`LICENSE-brand.txt` stays). The
  app never compiled them in; the README's logo images are gone.
- **Icons.** Every file in `assets/app-icon/` is rendered from `ubunye/brand/assets/icon.svg`:
  `photocraft.svg`, `photocraft-small.svg`, `photocraft-1024.png` (macOS grid), `photocraft.ico`,
  `photocraft.icns`, `hicolor/<size>/apps/org.ubunye.creative.photo.png` (16 to 512) and the scalable
  SVG. `LICENSE.txt` and `README.md` there are ours.
- **Packaging.** Linux `.desktop`, MIME and AppStream files and both Flatpak manifests are renamed to
  the new id and carry our name, vendor and links. The WiX installer has our name, Manufacturer,
  UpgradeCode, component GUID, registry key (`Software\Ubunye AI Ecosystems\Ubunye Photo`), ProgIDs
  and ARP links. `Info.plist` has our display name, bundle id and UTIs. nfpm has our vendor and
  homepage; the deb/rpm maintainer default is in `package.sh`.
- **Web build.** Title, loading text and a favicon (our icon).
- **Notices.** NOTICE and ATTRIBUTION.md each gained an appended "Ubunye Photo distribution" section;
  nothing above it changed. README.md has an Ubunye header with the credit; upstream's README follows
  below a marker.

## Deliberately kept

- **Adobe and other vendors' names** (Photoshop, PSD/PSB, Lightroom, Camera Raw, Affinity, Paint.NET,
  Krita, GIMP): untouched, by owner decision.
- **Crate and binary names** (`photocraft`, `photocraft-cli`, `photocraft-*` crates), Rust
  identifiers, the `PHOTOCRAFT_*` environment variables, MCP tool names, command ids (even
  `help.discord` and `help.artcraftWebsite`): renaming them would conflict with nearly every upstream
  merge and break scripts and agents.
- **The `.pcraft` extension and format magic**; Ubunye Photo opens and saves `.pcraft` files exactly
  as PhotoCraft does. Only the MIME/UTI/ProgID names around it moved to our namespace.
- **Icon file names** `photocraft.*` in `assets/app-icon/` (only their content changed), so build
  scripts and `build.rs` need no change.
- **Test fixtures, golden files and internal names** that contain "PhotoCraft" (the `KEEP_LINE`
  allow-list in `apply.py`), the ICC profile descriptions "(Photocraft)" and the `.cube` LUT header,
  which are file metadata other tools match on.
- **Upstream documentation**: `book/`, `docs/`, `AGENTS.md` and the upstream half of `README.md`
  still describe PhotoCraft; they are upstream's and merge daily.
- **Code comments** that say PhotoCraft: not user-visible.

## Open items

- **macOS bundle and DMG.** The bundle is still `PhotoCraft.app` with executable `PhotoCraft` and
  `PhotoCraft.icns`, and the DMG background (`packaging/macos/dmg/background.tiff`, outlined text and
  the kitsune) and Finder layout (`dmg-layout.DS_Store`, volume name `PhotoCraft`) are upstream's.
  Regenerating them needs `ds_store`/`mac_alias` and new outlined artwork; do it with the final icon.
- **Windows signing** (`packaging/windows/sign.ps1` still signs with description "PhotoCraft" and the
  storytold URL): release signing needs owner approval (ubunye/CLAUDE.md rule 8).
- **Shared names with an installed upstream copy:** both apps install `photocraft.exe` and register
  `Applications\photocraft.exe` and App Paths on Windows, and `/usr/bin/photocraft` on Linux. Settings,
  ids, ProgIDs, MIME and UTI no longer collide; the binary names do until we rename binaries.
- **Release artefact names** stay `photocraft-<version>-...`. Publishing to
  `downloads.thabangvision.com/photo/stable/` must also upload the AppImage `.zsync` under the stable
  name above, or AppImage delta updates find nothing.
- **Community link** points at GitHub Discussions on our repo; enable Discussions there, or change
  `community` in `ubunye/brand.toml` and `COMMUNITY` in `crates/ui-egui/src/ubunye.rs`.
- **Maintainer e-mail** `maintainers@thabangvision.com` in `brand.toml` is a placeholder.
- **Final icon.** The art will change once more: replace `ubunye/brand/assets/icon.svg` and run
  `python -I ubunye/brand/apply.py`.

## How it works: `ubunye/brand/apply.py`

Python 3.11+, standard library only, run with `python -I`. It is idempotent and keeps each file's
line endings. In order it: renames files that carry the upstream id; applies id, vendor and link
rules (`ID_RULES`); applies file-specific exact rules (`EXACT_RULES`); replaces the product name in
Rust string lines under `apps/` and `crates/` and in the catalogs, skipping the `KEEP_LINE`
allow-list; writes Ubunye's catalog rows (`OVERLAY`); removes the ArtCraft logo files; writes the icon
licence and README; and, when `icon.svg` changed (or with `--icons`), renders every icon (resvg for
PNG; `.ico` and `.icns` are packed by the script). It also writes the product name into
`ubunye/brand.toml` and `crates/ui-egui/src/ubunye.rs`.

**To rename the product:** edit `NAME` (and `SUITE`) at the top of `apply.py`, add the old name to
`FORMER_NAMES`, run the script, then build and test.

`--check` changes nothing. It fails on ArtCraft logo files; `getartcraft`, `discord.gg/artcraft`,
`ai.storyteller`, `Learning Machines`, `storytold/photocraft` or ArtCraft logo paths in code,
packaging, catalogs or CI (credit lines allowed); "PhotoCraft" in a user-visible Rust string or
catalog row outside the allow-list; a former product name; an exact rule that no longer matches
(upstream changed that text); missing catalog rows; stale icons; and brand constants that drift
from `apply.py`/`brand.toml`.

## After an upstream sync

1. Merge upstream into `integration/upstream` as usual. If upstream edited a renamed file
   (for example `packaging/linux/ai.storyteller.photocraft.desktop`), git usually follows the rename;
   if both names exist, move upstream's change into the new name and delete the old file.
2. `python -I ubunye/brand/apply.py`, then `python -I ubunye/brand/apply.py --check`.
3. If `--check` reports a stale rule, look at what upstream changed and update that rule in
   `apply.py`. If it reports a new "PhotoCraft" string, it is usually user-visible (apply.py then
   rebrands it on the next run; nothing to do) or a fixture (add it to `KEEP_LINE`). A new tl! string
   that apply.py renamed gets its catalog rows renamed too.
4. `cargo test -p photocraft-ui-egui -p photocraft` (and any crate the script touched), then commit
   the script's changes in their own commit.

Hand edits the script does not make are listed in `ubunye/patched-paths.txt`.
