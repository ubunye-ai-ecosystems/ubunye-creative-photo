//! The Share & Collaborate window (the people button at the right of the top bar, and
//! Window > Share & Collaborate): ThabangVision collaboration for the document in this window.
//! This file is Ubunye's own (not upstream PhotoCraft's), so upstream merges never touch it. It
//! follows Ubunye Light's `crates/ui-egui/src/ubunye/kopano_panel.rs`.
//!
//! People see ThabangVision collaboration only: who they are signed in as, the shared documents
//! with their teammates by name (green dot when online), recent activity in plain words, and
//! Share. No protocol names, device ids or project ids outside the collapsed "Advanced (testing)"
//! section, which keeps the id-based Join until ThabangVision email invites reach the teammate
//! by themselves. Invites are by email; the window shows teammates by name, never their emails,
//! and the answer to an invite is always "Invite sent". The code keeps the protocol's name (Kopano) internally.
//!
//! Everything is a command: `kopano.panel` shows or hides the window, `kopano.share` and
//! `kopano.join` are what its buttons run, `kopano.signIn` is the sign-in button,
//! `kopano.status` is what it shows (with each button's screen rect under `widgets`, for agents
//! and tests that press them).
//!
//! Two ways to work, chosen when sharing: "Work together live" (every edit goes to everyone) and
//! "I approve changes" (the default: teammates' edits wait in "Changes to review" until the
//! owner approves them; `kopano.approve`, `kopano.decline`, `kopano.view`, `kopano.choose`,
//! `kopano.setApprover`). Live clashes get a pick (`kopano.resolve`).
//!
//! The service reaches this window through Photo's control port, which needs a token. With
//! `--control PORT` that is the port (and its token file); without it, the first Share or Join
//! starts the control server on demand ([`StartControl`], supplied by the desktop app) with a
//! fresh token in a file only this user can read, and hands both to the service.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use egui::{Color32, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, vec2};
use serde_json::{Value, json};

use super::kopano::{self, Client, Config, Job, State};
use crate::PhotocraftApp;
use crate::control::ControlRequest;
use crate::theme::Tokens;

/// The control port the service reaches this window on, and the file holding its token.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlLink {
    pub port: u16,
    pub token_file: PathBuf,
}

/// What [`StartControl`] gives back: the link, and the request channel when this call started the
/// server (the app then serves it like one started with `--control`).
pub struct Started {
    pub link: ControlLink,
    pub rx: Option<Receiver<ControlRequest>>,
}

/// Start the control server if it isn't running (the desktop app supplies it). It may also set
/// the automation gates in `Services`, the same ones `--control` sets.
pub type StartControl = Box<dyn FnMut(&egui::Context, &mut crate::Services) -> Result<Started, String>>;

/// The window's title, menu item and top-bar tooltip.
pub const TITLE: &str = "Share & Collaborate";
/// Online teammates' dot.
const GREEN: Color32 = Color32::from_rgb(0x4c, 0xc3, 0x6b);
/// Activity lines shown (the rest are in `kopano.status`).
const SHOWN: usize = 6;
/// The window's width.
const WIDTH: f32 = 340.0;

pub struct KopanoPanel {
    pub open: bool,
    /// The "Advanced (testing)" section is open.
    pub advanced: bool,
    /// ... and was open last frame (opening it scrolls it into view).
    advanced_shown: bool,
    /// The window's contents' height last frame (the window takes it, up to what fits).
    content_h: f32,
    pub client: Client,
    /// From the desktop app; none on the web and in tests that don't give one.
    pub start_control: Option<StartControl>,
    /// The control port (and token file) the service was given, once known.
    pub control: Option<ControlLink>,
    pub share_name: String,
    pub share_account: String,
    /// "approve" (default) or "live": how the next Share works.
    pub share_mode: String,
    /// The owner's pick per clash before approving: (project, item key) -> "member" | "main".
    pub choices: BTreeMap<(String, String), String>,
    pub join_project: String,
    pub join_from: String,
    /// Shared documents an `attach` was sent for (once each).
    pub attached: BTreeSet<String>,
    /// The last thing Copy put on the clipboard.
    pub copied: Option<String>,
    /// Screen rects of the window's buttons and fields last frame (`kopano.status` `widgets`).
    pub widgets: BTreeMap<String, [f32; 4]>,
    /// When the background work last ran (wall ms).
    last_tick: f64,
    /// This document's tiles as last listed: (when, document id, revision, tiles).
    ours: (f64, u64, u64, BTreeSet<String>),
    /// What was last written to each owned document's record.
    recorded: BTreeMap<String, BTreeSet<String>>,
}

impl Default for KopanoPanel {
    fn default() -> Self {
        KopanoPanel::with_config(Config::from_env())
    }
}

impl KopanoPanel {
    pub fn with_config(config: Config) -> Self {
        KopanoPanel {
            open: false,
            advanced: false,
            advanced_shown: false,
            content_h: 0.0,
            client: Client::new(config),
            start_control: None,
            control: None,
            share_name: String::new(),
            share_account: String::new(),
            share_mode: "approve".to_string(),
            choices: BTreeMap::new(),
            join_project: String::new(),
            join_from: String::new(),
            attached: BTreeSet::new(),
            copied: None,
            widgets: BTreeMap::new(),
            last_tick: f64::NEG_INFINITY,
            ours: (f64::NEG_INFINITY, 0, 0, BTreeSet::new()),
            recorded: BTreeMap::new(),
        }
    }
}

/// Plain English for now; the panel's words are not in the catalogs yet (the menu item is).
fn tr(s: &str) -> &str {
    crate::i18n::t(s)
}

/// The link the service reaches this window on, starting the control server if needed.
fn control_link(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<ControlLink, String> {
    if let Some(l) = &app.kopano.control {
        return Ok(l.clone());
    }
    let Some(start) = app.kopano.start_control.as_mut() else {
        return Err("This window can't be shared: it has no control port (start Photo with --control PORT --control-token-file FILE)".to_string());
    };
    let started = start(ctx, &mut app.services)?;
    if let Some(rx) = started.rx {
        app.control_rx = Some(rx);
    }
    app.kopano.control = Some(started.link.clone());
    Ok(started.link)
}

fn text_param(p: &Value, key: &str, fallback: &str) -> String {
    p.get(key).and_then(Value::as_str).unwrap_or(fallback).trim().to_string()
}

fn set_job(app: &PhotocraftApp, ok: bool, text: impl Into<String>) {
    kopano::lock(&app.kopano.client.shared).job = Some(Job { busy: false, ok, text: text.into(), project: None });
}

/// The collaboration commands; `None` for any other id.
pub fn command(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "kopano.panel" => {
            app.kopano.open = p.get("open").and_then(Value::as_bool).unwrap_or(!app.kopano.open);
            if let Some(a) = p.get("advanced").and_then(Value::as_bool) {
                app.kopano.advanced = a;
            }
            if app.kopano.open {
                app.kopano.client.start(ctx);
            }
            ctx.request_repaint();
            Ok(json!({"open": app.kopano.open, "advanced": app.kopano.advanced}))
        }
        "kopano.signIn" => {
            // ThabangVision sign-in (the device certificate) is not built yet
            let text = "Sign-in is coming soon.";
            set_job(app, true, text);
            Ok(json!({"message": text}))
        }
        "kopano.share" => share(app, ctx, p),
        "kopano.join" => join(app, ctx, p),
        "kopano.mode" => {
            let m = text_param(p, "mode", "approve");
            if m != "live" && m != "approve" {
                return Some(Err("The way to work is live or approve".to_string()));
            }
            app.kopano.share_mode = m;
            Ok(json!({"mode": app.kopano.share_mode}))
        }
        "kopano.approve" => review_op(app, ctx, "approve", p),
        "kopano.decline" => review_op(app, ctx, "decline", p),
        "kopano.view" => review_op(app, ctx, "view", p),
        "kopano.choose" => {
            let (project, key, pick) = (text_param(p, "project", ""), text_param(p, "key", ""), text_param(p, "pick", "member"));
            if project.is_empty() || key.is_empty() || (pick != "member" && pick != "main") {
                return Some(Err("Say which document, which setting, and member or main".to_string()));
            }
            app.kopano.choices.insert((project, key), pick.clone());
            Ok(json!({"pick": pick}))
        }
        "kopano.setApprover" => {
            let req = json!({"op": "set-approver", "project": text_param(p, "project", ""), "account": text_param(p, "account", ""),
                "approver": p.get("approver").and_then(Value::as_bool).unwrap_or(true)});
            app.kopano.client.request(ctx, "Saving…", req).map(|()| json!({"started": true}))
        }
        "kopano.resolve" => {
            let req =
                json!({"op": "resolve", "project": text_param(p, "project", ""), "clash": text_param(p, "clash", ""), "pick": text_param(p, "pick", "a")});
            app.kopano.client.request(ctx, "Settling…", req).map(|()| json!({"started": true}))
        }
        "kopano.status" => {
            let state = app.kopano.client.state();
            let mut v = serde_json::to_value(&state).unwrap_or_default();
            if let Some(o) = v.as_object_mut() {
                o.insert("shareMode".into(), json!(app.kopano.share_mode));
                o.insert("open".into(), json!(app.kopano.open));
                o.insert("advanced".into(), json!(app.kopano.advanced));
                o.insert("controlPort".into(), json!(app.kopano.control.as_ref().map(|c| c.port)));
                o.insert("apiPort".into(), json!(app.kopano.client.config.port));
                o.insert("widgets".into(), json!(app.kopano.widgets));
                let k = &app.kopano;
                o.insert(
                    "fields".into(),
                    json!({"shareName": k.share_name, "shareAccount": k.share_account, "joinProject": k.join_project, "joinFrom": k.join_from}),
                );
            }
            Ok(v)
        }
        _ => return None,
    })
}

/// `kopano.share {name, email, mode}`: share the document in this window; `email` names the
/// teammates who may join (comma separated; `account` is the same). Without params, the window's
/// fields.
fn share(app: &mut PhotocraftApp, ctx: &egui::Context, p: &Value) -> Result<Value, String> {
    let name = text_param(p, "name", &app.kopano.share_name);
    let accounts: Vec<String> = text_param(p, "email", &text_param(p, "account", &app.kopano.share_account))
        .split([',', ' ', ';'])
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(str::to_string)
        .collect();
    if app.session.active().is_none() {
        return Err("Open the document you want to share first".to_string());
    }
    if name.is_empty() {
        return Err("Give the shared document a name".to_string());
    }
    if accounts.is_empty() {
        return Err("Type your teammate's email, so they can join".to_string());
    }
    let mode = text_param(p, "mode", &app.kopano.share_mode);
    if mode != "live" && mode != "approve" {
        return Err("The way to work is live or approve".to_string());
    }
    one_per_window(app)?;
    let link = control_link(app, ctx)?;
    let members: Vec<Value> = accounts.iter().map(|a| json!({"account": a, "role": "editor", "lanes": []})).collect();
    let req = json!({"op": "create-project", "name": name, "app": kopano::binding(link.port, &link.token_file), "members": members, "mode": mode});
    app.kopano.client.request(ctx, "Sharing…", req)?;
    // the invite is on its way: the teammate's email does not stay on screen
    app.kopano.share_name.clear();
    app.kopano.share_account.clear();
    Ok(json!({"started": true, "controlPort": link.port}))
}

/// The shared document this window is bound to, if any (the service drives the window's active
/// document, so one window shares one document).
fn bound_here(app: &PhotocraftApp, state: &State) -> Option<String> {
    let mine = app.kopano.control.as_ref().map(|c| c.port)?;
    state.projects.iter().find(|p| p.app_port == Some(mine)).map(|p| p.name.clone())
}

fn one_per_window(app: &PhotocraftApp) -> Result<(), String> {
    match bound_here(app, &app.kopano.client.state()) {
        Some(name) => Err(format!("This window already shares {name}. Open the next document in a new Photo window to share it too.")),
        None => Ok(()),
    }
}

/// `kopano.approve|decline|view {project, member, version, show}`: what the review buttons run.
/// Approve sends the owner's picks for the clashes of that member's changes.
fn review_op(app: &mut PhotocraftApp, ctx: &egui::Context, op: &str, p: &Value) -> Result<Value, String> {
    let project = text_param(p, "project", "");
    let member = text_param(p, "member", "");
    let show = text_param(p, "show", "member");
    if project.is_empty() || (member.is_empty() && show != "off") {
        return Err("Say which document and whose changes".to_string());
    }
    let mut req = json!({"op": op, "project": project, "member": member});
    if let Some(v) = p.get("version").and_then(Value::as_str) {
        req["version"] = json!(v);
    }
    let busy = match op {
        "approve" => {
            let mut choices = serde_json::Map::new();
            for ((pid, key), pick) in &app.kopano.choices {
                if *pid == project {
                    choices.insert(key.clone(), json!(pick));
                }
            }
            if let Some(c) = p.get("choices").and_then(Value::as_object) {
                choices.extend(c.clone());
            }
            req["choices"] = Value::Object(choices);
            "Approving…"
        }
        "decline" => "Declining…",
        _ => {
            req["show"] = json!(show);
            "Showing…"
        }
    };
    app.kopano.client.request(ctx, busy, req)?;
    if op == "approve" || op == "decline" {
        app.kopano.choices.retain(|(pid, _), _| *pid != project);
    }
    Ok(json!({"started": true}))
}

/// `kopano.join {project, from}`: join a teammate's shared document into this window. With no
/// document open, a blank one is made first: the shared document comes into it.
fn join(app: &mut PhotocraftApp, ctx: &egui::Context, p: &Value) -> Result<Value, String> {
    let project = text_param(p, "project", &app.kopano.join_project).to_lowercase();
    let from = text_param(p, "from", &app.kopano.join_from).to_lowercase();
    if !kopano::is_hex_id(&project) {
        return Err("The project id is 64 letters and digits (your teammate copies it under Advanced)".to_string());
    }
    if !kopano::is_hex_id(&from) {
        return Err("The teammate's device id is 64 letters and digits (they copy it under Advanced)".to_string());
    }
    one_per_window(app)?;
    if app.session.active().is_none() {
        app.run("file.new", json!({"name": "Shared document", "width": 800, "height": 600, "background": "white"}))?;
    }
    let link = control_link(app, ctx)?;
    let req = json!({"op": "join", "project": project, "from": from, "app": kopano::binding(link.port, &link.token_file)});
    app.kopano.client.request(ctx, "Joining…", req)?;
    Ok(json!({"started": true, "controlPort": link.port}))
}

/// Two people, drawn in code (our own original drawing): the top bar's Share & Collaborate icon.
pub fn paint_icon(p: &egui::Painter, r: Rect, color: Color32) {
    let k = r.width().min(r.height()) / 18.0;
    let at = |x: f32, y: f32| Pos2::new(r.min.x + x * k, r.min.y + y * k);
    let s = Stroke::new(1.4 * k.max(0.7), color);
    // a shoulder line: a half ellipse under a head
    let shoulders = |cx: f32, top: f32, w: f32| -> Vec<Pos2> {
        (0..=12)
            .map(|i| {
                let a = std::f32::consts::PI * (i as f32 / 12.0);
                at(cx - w * a.cos(), top + 5.0 - 5.0 * a.sin())
            })
            .collect()
    };
    // the one behind, then the one in front
    p.circle_stroke(at(12.0, 5.2), 2.4 * k, s);
    p.add(egui::Shape::line(shoulders(12.5, 10.5, 4.2), s));
    p.circle_stroke(at(6.8, 6.6), 2.8 * k, s);
    p.add(egui::Shape::line(shoulders(6.8, 12.6, 5.0), s));
}

/// The top-bar button (right-hand group): opens or closes the window. `size` is the square box.
pub fn top_bar_button(app: &mut PhotocraftApp, ui: &mut egui::Ui, size: f32) {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Share & Collaborate")));
    let tint = crate::icons::button_chrome(ui, rect, app.kopano.open, resp.hovered());
    paint_icon(ui.painter(), Rect::from_center_size(rect.center(), vec2(size * 0.62, size * 0.62)), tint);
    app.kopano.widgets.insert("icon:collaborate".into(), rect_of(rect));
    if resp.on_hover_text(tl!("Share & Collaborate")).clicked() {
        let ctx = ui.ctx().clone();
        if let Err(e) = crate::menus::invoke(app, &ctx, "kopano.panel", json!({})) {
            log::warn!("collaboration: {e}");
        }
    }
}

fn rect_of(r: Rect) -> [f32; 4] {
    [r.min.x, r.min.y, r.width(), r.height()]
}

fn dot(ui: &mut egui::Ui, on: bool) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(10.0, 14.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 4.0, if on { GREEN } else { t.text_faint });
}

fn heading(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(6.0);
    ui.label(RichText::new(tr(text)).font(crate::theme::semibold(12.0)).color(t.text));
}

fn dim(ui: &mut egui::Ui, text: impl Into<String>) {
    let t = Tokens::get(ui.ctx());
    ui.add(egui::Label::new(RichText::new(text.into()).color(t.text_dim).size(11.5)).wrap());
}

/// A button of the window, its rect kept under `id` in `kopano.status` `widgets`.
fn button(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: &str, label: &str, selected: bool) -> bool {
    let r = if selected { crate::widgets::pill_tab(ui, label, true) } else { crate::widgets::secondary_button(ui, tr(label), 0.0) };
    app.kopano.widgets.insert(format!("button:{id}"), rect_of(r.rect));
    r.clicked()
}

/// A choice between two (the way to work): the chosen one looks pressed.
fn choice(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: &str, label: &str, selected: bool) -> bool {
    let r = crate::widgets::pill_tab(ui, tr(label), selected);
    r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, tr(label)));
    app.kopano.widgets.insert(format!("button:{id}"), rect_of(r.rect));
    r.clicked()
}

/// A one-line text field, its rect kept under `id`.
fn field(app_widgets: &mut BTreeMap<String, [f32; 4]>, ui: &mut egui::Ui, id: &str, text: &mut String, hint: &str) {
    let r = ui.add(egui::TextEdit::singleline(text).id(egui::Id::new(id)).hint_text(tr(hint)).desired_width(WIDTH - 10.0));
    app_widgets.insert(format!("field:{id}"), rect_of(r.rect));
}

/// Run a button's command; a refusal shows on the window's last line.
fn press(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str) {
    press_with(app, ctx, id, json!({}));
}

fn press_with(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: Value) {
    if let Err(e) = crate::menus::invoke(app, ctx, id, params) {
        set_job(app, false, e);
    }
}

/// An item of the document in plain words: the layer's name in this window, else "a layer".
fn item_name(app: &PhotocraftApp, key: &str) -> String {
    if kopano::owner_of(key) == "doc" {
        return tr("Document").to_string();
    }
    let Some(want) = kopano::layer_key_of(key) else { return tr("A layer").to_string() };
    let Some(d) = app.session.active() else { return tr("A layer").to_string() };
    // the service's layer keys, as it wrote them into the document (local id -> key)
    let keys: BTreeMap<String, String> =
        d.doc.metadata.text.iter().rev().find(|(k, _)| k == kopano::LAYER_KEYS).and_then(|(_, v)| serde_json::from_str(v).ok()).unwrap_or_default();
    keys.iter()
        .find(|(_, k)| k.as_str() == want)
        .and_then(|(id, _)| id.parse::<u64>().ok())
        .and_then(|id| d.doc.layer(photocraft_doc::LayerId(id)))
        .map(|l| l.name.clone())
        .unwrap_or_else(|| tr("A layer").to_string())
}

/// The "I approve changes" part of one document: who approves, what waits, where my work is.
fn review_section(app: &mut PhotocraftApp, ui: &mut egui::Ui, p: &kopano::Project, r: &kopano::Review) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let tag = kopano::short(&p.id).to_string();
    let who = if r.is_owner { tr("You approve changes").to_string() } else { format!("{} {}", r.owner_name, tr("approves changes")) };
    dim(ui, who);
    if let Some(text) = r.mine_text() {
        let declined = r.mine.as_ref().is_some_and(|m| m.state == "declined");
        ui.add(egui::Label::new(RichText::new(text).color(if declined { t.warning } else { t.text }).size(11.5)).wrap());
    }
    if !r.approver {
        return;
    }
    heading(ui, "Changes to review");
    if r.pending.is_empty() {
        dim(ui, tr("Nothing waiting."));
    }
    for e in &r.pending {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(format!("{} changed {}", e.name, e.what())).color(t.text));
        });
        let viewing = r.viewing.as_ref().filter(|v| v.account == e.account);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let base = json!({"project": p.id, "member": e.account, "version": e.version});
            if viewing.is_none() {
                if button(app, ui, &format!("kopano-view-{tag}-{}", e.account), "View", false) {
                    let mut q = base.clone();
                    q["show"] = json!("member");
                    press_with(app, &ctx, "kopano.view", q);
                }
            } else {
                let side = viewing.map(|v| v.side.as_str()).unwrap_or("member");
                if choice(app, ui, &format!("kopano-before-{tag}"), "Before", side == "main") {
                    let mut q = base.clone();
                    q["show"] = json!("main");
                    press_with(app, &ctx, "kopano.view", q);
                }
                if choice(app, ui, &format!("kopano-after-{tag}"), "After", side == "member") {
                    let mut q = base.clone();
                    q["show"] = json!("member");
                    press_with(app, &ctx, "kopano.view", q);
                }
                if button(app, ui, &format!("kopano-viewDone-{tag}"), "Done", false) {
                    press_with(app, &ctx, "kopano.view", json!({"project": p.id, "show": "off"}));
                }
            }
            if button(app, ui, &format!("kopano-approve-{tag}-{}", e.account), "Approve", false) {
                press_with(app, &ctx, "kopano.approve", base.clone());
            }
            if button(app, ui, &format!("kopano-decline-{tag}-{}", e.account), "Decline", false) {
                press_with(app, &ctx, "kopano.decline", base.clone());
            }
        });
        if let Some(v) = viewing {
            let side = if v.side == "main" { tr("your version (before)").to_string() } else { format!("{}'s version (after)", e.name) };
            dim(ui, format!("{} {side}. {}", tr("This window shows"), tr("Editing waits until you press Done.")));
        }
        for c in &e.clashes {
            let pick = app.kopano.choices.get(&(p.id.clone(), c.key.clone())).cloned().unwrap_or_else(|| "member".to_string());
            dim(ui, format!("{} · {}: {} {} · {} {}", item_name(app, &c.item), kopano::setting_text(&c.setting), tr("yours"), c.main, e.name, c.member));
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let id = kopano::short(kopano::layer_key_of(&c.item).unwrap_or(&c.item)).to_string();
                if choice(app, ui, &format!("kopano-keepMain-{tag}-{id}"), "Keep yours", pick == "main") {
                    press_with(app, &ctx, "kopano.choose", json!({"project": p.id, "key": c.key, "pick": "main"}));
                }
                if choice(app, ui, &format!("kopano-takeMember-{tag}-{id}"), "Take theirs", pick == "member") {
                    press_with(app, &ctx, "kopano.choose", json!({"project": p.id, "key": c.key, "pick": "member"}));
                }
            });
        }
    }
    if !r.declined.is_empty() {
        dim(ui, format!("{} {}", tr("Declined:"), r.declined.join(", ")));
    }
}

/// A live document's clashes: both values of the setting, pick one.
fn clash_section(app: &mut PhotocraftApp, ui: &mut egui::Ui, p: &kopano::Project) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    heading(ui, "Clashes");
    dim(ui, tr("You and a teammate changed the same setting at once. Pick the one to keep."));
    for (i, c) in p.clashes.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(format!("{} · {}", item_name(app, &c.item), kopano::setting_text(&c.setting))).color(t.text));
        });
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let tag = kopano::short(&p.id).to_string();
            if button(app, ui, &format!("kopano-pickA-{tag}-{i}"), &c.a, false) {
                press_with(app, &ctx, "kopano.resolve", json!({"project": p.id, "clash": c.id, "pick": "a"}));
            }
            if button(app, ui, &format!("kopano-pickB-{tag}-{i}"), &c.b, false) {
                press_with(app, &ctx, "kopano.resolve", json!({"project": p.id, "clash": c.id, "pick": "b"}));
            }
        });
    }
}

/// How often the background work runs, and how often this document's tiles are listed again.
const TICK_MS: f64 = 2000.0;
const LIST_MS: f64 = 30000.0;

/// The active document's tiles (as the service names them), listed again at most every 30 s and
/// only after the document changed. `None` with no document open.
fn our_tiles(app: &mut PhotocraftApp, now: f64) -> Option<BTreeSet<String>> {
    let (doc, revision) = app.session.active().map(|d| (d.doc.id.0, d.revision))?;
    let (at, d0, r0, _) = app.kopano.ours;
    if (d0, r0) != (doc, revision) && (now - at >= LIST_MS || d0 != doc) {
        // the same bundle the service reads; unchanged since its last export, it is the cached one
        let tiles = match app.session.execute("kopano.export", json!({})) {
            Ok(v) => {
                let mut out = BTreeSet::new();
                kopano::tile_hashes(v.pointer("/manifest/document").unwrap_or(&Value::Null), &mut out);
                out
            }
            Err(e) => {
                log::warn!("collaboration: could not list this document's tiles: {e}");
                BTreeSet::new()
            }
        };
        app.kopano.ours = (now, doc, revision, tiles);
    }
    Some(app.kopano.ours.3.clone())
}

/// Background work each frame, open or not (only where the window can be reached: the desktop
/// app): follow the service, remember which tiles each document this window shares holds, and
/// after a restart on a new port link the shared documents this document holds back to this
/// window (`attach`).
pub fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.kopano.start_control.is_none() && app.kopano.control.is_none() {
        return;
    }
    let now = kopano::now_ms();
    if now - app.kopano.last_tick < TICK_MS {
        return;
    }
    app.kopano.last_tick = now;
    app.kopano.client.start(ctx);
    let state = app.kopano.client.state();
    let Some(home) = app.kopano.client.config.home.clone().filter(|_| state.running) else { return };
    let mine = app.kopano.control.as_ref().map(|c| c.port);
    for p in &state.projects {
        let Some(port) = p.app_port else { continue };
        if Some(port) == mine {
            let Some(ours) = our_tiles(app, now).filter(|t| !t.is_empty()) else { continue };
            if app.kopano.recorded.get(&p.id) != Some(&ours) {
                match kopano::write_record(&home, &p.id, &ours) {
                    Ok(()) => {
                        app.kopano.recorded.insert(p.id.clone(), ours);
                    }
                    Err(e) => log::warn!("collaboration: could not remember the tiles of {}: {e}", p.name),
                }
            }
        } else if p.app_alive == Some(false) && !app.kopano.attached.contains(&p.id) && bound_here(app, &state).is_none() {
            let Some(ours) = our_tiles(app, now) else { continue };
            if !kopano::holds(&kopano::read_record(&home, &p.id), &ours) {
                continue;
            }
            app.kopano.attached.insert(p.id.clone());
            match control_link(app, ctx) {
                Ok(link) => app.kopano.client.attach(ctx, &p.id, &p.name, kopano::binding(link.port, &link.token_file)),
                Err(error) => kopano::lock(&app.kopano.client.shared).note(Some(p.id.clone()), kopano::Said::AppProblem { error }, now),
            }
        }
    }
}

fn float_frame(t: &Tokens) -> egui::Frame {
    egui::Frame::NONE
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(t.radius_lg as u8))
        .shadow(egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: t.shadow })
        .inner_margin(egui::Margin::same(10))
}

/// Each frame: the background work, then the window when it is open.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    tick(app, ctx);
    if !app.kopano.open {
        app.kopano.widgets.retain(|k, _| k.starts_with("icon:"));
        return;
    }
    app.kopano.client.start(ctx);
    let state = app.kopano.client.state();
    let t = Tokens::get(ctx);
    let canvas = app.last_canvas_rect;
    // over the canvas, left of the panels; below the top bars when no document shows a canvas
    let screen = ctx.content_rect();
    let area = if canvas.width() > WIDTH { canvas } else { screen };
    let x = (area.right() - WIDTH - 36.0).max(area.left() + 8.0);
    let top = area.top().max(screen.top() + 80.0) + 12.0;
    // never over the menus and top bars (a remembered position may be up there)
    let room = Rect::from_min_max(Pos2::new(screen.left(), screen.top() + 80.0), screen.max);
    // a long window scrolls instead of running off the screen; as tall as fits where it is now
    let now_top = ctx.memory(|m| m.area_rect(egui::Id::new("kopano-panel"))).map_or(top, |r| r.top().max(room.top()));
    let max_h = (screen.bottom() - now_top - 40.0).max(200.0);
    // egui keeps a fixed-size window at its first height: size it to last frame's contents
    let want = (app.kopano.content_h + 4.0).clamp(120.0, max_h);
    app.kopano.widgets.retain(|k, _| k.starts_with("icon:"));
    let mut close = false;
    let shown = egui::Window::new(tl!("Share & Collaborate"))
        .id(egui::Id::new("kopano-panel"))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .frame(float_frame(&t))
        .default_pos(egui::pos2(x, top))
        .constrain_to(room)
        .min_width(WIDTH)
        .max_width(WIDTH)
        .vscroll(true)
        .min_height(want)
        .max_height(want)
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 5.0;
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Share & Collaborate")).color(t.text).size(12.5).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let r = crate::icons::button(ui, "x", 20.0, false, "Close");
                    app.kopano.widgets.insert("button:kopano-close".into(), rect_of(r.rect));
                    if r.clicked() {
                        close = true;
                    }
                });
            });
            crate::widgets::hairline(ui);
            body(app, ui, &state);
            let h = ui.min_rect().height();
            if (h - app.kopano.content_h).abs() > 0.5 {
                app.kopano.content_h = h;
                ui.ctx().request_repaint();
            }
        });
    if let Some(r) = shown {
        app.kopano.widgets.insert("region:collaborate".into(), rect_of(r.response.rect));
    }
    if close {
        app.kopano.open = false;
    }
    // "3 s ago" keeps counting
    ctx.request_repaint_after(std::time::Duration::from_secs(1));
}

fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, state: &State) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    match state.me.as_ref().filter(|_| state.running) {
        Some(me) if state.signed_in => {
            ui.horizontal(|ui| {
                dot(ui, true);
                ui.label(RichText::new(format!("{} {}", tr("Signed in to ThabangVision as"), me.name)).font(crate::theme::semibold(12.5)).color(t.text));
            });
        }
        // the service runs, but this computer's ThabangVision sign-in is missing or ran out
        Some(_) => {
            dim(ui, tr("Work on this document together with your team."));
            if button(app, ui, "kopano-signIn", "Sign in with ThabangVision", false) {
                press(app, &ctx, "kopano.signIn");
            }
        }
        None => {
            ui.horizontal(|ui| {
                dot(ui, false);
                ui.label(RichText::new(tr(kopano::NOT_RUNNING)).font(crate::theme::semibold(12.5)).color(t.text));
            });
            dim(ui, tr("Start ThabangVision collaboration on this computer. This window finds it by itself."));
            if let Some(p) = &state.problem
                && p != kopano::NOT_RUNNING
            {
                dim(ui, p.clone());
            }
        }
    }

    heading(ui, "Shared documents");
    if state.projects.is_empty() {
        dim(ui, tr("Nothing shared yet."));
    }
    let mine = app.kopano.control.as_ref().map(|c| c.port);
    for p in &state.projects {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&p.name).font(crate::theme::semibold(12.0)).color(t.text));
            if p.app_port.is_some() && p.app_port == mine {
                ui.label(RichText::new(tr("in this window")).color(t.text_dim).size(11.0));
            }
        });
        if p.teammates.is_empty() {
            dim(ui, tr("No teammate has joined yet."));
        }
        for m in &p.teammates {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                let on = m.online && state.running;
                dot(ui, on);
                ui.label(RichText::new(m.label()).color(t.text));
                ui.label(RichText::new(tr(if on { "online" } else { "offline" })).color(t.text_dim).size(11.5));
            });
        }
        match &p.review {
            Some(r) => review_section(app, ui, p, r),
            None if p.mode.as_deref() == Some("live") => dim(ui, tr("Works live: every edit goes to everyone.")),
            None => {}
        }
        if !p.clashes.is_empty() {
            clash_section(app, ui, p);
        }
    }

    heading(ui, "Recent activity");
    if state.activity.is_empty() {
        dim(ui, tr("Nothing yet."));
    }
    let now = kopano::now_ms();
    // trouble reaching a window is only news in the window it is about
    let elsewhere = |a: &kopano::Activity| {
        matches!(a.said, kopano::Said::AppProblem { .. })
            && state.projects.iter().find(|p| Some(&p.id) == a.project.as_ref()).is_none_or(|p| p.app_port.is_none() || p.app_port != mine)
    };
    for a in state.activity.iter().filter(|a| !elsewhere(a)).take(SHOWN) {
        let times = if a.times > 1 { format!(" · {} times", a.times) } else { String::new() };
        let line = format!("{}{times} · {}", a.said.text(), kopano::ago((now - a.at_ms) / 1000.0));
        let warn = matches!(a.said, kopano::Said::AppProblem { .. } | kopano::Said::Merged { clashes: 1.., .. });
        ui.add(egui::Label::new(RichText::new(line).color(if warn { t.warning } else { t.text }).size(11.5)).wrap());
    }

    ui.add_space(4.0);
    crate::widgets::hairline(ui);
    heading(ui, "Share this document");
    if app.session.active().is_none() {
        dim(ui, tr("Open the document you want to share first."));
    }
    field(&mut app.kopano.widgets, ui, "kopano:shareName", &mut app.kopano.share_name, "Name for the shared document");
    field(&mut app.kopano.widgets, ui, "kopano:shareAccount", &mut app.kopano.share_account, "Teammate's email");
    ui.horizontal(|ui| {
        let approve = app.kopano.share_mode != "live";
        if choice(app, ui, "kopano-modeApprove", "I approve changes", approve) {
            app.kopano.share_mode = "approve".to_string();
        }
        if choice(app, ui, "kopano-modeLive", "Work together live", !approve) {
            app.kopano.share_mode = "live".to_string();
        }
    });
    dim(
        ui,
        tr(if app.kopano.share_mode == "live" {
            "Every edit goes to everyone at once."
        } else {
            "Your teammates see their own edits at once. Your document changes only when you approve."
        }),
    );
    let r = crate::widgets::primary_button(ui, tr("Share"), 90.0);
    app.kopano.widgets.insert("button:kopano-share".into(), rect_of(r.rect));
    if r.clicked() {
        press(app, &ctx, "kopano.share");
    }

    ui.add_space(4.0);
    let header = egui::CollapsingHeader::new(RichText::new(tr("Advanced (testing)")).color(t.text_dim).size(11.5))
        .id_salt("kopano-advanced")
        .open(Some(app.kopano.advanced))
        .show(ui, |ui| advanced(app, ui, state, &ctx));
    app.kopano.widgets.insert("button:kopano-advanced".into(), rect_of(header.header_response.rect));
    if header.header_response.clicked() {
        app.kopano.advanced = !app.kopano.advanced;
    }
    // just opened: bring it to the top of the window, so Join is in view on a short screen
    if app.kopano.advanced && !app.kopano.advanced_shown {
        header.header_response.scroll_to_me(Some(egui::Align::TOP));
    }
    app.kopano.advanced_shown = app.kopano.advanced;

    if let Some(job) = &state.job {
        ui.add_space(4.0);
        let color = if job.busy {
            t.text_dim
        } else if job.ok {
            t.text
        } else {
            t.warning
        };
        ui.add(egui::Label::new(RichText::new(&job.text).color(color).size(11.5)).wrap());
    }
}

/// Ids and the id-based Join, until email invites reach the teammate by themselves.
fn advanced(app: &mut PhotocraftApp, ui: &mut egui::Ui, state: &State, ctx: &egui::Context) {
    let t = Tokens::get(ui.ctx());
    dim(ui, tr("For testing: until email invites arrive by themselves, your teammate joins with these ids."));
    if let Some(me) = &state.me {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} {}", tr("Your device id"), kopano::short(&me.device))).color(t.text_dim).monospace());
            if button(app, ui, "kopano-copyDevice", "Copy", false) {
                copy(app, ctx, &me.device);
            }
        });
    }
    for p in &state.projects {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} {}", p.name, kopano::short(&p.id))).color(t.text_dim).monospace());
            if button(app, ui, &format!("kopano-copyProject-{}", kopano::short(&p.id)), "Copy", false) {
                copy(app, ctx, &p.id);
            }
        });
    }
    heading(ui, "Join a shared document");
    dim(ui, tr("The shared document comes into the document open in this window."));
    field(&mut app.kopano.widgets, ui, "kopano:joinProject", &mut app.kopano.join_project, "Project id");
    field(&mut app.kopano.widgets, ui, "kopano:joinFrom", &mut app.kopano.join_from, "Teammate's device id");
    if button(app, ui, "kopano-join", "Join", false) {
        press(app, ctx, "kopano.join");
    }
}

/// Copy `text` to the clipboard (and say so).
fn copy(app: &mut PhotocraftApp, ctx: &egui::Context, text: &str) {
    ctx.copy_text(text.to_string());
    app.kopano.copied = Some(text.to_string());
    set_job(app, true, tr("Copied"));
}
