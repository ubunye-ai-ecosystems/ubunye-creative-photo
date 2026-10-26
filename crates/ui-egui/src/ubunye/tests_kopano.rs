//! Collaboration client and panel (Ubunye): paths and token, events in plain words, a fake
//! `kopanod` over real loopback TCP (incl. a restart), and the Share & Collaborate window driven
//! like a person does: pointer clicks at its buttons and typing into its fields, through the
//! control channel. Follows Ubunye Light's `tests_kopano.rs`.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::kopano::*;
use super::kopano_panel::{ControlLink, KopanoPanel, Started};
use crate::PhotocraftApp;

const ALICE: &str = "c013b977c6b7b2118e21cbcb4cfd63800a5f798409997d4007fbe00b8438d344";
const M1: &str = "2635f5f796bbc17f7d7a42b10a64c795068d3cbeb4a8d6e95157b87d26300486";
const PID: &str = "aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44";
const NEW_PID: &str = "bb11bb22cc33dd44ee55ff66aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44";
const TOKEN: &str = "secret-token";
const UID: &str = "5f0c1e2d3c4b5a69";

fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
}

#[test]
fn home_folder_follows_kopanod() {
    let all = [("KOPANO_HOME", ""), ("LOCALAPPDATA", r"C:\Users\a\AppData\Local"), ("HOME", "/home/a"), ("XDG_DATA_HOME", "")];
    assert_eq!(home_with(env(&all), "windows"), Some(PathBuf::from(r"C:\Users\a\AppData\Local").join("Kopano")));
    assert_eq!(home_with(env(&all), "macos"), Some(PathBuf::from("/home/a/Library/Application Support/Kopano")));
    assert_eq!(home_with(env(&all), "linux"), Some(PathBuf::from("/home/a/.local/share/kopano")));
    assert_eq!(home_with(env(&[("HOME", "/home/a"), ("XDG_DATA_HOME", "/data")]), "linux"), Some(PathBuf::from("/data/kopano")));
    for os in ["windows", "macos", "linux"] {
        assert_eq!(home_with(env(&[("KOPANO_HOME", "/k"), ("HOME", "/home/a")]), os), Some(PathBuf::from("/k")));
    }
    assert_eq!(home_with(env(&[]), "windows"), None);
}

#[test]
fn api_port_follows_kopanod() {
    assert_eq!(api_port_with(env(&[])), DEFAULT_API_PORT);
    assert_eq!(DEFAULT_API_PORT, 47100);
    assert_eq!(api_port_with(env(&[("KOPANO_API_PORT", " 47121 ")])), 47121);
    for bad in ["", "nope", "0", "70000"] {
        assert_eq!(api_port_with(env(&[("KOPANO_API_PORT", bad)])), DEFAULT_API_PORT, "{bad:?}");
    }
}

static N: AtomicUsize = AtomicUsize::new(0);

fn temp_home() -> PathBuf {
    let d = std::env::temp_dir().join(format!("photo-collab-panel-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn token_is_read_trimmed_and_a_missing_one_is_explained() {
    let home = temp_home();
    assert!(read_token(&home).unwrap_err().contains("missing"));
    std::fs::write(home.join("api.token"), "  ").unwrap();
    assert!(read_token(&home).unwrap_err().contains("empty"));
    std::fs::write(home.join("api.token"), "abc123\n").unwrap();
    assert_eq!(read_token(&home).unwrap(), "abc123");
}

fn state_with_project() -> State {
    let mut s = State::default();
    s.apply_hello(&json!({"device": ALICE, "name": "Alice", "account": "acct-alice", "projects": [{"id": PID, "name": "Portrait"}]}));
    s
}

#[test]
fn events_read_as_plain_words() {
    let mut s = state_with_project();
    s.apply_event(&json!({"event": "teammate", "project": PID, "device": M1, "name": "Member1", "online": true}), 1000.0);
    s.apply_event(&json!({"event": "synced", "project": PID, "from": "Member1", "how": "fast-forward", "clashes": 0, "photos": 14}), 2000.0);
    s.apply_event(&json!({"event": "saved", "project": PID, "head": "abcd", "photos": 6}), 3000.0);
    s.apply_event(&json!({"event": "synced", "project": PID, "from": "Member1", "how": "merged", "clashes": 3}), 4000.0);
    s.apply_event(&json!({"event": "synced", "project": PID, "from": "Member1", "how": "same content, settled", "clashes": 0}), 5000.0);
    s.apply_event(&json!({"event": "teammate", "project": PID, "device": M1, "online": false}), 6000.0);
    s.apply_event(&json!({"event": "app", "project": PID, "error": "control port closed"}), 7000.0);
    s.apply_event(&json!({"event": "something-new"}), 8000.0);
    let lines: Vec<String> = s.activity.iter().rev().map(|a| a.said.text()).collect();
    assert_eq!(
        lines,
        [
            "Member1 is online",
            // the service counts tiles as "photos": a Photo window says what changed in its words
            "Member1 changed the document",
            "You changed the document",
            "Your changes and Member1's were put together · 3 clashes",
            "Up to date with Member1",
            "Member1 went offline",
            "Lost touch with this window for a moment; it reconnects by itself",
        ]
    );
    assert_eq!(s.projects[0].head.as_deref(), Some("abcd"));
    assert_eq!(s.projects[0].teammates, [Teammate { device: M1.into(), account: None, name: Some("Member1".into()), online: false }]);
    s.apply_event(&json!({"event": "teammate", "project": PID, "device": NEW_PID, "online": true}), 9000.0);
    assert_eq!(s.activity[0].said.text(), "bb11bb22 is online");
}

#[test]
fn the_same_thing_again_counts_instead_of_repeating() {
    let mut s = state_with_project();
    for t in 0..5 {
        s.apply_event(&json!({"event": "saved", "project": PID}), f64::from(t) * 100.0);
    }
    assert_eq!(s.activity.len(), 1);
    assert_eq!((s.activity[0].times, s.activity[0].at_ms), (5, 400.0));
    for t in 0..(ACTIVITY_KEEP + 10) {
        s.note(None, Said::Changed { who: format!("m{t}") }, 0.0);
    }
    assert_eq!(s.activity.len(), ACTIVITY_KEEP);
}

#[test]
fn time_ago_in_plain_words() {
    assert_eq!(ago(0.4), "just now");
    assert_eq!(ago(-5.0), "just now");
    assert_eq!(ago(f64::NAN), "just now");
    assert_eq!(ago(3.7), "3 s ago");
    assert_eq!(ago(125.0), "2 min ago");
    assert_eq!(ago(7300.0), "2 h ago");
    assert_eq!(ago(200000.0), "2 d ago");
}

#[test]
fn status_names_teammates_and_leaves_me_out() {
    let mut s = state_with_project();
    s.apply_status(PID, &json!({"head": "ff00", "teammates": [M1, ALICE], "online": {M1: "Member1"}}));
    assert_eq!(s.projects[0].teammates, [Teammate { device: M1.into(), account: None, name: Some("Member1".into()), online: true }]);
    s.apply_status(PID, &json!({"head": "ff00", "teammates": [M1], "online": {}}));
    assert_eq!(s.projects[0].teammates[0].label(), "Member1");
    assert!(!s.projects[0].teammates[0].online);
    s.apply_hello(&json!({"device": ALICE, "name": "Alice", "account": "acct-alice", "projects": [{"id": PID, "name": "Portrait"}]}));
    assert_eq!(s.projects[0].head.as_deref(), Some("ff00"));
    assert_eq!(s.projects[0].teammates.len(), 1);
    // junk is ignored, never a panic
    s.apply_status(PID, &json!({"teammates": 5, "online": [1], "review": null}));
    s.apply_hello(&json!({"projects": [{"name": 3}, 7]}));
    s.apply_event(&json!({"event": "teammate"}), 0.0);
    s.apply_event(&json!(null), 0.0);
}

#[test]
fn ids_are_checked() {
    assert!(is_hex_id(M1));
    assert!(!is_hex_id(&M1[..63]));
    assert!(!is_hex_id(&format!("{}g", &M1[..63])));
    assert_eq!(short(M1), "2635f5f7");
    assert_eq!(short("ab"), "ab");
}

#[test]
fn the_binding_is_photo_control_with_its_token_file() {
    let b = binding(18581, std::path::Path::new("T:/k/photo/control-18581.token"));
    assert_eq!(b, json!({"kind": "photo-control", "port": 18581, "token_file": "T:/k/photo/control-18581.token"}));
    assert_eq!(app_port(Some(&b)), Some(18581));
    // what kopanod hands back (with its own media folder field)
    assert_eq!(app_port(Some(&json!({"kind": "photo-control", "port": 18582, "token_file": "x", "media_dir": null}))), Some(18582));
    assert_eq!(app_port(Some(&json!({"kind": "light-control", "port": 18101}))), None);
    assert_eq!(app_port(Some(&json!({"kind": "photo-control", "port": 70000}))), None);
}

#[test]
fn changes_are_counted_in_layers() {
    let k = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(changed_text(&k(&["layer/a@pixels@0_0", "layer/a@pixels@1_0", "layer/a"])), "1 layer");
    assert_eq!(changed_text(&k(&["layer/a@pixels@0_0", "layer/b", "doc"])), "2 layers and the document settings");
    assert_eq!(changed_text(&k(&["doc"])), "the document settings");
    assert_eq!(changed_text(&[]), "the document");
    assert_eq!(owner_of("layer/a@mask@2_3#develop"), "layer/a");
    assert_eq!(layer_key_of("layer/a@mask@2_3"), Some("a"));
    assert_eq!(layer_key_of("doc"), None);
    assert_eq!(setting_text("content.adjustment.exposure"), "Exposure");
    assert_eq!(setting_text("opacity"), "Opacity");
}

#[test]
fn tile_hashes_are_found_in_an_export() {
    let doc = json!({"layers": [{"name": "Background", "content": {"pixels": {"tiles": [{"tx": 0, "ty": 0, "hash": "h1"}, {"tx": 1, "ty": 0, "hash": "h2"}]}}},
        {"name": "Group", "content": {"children": [{"mask": {"tiles": [{"tx": 0, "ty": 1, "hash": "h3"}]}}]}}], "hash": "not-a-tile"});
    let mut out = BTreeSet::new();
    tile_hashes(&doc, &mut out);
    assert_eq!(out, ["h1", "h2", "h3"].iter().map(|s| s.to_string()).collect());
}

// ---- a fake kopanod over loopback TCP ------------------------------------------------------

#[derive(Default)]
struct FakeData {
    /// The Photo control port the project is bound to (0: none); `attach` changes it.
    app_port: AtomicUsize,
    signed_out: AtomicBool,
    approve_owner: AtomicBool,
    /// With `approve_owner`: Member2's changes wait too, clashing on the same setting.
    two_members: AtomicBool,
    approve_member: AtomicBool,
    live_clash: AtomicBool,
    /// Not signed in, a sign-in waits in the browser for code BCDF-1234.
    signing_in: AtomicBool,
    /// ThabangVision people on the project; this side is its owner unless `member_role`.
    people: AtomicBool,
    member_role: AtomicBool,
    /// "Shared with me" lists one document that is not on this computer.
    shared_rows: AtomicBool,
}

struct Fake {
    got: Arc<Mutex<Vec<Value>>>,
    data: Arc<FakeData>,
    stop: Arc<AtomicBool>,
    accept: Option<std::thread::JoinHandle<()>>,
}

impl Fake {
    fn start(listener: TcpListener) -> Self {
        listener.set_nonblocking(true).unwrap();
        let got: Arc<Mutex<Vec<Value>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let data: Arc<FakeData> = Arc::default();
        let (g, st, ap) = (got.clone(), stop.clone(), data.clone());
        let accept = std::thread::spawn(move || {
            let mut workers = vec![];
            while !st.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((c, _)) => {
                        let (g, st, ap) = (g.clone(), st.clone(), ap.clone());
                        workers.push(std::thread::spawn(move || serve(c, &g, &st, &ap)));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
            for w in workers {
                let _ = w.join();
            }
        });
        Fake { got, data, stop, accept: Some(accept) }
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(a) = self.accept.take() {
            a.join().unwrap();
        }
    }

    fn requests(&self, op: &str) -> Vec<Value> {
        self.got.lock().unwrap().iter().filter(|r| r["op"] == op).cloned().collect()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop();
    }
}

fn app(port: &AtomicUsize) -> Value {
    match port.load(Ordering::SeqCst) {
        0 => json!({"kind": "toy"}),
        p => json!({"kind": "photo-control", "port": p, "token_file": "T:/x.token", "media_dir": null}),
    }
}

fn serve(c: TcpStream, got: &Mutex<Vec<Value>>, stop: &AtomicBool, data: &FakeData) {
    let app_port = &data.app_port;
    c.set_nonblocking(false).unwrap();
    c.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    let mut w = c.try_clone().unwrap();
    let mut r = BufReader::new(c);
    let mut line = String::new();
    while !stop.load(Ordering::SeqCst) {
        match r.read_line(&mut line) {
            Ok(0) => return,
            Ok(_) => {}
            Err(_) => continue,
        }
        let req: Value = serde_json::from_str(line.trim()).unwrap_or(Value::Null);
        line.clear();
        if req["token"] != TOKEN {
            let _ = writeln!(w, "{}", json!({"ok": false, "error": "bad token"}));
            return;
        }
        got.lock().unwrap().push(req.clone());
        let result = match req["op"].as_str().unwrap_or_default() {
            "subscribe" => {
                let _ = writeln!(w, "{}", json!({"ok": true, "result": "subscribed"}));
                for ev in [
                    json!({"event": "teammate", "project": PID, "device": M1, "name": "Member1", "online": true}),
                    json!({"event": "synced", "project": PID, "from": "Member1", "how": "fast-forward", "clashes": 0, "photos": 9, "head": "1234"}),
                ] {
                    let _ = writeln!(w, "{ev}");
                }
                while !stop.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(10));
                }
                return;
            }
            "hello" => {
                let waiting = data.signing_in.load(Ordering::SeqCst);
                let out = data.signed_out.load(Ordering::SeqCst) || waiting;
                json!({"device": ALICE, "name": if out { "" } else { "Alice" }, "account": if out { "" } else { "acct-alice" }, "signed_in": !out,
                    "signed_in_until_ms": 1.8e12,
                    "signing_in": if waiting { json!({"code": "BCDF-1234", "link": "http://localhost:3100/link?code=BCDF-1234", "until_ms": 1.8e12}) } else { Value::Null },
                    "message": if out && !waiting { json!("This computer was unlinked from ThabangVision. Sign in again.") } else { Value::Null },
                    "projects": if out { json!([]) } else { json!([{"id": PID, "name": "Portrait", "app": app(app_port)}]) }})
            }
            "people" if data.people.load(Ordering::SeqCst) => {
                let owner = !data.member_role.load(Ordering::SeqCst);
                json!({"shared": "tv-1", "people": [
                    {"user": "u-alice", "name": "Alice", "role": if owner { "owner" } else { "editor" }, "status": "active", "me": true, "online": true, "on_project": true},
                    {"user": "u-bongi", "name": "Bongi", "role": if owner { "editor" } else { "owner" }, "status": "active", "me": false, "online": true, "on_project": true},
                    {"user": "u-sipho", "name": "Sipho", "role": "editor", "status": "invited", "me": false, "online": false, "on_project": false}]})
            }
            "people" => json!({"shared": false, "people": []}),
            "shared-projects" => {
                let rows = if data.shared_rows.load(Ordering::SeqCst) {
                    json!([{"shared": "tv-shared-77", "name": "Lindiwe's poster", "project": NEW_PID, "role": "editor", "status": "invited", "here": false}])
                } else {
                    json!([])
                };
                json!({"mine": [], "shared": rows})
            }
            "sign-in" => json!({"code": "BCDF-1234", "link": "http://localhost:3100/link?code=BCDF-1234", "until_ms": 1.8e12}),
            "sign-in-cancel" => json!("not signing in"),
            "sign-out" => json!("signed out"),
            "invite" => json!({"message": "Invite sent."}),
            "remove-person" => json!({"removed": true}),
            "leave" => json!({"left": true}),
            "join-shared" => json!({"project": NEW_PID, "name": "Lindiwe's poster"}),
            "status" => {
                let mut st = json!({"head": "1234abcd", "name": "Portrait", "app": app(app_port),
                    "teammates": [{"device": M1, "name": "Member1", "account": "acct-m1", "online": true}], "online": {M1: "Member1"},
                    "members": [{"account": "acct-alice", "role": "admin", "owner": true}, {"account": "acct-m1", "role": "editor", "owner": false}]});
                if data.approve_owner.load(Ordering::SeqCst) {
                    st["mode"] = json!("approve");
                    st["review"] = json!({"is_owner": true, "approver": true, "owner_name": "Alice", "declined": [], "mine": null, "viewing": null,
                        "pending": [{"account": "acct-m1", "name": "Member1", "version": "v1", "photos": 14, "summary": "changed 14 photos",
                            "keys": [format!("layer/{UID}"), format!("layer/{UID}@pixels@0_0"), format!("layer/{UID}@pixels@1_0")],
                            "changes": [{"photo": format!("layer/{UID}"), "fields": []}],
                            "clashes": [{"key": format!("layer/{UID}"), "photo": format!("layer/{UID}"), "fields": ["opacity"],
                                "settings": [{"setting": "opacity", "main": 0.5, "member": 1.0}]}]}]});
                    if data.two_members.load(Ordering::SeqCst) {
                        let mut m2 = st["review"]["pending"][0].clone();
                        m2["account"] = json!("acct-m2");
                        m2["name"] = json!("Member2");
                        m2["version"] = json!("v2");
                        st["review"]["pending"].as_array_mut().unwrap().push(m2);
                    }
                } else if data.approve_member.load(Ordering::SeqCst) {
                    st["mode"] = json!("approve");
                    st["review"] = json!({"is_owner": false, "approver": false, "owner_name": "Alice", "declined": [], "pending": [], "viewing": null,
                        "mine": {"state": "waiting", "photos": 12}});
                } else if data.live_clash.load(Ordering::SeqCst) {
                    st["mode"] = json!("live");
                    st["clashes"] = json!([{"clash": "kopano/conflict/k/11-22", "key": "doc", "photo": "doc", "fields": ["Resolution"],
                        "settings": [{"setting": "resolution_dpi", "a": 300.0, "b": 72.0}]}]);
                }
                st
            }
            "approve" => json!({"main": "m2", "clashes_decided": 1}),
            "decline" => json!({"declined": "v1"}),
            "view" => json!({"viewing": {"side": req["show"]}}),
            "resolve" => json!({"settled": req["clash"]}),
            "set-approver" => json!({"approver": true}),
            "create-project" => json!({"project": NEW_PID, "head": "99", "items": 98, "photos": 98, "mode": req["mode"]}),
            "attach" => {
                app_port.store(req["app"]["port"].as_u64().unwrap_or(0) as usize, Ordering::SeqCst);
                json!("attached")
            }
            "join" => json!({"project": req["project"]}),
            _ => {
                let _ = writeln!(w, "{}", json!({"ok": false, "error": "unknown op"}));
                continue;
            }
        };
        let _ = writeln!(w, "{}", json!({"ok": true, "result": result}));
    }
}

fn wait(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let t0 = Instant::now();
    while !done() {
        assert!(t0.elapsed() < timeout, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn fake_service() -> (Fake, Config, PathBuf) {
    let home = temp_home();
    std::fs::write(home.join("api.token"), format!("{TOKEN}\n")).unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    (Fake::start(listener), Config { port, home: Some(home.clone()) }, home)
}

const T: Duration = Duration::from_secs(20);

#[test]
fn client_follows_a_fake_service_and_finds_it_again_after_a_restart() {
    let (mut fake, config, _home) = fake_service();
    let mut client = Client::new(config.clone());
    client.start(&egui::Context::default());
    wait("hello and status", T, || {
        let s = client.state();
        s.running && s.me.as_ref().is_some_and(|m| m.name == "Alice") && s.projects.first().is_some_and(|p| p.teammates.iter().any(|t| t.online))
    });
    wait("events", T, || client.state().activity.iter().any(|a| a.said.text() == "Member1 changed the document"));
    let s = client.state();
    assert_eq!(s.projects[0].name, "Portrait");
    assert_eq!(s.projects[0].app_port, Some(0).filter(|p| *p != 0));
    client
        .request(
            &egui::Context::default(),
            "Sharing…",
            json!({"op": "create-project", "name": "Portrait 2", "app": binding(18123, std::path::Path::new("T:/t.token")), "members": [{"account": "acct-m1", "role": "editor", "lanes": []}]}),
        )
        .unwrap();
    wait("the share", T, || client.state().job.is_some_and(|j| !j.busy));
    let job = client.state().job.unwrap();
    assert!(job.ok, "{job:?}");
    assert_eq!(job.project.as_deref(), Some(NEW_PID));
    assert!(job.text.starts_with("Shared this document. Invite sent."), "{}", job.text);
    let made = fake.requests("create-project");
    assert_eq!(made[0]["app"], json!({"kind": "photo-control", "port": 18123, "token_file": "T:/t.token"}));
    assert_eq!(made[0]["token"], TOKEN);
    fake.stop();
    wait("not running", T, || client.state().problem.as_deref() == Some(NOT_RUNNING));
    let s = client.state();
    assert!(!s.running);
    assert!(s.projects[0].teammates.iter().all(|t| !t.online));
    let listener = TcpListener::bind(("127.0.0.1", config.port)).unwrap();
    let fake2 = Fake::start(listener);
    wait("running again", T, || client.state().running);
    wait("events again", T, || !fake2.requests("subscribe").is_empty());
}

#[test]
fn a_wrong_token_is_explained() {
    let (_fake, config, home) = fake_service();
    std::fs::write(home.join("api.token"), "other").unwrap();
    let mut client = Client::new(config);
    client.start(&egui::Context::default());
    wait("the refusal", T, || client.state().problem.is_some());
    let s = client.state();
    assert!(!s.running);
    assert!(s.problem.unwrap().contains("api.token"));
}

#[test]
fn records_say_which_saved_file_a_shared_document_is() {
    let home = temp_home();
    assert_eq!(read_record(&home, PID), None);
    write_record(&home, PID, "T:/work/portrait.pcraft").unwrap();
    assert_eq!(record_path(&home, PID).unwrap(), home.join("photo").join(format!("{PID}.json")));
    assert_eq!(read_record(&home, PID).as_deref(), Some("T:/work/portrait.pcraft"));
    assert!(write_record(&home, "../evil", "x").is_err());
    // an old record of tile hashes is no identity: not trusted
    std::fs::write(record_path(&home, PID).unwrap(), r#"{"tiles":["a","b"]}"#).unwrap();
    assert_eq!(read_record(&home, PID), None);
    std::fs::write(record_path(&home, PID).unwrap(), "junk").unwrap();
    assert_eq!(read_record(&home, PID), None);
}

#[test]
fn errors_about_the_old_port_right_after_relinking_are_not_shown() {
    let mut s = state_with_project();
    s.note(Some(PID.into()), Said::Reattached { name: "Portrait".into() }, 1000.0);
    s.apply_event(&json!({"event": "app", "project": PID, "error": "port 18101 refused"}), 5000.0);
    assert_eq!(s.activity.len(), 1);
    s.apply_event(&json!({"event": "app", "project": PID, "error": "port 18103 refused"}), 1000.0 + RELINK_QUIET_MS + 1.0);
    assert_eq!(s.activity[0].said.text(), "Lost touch with this window for a moment; it reconnects by itself");
}

#[test]
fn review_lines_read_in_plain_words() {
    let owner = Review::from_value(&json!({"is_owner": true, "approver": true, "owner_name": "Alice",
        "pending": [{"account": "acct-m1", "name": "Member1", "version": "v1", "photos": 14, "keys": ["layer/a", "layer/a@pixels@0_0", "layer/b@pixels@3_3"],
            "clashes": [{"key": "layer/a", "photo": "layer/a", "settings": [{"setting": "opacity", "main": 0.5, "member": 1.0}]}]}]}));
    assert_eq!(owner.pending[0].what(), "2 layers");
    assert_eq!(owner.pending[0].clashes[0].main, "0.50");
    assert_eq!(owner.pending[0].clashes[0].member, "1");
    assert_eq!(owner.mine_text(), None);
    // an older service lists `changes` only
    let old = Review::from_value(&json!({"pending": [{"name": "M", "changes": [{"photo": "layer/a@pixels@0_0"}, {"photo": "doc"}]}]}));
    assert_eq!(old.pending[0].what(), "1 layer and the document settings");
    let mine = |state: &str| Review::from_value(&json!({"owner_name": "Alice", "mine": {"state": state, "photos": 2}})).mine_text().unwrap();
    assert_eq!(mine("waiting"), "You changed this document · Waiting for Alice's approval");
    assert!(mine("approved").starts_with("Approved: Alice took your changes"));
    assert!(mine("declined").starts_with("Declined: Alice kept the document as it was"));
    let mut s = state_with_project();
    s.apply_event(
        &json!({"event": "review", "project": PID, "is_owner": true, "approver": true, "owner_name": "Alice",
        "pending": [{"account": "acct-m1", "name": "Member1", "version": "v1", "photos": 14, "keys": ["layer/a@pixels@0_0"]}]}),
        1000.0,
    );
    assert_eq!(s.activity[0].said.text(), "Member1 changed 1 layer · waiting for your review");
    let mut m = state_with_project();
    m.apply_event(&json!({"event": "review", "project": PID, "owner_name": "Alice", "mine": {"state": "waiting", "photos": 1}}), 1000.0);
    m.apply_event(&json!({"event": "review", "project": PID, "owner_name": "Alice", "mine": {"state": "approved", "photos": 0}}), 2000.0);
    assert_eq!(m.activity[0].said.text(), "Alice approved your changes");
    m.apply_event(&json!({"event": "review", "project": PID, "owner_name": "Alice", "mine": {"state": "declined", "photos": 1}}), 3000.0);
    assert_eq!(m.activity[0].said.text(), "Alice declined your changes");
    no_protocol_words(&m.activity.iter().map(|a| a.said.text()).collect::<Vec<_>>().join("\n"));
}

// ---- the window in the app, driven like a person through the control channel ----------------

type H = egui_kittest::Harness<'static, PhotocraftApp>;

fn harness(config: Config, with_doc: bool) -> H {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 1700.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        if with_doc {
            app.run("file.new", json!({"name": "Portrait", "width": 600, "height": 400, "background": "white"})).expect("new document");
        }
        app.kopano = KopanoPanel::with_config(config);
        app
    });
    h.run_steps(4);
    h
}

/// One frame, with the window's queued pointer and key input fed in (eframe's raw input hook,
/// which the test harness does not call).
fn frame(h: &mut H) {
    let events = h.state_mut().take_synthetic_step();
    h.input_mut().events.extend(events);
    h.step();
}

/// One control request, answered after the frames it needs (pointer input waits for its frame).
fn request(h: &mut H, method: &str, params: Value) -> Value {
    let (tx, rx) = std::sync::mpsc::channel();
    h.state_mut().control_rx = Some(rx);
    let (req, reply) = crate::control::ControlRequest::new(method, params);
    tx.send(req).unwrap();
    let t0 = Instant::now();
    loop {
        frame(h);
        if let Ok(v) = reply.try_recv() {
            return v;
        }
        assert!(t0.elapsed() < T, "no reply to {method}");
    }
}

fn ok(v: Value) -> Value {
    assert_eq!(v["ok"], true, "{v}");
    v["result"].clone()
}

fn run(h: &mut H, command: &str, params: Value) -> Value {
    request(h, "engine.execute", json!({"command": command, "params": params}))
}

fn drawn_text(h: &H) -> String {
    fn collect(shape: &egui::Shape, text: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(shape) => text.push(shape.galley.job.text.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, text)),
            _ => {}
        }
    }
    let mut text = Vec::new();
    for shape in &h.output().shapes {
        collect(&shape.shape, &mut text);
    }
    text.join("\n")
}

fn step_until(h: &mut H, mut done: impl FnMut(&mut H) -> bool) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < T {
        frame(h);
        if done(h) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    false
}

fn shows(h: &mut H, text: &str) -> bool {
    step_until(h, |h| drawn_text(h).contains(text))
}

/// Click the window's `widget` (from `kopano.status` `widgets`) with the pointer.
fn click(h: &mut H, widget: &str) {
    assert!(step_until(h, |h| h.state().kopano.widgets.contains_key(widget)), "{widget} not on screen: {:?}", h.state().kopano.widgets.keys());
    let r = h.state().kopano.widgets[widget];
    ok(request(h, "ui.click", json!({"x": r[0] + r[2] / 2.0, "y": r[1] + r[3] / 2.0})));
    h.step();
}

/// Click into a field and type.
fn type_into(h: &mut H, field: &str, text: &str) {
    click(h, &format!("field:{field}"));
    ok(request(h, "ui.type", json!({"text": text})));
    h.step();
}

fn no_protocol_words(text: &str) {
    for word in ["Kopano", "kopano", "protocol"] {
        assert!(!text.contains(word), "{word:?} on screen: {text}");
    }
}

fn start_control_counting(started: Arc<AtomicUsize>) -> super::kopano_panel::StartControl {
    Box::new(move |_ctx, services| {
        started.fetch_add(1, Ordering::SeqCst);
        // what the desktop app's on-demand start also does
        services.automation_command = Some(Box::new(|_, _| Ok(())));
        Ok(Started { link: ControlLink { port: 18999, token_file: PathBuf::from("T:/k/photo/control-18999.token") }, rx: None })
    })
}

#[test]
fn window_says_when_collaboration_is_not_running_and_opens_from_the_top_bar_and_the_window_menu() {
    let home = temp_home();
    let port = TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let mut h = harness(Config { port, home: Some(home) }, true);
    // the people button in the top bar
    click(&mut h, "icon:collaborate");
    assert!(shows(&mut h, NOT_RUNNING), "{}", drawn_text(&h));
    let p = drawn_text(&h);
    assert!(p.contains("Start ThabangVision collaboration on this computer") && p.contains("Share & Collaborate") && p.contains("Share this document"), "{p}");
    no_protocol_words(&p);
    // Join is under Advanced (testing): closed, it isn't on screen
    assert!(!p.contains("Join a shared document"));
    click(&mut h, "button:kopano-advanced");
    assert!(shows(&mut h, "Join a shared document"), "{}", drawn_text(&h));
    for _ in 0..30 {
        h.step();
    }
    // Join checks its fields before anything else
    click(&mut h, "button:kopano-join");
    assert!(shows(&mut h, "The project id is 64 letters"), "{}", drawn_text(&h));
    // Window menu lists it, with a check while open; the command closes it again
    let menus = ok(request(&mut h, "ui.menu.list", json!({})));
    let item = menus.as_array().unwrap().iter().find(|m| m["id"] == "kopano.panel").cloned().expect("a Window menu item");
    assert_eq!(
        (item["path"][0].as_str(), item["label"].as_str(), item["checked"].as_bool()),
        (Some("Window"), Some("Share & Collaborate"), Some(true)),
        "{item}"
    );
    assert_eq!(ok(request(&mut h, "ui.menu.invoke", json!({"command": "kopano.panel"})))["open"], false);
    h.step();
    assert!(!drawn_text(&h).contains(NOT_RUNNING));
    // the close button closes it too
    click(&mut h, "icon:collaborate");
    assert!(shows(&mut h, NOT_RUNNING));
    click(&mut h, "button:kopano-close");
    assert!(step_until(&mut h, |h| !h.state().kopano.open));
}

#[test]
fn without_a_sign_in_the_window_offers_one() {
    let (fake, config, _home) = fake_service();
    fake.data.signed_out.store(true, Ordering::SeqCst);
    let mut h = harness(config, true);
    ok(run(&mut h, "kopano.panel", json!({})));
    assert!(shows(&mut h, "Sign in with ThabangVision"), "{}", drawn_text(&h));
    // nothing else shows until signed in; the service's sentence says why
    assert!(drawn_text(&h).contains("unlinked from ThabangVision") && !drawn_text(&h).contains("Share this document"), "{}", drawn_text(&h));
    click(&mut h, "button:kopano-signIn");
    wait("the sign-in", T, || fake.requests("sign-in").len() == 1);
    assert!(shows(&mut h, "Check that the browser shows BCDF-1234."), "{}", drawn_text(&h));
    no_protocol_words(&drawn_text(&h));
}

#[test]
fn window_shares_this_document_through_its_fields_and_starts_the_control_port_on_demand() {
    let (fake, config, _home) = fake_service();
    let mut h = harness(config, true);
    let started = Arc::new(AtomicUsize::new(0));
    h.state_mut().kopano.start_control = Some(start_control_counting(started.clone()));
    ok(run(&mut h, "kopano.panel", json!({})));
    assert!(
        step_until(&mut h, |h| {
            let p = drawn_text(h);
            p.contains("Signed in to ThabangVision as Alice") && p.contains("Member1") && p.contains("online") && p.contains("Member1 changed the document")
        }),
        "{}",
        drawn_text(&h)
    );
    let p = drawn_text(&h);
    no_protocol_words(&p);
    assert!(!p.contains("c013b977") && !p.contains(&PID[..8]), "{p}");
    click(&mut h, "button:kopano-advanced");
    assert!(shows(&mut h, "c013b977"), "{}", drawn_text(&h));
    for _ in 0..30 {
        h.step();
    }
    click(&mut h, "button:kopano-copyDevice");
    assert_eq!(h.state().kopano.copied.as_deref(), Some(ALICE));

    type_into(&mut h, "kopano:shareName", "Portrait together");
    type_into(&mut h, "kopano:shareAccount", "acct-m1");
    assert_eq!(h.state().kopano.share_name, "Portrait together");
    click(&mut h, "button:kopano-share");
    assert!(shows(&mut h, "Shared this document. Invite sent."), "{}", drawn_text(&h));
    let made = fake.requests("create-project");
    assert_eq!(made.len(), 1, "{made:?}");
    assert_eq!(made[0]["name"], "Portrait together");
    assert_eq!(made[0]["mode"], "approve", "I approve changes is the default");
    assert_eq!(made[0]["app"], json!({"kind": "photo-control", "port": 18999, "token_file": "T:/k/photo/control-18999.token"}));
    assert_eq!(made[0]["members"], json!([{"account": "acct-m1", "role": "editor", "lanes": []}]));
    // the start also set the automation gates
    assert!(h.state().services.automation_command.is_some());
    // Join uses the same control port: it was started once
    ok(run(&mut h, "kopano.join", json!({"project": PID, "from": M1})));
    wait("the join", T, || fake.requests("join").len() == 1);
    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert_eq!(fake.requests("join")[0]["app"]["kind"], "photo-control");
    let status = ok(run(&mut h, "kopano.status", json!({})));
    assert_eq!(status["controlPort"], 18999);
    assert_eq!(status["running"], true);
    assert!(status["widgets"]["button:kopano-share"].is_array(), "{}", status["widgets"]);
}

#[test]
fn share_needs_a_document_a_name_and_a_control_port() {
    let (_fake, config, _home) = fake_service();
    let mut h = harness(config.clone(), true);
    let r = run(&mut h, "kopano.share", json!({"name": "x", "account": "acct-m1"}));
    assert_eq!(r["ok"], false);
    assert!(r["error"].as_str().unwrap().contains("can't be shared right now"), "{r}");
    let r = run(&mut h, "kopano.share", json!({"name": "", "account": "acct-m1"}));
    assert!(r["error"].as_str().unwrap().contains("name"), "{r}");
    let mut h = harness(config, false);
    let r = run(&mut h, "kopano.share", json!({"name": "x", "account": "acct-m1"}));
    assert!(r["error"].as_str().unwrap().contains("Open the document"), "{r}");
}

#[test]
fn join_with_no_document_open_makes_one_for_the_shared_document() {
    let (fake, config, _home) = fake_service();
    let mut h = harness(config, false);
    h.state_mut().kopano.start_control = Some(start_control_counting(Arc::default()));
    assert!(h.state().session.active().is_none());
    ok(run(&mut h, "kopano.panel", json!({"advanced": true})));
    assert!(shows(&mut h, "Join a shared document"), "{}", drawn_text(&h));
    for _ in 0..30 {
        h.step();
    }
    type_into(&mut h, "kopano:joinProject", PID);
    type_into(&mut h, "kopano:joinFrom", M1);
    click(&mut h, "button:kopano-join");
    wait("the join", T, || fake.requests("join").len() == 1);
    assert!(shows(&mut h, "Joined. Your teammate's document comes into this window"), "{}", drawn_text(&h));
    assert_eq!(h.state().session.active().map(|d| d.doc.name.clone()).as_deref(), Some("Shared document"));
    assert_eq!(fake.requests("join")[0]["project"], PID);
    assert_eq!(fake.requests("join")[0]["from"], M1);
}

/// Review finding 6: Join brings the shared document into a new document; the open work stays.
#[test]
fn join_never_replaces_the_open_document() {
    let (fake, config, _home) = fake_service();
    let mut h = harness(config, true);
    h.state_mut().kopano.start_control = Some(start_control_counting(Arc::default()));
    let mine = h.state().session.active().unwrap().doc.id.0;
    ok(run(&mut h, "kopano.join", json!({"project": PID, "from": M1})));
    wait("the join", T, || fake.requests("join").len() == 1);
    let s = &h.state().session;
    assert_eq!(s.documents().len(), 2, "a new document for the shared one");
    let new = s.active().unwrap().doc.id.0;
    assert_ne!(new, mine);
    assert_eq!(h.state().kopano.pinned, Some(new), "the service works on the new document only");
    assert!(s.documents().iter().any(|d| d.doc.id.0 == mine && d.doc.name == "Portrait"), "the open work is still there");
}

/// Review finding 1: the service reads and updates the shared document, whichever is in front.
#[test]
fn the_service_works_on_the_shared_document_not_the_one_in_front() {
    let (fake, config, _home) = fake_service();
    let mut h = harness(config, true);
    h.state_mut().kopano.start_control = Some(start_control_counting(Arc::default()));
    let shared = h.state().session.active().unwrap().doc.id.0;
    ok(run(&mut h, "kopano.share", json!({"name": "Portrait together", "account": "a@example.com"})));
    wait("the share", T, || fake.requests("create-project").len() == 1);
    assert_eq!(h.state().kopano.pinned, Some(shared));
    let a = ok(run(&mut h, "kopano.export", json!({})));
    // the person opens other work in front
    ok(run(&mut h, "file.new", json!({"name": "Other", "width": 32, "height": 32, "background": "black"})));
    let other = h.state().session.active().unwrap().doc.id.0;
    assert_ne!(other, shared);
    // what the service asks for (no document named) is about the shared document
    let b = ok(run(&mut h, "kopano.export", json!({})));
    assert_eq!(a["manifest"]["document"], b["manifest"]["document"]);
    let before = h.state().session.active().unwrap().revision;
    let r = run(&mut h, "kopano.import", json!({"manifest": a["manifest"]}));
    assert_eq!(r["ok"], false, "an update never lands in the document in front: {r}");
    assert_eq!(h.state().session.active().unwrap().doc.id.0, other);
    assert_eq!(h.state().session.active().unwrap().revision, before, "the other document is untouched");
}

/// Review finding 2: after a restart, the saved file of a shared document is open here: the
/// window asks before linking it again, and links only on "Work together again".
#[test]
fn after_a_restart_the_window_asks_before_linking_its_shared_document_again() {
    let (fake, config, home) = fake_service();
    // the document was bound to a port nobody answers on any more (Photo restarted)
    let dead = TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    fake.data.app_port.store(usize::from(dead), Ordering::SeqCst);
    let mut h = harness(config, true);
    h.state_mut().kopano.control = Some(ControlLink { port: 18777, token_file: PathBuf::from("T:/k/photo/control-18777.token") });
    h.state_mut().session.active_mut().unwrap().path = Some("T:/work/portrait.pcraft".into());
    write_record(&home, PID, "T:/work/portrait.pcraft").unwrap();
    ok(run(&mut h, "kopano.panel", json!({})));
    assert!(shows(&mut h, "Work on it together again?"), "{}", drawn_text(&h));
    no_protocol_words(&drawn_text(&h));
    assert!(!drawn_text(&h).contains("portrait.pcraft"), "no file paths on screen");
    for _ in 0..50 {
        h.step();
    }
    assert!(fake.requests("attach").is_empty(), "never linked without asking");
    click(&mut h, &format!("button:kopano-relink-{}", short(PID)));
    assert!(step_until(&mut h, |_| fake.data.app_port.load(Ordering::SeqCst) == 18777), "attach not sent: {:?}", fake.requests("attach"));
    assert_eq!(fake.requests("attach")[0]["app"], json!({"kind": "photo-control", "port": 18777, "token_file": "T:/k/photo/control-18777.token"}));
    assert!(step_until(&mut h, |h| h.state().kopano.client.state().activity.iter().any(|a| a.said.text() == "This window is linked to Portrait again")));
    assert_eq!(h.state().kopano.pinned, Some(h.state().session.active().unwrap().doc.id.0));
    for _ in 0..50 {
        h.step();
    }
    assert_eq!(fake.requests("attach").len(), 1, "attached once");
}

/// Review finding 2: white tiles are no identity. A blank document whose tiles match, or a
/// different saved file, is not offered; "Not now" is remembered.
#[test]
fn a_document_that_only_looks_alike_is_never_linked() {
    let (fake, config, home) = fake_service();
    let dead = TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    fake.data.app_port.store(usize::from(dead), Ordering::SeqCst);
    let mut h = harness(config, true);
    h.state_mut().kopano.control = Some(ControlLink { port: 18778, token_file: PathBuf::from("T:/x.token") });
    // the old kind of record: this white document's own tiles
    let export = h.state_mut().session.execute("kopano.export", json!({})).unwrap();
    let mut ours = BTreeSet::new();
    tile_hashes(&export["manifest"]["document"], &mut ours);
    write_record(&home, PID, "placeholder").unwrap();
    std::fs::write(record_path(&home, PID).unwrap(), json!({"tiles": ours}).to_string()).unwrap();
    assert!(step_until(&mut h, |h| h.state().kopano.client.state().projects.first().is_some_and(|p| p.app_alive == Some(false))));
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(3) {
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(h.state().kopano.offers.is_empty() && fake.requests("attach").is_empty());
    // a different saved file: not offered either
    h.state_mut().session.active_mut().unwrap().path = Some("T:/work/other.pcraft".into());
    write_record(&home, PID, "T:/work/portrait.pcraft").unwrap();
    for _ in 0..60 {
        h.step();
        std::thread::sleep(Duration::from_millis(40));
    }
    assert!(h.state().kopano.offers.is_empty());
    // the same file: offered; Not now: not offered again
    h.state_mut().session.active_mut().unwrap().path = Some("T:/work/portrait.pcraft".into());
    assert!(step_until(&mut h, |h| !h.state().kopano.offers.is_empty()));
    ok(run(&mut h, "kopano.notNow", json!({"project": PID})));
    for _ in 0..60 {
        h.step();
        std::thread::sleep(Duration::from_millis(40));
    }
    assert!(h.state().kopano.offers.is_empty() && fake.requests("attach").is_empty());
}

/// Review finding 7: problems are said in plain words, never with a file path, the service's
/// words or the protocol's name.
#[test]
fn problems_are_said_in_plain_words() {
    for raw in [
        "the collaboration key file is missing (C:\\Users\\x\\AppData\\Local\\Kopano\\api.token)",
        "bad token",
        "unknown op create-project",
        "Photo command kopano.export: Photo stopped answering",
    ] {
        let said = plain(raw);
        assert!(!said.contains('\\') && !said.contains('/') && !said.to_lowercase().contains("kopano") && !said.contains("op "), "{raw} -> {said}");
    }
    assert_eq!(plain("bad token"), "The collaboration service is not running. Restart Photo.");
    assert_eq!(plain("Give the shared document a name"), "Give the shared document a name");
    // on screen: a broken key file
    let home = temp_home();
    std::fs::write(home.join("api.token"), "").unwrap();
    let config = Config { home: Some(home.clone()), port: 1 };
    let mut h = harness(config, true);
    ok(run(&mut h, "kopano.panel", json!({})));
    assert!(step_until(&mut h, |h| h.state().kopano.client.state().problem.is_some()));
    for _ in 0..10 {
        h.step();
    }
    let p = drawn_text(&h);
    no_protocol_words(&p);
    assert!(!p.contains("api.token") && !p.contains(&home.display().to_string()), "{p}");
}

#[test]
fn the_owner_reviews_views_approves_and_declines_from_the_window() {
    let (fake, config, _home) = fake_service();
    fake.data.approve_owner.store(true, Ordering::SeqCst);
    let mut h = harness(config, true);
    h.state_mut().kopano.start_control = Some(start_control_counting(Arc::default()));
    // the service's layer key for this document's only layer, as an update writes it
    let layer = h.state().session.active().unwrap().doc.layers[0].id.0;
    std::sync::Arc::make_mut(&mut h.state_mut().session.active_mut().unwrap().doc)
        .metadata
        .text
        .push((LAYER_KEYS.into(), json!({layer.to_string(): UID}).to_string()));
    ok(run(&mut h, "kopano.panel", json!({})));
    assert!(
        step_until(&mut h, |h| {
            let p = drawn_text(h);
            p.contains("Changes to review") && p.contains("Member1 changed 1 layer") && p.contains("You approve changes")
        }),
        "{}",
        drawn_text(&h)
    );
    let p = drawn_text(&h);
    no_protocol_words(&p);
    let name = h.state().session.active().unwrap().doc.layers[0].name.clone();
    assert!(p.contains(&format!("{name} · Opacity")) && p.contains("Keep yours") && p.contains("Take theirs"), "{p}");
    let tag = short(PID);
    click(&mut h, &format!("button:kopano-view-{tag}-acct-m1"));
    wait("the view", T, || fake.requests("view").len() == 1);
    assert_eq!(fake.requests("view")[0]["show"], "member");
    assert_eq!(fake.requests("view")[0]["member"], "acct-m1");
    assert!(shows(&mut h, "Showing their version of the document"), "{}", drawn_text(&h));
    // the clash: keep main's value, then approve with that pick
    click(&mut h, &format!("button:kopano-keepMain-{tag}-{}-acct-m1", short(UID)));
    click(&mut h, &format!("button:kopano-approve-{tag}-acct-m1"));
    wait("the approve", T, || fake.requests("approve").len() == 1);
    let a = &fake.requests("approve")[0];
    assert_eq!((a["member"].as_str(), a["version"].as_str()), (Some("acct-m1"), Some("v1")));
    assert_eq!(a["choices"], json!({format!("layer/{UID}"): "main"}));
    assert!(shows(&mut h, "Approved. The changes are in your document."), "{}", drawn_text(&h));
    click(&mut h, &format!("button:kopano-decline-{tag}-acct-m1"));
    wait("the decline", T, || fake.requests("decline").len() == 1);
    assert!(shows(&mut h, "Declined. Your document stays as it was."), "{}", drawn_text(&h));
    // Share asks how to work: "I approve changes" unless live is picked
    type_into(&mut h, "kopano:shareName", "Portrait together");
    type_into(&mut h, "kopano:shareAccount", "acct-m1");
    assert!(drawn_text(&h).contains("I approve changes") && drawn_text(&h).contains("Work together live"));
    click(&mut h, "button:kopano-share");
    wait("the share", T, || fake.requests("create-project").len() == 1);
    assert_eq!(fake.requests("create-project")[0]["mode"], "approve");
    assert!(step_until(&mut h, |h| !h.state().kopano.client.state().job.as_ref().is_some_and(|j| j.busy)));
    click(&mut h, "button:kopano-modeLive");
    assert!(shows(&mut h, "Every edit goes to everyone at once."));
    // a sent invite clears the fields (the teammate's email does not stay on screen)
    assert!(h.state().kopano.share_account.is_empty() && h.state().kopano.share_name.is_empty());
    type_into(&mut h, "kopano:shareName", "Portrait together");
    type_into(&mut h, "kopano:shareAccount", "acct-m1");
    click(&mut h, "button:kopano-share");
    wait("the second share", T, || fake.requests("create-project").len() == 2);
    assert_eq!(fake.requests("create-project")[1]["mode"], "live");
}

#[test]
fn a_member_sees_where_his_changes_are() {
    let (fake, config, _home) = fake_service();
    fake.data.approve_member.store(true, Ordering::SeqCst);
    let mut h = harness(config, true);
    ok(run(&mut h, "kopano.panel", json!({})));
    assert!(shows(&mut h, "Waiting for Alice's approval"), "{}", drawn_text(&h));
    let p = drawn_text(&h);
    assert!(p.contains("Alice approves changes") && p.contains("You changed this document") && !p.contains("Changes to review"), "{p}");
    no_protocol_words(&p);
}

#[test]
fn a_live_clash_offers_both_values() {
    let (fake, config, _home) = fake_service();
    fake.data.live_clash.store(true, Ordering::SeqCst);
    let mut h = harness(config, true);
    ok(run(&mut h, "kopano.panel", json!({})));
    assert!(shows(&mut h, "Pick the one to keep"), "{}", drawn_text(&h));
    let p = drawn_text(&h);
    assert!(p.contains("Document · Resolution dpi") && p.contains("300") && p.contains("72") && p.contains("Works live"), "{p}");
    click(&mut h, &format!("button:kopano-pickB-{}-0", short(PID)));
    wait("the resolve", T, || fake.requests("resolve").len() == 1);
    assert_eq!(fake.requests("resolve")[0]["pick"], "b");
    assert_eq!(fake.requests("resolve")[0]["clash"], "kopano/conflict/k/11-22");
    no_protocol_words(&drawn_text(&h));
}

#[test]
fn one_window_shares_one_document() {
    // the service drives the window's active document: a second Share or Join is refused
    let (fake, config, _home) = fake_service();
    fake.data.app_port.store(18999, Ordering::SeqCst);
    let mut h = harness(config, true);
    h.state_mut().kopano.control = Some(ControlLink { port: 18999, token_file: PathBuf::from("T:/x.token") });
    ok(run(&mut h, "kopano.panel", json!({})));
    assert!(shows(&mut h, "in this window"), "{}", drawn_text(&h));
    for (cmd, params) in [("kopano.share", json!({"name": "Another", "account": "acct-m1"})), ("kopano.join", json!({"project": NEW_PID, "from": M1}))] {
        let r = run(&mut h, cmd, params);
        assert!(r["error"].as_str().unwrap_or_default().contains("This window already shares Portrait"), "{cmd}: {r}");
    }
    assert!(fake.requests("create-project").is_empty() && fake.requests("join").is_empty());
}

#[test]
fn a_change_seen_by_a_poll_before_its_event_is_said_once() {
    let review = json!({"is_owner": true, "approver": true, "owner_name": "Alice",
        "pending": [{"account": "acct-m1", "name": "Member1", "version": "v1", "keys": ["layer/a@pixels@0_0"]}]});
    let mut s = state_with_project();
    s.apply_status(PID, &json!({"teammates": [], "mode": "approve", "review": review}));
    let mut ev = review.clone();
    ev["event"] = json!("review");
    ev["project"] = json!(PID);
    s.apply_event(&ev, 2000.0);
    s.apply_status(PID, &json!({"teammates": [], "mode": "approve", "review": review}));
    let lines: Vec<String> = s.activity.iter().map(|a| a.said.text()).collect();
    assert_eq!(lines, ["Member1 changed 1 layer · waiting for your review"]);
    // the member's side: a poll sees "approved" first
    let mut m = state_with_project();
    m.apply_status(PID, &json!({"teammates": [], "review": {"owner_name": "Alice", "mine": {"state": "waiting"}}}));
    m.apply_status(PID, &json!({"teammates": [], "review": {"owner_name": "Alice", "mine": {"state": "approved"}}}));
    m.apply_event(&json!({"event": "review", "project": PID, "owner_name": "Alice", "mine": {"state": "approved"}}), 3000.0);
    assert_eq!(m.activity.iter().map(|a| a.said.text()).collect::<Vec<_>>(), ["Alice approved your changes"]);
}

/// Review finding 5: two members changed the same setting. Each member's clash picks are their
/// own: approving Member1 sends only Member1's picks and keeps Member2's for later.
#[test]
fn clash_picks_belong_to_one_member() {
    let (fake, config, _home) = fake_service();
    fake.data.approve_owner.store(true, Ordering::SeqCst);
    fake.data.two_members.store(true, Ordering::SeqCst);
    let mut h = harness(config, true);
    h.state_mut().kopano.start_control = Some(start_control_counting(Arc::default()));
    let layer = h.state().session.active().unwrap().doc.layers[0].id.0;
    std::sync::Arc::make_mut(&mut h.state_mut().session.active_mut().unwrap().doc)
        .metadata
        .text
        .push((LAYER_KEYS.into(), json!({layer.to_string(): UID}).to_string()));
    ok(run(&mut h, "kopano.panel", json!({})));
    assert!(step_until(&mut h, |h| drawn_text(h).contains("Member2 changed 1 layer")), "{}", drawn_text(&h));
    let tag = short(PID);
    // Member1: keep yours; Member2: take theirs, then keep yours
    click(&mut h, &format!("button:kopano-keepMain-{tag}-{}-acct-m1", short(UID)));
    click(&mut h, &format!("button:kopano-takeMember-{tag}-{}-acct-m2", short(UID)));
    ok(run(&mut h, "kopano.choose", json!({"project": PID, "member": "acct-m2", "key": format!("layer/{UID}"), "pick": "main"})));
    ok(run(&mut h, "kopano.choose", json!({"project": PID, "member": "acct-m2", "key": "layer/other", "pick": "member"})));
    click(&mut h, &format!("button:kopano-approve-{tag}-acct-m1"));
    wait("the approve", T, || fake.requests("approve").len() == 1);
    assert_eq!(fake.requests("approve")[0]["choices"], json!({format!("layer/{UID}"): "main"}), "only Member1's picks");
    // Member2's picks survive Member1's approval
    let left: Vec<_> = h.state().kopano.choices.keys().cloned().collect();
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(left.iter().all(|(_, who, _)| who == "acct-m2"), "{left:?}");
    assert!(step_until(&mut h, |h| !h.state().kopano.client.state().job.as_ref().is_some_and(|j| j.busy)));
    ok(run(&mut h, "kopano.decline", json!({"project": PID, "member": "acct-m2", "version": "v2"})));
    assert!(h.state().kopano.choices.is_empty());
    // a pick without the member is refused
    assert_eq!(run(&mut h, "kopano.choose", json!({"project": PID, "key": "layer/x", "pick": "main"}))["ok"], false);
}

/// Sign in with ThabangVision: the code to check in the browser, Cancel, and Sign out.
#[test]
fn signing_in_shows_the_code_and_can_be_stopped_and_signed_out() {
    let (fake, config, _home) = fake_service();
    fake.data.signing_in.store(true, Ordering::SeqCst);
    let mut h = harness(config, true);
    ok(run(&mut h, "kopano.panel", json!({})));
    assert!(shows(&mut h, "Check that the browser shows BCDF-1234"), "{}", drawn_text(&h));
    assert!(drawn_text(&h).contains("Approve this computer there") && !drawn_text(&h).contains("Shared documents"));
    click(&mut h, "button:kopano-signInCancel");
    wait("the cancel", T, || fake.requests("sign-in-cancel").len() == 1);
    assert!(shows(&mut h, "Sign-in stopped."), "{}", drawn_text(&h));
    // approved in the browser: signed in, and Sign out is there
    fake.data.signing_in.store(false, Ordering::SeqCst);
    assert!(shows(&mut h, "Signed in to ThabangVision as Alice"), "{}", drawn_text(&h));
    click(&mut h, "button:kopano-signOut");
    wait("the sign-out", T, || fake.requests("sign-out").len() == 1);
    assert!(shows(&mut h, "Signed out of ThabangVision on this computer."), "{}", drawn_text(&h));
    no_protocol_words(&drawn_text(&h));
}

/// Share by email: the document is made, then each email is invited; the answer is "Invite
/// sent". Live mode cannot take invites, so it is refused up front in plain words.
#[test]
fn share_by_email_invites_each_teammate() {
    let (fake, config, _home) = fake_service();
    let mut h = harness(config, true);
    h.state_mut().kopano.start_control = Some(start_control_counting(Arc::default()));
    let r = run(&mut h, "kopano.share", json!({"name": "Poster", "email": "bongi@example.com", "mode": "live"}));
    assert!(r["error"].as_str().unwrap().contains("I approve changes"), "{r}");
    ok(run(&mut h, "kopano.share", json!({"name": "Poster", "email": "bongi@example.com, sipho@example.com"})));
    wait("the invites", T, || fake.requests("invite").len() == 2);
    let made = fake.requests("create-project");
    assert_eq!(made[0]["members"], json!([]), "emails are invited, not made members");
    let inv = fake.requests("invite");
    assert_eq!((inv[0]["project"].as_str(), inv[0]["email"].as_str()), (Some(NEW_PID), Some("bongi@example.com")));
    assert_eq!(inv[1]["email"], "sipho@example.com");
    ok(run(&mut h, "kopano.panel", json!({})));
    assert!(shows(&mut h, "Shared this document. Invite sent."), "{}", drawn_text(&h));
    assert!(!drawn_text(&h).contains("@example.com"), "no emails on screen: {}", drawn_text(&h));
}

/// The people on a shared document by name and role (never emails); the owner invites and
/// takes people off; a member may leave.
#[test]
fn people_by_name_with_invite_remove_and_leave() {
    let (fake, config, _home) = fake_service();
    fake.data.people.store(true, Ordering::SeqCst);
    let mut h = harness(config, true);
    ok(run(&mut h, "kopano.panel", json!({})));
    assert!(shows(&mut h, "Bongi"), "{}", drawn_text(&h));
    let p = drawn_text(&h);
    assert!(p.contains("Sipho") && p.contains("invited") && p.contains("can edit") && !p.contains('@'), "{p}");
    let tag = short(PID);
    type_into(&mut h, &format!("kopano:invite-{tag}"), "thandi@example.com");
    click(&mut h, &format!("button:kopano-invite-{tag}"));
    wait("the invite", T, || fake.requests("invite").len() == 1);
    assert_eq!((fake.requests("invite")[0]["project"].as_str(), fake.requests("invite")[0]["email"].as_str()), (Some(PID), Some("thandi@example.com")));
    assert!(shows(&mut h, "Invite sent."), "{}", drawn_text(&h));
    assert!(!drawn_text(&h).contains("thandi@example.com"), "the email does not stay on screen");
    click(&mut h, &format!("button:kopano-remove-{tag}-u-bongi"));
    wait("the removal", T, || fake.requests("remove-person").len() == 1);
    assert_eq!(fake.requests("remove-person")[0]["user"], "u-bongi");
    // a member: no invite or remove, but Leave
    fake.data.member_role.store(true, Ordering::SeqCst);
    assert!(step_until(&mut h, |h| h.state().kopano.widgets.contains_key(&format!("button:kopano-leave-{tag}"))));
    assert!(!h.state().kopano.widgets.contains_key(&format!("button:kopano-invite-{tag}")));
    click(&mut h, &format!("button:kopano-leave-{tag}"));
    wait("the leave", T, || fake.requests("leave").len() == 1);
    assert!(shows(&mut h, "You left the document."), "{}", drawn_text(&h));
}

/// "Shared with me": Open here brings it into a new document with nothing typed.
#[test]
fn shared_with_me_opens_in_a_new_document() {
    let (fake, config, _home) = fake_service();
    fake.data.shared_rows.store(true, Ordering::SeqCst);
    let mut h = harness(config, true);
    h.state_mut().kopano.start_control = Some(start_control_counting(Arc::default()));
    let mine = h.state().session.active().unwrap().doc.id.0;
    ok(run(&mut h, "kopano.panel", json!({})));
    assert!(shows(&mut h, "Lindiwe's poster"), "{}", drawn_text(&h));
    click(&mut h, &format!("button:kopano-openShared-{}", short("tv-shared-77")));
    wait("the open", T, || fake.requests("join-shared").len() == 1);
    let r = &fake.requests("join-shared")[0];
    assert_eq!(r["shared"], "tv-shared-77");
    assert_eq!(r["app"]["kind"], "photo-control");
    let s = &h.state().session;
    assert_eq!(s.documents().len(), 2);
    assert_ne!(s.active().unwrap().doc.id.0, mine, "a new document, the open one kept");
    assert_eq!(h.state().kopano.pinned, Some(s.active().unwrap().doc.id.0));
}
