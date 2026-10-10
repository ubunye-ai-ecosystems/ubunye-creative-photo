#!/usr/bin/env python3
"""Re-apply the Ubunye Creative Photo brand to the upstream PhotoCraft tree.

Run from anywhere (Python 3.11+, standard library only):

    python -I ubunye/brand/apply.py            apply every rule, regenerate icons if the art changed
    python -I ubunye/brand/apply.py --icons    also force the icon renders
    python -I ubunye/brand/apply.py --check    change nothing; list brand leaks and stale rules,
                                               exit 1 if there are any

It is idempotent: a second run changes nothing. After an upstream merge, run it, then run
--check, then build and test. Values come from ubunye/brand.toml. See ubunye/REBRANDING.md.

What it does, in order:
  1. Renames upstream files whose names carry the upstream app id (Linux desktop, MIME, AppStream,
     Flatpak manifests).
  2. Applies the id, vendor and link rules (ID_RULES) to code, packaging and CI files.
  3. Applies exact, file-specific rules (EXACT_RULES) for things a pattern cannot express safely.
  4. Replaces the product name "PhotoCraft" with "Ubunye Creative Photo" in user-visible Rust
     string lines (apps/, crates/) and in the 16 translation catalogs, skipping the allow-list
     (KEEP_LINE): credit lines, test fixtures and internal names.
  5. Adds or rewrites the catalog rows for Ubunye's own UI strings (OVERLAY), in every language.
  6. Removes the ArtCraft marks from docs/brand/ (the licence file stays).
  7. Copies ubunye/brand/assets/icon.svg into assets/app-icon/ and renders every derived icon
     (PNG via resvg, .ico and .icns packed here), then records the art's hash.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BRAND_TOML = ROOT / "ubunye" / "brand.toml"
ICON_SRC = ROOT / "ubunye" / "brand" / "assets" / "icon.svg"
ICON_STAMP = ROOT / "ubunye" / "brand" / "icon.sha256"
APP_ICON_DIR = ROOT / "assets" / "app-icon"

with BRAND_TOML.open("rb") as f:
    BRAND = tomllib.load(f)

# THE PRODUCT NAME. Change it here and rerun this script: every user-visible string, installer,
# package, catalog and the Rust constant (crates/ui-egui/src/ubunye.rs) follow, and
# ubunye/brand.toml's `name` is rewritten to match. Put the old name in FORMER_NAMES so the
# rerun can find it.
NAME = "Ubunye Photo"
FORMER_NAMES = ["Ubunye Creative Photo"]
# Parent brand, shown in About ("<name>, part of <suite>.").
SUITE = "Ubunye Creative Studio"
VENDOR = BRAND["product"]["vendor"]  # Ubunye AI Ecosystems
APP_ID = BRAND["identifiers"]["app_id"]  # org.ubunye.creative.photo
UPGRADE_CODE = BRAND["identifiers"]["windows_upgrade_code"]
MSI_COMPONENT_GUID = BRAND["identifiers"]["windows_main_component_guid"]
MIME = BRAND["identifiers"]["mime_type"]  # application/x-ubunye-creative-photo
PROGID = BRAND["identifiers"]["progid_prefix"]  # UbunyeCreativePhoto
DATA_DIR = BRAND["identifiers"]["portable_data_dir"]  # UbunyeCreativePhotoData
HOMEPAGE = BRAND["links"]["homepage"]
SOURCE = BRAND["links"]["source"]
ISSUES = BRAND["links"]["issues"]
COMMUNITY = BRAND["links"]["community"]
UPDATES = BRAND["links"]["appimage_updates"]  # https://downloads.thabangvision.com/photo/stable/
REPO = SOURCE.removeprefix("https://github.com/")
OLD_ID = "ai.storyteller.photocraft"

# ---------------------------------------------------------------------------------------------
# File scopes
# ---------------------------------------------------------------------------------------------

# Files that link to upstream on purpose (the credit): the link rules leave them alone.
CREDIT_FILES = {"crates/ui-egui/src/ubunye.rs", "crates/ui-egui/src/links.rs"}
SKIP_DIRS = {".git", "target", "node_modules", "dist", "corpus", "build-flatpak", ".flatpak-builder"}
# Upstream files we deliberately never touch (signing is out of scope without owner approval).
NEVER_TOUCH = {"packaging/windows/sign.ps1"}


def rel(p: Path) -> str:
    return p.relative_to(ROOT).as_posix()


def walk(*tops: str, suffixes: tuple[str, ...] | None = None) -> list[Path]:
    out = []
    for top in tops:
        base = ROOT / top
        if base.is_file():
            out.append(base)
            continue
        for dirpath, dirnames, filenames in os.walk(base):
            dirnames[:] = sorted(d for d in dirnames if d not in SKIP_DIRS)
            for fn in sorted(filenames):
                p = Path(dirpath) / fn
                if suffixes is None or p.name.endswith(suffixes):
                    out.append(p)
    return [p for p in out if rel(p) not in NEVER_TOUCH]


def rust_files() -> list[Path]:
    return walk("apps", "crates", suffixes=(".rs",))


def tsv_files() -> list[Path]:
    return walk("crates/ui-egui/src/i18n", suffixes=(".tsv",))


TEXT_SUFFIXES = (".rs", ".sh", ".ps1", ".yml", ".yaml", ".xml", ".in", ".desktop", ".wxs", ".wxl", ".txt", ".html", ".md", ".svg", ".plist")


def id_scope() -> list[Path]:
    files = walk("packaging", suffixes=TEXT_SUFFIXES + ("AppRun",))
    files += [p for p in walk("packaging/linux") if p.name == "AppRun"]
    files += rust_files()
    files += walk("apps/photocraft-web/index.html", ".github/workflows/release.yml", ".github/workflows/packaging-lint.yml")
    # The DMG artwork is an open item (see REBRANDING.md); its README is upstream documentation.
    files = [p for p in files if not rel(p).startswith("packaging/macos/dmg/") and rel(p) not in CREDIT_FILES]
    return sorted(set(files))


# ---------------------------------------------------------------------------------------------
# Text helpers (byte-exact: keep each file's line endings and encoding)
# ---------------------------------------------------------------------------------------------


def read(p: Path) -> str | None:
    try:
        return p.read_bytes().decode("utf-8")
    except (UnicodeDecodeError, FileNotFoundError):
        return None


def write(p: Path, text: str) -> None:
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_bytes(text.encode("utf-8"))


def eol(text: str) -> str:
    return "\r\n" if "\r\n" in text else "\n"


# ---------------------------------------------------------------------------------------------
# 1. Renames
# ---------------------------------------------------------------------------------------------

RENAMES = [
    (f"packaging/linux/{OLD_ID}.desktop", f"packaging/linux/{APP_ID}.desktop"),
    (f"packaging/linux/{OLD_ID}.metainfo.xml.in", f"packaging/linux/{APP_ID}.metainfo.xml.in"),
    (f"packaging/linux/{OLD_ID}.mime.xml", f"packaging/linux/{APP_ID}.mime.xml"),
    (f"packaging/linux/flatpak/{OLD_ID}.yml", f"packaging/linux/flatpak/{APP_ID}.yml"),
    (f"packaging/linux/flatpak/{OLD_ID}.bundle.yml", f"packaging/linux/flatpak/{APP_ID}.bundle.yml"),
]

# ---------------------------------------------------------------------------------------------
# 2. Id, vendor and link rules (regex, replacement), applied to id_scope()
# ---------------------------------------------------------------------------------------------

ID_RULES = [
    (r"ai\\\.storyteller\\\.photocraft", re.escape(APP_ID).replace("\\", "\\\\")),  # inside grep -E patterns
    (r"ai\.storyteller\.photocraft", APP_ID),
    (r'id="ai\.storyteller"', 'id="org.ubunye"'),
    (r"application/x-photocraft\b", MIME),
    (r"\bPhotocraft\.(Document|PhotoshopDocument|Image)\b", PROGID + r".\1"),
    (r"Learning Machines LLC", VENDOR),
    (r"https://getartcraft\.com/apps/photocraft", HOMEPAGE),
    (r"https://discord\.gg/artcraft", COMMUNITY),
    (r"https://github\.com/storytold/photocraft/issues", ISSUES),
    (r"raw\.githubusercontent\.com/storytold/photocraft/", f"raw.githubusercontent.com/{REPO}/"),
    (r"https://github\.com/storytold/photocraft(?![-\w/])", SOURCE),
]

# ---------------------------------------------------------------------------------------------
# 3. Exact rules: (path, old, new). Multi-line strings use \n; CRLF files are matched as CRLF.
# ---------------------------------------------------------------------------------------------

LINUX = "packaging/linux"
WXS = "packaging/windows/photocraft.wxs"
PLIST = "packaging/macos/Info.plist.in"
EXACT_RULES = [
    # Windows resources of both executables.
    ("apps/photocraft/build.rs", '"Copyright (c) the PhotoCraft authors. MIT OR Apache-2.0."', f'"Copyright (c) the PhotoCraft authors and {VENDOR}. MIT OR Apache-2.0."'),
    ("apps/photocraft-cli/build.rs", '"Copyright (c) the PhotoCraft authors. MIT OR Apache-2.0."', f'"Copyright (c) the PhotoCraft authors and {VENDOR}. MIT OR Apache-2.0."'),
    # --help / --version name the product (the binaries keep their upstream names).
    ("apps/photocraft-cli/src/lib.rs", "photocraft-cli: headless Photocraft\n", f"photocraft-cli: headless {NAME} (based on PhotoCraft)\n"),
    ("apps/photocraft-cli/src/lib.rs", 'writeln!(out, "photocraft-cli {}", ', f'writeln!(out, "photocraft-cli {{}} ({NAME}, based on PhotoCraft)", '),
    ("apps/photocraft/src/main.rs", 'println!("photocraft {}", ', f'println!("photocraft {{}} ({NAME}, based on PhotoCraft)", '),
    # Settings live in a directory of our own, so an installed upstream copy never shares them.
    ("apps/photocraft/src/app_dirs.rs", '"Library/Application Support/Photocraft"', f'"Library/Application Support/{APP_ID}"'),
    ("apps/photocraft/src/app_dirs.rs", '"home/Library/Application Support/Photocraft"', f'"home/Library/Application Support/{APP_ID}"'),
    ("apps/photocraft/src/app_dirs.rs", 'PathBuf::from(a).join("Photocraft")', f'PathBuf::from(a).join("{APP_ID}")'),
    ("apps/photocraft/src/app_dirs.rs", 'root.join("appdata").join("Photocraft")', f'root.join("appdata").join("{APP_ID}")'),
    ("apps/photocraft/src/app_dirs.rs", '.map(|c| c.join("photocraft"))', f'.map(|c| c.join("{APP_ID}"))'),
    ("apps/photocraft/src/app_dirs.rs", '"home/.config/photocraft"', f'"home/.config/{APP_ID}"'),
    ("apps/photocraft/src/app_dirs.rs", '"PhotoCraft.portable"', '"UbunyeCreativePhoto.portable"'),
    ("apps/photocraft/src/app_dirs.rs", '"PhotoCraftData"', f'"{DATA_DIR}"'),
    ("apps/photocraft/src/services.rs", "`<exe dir>/PhotoCraftData`", f"`<exe dir>/{DATA_DIR}`"),
    # Help menu, title bar and their tests: upstream's Discord and ArtCraft links.
    ("crates/ui-egui/src/menus.rs", '"Join the ArtCraft Discord…"', '"Join the Community…"'),
    ("crates/ui-egui/src/menus.rs", '("help.artcraftWebsite", "ArtCraft Website",', '("help.artcraftWebsite", "Upstream PhotoCraft Project",'),
    ("crates/ui-egui/src/panels.rs", 'tl!("Discord")', 'tl!("Community")'),
    ("crates/ui-egui/src/panels.rs", 'format!("Join the ArtCraft Discord ({})"', 'format!("Join the Community ({})"'),
    ("crates/ui-egui/src/titlebar.rs", 't == "Discord"', 't == "Community"'),
    # Web build: favicon from our icon.
    ("apps/photocraft-web/index.html", "  <title>PhotoCraft</title>\n", f'  <title>{NAME}</title>\n  <link data-trunk rel="icon" type="image/svg+xml" href="../../assets/app-icon/photocraft.svg" />\n'),
    # Linux.
    (f"{LINUX}/{APP_ID}.desktop", "Name=PhotoCraft\n", f"Name={NAME}\n"),
    (f"{LINUX}/{APP_ID}.desktop", "StartupWMClass=photocraft\n", f"StartupWMClass={APP_ID}\n"),
    (f"{LINUX}/{APP_ID}.mime.xml", "<comment>PhotoCraft document</comment>", f"<comment>{NAME} document</comment>"),
    (f"{LINUX}/{APP_ID}.metainfo.xml.in", "<name>PhotoCraft</name>", f"<name>{NAME}</name>"),
    (f"{LINUX}/{APP_ID}.metainfo.xml.in", "      PhotoCraft is an open-source, native image editor", f"      {NAME} is an open-source, native image editor based on PhotoCraft,"),
    (f"{LINUX}/package.sh", '"${PHOTOCRAFT_MAINTAINER:-PhotoCraft maintainers <photocraft@storyteller.ai>}"', f'"${{PHOTOCRAFT_MAINTAINER:-{VENDOR} <{BRAND["product"]["maintainer_email"]}>}}"'),
    (
        f"{LINUX}/package.sh",
        '  REPO="${GITHUB_REPOSITORY:-storytold/photocraft}"\n  UPDATE_INFO="gh-releases-zsync|${REPO%%/*}|${REPO#*/}|latest|photocraft-*-linux-$ARCH.AppImage.zsync"\n',
        f'  # Ubunye: updates come from our own download host, never from upstream\'s GitHub releases.\n  # Publishing must upload this .zsync (and its AppImage) under the stable name below.\n  UPDATE_INFO="zsync|{UPDATES}ubunye-creative-photo-linux-$ARCH.AppImage.zsync"\n',
    ),
    (".github/workflows/release.yml", "grep -F 'gh-releases-zsync|'", f"grep -F 'zsync|{UPDATES}'"),
    # Windows installer.
    (WXS, 'UpgradeCode="D9660F32-2D0A-4ABE-B73D-17BE25F1542C"', f'UpgradeCode="{UPGRADE_CODE}"'),
    (WXS, 'Guid="A6DE41B7-A80F-4C59-A17E-BB68065BF67E"', f'Guid="{MSI_COMPONENT_GUID}"'),
    (WXS, 'Key="Software\\PhotoCraft"', f'Key="Software\\{VENDOR}\\{NAME}"'),
    (WXS, f'<Property Id="ARPHELPLINK" Value="{COMMUNITY}" />', f'<Property Id="ARPHELPLINK" Value="{ISSUES}" />'),
    # macOS bundle (the executable and .icns keep their upstream file names).
    (PLIST, "<key>CFBundleDisplayName</key>\n  <string>PhotoCraft</string>", f"<key>CFBundleDisplayName</key>\n  <string>{NAME}</string>"),
    (PLIST, "<key>CFBundleName</key>\n  <string>PhotoCraft</string>", f"<key>CFBundleName</key>\n  <string>{NAME}</string>"),
    (PLIST, "<string>Copyright © the PhotoCraft authors. MIT OR Apache-2.0.</string>", f"<string>Copyright © the PhotoCraft authors and {VENDOR}. MIT OR Apache-2.0.</string>"),
    (PLIST, "<string>PhotoCraft Document</string>", f"<string>{NAME} Document</string>"),
    # Portable mode note shipped in the Windows zip.
    ("packaging/windows/portable.txt", "PhotoCraftData", DATA_DIR),
    ("packaging/windows/portable.txt", "%APPDATA%\\Photocraft", f"%APPDATA%\\{APP_ID}"),
]

# Files where every remaining product name in the file is user-visible (after EXACT_RULES).
PRODUCT_FILES = [WXS, "packaging/windows/portable.txt", "apps/photocraft-web/index.html"]

# ---------------------------------------------------------------------------------------------
# 4. Product name in Rust and the catalogs
# ---------------------------------------------------------------------------------------------

PRODUCT = re.compile(r"(?<![A-Za-z0-9_.])PhotoCraft(?![A-Za-z0-9_])")
# Lines (Rust) or catalog sources that keep "PhotoCraft": credits, fixtures and internal names.
KEEP_LINE = re.compile(
    "|".join(
        [
            r"based on PhotoCraft",
            r"Upstream PhotoCraft",
            r"PhotoCraft (authors|contributors|team)\b",
            r"\.PhotoCraft-face-",  # internal font alias
            r"PhotoCraft Studio CMYK",  # test fixture
            r"PhotoCraft projects",  # test fixture (folder name)
            r"PhotoCraft é世界",  # test fixture (type layer text)
            r"PhotoCraft transform preview",  # test fixture
            r"PhotoCraft 2026",  # benchmark text
            r"DEFLATE-compressed PhotoCraft",  # test fixture
        ]
    ),
    re.IGNORECASE,
)


def is_comment(line: str) -> bool:
    s = line.lstrip()
    return s.startswith("//") or s.startswith("/*") or s.startswith("*")


def brand_rust_line(line: str) -> str:
    if is_comment(line) or KEEP_LINE.search(line):
        return line
    return PRODUCT.sub(NAME, line)


def brand_tsv_line(line: str) -> str:
    if line.startswith("#") or "\t" not in line:
        return line
    cols = line.split("\t")
    if len(cols) != 3 or KEEP_LINE.search(cols[1]):
        return line
    return "\t".join(PRODUCT.sub(NAME, c) for c in cols)


# ---------------------------------------------------------------------------------------------
# 5. Catalog overlay: Ubunye's own strings. (old English sources this row replaces, new source,
#    translations). A row with an old source is rewritten in place; otherwise the row is appended.
# ---------------------------------------------------------------------------------------------

LANGS = ["ja", "zh-hans", "zh-hant", "es", "ru", "uk", "cs", "fr", "id", "ko", "pl", "de", "pt-br", "el", "nl", "it"]


def row(*t: str) -> dict[str, str]:
    assert len(t) == len(LANGS)
    return dict(zip(LANGS, t))


OVERLAY = [
    (
        ["Join the ArtCraft Discord…"],
        "Join the Community…",
        row(
            "コミュニティに参加…", "加入社区…", "加入社群…", "Únete a la comunidad…", "Присоединиться к сообществу…",
            "Приєднатися до спільноти…", "Připojit se ke komunitě…", "Rejoindre la communauté…", "Gabung ke komunitas…",
            "커뮤니티 참여…", "Dołącz do społeczności…", "Der Community beitreten…", "Participe da comunidade…",
            "Συμμετοχή στην κοινότητα…", "Deelnemen aan de community…", "Unisciti alla community…",
        ),
    ),
    (
        ["Join us on Discord", "Join the ArtCraft Discord"],
        "Join the Community",
        row(
            "コミュニティに参加", "加入社区", "加入社群", "Únete a la comunidad", "Присоединяйтесь к сообществу",
            "Приєднуйтеся до спільноти", "Přidejte se ke komunitě", "Rejoignez la communauté", "Gabung ke komunitas",
            "커뮤니티 참여", "Dołącz do społeczności", "Tritt der Community bei", "Participe da comunidade",
            "Ελάτε στην κοινότητα", "Doe mee met de community", "Unisciti alla community",
        ),
    ),
    (
        ["Discord"],
        "Community",
        row(
            "コミュニティ", "社区", "社群", "Comunidad", "Сообщество", "Спільнота", "Komunita", "Communauté", "Komunitas",
            "커뮤니티", "Społeczność", "Community", "Comunidade", "Κοινότητα", "Community", "Community",
        ),
    ),
    (
        ["ArtCraft Website", "ArtCraft"],
        "Upstream PhotoCraft Project",
        row(
            "アップストリームの PhotoCraft プロジェクト", "上游 PhotoCraft 项目", "上游 PhotoCraft 專案", "Proyecto PhotoCraft original",
            "Исходный проект PhotoCraft", "Першоджерельний проєкт PhotoCraft", "Původní projekt PhotoCraft", "Projet PhotoCraft d'origine",
            "Proyek PhotoCraft asal", "업스트림 PhotoCraft 프로젝트", "Projekt źródłowy PhotoCraft", "Ursprüngliches PhotoCraft-Projekt",
            "Projeto PhotoCraft original", "Αρχικό έργο PhotoCraft", "Oorspronkelijk PhotoCraft-project", "Progetto PhotoCraft originale",
        ),
    ),
    (
        [],
        "Based on PhotoCraft by the ArtCraft team",
        row(
            "ArtCraft チームの PhotoCraft をベースにしています", "基于 ArtCraft 团队的 PhotoCraft", "基於 ArtCraft 團隊的 PhotoCraft",
            "Basado en PhotoCraft del equipo de ArtCraft", "Основано на PhotoCraft от команды ArtCraft", "Створено на основі PhotoCraft від команди ArtCraft",
            "Založeno na PhotoCraft od týmu ArtCraft", "Basé sur PhotoCraft de l'équipe ArtCraft", "Berdasarkan PhotoCraft oleh tim ArtCraft",
            "ArtCraft 팀의 PhotoCraft 기반", "Oparty na PhotoCraft zespołu ArtCraft", "Basiert auf PhotoCraft vom ArtCraft-Team",
            "Baseado no PhotoCraft da equipe ArtCraft", "Βασίζεται στο PhotoCraft της ομάδας ArtCraft", "Gebaseerd op PhotoCraft van het ArtCraft-team",
            "Basato su PhotoCraft del team ArtCraft",
        ),
    ),
    (
        [],
        "{name}, part of {suite}.",
        row(
            "{name}（{suite} の一部）。", "{name}，{suite} 的一部分。", "{name}，{suite} 的一部分。", "{name}, parte de {suite}.",
            "{name}, часть {suite}.", "{name}, частина {suite}.", "{name}, součást {suite}.", "{name}, membre de {suite}.",
            "{name}, bagian dari {suite}.", "{name}, {suite}의 일부.", "{name}, część {suite}.", "{name}, Teil von {suite}.",
            "{name}, parte do {suite}.", "{name}, μέρος του {suite}.", "{name}, onderdeel van {suite}.", "{name}, parte di {suite}.",
        ),
    ),
    (
        [],
        "License: MIT OR Apache-2.0",
        row(
            "ライセンス: MIT OR Apache-2.0", "许可证：MIT OR Apache-2.0", "授權條款：MIT OR Apache-2.0", "Licencia: MIT OR Apache-2.0",
            "Лицензия: MIT OR Apache-2.0", "Ліцензія: MIT OR Apache-2.0", "Licence: MIT OR Apache-2.0", "Licence : MIT OR Apache-2.0",
            "Lisensi: MIT OR Apache-2.0", "라이선스: MIT OR Apache-2.0", "Licencja: MIT OR Apache-2.0", "Lizenz: MIT OR Apache-2.0",
            "Licença: MIT OR Apache-2.0", "Άδεια: MIT OR Apache-2.0", "Licentie: MIT OR Apache-2.0", "Licenza: MIT OR Apache-2.0",
        ),
    ),
]


def former_name_scope() -> list[Path]:
    """Every file this script writes the product name into."""
    files = set(id_scope()) | set(rust_files()) | set(tsv_files()) | {ROOT / p for p in PRODUCT_FILES}
    files |= {ROOT / p for p, _, _ in EXACT_RULES}
    files |= {APP_ICON_DIR / "LICENSE.txt", APP_ICON_DIR / "README.md", ROOT / "README.md", ROOT / "UPSTREAM.md"}
    files |= set(walk("ubunye", suffixes=(".md",))) - {ROOT / "ubunye/CLAUDE.md", ROOT / "ubunye/REBRANDING.md"}
    return sorted(files)


# (file, regex, replacement): the name and suite constants outside this script.
SYNC_CONSTANTS = [
    (BRAND_TOML, r'^name = "[^"\r\n]*"', f'name = "{NAME}"'),
    (BRAND_TOML, r'^suite = "[^"\r\n]*"', f'suite = "{SUITE}"'),
    (ROOT / "crates/ui-egui/src/ubunye.rs", r'^pub const NAME: &str = "[^"\r\n]*";', f'pub const NAME: &str = "{NAME}";'),
    (ROOT / "crates/ui-egui/src/ubunye.rs", r'^pub const SUITE: &str = "[^"\r\n]*";', f'pub const SUITE: &str = "{SUITE}";'),
]


def apply_overlay(text: str, lang: str) -> str:
    nl = eol(text)
    lines = text.split(nl)
    for olds, new, tr in OVERLAY:
        want = f"\t{new}\t{tr[lang]}"
        found = False
        for i, line in enumerate(lines):
            cols = line.split("\t")
            if len(cols) == 3 and cols[0] == "" and (cols[1] in olds or cols[1] == new):
                if found:
                    lines[i] = None  # a second row for the same source would be a duplicate
                else:
                    lines[i] = want
                    found = True
        lines = [line for line in lines if line is not None]
        if not found:
            at = len(lines) - 1 if lines and lines[-1] == "" else len(lines)
            lines.insert(at, want)
    return nl.join(lines)


# ---------------------------------------------------------------------------------------------
# 6/7. Marks and icons
# ---------------------------------------------------------------------------------------------

ARTCRAFT_MARKS = sorted((ROOT / "docs" / "brand").glob("artcraft-*"))

ICON_LICENSE = f"""{NAME} app icon (photocraft.svg, photocraft-small.svg and every file derived from them in
this directory: photocraft-1024.png, photocraft.icns, photocraft.ico, hicolor/).

{VENDOR} artwork adapted from the ThabangVision logo, MIT OR Apache-2.0; letter outline from
DM Serif Display (SIL OFL 1.1).

Licensed under either of

  * Apache License, Version 2.0 (https://www.apache.org/licenses/LICENSE-2.0)
  * MIT license (https://opensource.org/licenses/MIT)

at your option.

The master is ubunye/brand/assets/icon.svg. Regenerate the files here with
python -I ubunye/brand/apply.py --icons (or packaging/icons.sh).
"""

ICON_README = f"""# {NAME} app icon

{VENDOR} artwork adapted from the ThabangVision logo; the letters are an outline from DM Serif
Display (SIL OFL 1.1), so rendering needs no font. Licence: see `LICENSE.txt`.

This replaces upstream PhotoCraft's kitsune icon. The master lives at
`ubunye/brand/assets/icon.svg`; `python -I ubunye/brand/apply.py` copies it here and renders the rest.

## Geometry

A 512-unit tile (`viewBox="0 0 512 512"`), rounded square with `rx=112`. The macOS render pads it
onto Apple's 824/1024 icon grid; the Windows and Linux renders crop 22 units off each side so the
art reads at 16 to 48 px (the same recipe as `packaging/icons.sh`).

## Files

- `photocraft.svg`: copy of the master. `photocraft-small.svg`: the same art (the hicolor scalable icon).
- `photocraft-1024.png`: 1024 px render on the macOS grid.
- `photocraft.icns`: macOS bundle icon (`CFBundleIconFile`).
- `photocraft.ico`: Windows icon, 16 to 256 px, embedded in the `.exe` by `apps/photocraft/build.rs`.
- `hicolor/<size>/apps/{APP_ID}.png` (16 to 512) and `hicolor/scalable/apps/{APP_ID}.svg`: Linux icon theme.

The file names `photocraft.*` stay as upstream has them, so build scripts need no change.
"""

HICOLOR = [16, 24, 32, 48, 64, 128, 256, 512]
ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]
# icns element types that hold PNG data, and the pixel size of each.
ICNS_TYPES = [(b"icp4", 16), (b"icp5", 32), (b"icp6", 64), (b"ic07", 128), (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024), (b"ic11", 32), (b"ic12", 64), (b"ic13", 256), (b"ic14", 512)]


def find_resvg() -> str | None:
    for cand in [os.environ.get("RESVG"), shutil.which("resvg"), "T:/rust/cargo/bin/resvg.exe"]:
        if cand and Path(cand).is_file():
            return cand
    return None


def icon_hash() -> str:
    # Line endings normalised, so the stamp matches on Windows (autocrlf) and Linux checkouts.
    return hashlib.sha256(ICON_SRC.read_bytes().replace(b"\r\n", b"\n")).hexdigest()


def render_icons(log: list[str]) -> None:
    resvg = find_resvg()
    if not resvg:
        sys.exit("error: resvg not found (set RESVG or put it on PATH); icons not rendered")
    svg = ICON_SRC.read_text(encoding="utf-8")
    if 'viewBox="0 0 512 512"' not in svg:
        sys.exit(f'error: expected viewBox="0 0 512 512" in {ICON_SRC}')
    with tempfile.TemporaryDirectory() as tmp:
        t = Path(tmp)
        mac = t / "mac.svg"
        mac.write_text(svg.replace('viewBox="0 0 512 512"', 'viewBox="-62 -62 636 636"', 1), encoding="utf-8")
        tight = t / "tight.svg"
        tight.write_text(svg.replace('viewBox="0 0 512 512"', 'viewBox="22 22 468 468"', 1), encoding="utf-8")

        def render(src: Path, size: int, out: Path) -> bytes:
            out.parent.mkdir(parents=True, exist_ok=True)
            subprocess.run([resvg, "--skip-system-fonts", "-w", str(size), "-h", str(size), str(src), str(out)], check=True, stdin=subprocess.DEVNULL)
            return out.read_bytes()

        shutil.copyfile(ICON_SRC, APP_ICON_DIR / "photocraft.svg")
        shutil.copyfile(ICON_SRC, APP_ICON_DIR / "photocraft-small.svg")
        render(mac, 1024, APP_ICON_DIR / "photocraft-1024.png")
        for s in HICOLOR:
            render(tight, s, APP_ICON_DIR / "hicolor" / f"{s}x{s}" / "apps" / f"{APP_ID}.png")
        scalable = APP_ICON_DIR / "hicolor" / "scalable" / "apps" / f"{APP_ID}.svg"
        scalable.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ICON_SRC, scalable)
        ico = [(s, render(tight, s, t / f"ico-{s}.png")) for s in ICO_SIZES]
        (APP_ICON_DIR / "photocraft.ico").write_bytes(pack_ico(ico))
        icns = [(code, render(mac, s, t / f"icns-{code.decode()}.png")) for code, s in ICNS_TYPES]
        (APP_ICON_DIR / "photocraft.icns").write_bytes(pack_icns(icns))
    ICON_STAMP.write_text(icon_hash() + "\n", encoding="utf-8")
    log.append("rendered icons from ubunye/brand/assets/icon.svg")


def pack_ico(images: list[tuple[int, bytes]]) -> bytes:
    """An .ico with PNG entries (Windows Vista and later read PNG at every size)."""
    head = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    dirs, data = b"", b""
    for size, png in images:
        dim = 0 if size >= 256 else size
        dirs += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(png), offset + len(data))
        data += png
    return head + dirs + data


def pack_icns(images: list[tuple[bytes, bytes]]) -> bytes:
    body = b"".join(code + struct.pack(">I", 8 + len(png)) + png for code, png in images)
    return b"icns" + struct.pack(">I", 8 + len(body)) + body


def old_icon_files() -> list[Path]:
    return sorted(APP_ICON_DIR.glob(f"hicolor/*/apps/{OLD_ID}.*"))


# ---------------------------------------------------------------------------------------------
# Apply
# ---------------------------------------------------------------------------------------------


def exact(text: str, old: str, new: str) -> tuple[str, str]:
    """Apply one exact rule. Returns (text, state) with state 'applied', 'ok' or 'stale'."""
    nl = eol(text)
    old, new = old.replace("\n", nl), new.replace("\n", nl)
    if old == new:
        return text, ("ok" if old in text else "stale")
    if old in text:
        return text.replace(old, new), "applied"
    return text, ("ok" if new in text else "stale")


def git_mv(old: Path, new: Path) -> None:
    r = subprocess.run(["git", "-C", str(ROOT), "mv", rel(old), rel(new)], capture_output=True, text=True)
    if r.returncode != 0:
        new.parent.mkdir(parents=True, exist_ok=True)
        old.rename(new)


def apply(force_icons: bool) -> int:
    log: list[str] = []
    stale: list[str] = []
    for old, new in RENAMES:
        o, n = ROOT / old, ROOT / new
        if o.exists() and n.exists():
            stale.append(f"both {old} and {new} exist: merge upstream's changes into {new}, then delete {old}")
        elif o.exists():
            git_mv(o, n)
            log.append(f"renamed {old} -> {new}")
    for p in id_scope():
        text = read(p)
        if text is None:
            continue
        out = text
        for pat, repl in ID_RULES:
            out = re.sub(pat, repl, out)
        if out != text:
            write(p, out)
            log.append(f"ids/links: {rel(p)}")
    for p in former_name_scope():
        text = read(p)
        if text is None:
            continue
        out = text
        for old in FORMER_NAMES:
            out = out.replace(old, NAME)
        if out != text:
            write(p, out)
            log.append(f"renamed product: {rel(p)}")
    for p, pat, repl in SYNC_CONSTANTS:
        text = read(p) or ""
        out = re.sub(pat, repl, text, count=1, flags=re.M)
        if out != text:
            write(p, out)
            log.append(f"synced name/suite: {rel(p)}")
    for path, old, new in EXACT_RULES:
        p = ROOT / path
        text = read(p)
        if text is None:
            stale.append(f"{path}: missing")
            continue
        out, state = exact(text, old, new)
        if state == "stale":
            stale.append(f"{path}: rule no longer matches: {old[:70]!r}")
        if out != text:
            write(p, out)
            log.append(f"exact: {path}")
    for path in PRODUCT_FILES:
        p = ROOT / path
        text = read(p)
        if text is None:
            continue
        out = PRODUCT.sub(NAME, text)
        if out != text:
            write(p, out)
            log.append(f"product name: {path}")
    for p in rust_files():
        text = read(p)
        if text is None or "PhotoCraft" not in text:
            continue
        nl = eol(text)
        out = nl.join(brand_rust_line(line) for line in text.split(nl))
        if out != text:
            write(p, out)
            log.append(f"product name: {rel(p)}")
    for p in tsv_files():
        text = read(p)
        if text is None:
            continue
        nl = eol(text)
        out = nl.join(brand_tsv_line(line) for line in text.split(nl))
        out = apply_overlay(out, p.stem)
        if out != text:
            write(p, out)
            log.append(f"catalog: {rel(p)}")
    for mark in ARTCRAFT_MARKS:
        if mark.exists():
            mark.unlink()
            log.append(f"removed {rel(mark)}")
    for p, body in [(APP_ICON_DIR / "LICENSE.txt", ICON_LICENSE), (APP_ICON_DIR / "README.md", ICON_README)]:
        cur = read(p)
        want = body.replace("\n", eol(cur)) if cur else body
        if cur != want:
            write(p, want)
            log.append(f"wrote {rel(p)}")
    for f in old_icon_files():
        f.unlink()
        log.append(f"removed {rel(f)}")
    stamp = ICON_STAMP.read_text(encoding="utf-8").strip() if ICON_STAMP.exists() else ""
    if force_icons or stamp != icon_hash():
        render_icons(log)
    print("\n".join(log) if log else "nothing to change")
    if stale:
        print("\nSTALE RULES (update ubunye/brand/apply.py):\n  " + "\n  ".join(stale))
        return 1
    return 0


# ---------------------------------------------------------------------------------------------
# Check
# ---------------------------------------------------------------------------------------------

LEAK = re.compile(r"getartcraft|discord\.gg/artcraft|ai\.storyteller|Learning Machines|storytold/photocraft(?!-corpus)|artcraft-(logo|mark)")
# Files where these leaks are expected: licences, notices, upstream history and documentation.
LEAK_ALLOWED_FILES = CREDIT_FILES
README_MARKER = "<!-- ubunye: upstream README below -->"


def leak_scope() -> list[Path]:
    files = walk("apps", "crates", "packaging", "assets/app-icon", suffixes=TEXT_SUFFIXES + (".tsv", ".toml"))
    files += [p for p in walk("packaging/linux") if p.name == "AppRun"]
    files += walk(".github/workflows/release.yml", ".github/workflows/packaging-lint.yml", "Dockerfile")
    return sorted({p for p in files if not rel(p).startswith("packaging/macos/dmg/")})


def check() -> int:
    problems: list[str] = []
    for mark in ARTCRAFT_MARKS:
        problems.append(f"ArtCraft mark still present: {rel(mark)}")
    for p in leak_scope():
        text = read(p)
        if text is None:
            continue
        r = rel(p)
        for n, line in enumerate(text.splitlines(), 1):
            code = p.suffix == ".rs"
            if code and is_comment(line):
                continue
            if p.suffix == ".tsv" and line.startswith("#"):
                continue
            if LEAK.search(line) and r not in LEAK_ALLOWED_FILES:
                if "storytold/photocraft" in line and re.search(r"(?i)upstream|based on", line):
                    continue
                problems.append(f"{r}:{n}: {line.strip()[:120]}")
            if code and PRODUCT.search(line) and not KEEP_LINE.search(line):
                problems.append(f"{r}:{n}: product name: {line.strip()[:120]}")
            if code and "ArtCraft" in line and not re.search(r"ArtCraft team|ARTCRAFT_WEBSITE|artcraftWebsite", line):
                problems.append(f"{r}:{n}: ArtCraft: {line.strip()[:120]}")
            if p.suffix == ".tsv" and "\t" in line:
                cols = line.split("\t")
                if len(cols) == 3 and not KEEP_LINE.search(cols[1]):
                    if PRODUCT.search(line):
                        problems.append(f"{r}:{n}: product name: {line.strip()[:120]}")
                    if "ArtCraft" in line or "Discord" in line:
                        problems.append(f"{r}:{n}: upstream community: {line.strip()[:120]}")
    for path in PRODUCT_FILES + [f"{LINUX}/{APP_ID}.desktop", f"{LINUX}/{APP_ID}.mime.xml"]:
        text = read(ROOT / path) or ""
        for n, line in enumerate(text.splitlines(), 1):
            if PRODUCT.search(line) and not KEEP_LINE.search(line) and not line.lstrip().startswith(("#", "<!--")):
                problems.append(f"{path}:{n}: product name: {line.strip()[:120]}")
    readme = read(ROOT / "README.md") or ""
    head = readme.split(README_MARKER)[0]
    if README_MARKER not in readme:
        problems.append("README.md: the Ubunye header (and its marker) is missing")
    if re.search(r"artcraft-(logo|mark)|getartcraft|discord\.gg", head):
        problems.append("README.md: the Ubunye header mentions upstream marks or links")
    for old, new in RENAMES:
        if (ROOT / old).exists():
            problems.append(f"not renamed: {old}")
    for path, old, new in EXACT_RULES:
        text = read(ROOT / path)
        if text is None:
            problems.append(f"{path}: missing")
            continue
        nl = eol(text)
        o, w = old.replace("\n", nl), new.replace("\n", nl)
        if o != w and o in text:
            problems.append(f"{path}: rule not applied: {old[:70]!r}")
        elif w not in text:
            problems.append(f"{path}: rule stale (neither text found): {old[:70]!r}")
    for p in tsv_files():
        text = read(p) or ""
        if apply_overlay(text, p.stem) != text:
            problems.append(f"{rel(p)}: Ubunye catalog rows missing or out of date")
    for p in former_name_scope():
        text = read(p) or ""
        for old in FORMER_NAMES:
            if old in text:
                problems.append(f"{rel(p)}: former product name {old!r} (rerun apply.py)")
    for f in old_icon_files():
        problems.append(f"upstream icon file still present: {rel(f)}")
    stamp = ICON_STAMP.read_text(encoding="utf-8").strip() if ICON_STAMP.exists() else ""
    if stamp != icon_hash():
        problems.append("icons are out of date with ubunye/brand/assets/icon.svg (run apply.py --icons)")
    if (APP_ICON_DIR / "photocraft.svg").read_bytes().replace(b"\r\n", b"\n") != ICON_SRC.read_bytes().replace(b"\r\n", b"\n"):
        problems.append("assets/app-icon/photocraft.svg differs from ubunye/brand/assets/icon.svg")
    for p, body in [(APP_ICON_DIR / "LICENSE.txt", ICON_LICENSE), (APP_ICON_DIR / "README.md", ICON_README)]:
        if (read(p) or "").replace("\r\n", "\n") != body:
            problems.append(f"{rel(p)} is not the Ubunye text")
    rs = read(ROOT / "crates/ui-egui/src/ubunye.rs") or ""
    for const, value in [("NAME", NAME), ("SUITE", SUITE), ("APP_ID", APP_ID), ("VENDOR", VENDOR), ("PRODUCT_PAGE", HOMEPAGE), ("SOURCE", SOURCE), ("ISSUES", ISSUES), ("COMMUNITY", COMMUNITY)]:
        if f'pub const {const}: &str = "{value}";' not in rs:
            problems.append(f"crates/ui-egui/src/ubunye.rs: {const} is not {value!r} (brand.toml)")
    if problems:
        print(f"{len(problems)} brand problem(s):")
        print("  " + "\n  ".join(problems))
        return 1
    print("brand check passed: no leaks, every rule applied, icons current")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--check", action="store_true", help="report leaks and stale rules; change nothing")
    ap.add_argument("--icons", action="store_true", help="re-render the icons even if the art is unchanged")
    a = ap.parse_args()
    if sys.stdout.encoding.lower() != "utf-8":
        sys.stdout.reconfigure(encoding="utf-8")
    return check() if a.check else apply(a.icons)


if __name__ == "__main__":
    sys.exit(main())
