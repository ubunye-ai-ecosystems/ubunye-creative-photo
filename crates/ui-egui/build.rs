//! Bake the contributor credits into the binary as static Rust tables, and embed the craft-i18n
//! interface catalogs. Nothing is read at run time.
//!
//! - contributors/contributors.json: commit stats per GitHub username, with a mark for whose people.toml
//!   entry is verified.
//! - contributors/people.toml: the names people chose for themselves. A name is used only when the
//!   JSON says the entry was verified (`people_entry`).
//!
//! A missing or malformed file gives empty tables (or no names) and a warning.
//!
//! - `CRAFT_I18N_DIR`, a craft-i18n checkout (storytold/craft-i18n, at the SHA in
//!   `craft-i18n.rev`): `craft_i18n.rs` holds `CRAFT_I18N`, one (locale, shared catalog, PhotoCraft
//!   catalog) entry per locale that has a `photocraft.tsv`, embedded raw. Unset, or not a checkout
//!   (a warning), it is empty and the interface is English only; with `CRAFT_I18N_REQUIRED=1` (CI
//!   and releases) that is an error instead. Recipe: craft-i18n's `docs/integration.md`.

use std::fmt::Write as _;

use serde_json::Value;

fn main() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contributors");
    let json = read(&dir.join("contributors.json"), "the About window shows no contributors", |t| serde_json::from_str::<Value>(t).map_err(|e| e.to_string()))
        .unwrap_or(Value::Null);
    let people =
        read(&dir.join("people.toml"), "contributors are shown by username only", |t| t.parse::<toml::Table>().map_err(|e| e.to_string())).unwrap_or_default();
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("credits.rs");
    if let Err(e) = std::fs::write(&out, generate(&json, &people)) {
        println!("cargo::warning=writing {}: {e}", out.display());
    }
    let out = out.with_file_name("craft_i18n.rs");
    if let Err(e) = std::fs::write(&out, craft_i18n()) {
        println!("cargo::error=writing {}: {e}", out.display());
    }
}

/// The catalog this app reads in each locale: `locales/<locale>/photocraft.tsv`.
const APP_CATALOG: &str = "photocraft.tsv";

/// `CRAFT_I18N` from `CRAFT_I18N_DIR`: the `Embedded` list the craft-i18n crate reads.
fn craft_i18n() -> String {
    println!("cargo::rerun-if-env-changed=CRAFT_I18N_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_I18N_REQUIRED");
    let required = std::env::var("CRAFT_I18N_REQUIRED").is_ok_and(|v| v.trim() == "1");
    let entries = match std::env::var_os("CRAFT_I18N_DIR").filter(|d| !d.is_empty()) {
        // Not canonicalized: on Windows that gives `\\?\C:\…`, which `include_str!` can't read.
        Some(dir) => match locales(&std::path::PathBuf::from(dir).join("locales")) {
            Ok(entries) => entries,
            Err(e) => {
                let level = if required { "error" } else { "warning" };
                println!("cargo::{level}=CRAFT_I18N_DIR: {e}; the interface is English only");
                Vec::new()
            }
        },
        None => {
            if required {
                println!("cargo::error=CRAFT_I18N_REQUIRED=1 but CRAFT_I18N_DIR is unset: a release must embed the translations");
            }
            Vec::new()
        }
    };
    let mut o = String::from("/// (locale, shared catalog, Ubunye Photo's catalog), one entry per locale; `\"\"` for a missing file.\n");
    o.push_str("pub static CRAFT_I18N: &craft_i18n::Embedded = &[\n");
    for (code, shared, app) in entries {
        // `{:?}` escapes the path (Windows backslashes) as a Rust string literal.
        let shared = shared.map_or_else(|| "\"\"".to_string(), |p| format!("include_str!({p:?})"));
        let _ = writeln!(o, "    ({code:?}, {shared}, include_str!({app:?})),");
    }
    o.push_str("];\n");
    o
}

/// One embedded locale: its code, its `common.tsv` if it has one, and its PhotoCraft catalog.
type Locale = (String, Option<std::path::PathBuf>, std::path::PathBuf);

/// Every locale under `dir` with PhotoCraft's catalog, by code. An error when `dir` isn't a
/// craft-i18n `locales/` directory.
fn locales(dir: &std::path::Path) -> Result<Vec<Locale>, String> {
    // Build scripts run in the crate's directory and `include_str!` resolves against the
    // generated file, so a relative path would mean two different places.
    if !dir.is_absolute() {
        return Err(format!("{} is not an absolute path", dir.display()));
    }
    // Before read_dir, so a checkout that appears later reruns the script.
    println!("cargo::rerun-if-changed={}", dir.display());
    let list = std::fs::read_dir(dir).map_err(|e| format!("{} is not a craft-i18n checkout ({e})", dir.display()))?;
    let mut out = Vec::new();
    for entry in list.flatten() {
        let path = entry.path();
        let Some(code) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
        let app = path.join(APP_CATALOG);
        if !app.is_file() {
            continue;
        }
        println!("cargo::rerun-if-changed={}", app.display());
        let shared = Some(path.join("common.tsv")).filter(|p| p.is_file());
        if let Some(p) = &shared {
            println!("cargo::rerun-if-changed={}", p.display());
        }
        out.push((code.clone(), shared, ubunye_catalog(&code, &app)));
    }
    if out.is_empty() {
        return Err(format!("{} has no locale with a {APP_CATALOG}", dir.display()));
    }
    out.sort();
    Ok(out)
}

// Modified by Ubunye AI Ecosystems, 2026: the craft-i18n catalogs say "PhotoCraft" and know none of
// our own strings, so each locale's PhotoCraft catalog is branded and given Ubunye's overlay rows
// (ubunye/i18n/overlay/<code>.tsv, written by ubunye/brand/apply.py) before it is embedded.

/// Phrases (lower case) whose rows keep "PhotoCraft": credits and fixtures (apply.py's KEEP_LINE).
const KEEP_NAME: [&str; 11] = [
    "based on photocraft",
    "upstream photocraft",
    "photocraft authors",
    "photocraft contributors",
    "photocraft team",
    ".photocraft-face-",
    "photocraft studio cmyk",
    "photocraft coated cmyk",
    "photocraft projects",
    "photocraft transform preview",
    "deflate-compressed photocraft",
];

/// `text` with the product name "PhotoCraft" (a whole word, not after a dot) replaced by `name`.
fn rename_product(text: &str, name: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut prev: Option<char> = None;
    while let Some(at) = rest.find("Ubunye Photo") {
        let (before, after) = rest.split_at(at);
        let after = after.get("Ubunye Photo".len()..).unwrap_or_default();
        let before_char = before.chars().last().or(prev);
        let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
        let whole = !before_char.is_some_and(|c| word(c) || c == '.') && !after.chars().next().is_some_and(word);
        out.push_str(before);
        out.push_str(if whole { name } else { "Ubunye Photo" });
        prev = Some('t');
        rest = after;
    }
    out.push_str(rest);
    out
}

/// One catalog row (`<tab>source<tab>translation`) with the product name replaced, unless it is a credit.
fn brand_row(line: &str, name: &str) -> String {
    let cols: Vec<&str> = line.split('\t').collect();
    let keep = cols.get(1).is_some_and(|source| {
        let lower = source.to_lowercase();
        KEEP_NAME.iter().any(|k| lower.contains(k))
    });
    if line.starts_with('#') || cols.len() != 3 || keep {
        return line.to_string();
    }
    cols.iter().map(|c| rename_product(c, name)).collect::<Vec<_>>().join("\t")
}

/// The product name from ubunye/brand.toml.
fn product_name() -> Option<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ubunye/brand.toml");
    println!("cargo::rerun-if-changed={}", path.display());
    let table = std::fs::read_to_string(&path).ok()?.parse::<toml::Table>().ok()?;
    table.get("product")?.get("name")?.as_str().map(str::to_string)
}

/// `app` (a craft-i18n PhotoCraft catalog) branded for Ubunye, plus our overlay rows, written under
/// `OUT_DIR`; the original path if that fails (the catalog is then used as it is).
fn ubunye_catalog(code: &str, app: &std::path::Path) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ubunye/i18n/overlay");
    // A locale without rows of its own gets our strings in English (`_default.tsv`).
    let own = dir.join(format!("{code}.tsv"));
    let overlay = if own.is_file() { own } else { dir.join("_default.tsv") };
    println!("cargo::rerun-if-changed={}", dir.display());
    let Some(name) = product_name() else {
        println!("cargo::warning=ubunye/brand.toml has no product name; the catalogs keep \"Ubunye Photo\"");
        return app.to_path_buf();
    };
    let Ok(text) = std::fs::read_to_string(app) else { return app.to_path_buf() };
    let mut out: String = text.lines().map(|l| brand_row(l, &name)).collect::<Vec<_>>().join("\n");
    out.push('\n');
    out.push_str(&std::fs::read_to_string(&overlay).unwrap_or_default());
    let dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("i18n").join(code);
    let target = dir.join(APP_CATALOG);
    match std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&target, out)) {
        Ok(()) => target,
        Err(e) => {
            println!("cargo::warning=writing {}: {e}", target.display());
            app.to_path_buf()
        }
    }
}
fn read<T>(path: &std::path::Path, fallback: &str, parse: impl Fn(&str) -> Result<T, String>) -> Option<T> {
    println!("cargo::rerun-if-changed={}", path.display());
    let r = std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| parse(&t));
    if let Err(e) = &r {
        println!("cargo::warning={}: {e}; {fallback}", path.display());
    }
    r.ok()
}

/// A non-empty string field of someone's people.toml entry, as a Rust `Option<&str>` literal.
fn name(v: Option<&str>) -> String {
    match v.map(str::trim).filter(|s| !s.is_empty()) {
        Some(x) => format!("Some({x:?})"),
        None => "None".into(),
    }
}

fn s(v: &Value, k: &str) -> String {
    format!("{:?}", v.get(k).and_then(Value::as_str).unwrap_or(""))
}

fn n(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

fn rows(json: &Value, k: &str) -> Vec<Value> {
    json.get(k).and_then(Value::as_array).cloned().unwrap_or_default()
}

fn generate(json: &Value, people: &toml::Table) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "pub static TOTAL_COMMITS: u64 = {};", n(json, "commits"));
    let _ = writeln!(o, "pub static CONTRIBUTORS: &[Contributor] = &[");
    for c in rows(json, "contributors") {
        let login = c.get("login").and_then(Value::as_str).unwrap_or("");
        if login.is_empty() {
            continue;
        }
        // Names only from a verified, self-chosen people.toml entry.
        let entry = people.get("people").and_then(|p| p.get(login)).filter(|_| c.get("people_entry").and_then(Value::as_bool) == Some(true));
        let field = |k: &str| entry.and_then(|e| e.get(k)).and_then(toml::Value::as_str);
        let synced = entry.and_then(|e| e.get("sync_github_name")).and_then(toml::Value::as_bool) == Some(true);
        let display =
            field("display_name").filter(|s| !s.trim().is_empty()).or_else(|| if synced { c.get("github_name").and_then(Value::as_str) } else { None });
        let _ = writeln!(
            o,
            "    Contributor {{ login: {login:?}, display_name: {}, real_name: {}, prs: {}, commits: {}, lines_added: {}, lines_deleted: {}, binary_added: {}, binary_deleted: {}, first_commit: {}, last_commit: {} }},",
            name(display),
            name(field("real_name")),
            n(&c, "prs"),
            n(&c, "commits"),
            n(&c, "lines_added"),
            n(&c, "lines_deleted"),
            n(&c, "binary_added"),
            n(&c, "binary_deleted"),
            s(&c, "first_commit"),
            s(&c, "last_commit"),
        );
    }
    let _ = writeln!(o, "];");
    let _ = writeln!(o, "pub static MODELS: &[Model] = &[");
    for m in rows(json, "models") {
        let _ = writeln!(
            o,
            "    Model {{ company: {}, model: {}, version: {}, commits: {}, lines_added: {}, lines_deleted: {} }},",
            s(&m, "company"),
            s(&m, "model"),
            s(&m, "version"),
            n(&m, "commits"),
            n(&m, "lines_added"),
            n(&m, "lines_deleted"),
        );
    }
    let _ = writeln!(o, "];");
    o
}
