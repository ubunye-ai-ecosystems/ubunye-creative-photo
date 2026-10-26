//! Kopano sync plumbing: lets a Kopano connector read and replace the active document as a
//! `.pcraft` bundle (a manifest plus content-addressed tiles and blobs), so co-editing moves only
//! the tiles that changed. This file is Ubunye's own (not upstream PhotoCraft's), so upstream
//! merges never touch it.
//!
//! - `kopano.export {}`: the active document's bundle manifest (tiles and blobs by BLAKE3 hash).
//!   Only new tiles are compressed (one incremental writer per document).
//! - `kopano.objects {hashes}`: those objects of the last export, as base64 zstd bytes (the
//!   bundle's own `tiles/<hash>.zst` / `blobs/<hash>.zst`). Replies stop near 5 MiB; the rest
//!   is listed in `more`, to ask again.
//! - `kopano.stage {objects}`: hand over objects (base64 zstd) the next import needs, a few at
//!   a time (control requests are at most 1 MiB).
//! - `kopano.import {manifest, revision?}`: replace the active document with that manifest as one
//!   undoable step; with `revision`, only if the document is still at that revision. Objects come from the staged ones, else from the last export. Every object is
//!   checked against its BLAKE3 hash by the bundle loader.
//!
//! Every command takes an optional `doc` (a document id): the shared document. Export and
//! objects read that document even when another one is in front; an import is refused unless
//! that document is the active one, so a teammate's update never lands in unrelated work.
//!
//! Nothing here touches the file system, so the commands are safe for automation.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use photocraft_format::zip::{ZipReader, ZipWriter};
use photocraft_format::{LoadOptions, PcraftWriter, SaveOptions};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// Raw bytes of objects one `kopano.objects` reply may carry (about 6.7 MiB as base64).
const REPLY_BUDGET: usize = 5 << 20;
/// Staged bytes before `kopano.stage` refuses more (an import must come first).
const STAGE_LIMIT: usize = 4 << 30;
/// Largest single object accepted or served.
const MAX_OBJECT: usize = 1 << 30;
/// Documents whose last export is kept, and the bytes those exports may hold together.
const MAX_CACHED: usize = 4;
const CACHE_BYTES: usize = 1 << 30;

struct Exported {
    writer: PcraftWriter,
    zip: Arc<Vec<u8>>,
    revision: u64,
    /// When it was last used (a counter), to drop the least recently used first.
    used: u64,
}

#[derive(Default)]
struct State {
    /// Last export per document id.
    docs: BTreeMap<u64, Exported>,
    /// Staged objects by hash (zstd bytes) and their total size.
    staged: BTreeMap<String, Vec<u8>>,
    staged_bytes: usize,
    uses: u64,
}

fn next_use(st: &mut State) -> u64 {
    st.uses += 1;
    st.uses
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    f(g.get_or_insert_with(State::default))
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn fmt_err(e: photocraft_format::FormatError) -> EngineError {
    EngineError::Other(format!("Kopano bundle: {e}"))
}

fn is_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// The document a command is for: `doc` when given (it must be open), else the active one.
fn target<'a>(s: &'a Session, p: &Value) -> Result<&'a crate::DocState> {
    match p.get("doc") {
        None | Some(Value::Null) => s.active().ok_or(EngineError::NoDocument),
        Some(v) => {
            let id = v.as_u64().ok_or_else(|| bad("kopano", "`doc` must be a document id"))?;
            s.documents().iter().find(|d| d.doc.id.0 == id).ok_or_else(|| EngineError::Other("the shared document is closed".into()))
        }
    }
}

/// Export the target document (incrementally) and remember the bundle.
fn export_active(s: &Session, p: &Value) -> Result<(u64, Arc<Vec<u8>>)> {
    let d = target(s, p)?;
    let open: BTreeSet<u64> = s.documents().iter().map(|d| d.doc.id.0).collect();
    let doc_id = d.doc.id.0;
    let revision = d.revision;
    let doc = d.doc.clone();
    with_state(|st| {
        let used = next_use(st);
        if let Some(e) = st.docs.get_mut(&doc_id)
            && e.revision == revision
        {
            e.used = used;
            return Ok((doc_id, e.zip.clone()));
        }
        let mut writer = st.docs.remove(&doc_id).map(|e| e.writer).unwrap_or_default();
        let (zip, _) = writer.save_zip(&doc, &SaveOptions::default()).map_err(fmt_err)?;
        let zip = Arc::new(zip);
        // closed documents go first, then the least recently used until both limits hold
        st.docs.retain(|id, _| open.contains(id));
        let bytes = |st: &State| st.docs.values().map(|e| e.zip.len()).sum::<usize>();
        while !st.docs.is_empty() && (st.docs.len() >= MAX_CACHED || bytes(st) + zip.len() > CACHE_BYTES) {
            let Some(oldest) = st.docs.iter().min_by_key(|(_, e)| e.used).map(|(id, _)| *id) else { break };
            st.docs.remove(&oldest);
        }
        let used = next_use(st);
        st.docs.insert(doc_id, Exported { writer, zip: zip.clone(), revision, used });
        Ok((doc_id, zip))
    })
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let (_, zip) = export_active(s, p)?;
    let r = ZipReader::new(&zip).map_err(fmt_err)?;
    let manifest: Value = serde_json::from_slice(&r.read_by_name("manifest.json", 256 << 20).map_err(fmt_err)?)
        .map_err(|e| EngineError::Other(format!("Kopano bundle manifest: {e}")))?;
    let revision = target(s, p).map(|d| d.revision).unwrap_or(0);
    Ok(json!({"manifest": manifest, "revision": revision}))
}

fn objects(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "kopano.objects";
    let hashes = p.get("hashes").and_then(Value::as_array).ok_or_else(|| bad(CMD, "`hashes` must be an array of BLAKE3 hex strings"))?;
    let (_, zip) = export_active(s, p)?;
    let r = ZipReader::new(&zip).map_err(fmt_err)?;
    let by_name: BTreeMap<&str, &photocraft_format::zip::Entry> = r.entries.iter().map(|e| (e.name.as_str(), e)).collect();
    let (mut out, mut more, mut missing, mut used) = (serde_json::Map::new(), Vec::new(), Vec::new(), 0usize);
    for h in hashes {
        let Some(h) = h.as_str().filter(|h| is_hash(h)) else { return Err(bad(CMD, format!("{h} is not a lowercase BLAKE3 hex hash"))) };
        if out.contains_key(h) {
            continue;
        }
        if used >= REPLY_BUDGET {
            more.push(json!(h));
            continue;
        }
        let entry = by_name.get(format!("tiles/{h}.zst").as_str()).or_else(|| by_name.get(format!("blobs/{h}.zst").as_str()));
        let Some(entry) = entry else {
            missing.push(json!(h));
            continue;
        };
        let bytes = r.read(entry, MAX_OBJECT).map_err(fmt_err)?;
        used = used.saturating_add(bytes.len());
        out.insert(h.to_string(), json!(base64::engine::general_purpose::STANDARD.encode(&bytes)));
    }
    Ok(json!({"objects": out, "more": more, "missing": missing}))
}

fn stage(_s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "kopano.stage";
    let objs = p.get("objects").and_then(Value::as_object).ok_or_else(|| bad(CMD, "`objects` must map BLAKE3 hashes to base64 zstd bytes"))?;
    let mut decoded = Vec::with_capacity(objs.len());
    for (h, v) in objs {
        if !is_hash(h) {
            return Err(bad(CMD, format!("`{h}` is not a lowercase BLAKE3 hex hash")));
        }
        let b64 = v.as_str().ok_or_else(|| bad(CMD, format!("object {h} must be a base64 string")))?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| bad(CMD, format!("object {h}: {e}")))?;
        if bytes.len() > MAX_OBJECT {
            return Err(bad(CMD, format!("object {h} is over {MAX_OBJECT} bytes")));
        }
        decoded.push((h.clone(), bytes));
    }
    with_state(|st| {
        let add: usize = decoded.iter().map(|(_, b)| b.len()).sum();
        if st.staged_bytes.saturating_add(add) > STAGE_LIMIT {
            return Err(EngineError::Other("too many staged Kopano objects; import or clear them first".into()));
        }
        for (h, b) in decoded {
            st.staged_bytes = st.staged_bytes.saturating_add(b.len());
            if let Some(old) = st.staged.insert(h, b) {
                st.staged_bytes = st.staged_bytes.saturating_sub(old.len());
            }
        }
        Ok(json!({"staged": st.staged.len(), "bytes": st.staged_bytes}))
    })
}

/// Tile and blob hashes a manifest names (bounded walk; tiles are `{tx, ty, hash}` records).
fn referenced(v: &Value, tiles: &mut BTreeSet<String>, blobs: &mut BTreeSet<String>) {
    let mut stack = vec![v];
    let mut budget = 50_000_000usize;
    while let Some(v) = stack.pop() {
        budget = budget.saturating_sub(1);
        if budget == 0 {
            return;
        }
        match v {
            Value::Object(m) => {
                if let (Some(h), true, true) = (m.get("hash").and_then(Value::as_str), m.contains_key("tx"), m.contains_key("ty")) {
                    if is_hash(h) {
                        tiles.insert(h.to_string());
                    }
                    continue;
                }
                stack.extend(m.values());
            }
            Value::Array(a) => stack.extend(a.iter()),
            Value::String(s) if is_hash(s) => {
                blobs.insert(s.clone());
            }
            _ => {}
        }
    }
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "kopano.import";
    let manifest = p.get("manifest").filter(|m| m.is_object()).ok_or_else(|| bad(CMD, "`manifest` must be a .pcraft manifest object"))?;
    // an update goes into the shared document only, never into whatever else is in front
    let shared = target(s, p)?.doc.id;
    if s.active().map(|d| d.doc.id) != Some(shared) {
        return Err(EngineError::Other("the shared document is not in front; its update waits until it is".into()));
    }
    if let Some(base) = p.get("revision") {
        let base = base.as_u64().ok_or_else(|| bad(CMD, "`revision` must be a whole number"))?;
        let now = s.active().map(|d| d.revision).ok_or(EngineError::NoDocument)?;
        if now != base {
            return Err(EngineError::Other(format!("the document changed since revision {base} (now {now}); export again and rebuild the update")));
        }
    }
    let (doc_id, last) = export_active(s, p)?;
    let last = ZipReader::new(&last).map_err(fmt_err)?;
    let by_name: BTreeMap<&str, &photocraft_format::zip::Entry> = last.entries.iter().map(|e| (e.name.as_str(), e)).collect();
    let (mut tiles, mut blobs) = (BTreeSet::new(), BTreeSet::new());
    referenced(manifest, &mut tiles, &mut blobs);
    let staged = with_state(|st| {
        st.staged_bytes = 0;
        std::mem::take(&mut st.staged)
    });
    let mut z = ZipWriter::new();
    let text = serde_json::to_vec(manifest).map_err(|e| bad(CMD, e.to_string()))?;
    z.add("manifest.json", &text).map_err(fmt_err)?;
    let (mut from_stage, mut from_doc) = (0usize, 0usize);
    for (dir, set) in [("tiles", &tiles), ("blobs", &blobs)] {
        for h in set {
            let name = format!("{dir}/{h}.zst");
            if let Some(b) = staged.get(h) {
                z.add(&name, b).map_err(fmt_err)?;
                from_stage += 1;
            } else if let Some(e) = ["tiles", "blobs"].iter().find_map(|d| by_name.get(format!("{d}/{h}.zst").as_str())) {
                z.add(&name, &last.read(e, MAX_OBJECT).map_err(fmt_err)?).map_err(fmt_err)?;
                from_doc += 1;
            }
            // A hash-like string that is no object (for example a name) is left out; the
            // loader reports any object it really needs.
        }
    }
    let bundle = z.finish().map_err(fmt_err)?;
    let fresh = photocraft_format::load_from_bytes_with(&bundle, &LoadOptions { preserve_ids: true, ..LoadOptions::default() }).map_err(fmt_err)?;
    s.edit("Teammate Update", |doc, active| {
        let (id, name) = (doc.id, doc.name.clone());
        *doc = fresh;
        // Keep the identity (colour state, views and caches are keyed by it) and the tab name.
        doc.id = id;
        doc.name = name;
        if active.is_none_or(|a| doc.layer(a).is_none()) {
            *active = doc.top_layer();
        }
        Ok(())
    })?;
    with_state(|st| {
        // The incremental writer is kept: its compressed objects are still valid by path.
        if let Some(e) = st.docs.get_mut(&doc_id) {
            e.revision = u64::MAX;
        }
    });
    let revision = s.active().map(|d| d.revision).unwrap_or(0);
    Ok(json!({"revision": revision, "fromStage": from_stage, "fromDocument": from_doc}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "kopano.export",
            label: "Collaboration Export",
            menu: &[],
            shortcut: None,
            params: r#"{} → {manifest, revision}"#,
            enabled: has_doc,
            run: export,
            journal: false,
        },
        CommandSpec {
            id: "kopano.objects",
            label: "Collaboration Objects",
            menu: &[],
            shortcut: None,
            params: r#"{"hashes":[blake3 hex,…]} → {objects:{hash: base64 zstd}, more:[hash], missing:[hash]}"#,
            enabled: has_doc,
            run: objects,
            journal: false,
        },
        CommandSpec {
            id: "kopano.stage",
            label: "Collaboration Stage",
            menu: &[],
            shortcut: None,
            params: r#"{"objects":{blake3 hex: base64 zstd}} → {staged, bytes}"#,
            enabled: crate::commands::always,
            run: stage,
            journal: false,
        },
        CommandSpec {
            id: "kopano.import",
            label: "Teammate Update",
            menu: &[],
            shortcut: None,
            params: r#"{"manifest": .pcraft manifest, "revision": n? (refused if the document moved)} → {revision, fromStage, fromDocument} (one undo step)"#,
            enabled: has_doc,
            run: import,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(v: &Value) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD.decode(v.as_str().unwrap()).unwrap()
    }

    #[test]
    fn export_then_import_round_trips_and_moves_only_new_tiles() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 600, "height": 300, "background": "white"})).unwrap();
        s.execute("paint.stroke", json!({"points": [[10, 10, 1], [40, 40, 1]], "size": 9, "color": "#ff0000"})).unwrap();
        let tiles = |v: &Value| -> BTreeSet<String> {
            let (mut t, mut b) = (BTreeSet::new(), BTreeSet::new());
            referenced(&v["manifest"], &mut t, &mut b);
            t
        };
        let a = s.execute("kopano.export", json!({})).unwrap();
        assert!(!tiles(&a).is_empty(), "{a}");
        // Paint far away: exactly the tile of that stroke is new.
        s.execute("paint.stroke", json!({"points": [[520, 100, 1], [530, 110, 1]], "size": 9, "color": "#0000ff"})).unwrap();
        let painted = s.execute("document.pixel", json!({"x": 525, "y": 105})).unwrap();
        let b = s.execute("kopano.export", json!({})).unwrap();
        let new: Vec<String> = tiles(&b).difference(&tiles(&a)).cloned().collect();
        assert_eq!(new.len(), 1, "one touched tile: {new:?}");
        let got = s.execute("kopano.objects", json!({"hashes": [new[0]]})).unwrap();
        assert!(!b64(&got["objects"][new[0].as_str()]).is_empty());
        // Undo the stroke, then bring it back through stage + import.
        s.execute("edit.undo", json!({})).unwrap();
        assert_ne!(s.execute("document.pixel", json!({"x": 525, "y": 105})).unwrap(), painted);
        s.execute("kopano.stage", json!({"objects": {new[0].as_str(): got["objects"][new[0].as_str()]}})).unwrap();
        let r = s.execute("kopano.import", json!({"manifest": b["manifest"]})).unwrap();
        assert_eq!(r["fromStage"], 1, "{r}");
        // A stale base revision is refused.
        assert!(s.execute("kopano.import", json!({"manifest": b["manifest"], "revision": 0})).is_err());
        assert_eq!(s.execute("document.pixel", json!({"x": 525, "y": 105})).unwrap(), painted);
        let c = s.execute("kopano.export", json!({})).unwrap();
        assert_eq!(c["manifest"]["document"]["layers"], b["manifest"]["document"]["layers"]);
        // One undo step brings the pre-import state back.
        s.execute("edit.undo", json!({})).unwrap();
        let d = s.execute("kopano.export", json!({})).unwrap();
        assert_eq!(d["manifest"]["document"]["layers"], a["manifest"]["document"]["layers"]);
    }

    #[test]
    fn the_shared_document_is_the_target_never_the_one_in_front() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 300, "height": 200, "background": "white"})).unwrap();
        let shared = s.active().unwrap().doc.id.0;
        let a = s.execute("kopano.export", json!({"doc": shared})).unwrap();
        // another document comes to the front
        s.execute("file.new", json!({"width": 64, "height": 64, "background": "black"})).unwrap();
        let other = s.active().unwrap().doc.id.0;
        assert_ne!(other, shared);
        let b = s.execute("kopano.export", json!({"doc": shared})).unwrap();
        assert_eq!(a["manifest"]["document"], b["manifest"]["document"], "export reads the shared document");
        // an update for the shared document is refused while the other one is in front
        let before = s.execute("kopano.export", json!({})).unwrap();
        let r = s.execute("kopano.import", json!({"manifest": a["manifest"], "doc": shared}));
        assert!(r.is_err(), "{r:?}");
        assert_eq!(s.execute("kopano.export", json!({})).unwrap()["manifest"]["document"], before["manifest"]["document"], "the other document is untouched");
        // a closed shared document is an error, not the active one
        assert!(s.execute("kopano.export", json!({"doc": 987654321u64})).is_err());
        // in front again, the update lands, as one plainly named undo step
        let i = s.documents().iter().position(|d| d.doc.id.0 == shared).unwrap();
        assert!(s.set_active(i));
        s.execute("kopano.import", json!({"manifest": a["manifest"], "doc": shared})).unwrap();
        assert_eq!(s.active().unwrap().history.undo_label().as_deref(), Some("Teammate Update"));
    }

    #[test]
    fn the_bundle_cache_forgets_closed_documents() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        let first = s.active().unwrap().doc.id.0;
        s.execute("kopano.export", json!({})).unwrap();
        s.execute("file.close", json!({"force": true})).unwrap();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        s.execute("kopano.export", json!({})).unwrap();
        assert!(with_state(|st| !st.docs.contains_key(&first)));
    }

    #[test]
    fn bad_params_are_errors_not_panics() {
        let mut s = Session::new();
        assert!(s.execute("kopano.export", json!({})).is_err());
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        for (cmd, p) in [
            ("kopano.objects", json!({})),
            ("kopano.objects", json!({"hashes": ["../x"]})),
            ("kopano.stage", json!({"objects": {"zz": "!!"}})),
            ("kopano.stage", json!({"objects": {"0000000000000000000000000000000000000000000000000000000000000000": "!!"}})),
            ("kopano.import", json!({})),
            ("kopano.import", json!({"manifest": {"nope": 1}})),
        ] {
            assert!(s.execute(cmd, p.clone()).is_err(), "{cmd} {p}");
        }
        let missing = s.execute("kopano.objects", json!({"hashes": ["0000000000000000000000000000000000000000000000000000000000000000"]})).unwrap();
        assert_eq!(missing["missing"].as_array().unwrap().len(), 1);
    }
}
