//! Ubunye: ThabangVision collaboration for this Photo (`kopanod`, the Kopano service). This file
//! is Ubunye's own (not upstream PhotoCraft's), so upstream merges never touch it. It follows
//! Ubunye Light's `apps/lightcraft/src/ubunye_collab.rs`.
//!
//! Two jobs:
//!
//! - [`start_in_background`]: Light, Photo and Film all carry `kopanod`, but a computer runs one,
//!   whatever apps are installed (`ubunye/packaging/README.md`). Windows: the MSI's shared copy in
//!   `Program Files\ThabangVision\Collaboration`, started at sign-in by a machine-wide value that
//!   Windows Installer keeps while any Ubunye app is installed. Linux `.deb` / `.rpm`:
//!   `/usr/bin/kopanod` and a packaged user unit link to the newest copy installed. macOS: the
//!   LaunchAgent points at the newest app's `kopanod`; an app repoints it only when that file is
//!   gone or older than its own. On every launch, off the UI thread, this finds the shared
//!   `kopanod`, else the one next to the app (none, as in a developer build: nothing happens),
//!   makes the device key once (`kopanod init`), leaves start at sign-in to the installer where
//!   there is one (clearing a per-user entry older builds made), otherwise turns it on
//!   (`kopanod autostart on`) unless it already starts a good `kopanod`, and starts `kopanod`
//!   when nothing answers on its port yet. All of it is silent; a failure is only logged and the
//!   Share & Collaborate window then says collaboration is not running.
//! - [`start_control`]: what the Share & Collaborate window calls on the first Share or Join. The
//!   service drives this window through Photo's control port, which needs a token. Started with
//!   `--control`, that is the port (its token goes into a file for the service when no
//!   `--control-token-file` was given). Otherwise a control server is started now on a free
//!   loopback port with a fresh token in `<KOPANO_HOME>/photo/control-<port>.token`, behind the
//!   same automation gates `--control` sets (no file access).
//!
//! `UBUNYE_COLLAB=0` turns the launch check off, `UBUNYE_COLLAB_AUTOSTART=0` only start at
//! sign-in. `KOPANO_BIN` points at another `kopanod`; `KOPANO_HOME` / `KOPANO_API_PORT` as in
//! `kopanod`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use photocraft_ui_egui::Services;
use photocraft_ui_egui::ubunye::kopano::Config;
use photocraft_ui_egui::ubunye::kopano_panel::{ControlLink, StartControl, Started};

/// The service's file name next to the app.
const EXE: &str = if cfg!(windows) { "kopanod.exe" } else { "kopanod" };
/// What the Run value is called on Windows (kopanod's own `RUN_VALUE`, and the MSI's).
const RUN_VALUE: &str = "ThabangVision collaboration";
/// The shared `kopanod` the MSI installs, under Program Files (`ubunye/packaging/windows/collab.wxs`).
const WINDOWS_SHARED: &str = r"ThabangVision\Collaboration\kopanod.exe";
/// The shared `kopanod` and its start-at-sign-in unit from the Linux packages.
const LINUX_SHARED: &str = "/usr/bin/kopanod";
const LINUX_UNIT_NAME: &str = "thabangvision-collab.service";
const LINUX_UNIT: &str = "/usr/lib/systemd/user/thabangvision-collab.service";
/// The log kopanod writes when this app starts it (kept short: trimmed past this size).
const LOG_MAX: u64 = 4 << 20;

/// Start the check on its own thread; returns at once.
pub fn start_in_background() {
    if off("UBUNYE_COLLAB") {
        return;
    }
    let spawned = std::thread::Builder::new().name("collab-start".into()).spawn(|| {
        if let Err(e) = ensure_running(&Config::from_env()) {
            log::warn!("collaboration service: {e}");
        }
    });
    if let Err(e) = spawned {
        log::warn!("collaboration service: could not start the check: {e}");
    }
}

fn off(var: &str) -> bool {
    std::env::var(var).is_ok_and(|v| matches!(v.trim(), "0" | "off" | "false" | "no"))
}

/// The `kopanod` to use: `KOPANO_BIN`, else the computer's shared one, else next to this program.
pub fn find_kopanod() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KOPANO_BIN").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    if let Some(p) = shared_kopanod() {
        return Some(p);
    }
    let me = std::env::current_exe().ok()?;
    let p = me.parent()?.join(EXE);
    p.is_file().then_some(p)
}

/// The one `kopanod` an installer set up for every app on this computer, with its own start at
/// sign-in (Windows MSI, Linux package). `None` on macOS, for a portable copy or an AppImage alone.
fn shared_kopanod() -> Option<PathBuf> {
    if std::env::var_os("KOPANO_BIN").is_some_and(|p| !p.is_empty()) {
        return None;
    }
    let p = if cfg!(windows) {
        let pf = ["ProgramW6432", "ProgramFiles"].iter().find_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()))?;
        PathBuf::from(pf).join(WINDOWS_SHARED)
    } else if cfg!(target_os = "linux") {
        if !Path::new(LINUX_UNIT).exists() {
            return None;
        }
        PathBuf::from(LINUX_SHARED)
    } else {
        return None;
    };
    p.is_file().then_some(p)
}

/// Is something answering on the service's port?
pub fn answers(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(400)).is_ok()
}

/// Where a per-user start-at-sign-in can point: a path that stays put. An AppImage runs from a
/// temporary mount, so its `kopanod` is copied into the service folder first; a Mac app still on
/// its disk image or not yet moved out of quarantine (App Translocation) gets no autostart until
/// it is. Windows: none (only the MSI's shared copy starts at sign-in; a portable copy can be
/// moved or deleted, which would leave a broken startup entry behind).
pub fn lasting_copy(exe: &Path, home: &Path) -> Result<Option<PathBuf>, String> {
    let s = exe.to_string_lossy();
    if cfg!(target_os = "macos") && (s.contains("/AppTranslocation/") || s.starts_with("/Volumes/")) {
        return Ok(None);
    }
    if cfg!(windows) {
        return Ok(None);
    }
    if cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_some() {
        let dir = home.join("bin");
        let to = dir.join(EXE);
        let same = std::fs::read(&to).ok().zip(std::fs::read(exe).ok()).is_some_and(|(a, b)| a == b);
        if !same {
            std::fs::create_dir_all(&dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
            let tmp = dir.join(format!("{EXE}.new"));
            std::fs::copy(exe, &tmp).map_err(|e| format!("could not copy {}: {e}", exe.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
            }
            std::fs::rename(&tmp, &to).map_err(|e| format!("could not place {}: {e}", to.display()))?;
        }
        return Ok(Some(to));
    }
    Ok(Some(exe.to_path_buf()))
}

/// The per-user start-at-sign-in entry kopanod writes, as text (`None`: there is none).
fn autostart_entry() -> Option<String> {
    if cfg!(windows) {
        let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
        let out = hidden(Command::new("reg").args(["query", key, "/v", RUN_VALUE])).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let home = PathBuf::from(std::env::var_os("HOME")?);
        let f = if cfg!(target_os = "macos") {
            home.join("Library/LaunchAgents/org.ubunye.kopano.plist")
        } else {
            home.join(".config/systemd/user/kopano.service")
        };
        std::fs::read_to_string(f).ok()
    }
}

/// Does the entry start this `exe` with this home and port?
pub fn entry_points_at(entry: &str, exe: &Path, home: &Path, port: u16) -> bool {
    let has = |s: &str| {
        if cfg!(windows) { entry.to_lowercase().contains(&s.to_lowercase()) } else { entry.contains(s) }
    };
    has(&exe.to_string_lossy()) && has(&home.to_string_lossy()) && has(&port.to_string())
}

/// The program a LaunchAgent starts: the first string of its ProgramArguments.
pub fn plist_program(plist: &str) -> Option<PathBuf> {
    let rest = &plist[plist.find("<key>ProgramArguments</key>")?..];
    let start = rest.find("<string>")? + "<string>".len();
    let len = rest[start..].find("</string>")?;
    let s = rest[start..start + len].replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&apos;", "'").replace("&quot;", "\"");
    Some(PathBuf::from(s))
}

/// The version of a `kopanod` inside a Mac app (`Contents/Resources/kopanod.version`, written by
/// the packaging), as numbers. Unknown is empty: older than any known version.
pub fn bundled_version(exe: &Path) -> Vec<u64> {
    exe.parent()
        .and_then(Path::parent)
        .and_then(|c| std::fs::read_to_string(c.join("Resources/kopanod.version")).ok())
        .map(|v| parse_version(&v))
        .unwrap_or_default()
}

/// "0.0.1.65" as [0, 0, 1, 65]; stops at the first part that is not a number.
pub fn parse_version(v: &str) -> Vec<u64> {
    v.trim().split('.').map_while(|p| p.parse().ok()).collect()
}

/// No console window for a child of this GUI app (Windows).
fn hidden(c: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
        c.creation_flags(0x0800_0000 | 0x0000_0200);
    }
    c.stdin(Stdio::null())
}

/// Run a command and wait (at most `limit`).
fn wait_for(mut c: Command, what: &str, limit: Duration) -> Result<String, String> {
    let mut child = hidden(&mut c).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("could not start {what}: {e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                return Err(format!("{what} took too long"));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!("{what}: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Run `kopanod <args>` and wait (at most `limit`), for init and autostart.
fn run_once(exe: &Path, args: &[&str], cfg: &(PathBuf, u16), limit: Duration) -> Result<String, String> {
    let mut c = Command::new(exe);
    c.args(args).arg("--home").arg(&cfg.0).args(["--api-port", &cfg.1.to_string()]);
    wait_for(c, &format!("kopanod {}", args.join(" ")), limit)
}

/// How `kopanod` gets started when the person signs in.
#[derive(Debug, PartialEq, Eq)]
enum AtSignIn {
    /// The installer's entry (Windows MSI, Linux package unit).
    Installer,
    /// A per-user entry `kopanod autostart` made (macOS, AppImage).
    PerUser,
    /// None (a portable copy, a Mac app still on its disk image, or turned off).
    No,
}

/// The whole check (see the module docs).
pub fn ensure_running(config: &Config) -> Result<(), String> {
    let Some(mine) = find_kopanod() else {
        log::info!("collaboration service: no kopanod next to this app (a developer build); not starting it");
        return Ok(());
    };
    let home = config.home.clone().ok_or("the collaboration folder is unknown here (set KOPANO_HOME)")?;
    let port = config.port;
    std::fs::create_dir_all(&home).map_err(|e| format!("could not make {}: {e}", home.display()))?;
    let cfg = (home.clone(), port);
    let (exe, at_sign_in) = if shared_kopanod().as_deref() == Some(mine.as_path()) {
        // the installer starts it at sign-in; a per-user entry from an older build would start a
        // second one (or a removed file), so it goes
        if autostart_entry().is_some() {
            match run_once(&mine, &["autostart", "off"], &cfg, Duration::from_secs(30)) {
                Ok(_) => log::info!("collaboration service: removed the old per-user start at sign-in"),
                Err(e) => log::warn!("collaboration service: {e}"),
            }
        }
        (mine, AtSignIn::Installer)
    } else {
        per_user(mine, &home, port, &cfg)
    };
    if !answers(port) {
        let id = run_once(&exe, &["init"], &cfg, Duration::from_secs(30))?;
        log::info!("collaboration service: this computer is {}", id.get(..8).unwrap_or(&id));
    }
    // Linux: the packaged unit is started through systemd, so it is the one that runs
    if at_sign_in == AtSignIn::Installer && cfg!(target_os = "linux") && !answers(port) {
        let mut c = Command::new("systemctl");
        c.args(["--user", "start", LINUX_UNIT_NAME]);
        if let Err(e) = wait_for(c, "systemctl --user start", Duration::from_secs(10)) {
            log::warn!("collaboration service: {e}");
        }
    }
    // macOS and Linux start it with the autostart entry; give that a moment
    let wait = if at_sign_in != AtSignIn::No && !cfg!(windows) { 3 } else { 0 };
    let start = Instant::now();
    while !answers(port) {
        if start.elapsed() > Duration::from_secs(wait) {
            return spawn_run(&exe, &home, port);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

/// Start at sign-in for this person (no installer did it): returns the `kopanod` to run.
fn per_user(mine: PathBuf, home: &Path, port: u16, cfg: &(PathBuf, u16)) -> (PathBuf, AtSignIn) {
    let mine = match lasting_copy(&mine, home) {
        Ok(Some(p)) => p,
        Ok(None) => return (mine, AtSignIn::No),
        Err(e) => {
            log::warn!("collaboration service: {e}");
            return (mine, AtSignIn::No);
        }
    };
    if off("UBUNYE_COLLAB_AUTOSTART") {
        return (mine, AtSignIn::No);
    }
    let entry = autostart_entry();
    if let Some(entry) = &entry {
        if cfg!(target_os = "macos") {
            // another Ubunye app's kopanod is as good when it is there and at least as new
            if let Some(theirs) = plist_program(entry).filter(|p| p.is_file()) {
                if entry_points_at(entry, &theirs, home, port) && bundled_version(&theirs) >= bundled_version(&mine) {
                    return (theirs, AtSignIn::PerUser);
                }
            }
        } else if entry_points_at(entry, &mine, home, port) {
            return (mine, AtSignIn::PerUser);
        }
    }
    match run_once(&mine, &["autostart", "on"], cfg, Duration::from_secs(30)) {
        Ok(_) => {
            log::info!("collaboration service: starts when you sign in to this computer");
            (mine, AtSignIn::PerUser)
        }
        Err(e) => {
            log::warn!("collaboration service: start at sign-in not set: {e}");
            (mine, AtSignIn::No)
        }
    }
}

/// Start `kopanod run` on its own; it keeps running after this app quits.
fn spawn_run(exe: &Path, home: &Path, port: u16) -> Result<(), String> {
    let log_path = home.join("kopanod.log");
    if std::fs::metadata(&log_path).is_ok_and(|m| m.len() > LOG_MAX) {
        let _ = std::fs::rename(&log_path, home.join("kopanod.old.log"));
    }
    let log = std::fs::OpenOptions::new().create(true).append(true).open(&log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
    let log2 = log.try_clone().map_err(|e| e.to_string())?;
    hidden(Command::new(exe).arg("run").arg("--home").arg(home).args(["--api-port", &port.to_string()]))
        .stdout(log)
        .stderr(log2)
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", exe.display()))?;
    log::info!("collaboration service: started {}", exe.display());
    Ok(())
}

/// Where this window's control token goes for the service (`<home>/photo/control-<port>.token`).
fn token_path(home: Option<&Path>, port: u16) -> Result<PathBuf, String> {
    let home = home.ok_or("the collaboration folder is unknown here (set KOPANO_HOME)")?;
    Ok(home.join("photo").join(format!("control-{port}.token")))
}

/// Write `token` to `path` for the service: a file only this user can read. Unix: mode 600.
/// Windows: an access list naming this user alone, set before the token goes in, so it does not
/// rest on what the folder passes down (fine in %LOCALAPPDATA%, not in a shared KOPANO_HOME).
fn write_token(path: &Path, token: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not make {}: {e}", dir.display()))?;
    }
    // a file left by an earlier window on the same port is stale
    let _ = std::fs::remove_file(path);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    use std::io::Write as _;
    let mut f = o.open(path).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    #[cfg(windows)]
    if let Err(e) = owner_only(path) {
        drop(f);
        let _ = std::fs::remove_file(path);
        return Err(e);
    }
    f.write_all(token.as_bytes()).map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// Windows: only the signed-in user may open `path` (no inherited entries; by SID, so any
/// account name works).
#[cfg(windows)]
fn owner_only(path: &Path) -> Result<(), String> {
    let mut c = Command::new("whoami");
    c.args(["/user", "/fo", "csv", "/nh"]);
    let me = wait_for(c, "whoami", Duration::from_secs(10))?;
    let sid = me.rsplit(',').next().map(|s| s.trim().trim_matches('"').to_string()).filter(|s| s.starts_with("S-1-"));
    let sid = sid.ok_or_else(|| format!("could not read this user's id from whoami: {me}"))?;
    let mut c = Command::new("icacls");
    c.arg(path).args(["/inheritance:r", "/grant:r", &format!("*{sid}:F")]);
    wait_for(c, "icacls", Duration::from_secs(10)).map(|_| ())
}

/// The automation gates `--control` sets: engine commands are checked, no file reads or writes.
pub fn install_gates(services: &mut Services) {
    let step: fn(&str, &serde_json::Value) -> photocraft_engine::Result<()> = photocraft_automation::workspace::authorize_desktop_engine_step;
    services.automation_authorize = Some(step);
    services.automation_command = Some(Box::new(|id: &str, params: &serde_json::Value| {
        photocraft_automation::workspace::authorize_desktop_engine_command(id, params).map_err(|error| error.to_string())
    }));
}

/// The Share & Collaborate window's [`StartControl`]. `existing`: the `--control` server this
/// window started with (its port, token and token file).
pub fn start_control(existing: Option<(u16, String, Option<PathBuf>)>, home: Option<PathBuf>) -> StartControl {
    let mut existing = existing;
    Box::new(move |ctx: &egui::Context, services: &mut Services| -> Result<Started, String> {
        if let Some((port, token, file)) = existing.take() {
            let token_file = match file {
                Some(f) => f,
                None => {
                    let f = token_path(home.as_deref(), port)?;
                    write_token(&f, &token)?;
                    f
                }
            };
            return Ok(Started { link: ControlLink { port, token_file }, rx: None });
        }
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("could not open a control port: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let token = photocraft_automation::security::generate_token().map_err(|e| e.to_string())?;
        let token_file = token_path(home.as_deref(), port)?;
        write_token(&token_file, &token)?;
        install_gates(services);
        let rx = crate::control_server::start_on(listener, token, ctx.clone());
        log::info!("collaboration: control server started on 127.0.0.1:{port} for the collaboration service");
        Ok(Started { link: ControlLink { port, token_file }, rx: Some(rx) })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    #[test]
    fn entry_must_name_this_exe_home_and_port() {
        let exe = PathBuf::from("/Apps/Photo/kopanod");
        let home = PathBuf::from("/h/kopano");
        let e = "ExecStart=\"/Apps/Photo/kopanod\" run --home \"/h/kopano\" --api-port 47100";
        assert!(entry_points_at(e, &exe, &home, 47100));
        assert!(!entry_points_at(e, &exe, &home, 18571));
        assert!(!entry_points_at(e, &PathBuf::from("/Other/kopanod"), &home, 47100));
        assert!(!entry_points_at(e, &exe, &PathBuf::from("/x"), 47100));
    }

    #[test]
    fn the_launch_agent_names_its_program() {
        let plist = "<plist version=\"1.0\"><dict>\n<key>Label</key><string>org.ubunye.kopano</string>\n<key>ProgramArguments</key><array><string>/Applications/Ubunye Light.app/Contents/MacOS/kopanod</string><string>run</string></array>\n</dict></plist>";
        assert_eq!(plist_program(plist), Some(PathBuf::from("/Applications/Ubunye Light.app/Contents/MacOS/kopanod")));
        assert_eq!(plist_program("<plist/>"), None);
        assert_eq!(plist_program("<key>ProgramArguments</key><array><string>/A &amp; B/kopanod</string>"), Some(PathBuf::from("/A & B/kopanod")));
    }

    #[test]
    fn versions_compare_as_numbers() {
        assert_eq!(parse_version("0.0.1.65\n"), vec![0, 0, 1, 65]);
        assert!(parse_version("0.0.1.100") > parse_version("0.0.1.65"));
        assert!(parse_version("0.1.0.1") > parse_version("0.0.9.900"));
        // unknown (an older app without the file) is older than anything known
        assert!(Vec::<u64>::new() < parse_version("0.0.1.1"));
        assert!(bundled_version(Path::new("/no/such/app/Contents/MacOS/kopanod")).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn the_token_file_is_this_users_alone() {
        let home = temp_home("acl");
        let f = home.join("photo").join("control-1.token");
        write_token(&f, "secret").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "secret");
        let mut c = Command::new("icacls");
        c.arg(&f);
        let acl = wait_for(c, "icacls", Duration::from_secs(10)).unwrap();
        // one entry, not inherited ("(I)"), not a group
        assert!(!acl.contains("(I)"), "{acl}");
        for group in ["Everyone", r"BUILTIN\Users", "Authenticated Users", r"BUILTIN\Administrators", "SYSTEM"] {
            assert!(!acl.contains(group), "{group} in {acl}");
        }
    }

    #[test]
    fn switches_read_plain_words() {
        assert!(!off("UBUNYE_COLLAB_TEST_UNSET_VARIABLE"));
    }

    #[test]
    fn a_missing_kopanod_is_not_an_error() {
        let cfg = Config { port: 1, home: Some(std::env::temp_dir().join("ubunye-collab-test-none")) };
        // the test binary has no kopanod next to it (and this computer may have a shared one)
        if find_kopanod().is_none() {
            assert_eq!(ensure_running(&cfg), Ok(()));
        }
    }

    fn temp_home(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("photo-collab-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn on_demand_control_serves_the_token_it_wrote_and_gates_automation() {
        let home = temp_home("demand");
        let mut start = start_control(None, Some(home.clone()));
        let mut services = Services::default();
        assert!(services.automation_command.is_none());
        let started = start(&egui::Context::default(), &mut services).unwrap();
        // the gates are on: file commands are refused like with --control
        let gate = services.automation_command.as_ref().unwrap();
        assert!(gate("file.open", &serde_json::json!({"path": "C:/x.psd"})).is_err());
        assert!(gate("kopano.export", &serde_json::json!({})).is_ok());
        assert!(services.automation_authorize.is_some());
        let link = started.link;
        assert_eq!(link.token_file, home.join("photo").join(format!("control-{}.token", link.port)));
        let token = std::fs::read_to_string(&link.token_file).unwrap();
        assert_eq!(token.len(), 64);
        let rx = started.rx.expect("a new server hands its requests over");
        // a client with the token is let in, and its request reaches the window
        let mut s = std::net::TcpStream::connect(("127.0.0.1", link.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut r = BufReader::new(s.try_clone().unwrap());
        writeln!(s, "{}", serde_json::json!({"id": 1, "method": "auth", "params": {"token": token}})).unwrap();
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        assert!(line.contains("\"authenticated\":true"), "{line}");
        writeln!(s, "{}", serde_json::json!({"id": 2, "method": "engine.execute", "params": {"command": "kopano.export"}})).unwrap();
        let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(req.method, "engine.execute");
        req.reply.send(serde_json::json!({"ok": true, "result": "seen"})).unwrap();
        line.clear();
        r.read_line(&mut line).unwrap();
        assert!(line.contains("seen"), "{line}");
        // a wrong token is shut out
        let mut bad = std::net::TcpStream::connect(("127.0.0.1", link.port)).unwrap();
        bad.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        writeln!(bad, "{}", serde_json::json!({"id": 1, "method": "auth", "params": {"token": "0".repeat(64)}})).unwrap();
        let mut l2 = String::new();
        BufReader::new(bad).read_line(&mut l2).unwrap();
        assert!(l2.contains("\"ok\":false"), "{l2}");
    }

    #[test]
    fn an_existing_control_port_gets_a_token_file_when_it_had_none() {
        let home = temp_home("existing");
        let token = "ab".repeat(32);
        let mut start = start_control(Some((18555, token.clone(), None)), Some(home.clone()));
        let mut services = Services::default();
        let started = start(&egui::Context::default(), &mut services).unwrap();
        assert!(started.rx.is_none(), "the --control server is already served");
        assert_eq!(started.link.port, 18555);
        assert_eq!(std::fs::read_to_string(&started.link.token_file).unwrap(), token);
        // with a token file of its own, that file is the one handed over
        let mut start = start_control(Some((18556, token, Some(PathBuf::from("T:/given.token")))), Some(home));
        assert_eq!(start(&egui::Context::default(), &mut services).unwrap().link.token_file, PathBuf::from("T:/given.token"));
    }
}
