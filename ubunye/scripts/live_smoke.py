#!/usr/bin/env python3
"""Smoke test of the LIVE Ubunye app: what downloads.thabangvision.com serves right now.

    python ubunye/scripts/live_smoke.py --app photo --out OUT_DIR
    python ubunye/scripts/live_smoke.py --app photo --out OUT_DIR --cli PATH   # only the media checks

1. Read https://downloads.thabangvision.com/<app>/latest.json and download this platform's installers
   (Linux: .deb, .rpm and AppImage; Windows: .msi and portable .zip; macOS: .dmg), checking size and sha256.
2. Install them the way a user does (apt, dnf in a Fedora container, msiexec, the DMG's .app).
3. Check the installed identity: name, version = latest.json, no upstream names in what the user sees,
   the updater and product links inside the shipped binary, and that the collaboration service is installed.
4. Run the installed app on real samples and check every output with a decoder that is not the app's own.
   On macOS the real window starts too and is driven over its control channel.

Writes OUT_DIR/summary.json and the outputs; exits non-zero on the first failed check. Same structure as
Ubunye Light's copy of this file; the app-specific part is the section named after the app.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
import zipfile
from pathlib import Path

UA = "ubunye-live-smoke/1.0 (+https://github.com/ubunye-ai-ecosystems)"
DOWNLOADS = "https://downloads.thabangvision.com"

# Internal format tests only: never shown as product demos.
MEDIA = {
    "a7iii.ARW": (
        "https://raw.pixls.us/download/data/Sony/ILCE-7M3/_DSC0009.ARW",  # raw.pixls.us, CC0
        "250784580ea527442c09004417bb0eead484f2bf3ee8f9121a776ac65bb50d0f",
    ),
    "church.jpg": (
        # Wikimedia Commons, "! Russisch-Orthodoxe Kirche (Dresden) Zwiebeltürme.JPG", CC0
        "https://upload.wikimedia.org/wikipedia/commons/1/1f/%21_Russisch-Orthodoxe_Kirche_%28Dresden%29_Zwiebelt%C3%BCrme.JPG",
        "71cb618a15f342b38f0d5b7316a7f82c834d72b52caea38b8135e50cadfb39d9",
    ),
}

APPS = {
    "photo": {
        "name": "Ubunye Photo",
        "app_id": "org.ubunye.creative.photo",
        "deb_package": "photocraft",  # nfpm keeps upstream's package name for this app
        "product_page": "https://www.thabangvision.com/tools/photo",
        "repo": "ubunye-ai-ecosystems/ubunye-creative-photo",
        "upstream_app_id": "ai.storyteller.photocraft",
        "upstream_words": ["PhotoCraft", "ArtCraft", "storytold", "getartcraft"],
        # credits the licence and brand require (About, --help, copyright lines): allowed
        "credits": ["Based on PhotoCraft by the ArtCraft team", "based on PhotoCraft", "the PhotoCraft authors"],
        "mac_bundle": "Ubunye Photo.app",
        "mac_exe": "PhotoCraft",
        "gui": "photocraft",
        "cli": "photocraft-cli",
        "cli_version_names_app": False,  # `photocraft-cli --version` is name, version, commit; --help names the app
    },
}

RESULTS: list[dict] = []


class Fail(Exception):
    pass


def check(ok: bool, what: str, detail: str = "") -> None:
    RESULTS.append({"check": what, "ok": bool(ok), "detail": detail[:500]})
    print(("PASS " if ok else "FAIL ") + what + (f": {detail}" if detail else ""), flush=True)
    if not ok:
        raise Fail(what + (f": {detail}" if detail else ""))


def fetch(url: str, dest: Path | None = None) -> bytes | None:
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    for attempt in range(4):
        try:
            with urllib.request.urlopen(req, timeout=300) as r:
                if dest is None:
                    return r.read()
                with open(dest, "wb") as fh:
                    shutil.copyfileobj(r, fh, 1 << 20)
                return None
        except Exception as e:  # network blips on runners: retry
            if attempt == 3:
                raise
            print(f"retry {url}: {e}", flush=True)
            time.sleep(5 * (attempt + 1))
    return None


def sha256(p: Path) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def run(cmd: list, env: dict | None = None, timeout: int = 900, check_rc: bool = True, quiet: bool = False) -> subprocess.CompletedProcess:
    print("$ " + " ".join(str(c) for c in cmd), flush=True)
    p = subprocess.run([str(c) for c in cmd], capture_output=True, text=True, encoding="utf-8", errors="replace",
                       env=env, timeout=timeout, stdin=subprocess.DEVNULL)
    if not quiet and p.stdout.strip():
        print(p.stdout[-4000:], flush=True)
    if not quiet and p.stderr.strip():
        print(p.stderr[-4000:], file=sys.stderr, flush=True)
    if check_rc and p.returncode != 0:
        check(False, "command succeeds", f"{cmd[0]} exited {p.returncode}: {p.stderr[-300:]}")
    return p


def os_name() -> str:
    return {"Linux": "linux", "Windows": "windows", "Darwin": "macos"}[platform.system()]


# ---------------------------------------------------------------------------------------------
# download + identity helpers
# ---------------------------------------------------------------------------------------------
def download_live(app: str, work: Path) -> tuple[dict, dict[str, Path]]:
    latest = json.loads(fetch(f"{DOWNLOADS}/{app}/latest.json"))
    version = latest["version"]
    print(f"live {app} {version}, published {latest.get('published')}", flush=True)
    want = {
        "linux": (".deb", ".rpm", ".AppImage"),
        "windows": (".msi", "-portable.zip"),
        "macos": (".dmg",),
    }[os_name()]
    files: dict[str, Path] = {}
    for f in latest["files"]:
        suffix = next((w for w in want if f["name"].endswith(w)), None)
        if not suffix or ("x86_64" not in f["name"] and "x64" not in f["name"] and "universal" not in f["name"]):
            continue
        dest = work / f["name"]
        fetch(f["url"], dest)
        check(dest.stat().st_size == f["size"], f"download size {f['name']}", f"{dest.stat().st_size} vs {f['size']}")
        if f.get("sha256"):
            check(sha256(dest) == f["sha256"], f"download sha256 {f['name']}")
        check(version in f["name"], f"file name carries the live version {f['name']}")
        files[suffix] = dest
    for w in want:
        check(w in files, f"latest.json lists a {w} for {os_name()}")
    return latest, files


def no_upstream_in_text(cfg: dict, text: str, where: str) -> None:
    cleaned = text
    for credit in cfg["credits"]:
        cleaned = cleaned.replace(credit, "")
    # case-sensitive: the binaries keep upstream's lower-case names
    found = [w for w in cfg["upstream_words"] if w in cleaned]
    check(not found, f"no upstream names in {where}", ", ".join(found))


def binary_identity(cfg: dict, exe: Path) -> None:
    data = exe.read_bytes()
    for must in (cfg["product_page"], cfg["repo"], cfg["app_id"]):
        check(must.encode() in data, f"{exe.name} carries {must}")
    check(cfg["upstream_app_id"].encode() not in data, f"{exe.name} has no upstream app id {cfg['upstream_app_id']}")
    # informational: upstream strings that may legitimately remain (the credit link, library internals)
    for w in (b"storytold/", b"getartcraft.com"):
        RESULTS.append({"check": f"info: occurrences of {w.decode()} in {exe.name}", "ok": True, "detail": str(data.count(w))})


def cli_identity(cfg: dict, cli: Path, version: str, env: dict) -> None:
    out = run([cli, "--version"], env=env).stdout
    check(version in out, f"{cli.name} --version says {version}", out.strip())
    help_text = run([cli, "--help"], env=env, check_rc=False)
    text = help_text.stdout + help_text.stderr
    named = out if cfg["cli_version_names_app"] else text
    check(cfg["name"] in named, f"{cli.name} names {cfg['name']}")
    no_upstream_in_text(cfg, text, f"{cli.name} --help")


def picture(p: Path, size: tuple[int, int] | None = None) -> dict:
    """The file exists, decodes with Pillow, has the size asked for, and is a picture rather than one flat colour."""
    from PIL import Image, ImageStat

    check(p.is_file() and p.stat().st_size > 0, f"output exists and is non-empty: {p.name}", str(p.stat().st_size) if p.exists() else "missing")
    with Image.open(p) as im:
        im.load()
        w, h = im.size
        stat = ImageStat.Stat(im.convert("L").resize((64, 64)))
        info = {"file": p.name, "bytes": p.stat().st_size, "width": w, "height": h, "mode": im.mode, "format": im.format,
                "mean": round(stat.mean[0], 1), "stddev": round(stat.stddev[0], 1)}
    check(w > 0 and h > 0, f"output decodes: {p.name}", json.dumps(info))
    check(info["stddev"] > 3, f"output is not a flat colour: {p.name}", f"stddev {info['stddev']}")
    if size:
        check((w, h) == size, f"output size {size[0]}x{size[1]}: {p.name}", f"{w}x{h}")
    return info


def samples(work: Path) -> dict[str, Path]:
    out = {}
    d = work / "media"
    d.mkdir(exist_ok=True)
    for name, (url, digest) in MEDIA.items():
        p = d / name
        fetch(url, p)
        check(sha256(p) == digest, f"sample sha256 {name}")
        out[name] = p
    return out


# ---------------------------------------------------------------------------------------------
# install, per platform (exercise() is the app's own part)
# ---------------------------------------------------------------------------------------------
def linux(cfg: dict, latest: dict, files: dict, imgs: dict, out: Path, env: dict) -> None:
    version = latest["version"]
    name = run(["dpkg-deb", "-f", files[".deb"], "Package"]).stdout.strip()
    check(name == cfg["deb_package"], f"deb package is named {cfg['deb_package']}", name)
    run(["sudo", "apt-get", "update", "-qq"])
    run(["sudo", "apt-get", "install", "-y", "-qq", str(files[".deb"])], timeout=1800)
    q = run(["dpkg-query", "-W", "-f=${Package} ${Version} ${Maintainer}", name]).stdout
    check(version in q, "deb package version = latest.json", q)
    desktop = Path(f"/usr/share/applications/{cfg['app_id']}.desktop")
    check(desktop.is_file(), f"desktop entry {desktop}")
    text = desktop.read_text(encoding="utf-8")
    check(f"Name={cfg['name']}" in text, "desktop entry names the app", text[:200])
    no_upstream_in_text(cfg, text, "desktop entry")
    gui = Path(shutil.which(cfg["gui"]) or "/nonexistent")
    cli = Path(shutil.which(cfg["cli"]) or "/nonexistent")
    check(gui.is_file() and cli.is_file(), "deb installs the app and the CLI on the PATH", f"{gui} {cli}")
    check(Path("/usr/bin/kopanod").exists(), "deb sets up /usr/bin/kopanod (collaboration)")
    check(Path("/usr/lib/systemd/user/thabangvision-collab.service").exists(), "deb sets up the collaboration start-at-sign-in unit")
    out_gui = run([gui, "--version"], env=env, timeout=60).stdout
    check(version in out_gui, "app --version", out_gui.strip())
    binary_identity(cfg, gui)
    cli_identity(cfg, cli, version, env)
    exercise(cfg, cli, imgs, out, "deb", env)
    # rpm: a clean Fedora container, as a Fedora user installs it
    r = files[".rpm"]
    run(["docker", "run", "--rm", "-v", f"{r.parent}:/w", "fedora:41", "sh", "-c",
         f"dnf install -y -q /w/{r.name} >/dev/null && {cfg['cli']} --version && test -e /usr/bin/kopanod && echo kopanod-linked"],
        timeout=1800)
    check(True, "rpm installs in Fedora 41, its CLI runs, /usr/bin/kopanod is linked")
    # AppImage: the same payload, unpacked (no FUSE on runners)
    ai = files[".AppImage"]
    ai.chmod(0o755)
    root = out.parent / "appimage"
    shutil.rmtree(root, ignore_errors=True)
    root.mkdir()
    subprocess.run([str(ai), "--appimage-extract"], cwd=root, check=True, capture_output=True)
    acli = root / "squashfs-root" / "usr" / "bin" / cfg["cli"]
    check(acli.is_file(), "AppImage carries the CLI", str(acli))
    check((root / "squashfs-root" / "usr" / "bin" / "kopanod").is_file(), "AppImage carries kopanod")
    cli_identity(cfg, acli, version, env)
    binary_identity(cfg, root / "squashfs-root" / "usr" / "bin" / cfg["gui"])
    exercise(cfg, acli, imgs, out, "appimage", env)


def win_uninstall_entry(name: str) -> dict:
    import winreg

    for view in (winreg.KEY_WOW64_64KEY, winreg.KEY_WOW64_32KEY):
        for hive in (winreg.HKEY_LOCAL_MACHINE, winreg.HKEY_CURRENT_USER):
            try:
                root = winreg.OpenKey(hive, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall", 0, winreg.KEY_READ | view)
            except OSError:
                continue
            for i in range(winreg.QueryInfoKey(root)[0]):
                sub = winreg.OpenKey(root, winreg.EnumKey(root, i))
                try:
                    if winreg.QueryValueEx(sub, "DisplayName")[0] == name:
                        vals = {}
                        for k in ("DisplayName", "DisplayVersion", "Publisher", "InstallLocation", "URLInfoAbout", "HelpLink"):
                            try:
                                vals[k] = str(winreg.QueryValueEx(sub, k)[0])
                            except OSError:
                                pass
                        return vals
                except OSError:
                    pass
    return {}


def windows(cfg: dict, latest: dict, files: dict, imgs: dict, out: Path, env: dict) -> None:
    version = latest["version"]
    run(["msiexec", "/i", str(files[".msi"]), "/qn", "/norestart", "/l*v", str(out / "msi-install.log")], timeout=1800)
    entry = win_uninstall_entry(cfg["name"])
    check(bool(entry), f"installed: Apps & features lists {cfg['name']}", json.dumps(entry))
    check(entry.get("DisplayVersion", "").startswith(version), "installed version = latest.json", json.dumps(entry))
    no_upstream_in_text(cfg, json.dumps(entry), "Apps & features entry")
    pf = Path(os.environ["ProgramFiles"])
    found = [d for d in (pf / cfg["name"], pf / "Ubunye AI Ecosystems" / cfg["name"]) if (d / f"{cfg['cli']}.exe").is_file()]
    check(bool(found), f"{cfg['cli']}.exe is in the install folder", str(pf))
    d = found[0]
    check((pf / "ThabangVision" / "Collaboration" / "kopanod.exe").is_file(), "MSI installs kopanod.exe (collaboration)")
    # not `app --version` here: a Windows release build shows it in a message box, which waits for a click
    binary_identity(cfg, d / f"{cfg['gui']}.exe")
    cli = d / f"{cfg['cli']}.exe"
    cli_identity(cfg, cli, version, env)
    exercise(cfg, cli, imgs, out, "msi", env)
    pz = out.parent / "portable"
    shutil.rmtree(pz, ignore_errors=True)
    with zipfile.ZipFile(files["-portable.zip"]) as z:
        z.extractall(pz)
    pcli = next(pz.rglob(f"{cfg['cli']}.exe"), None)
    check(pcli is not None, "portable zip carries the CLI")
    cli_identity(cfg, pcli, version, env)
    exercise(cfg, pcli, imgs, out, "portable", env)
    run(["msiexec", "/x", str(files[".msi"]), "/qn", "/norestart", "/l*v", str(out / "msi-uninstall.log")], timeout=1800)
    check(not win_uninstall_entry(cfg["name"]), "uninstall removes the app")


def macos(cfg: dict, latest: dict, files: dict, imgs: dict, out: Path, env: dict) -> None:
    import plistlib

    version = latest["version"]
    mnt = out.parent / "dmg"
    run(["hdiutil", "attach", "-nobrowse", "-readonly", "-mountpoint", mnt, files[".dmg"]])
    try:
        src = mnt / cfg["mac_bundle"]
        check(src.is_dir(), f"DMG holds {cfg['mac_bundle']}", ", ".join(x.name for x in mnt.iterdir()))
        app = out.parent / cfg["mac_bundle"]
        shutil.rmtree(app, ignore_errors=True)
        run(["ditto", src, app])
    finally:
        run(["hdiutil", "detach", mnt], check_rc=False)
    run(["codesign", "--verify", "--deep", "--strict", app], check_rc=False)
    info = plistlib.loads((app / "Contents" / "Info.plist").read_bytes())
    check(info.get("CFBundleIdentifier") == cfg["app_id"], "bundle id", str(info.get("CFBundleIdentifier")))
    check(info.get("CFBundleShortVersionString") == version, "bundle version = latest.json", str(info.get("CFBundleShortVersionString")))
    check(info.get("CFBundleDisplayName") == cfg["name"], "bundle display name", str(info.get("CFBundleDisplayName")))
    shown = {k: v for k, v in info.items() if isinstance(v, str) and k not in ("CFBundleExecutable", "CFBundleIconFile")}
    for k, val in shown.items():
        no_upstream_in_text(cfg, val, f"Info.plist {k}")
    macos_dir = app / "Contents" / "MacOS"
    exe = macos_dir / cfg["mac_exe"]
    v = run([exe, "--version"], env=env, timeout=60).stdout
    check(version in v, "app --version", v.strip())
    binary_identity(cfg, exe)
    check((macos_dir / "kopanod").is_file(), "the app carries kopanod (collaboration)")
    cli = macos_dir / cfg["cli"]
    check(cli.is_file(), "the app carries the CLI")
    cli_identity(cfg, cli, version, env)
    exercise(cfg, cli, imgs, out, "dmg", env)
    window(cfg, exe, cli, imgs, out, env)


def wait_for_port(proc: subprocess.Popen, port: int, seconds: int = 180) -> socket.socket:
    deadline = time.time() + seconds
    while True:
        try:
            return socket.create_connection(("127.0.0.1", port), timeout=300)
        except OSError:
            check(proc.poll() is None, "app keeps running while it starts", f"exit {proc.poll()}")
            if time.time() > deadline:
                check(False, "app opens its control port", "timed out")
            time.sleep(2)


def stop(proc: subprocess.Popen) -> None:
    proc.terminate()
    try:
        proc.wait(20)
    except subprocess.TimeoutExpired:
        proc.kill()


# ---------------------------------------------------------------------------------------------
# Photo
# ---------------------------------------------------------------------------------------------
def info_of(cli: Path, path: Path, env: dict) -> dict:
    return json.loads(run([cli, "info", path, "--compact"], env=env, quiet=True).stdout)


def mean_of(path: Path) -> float:
    from PIL import Image, ImageStat

    with Image.open(path) as im:
        return ImageStat.Stat(im.convert("L").resize((64, 64))).mean[0]


def exercise(cfg: dict, cli: Path, imgs: dict[str, Path], out: Path, tag: str, env: dict) -> None:
    from PIL import Image, ImageChops, ImageStat

    arw, jpg = imgs["a7iii.ARW"], imgs["church.jpg"]
    i = info_of(cli, arw, env)
    check((i["width"], i["height"], i["depth"]) == (6000, 4000, 16), f"opens the Sony a7 III raw at 6000x4000, 16-bit ({tag})",
          f"{i['width']}x{i['height']} {i['depth']}")
    i = info_of(cli, jpg, env)
    check((i["width"], i["height"]) == (4000, 3000), f"opens the camera JPEG at 4000x3000 ({tag})", f"{i['width']}x{i['height']}")

    raw_out = out / f"{tag}-raw.jpg"
    run([cli, "convert", arw, raw_out, "--quality", "85"], env=env)
    picture(raw_out, (6000, 4000))

    # PNG of the JPEG: the same picture as Pillow's own reading of the source
    png = out / f"{tag}-church.png"
    run([cli, "convert", jpg, png], env=env)
    picture(png, (4000, 3000))
    with Image.open(jpg) as a, Image.open(png) as b:
        d = ImageStat.Stat(ImageChops.difference(a.convert("RGB").resize((400, 300)), b.convert("RGB").resize((400, 300)))).mean
    check(max(d) < 4, f"the PNG matches the source JPEG ({tag})", f"mean difference {[round(x, 2) for x in d]}")

    # An edit: brighter and smaller, and it really changed the picture
    edit = out / f"{tag}-edit.webp"
    run([cli, "run", jpg, "--cmd", "image.adjustments.brightnessContrast", "--params", json.dumps({"brightness": 60}),
         "--cmd", "image.imageSize", "--params", json.dumps({"width": 1600, "height": 1200}), "--out", edit], env=env, quiet=True)
    picture(edit, (1600, 1200))
    before, after = mean_of(jpg), mean_of(edit)
    check(after > before + 5, f"brightness +60 made the picture brighter ({tag})", f"mean {before:.1f} -> {after:.1f}")

    # Our own document format and the layered formats, there and back
    doc = out / f"{tag}-raw.pcraft"
    run([cli, "run", arw, "--cmd", "image.imageSize", "--params", json.dumps({"width": 3000, "height": 2000}), "--out", doc], env=env, quiet=True)
    i = info_of(cli, doc, env)
    check((i["width"], i["height"]) == (3000, 2000), f"saved .pcraft reopens at 3000x2000 ({tag})", f"{i['width']}x{i['height']}")
    tif = out / f"{tag}-from-pcraft.tif"
    run([cli, "convert", doc, tif], env=env)
    picture(tif, (3000, 2000))
    psd = out / f"{tag}-church.psd"
    run([cli, "convert", jpg, psd], env=env)
    picture(psd, (4000, 3000))
    doc.unlink()  # large, and already proven by the .tif made from it


def window(cfg: dict, exe: Path, cli: Path, imgs: dict[str, Path], out: Path, env: dict) -> None:
    """The real app window, driven over its control channel (docs/control-protocol.md)."""
    port = 47981
    root = out.parent
    token = out.parent / "control.token"
    log = open(out / "app-stderr.log", "w")
    proc = subprocess.Popen([str(exe), "--control", str(port), "--control-token-file", str(token),
                             "--automation-read-root", str(root),
                             "--automation-write-root", str(root)], env=env, stdout=log, stderr=subprocess.STDOUT)
    try:
        s = wait_for_port(proc, port)
        f = s.makefile("rw", encoding="utf-8", newline="\n")

        def call(method: str, params: dict | None = None, name: str | None = None) -> dict:
            f.write(json.dumps({"id": method, "method": method, "params": params or {}}) + "\n")
            f.flush()
            reply = json.loads(f.readline())
            check(reply.get("ok") is True, name or f"control {method}", json.dumps(reply)[:300])
            return reply.get("result")

        call("auth", {"token": token.read_text().strip()}, "the app window accepts its control token")
        call("ui.inspect", None, "the app window answers ui.inspect")
        shot = out / "app-ui.png"
        call("ui.screenshot", {"path": str(shot)}, "the app window takes a screenshot")
        picture(shot)
    finally:
        stop(proc)
        log.close()


RUNNERS = {"linux": linux, "windows": windows, "macos": macos}


def main() -> int:
    for stream in (sys.stdout, sys.stderr):  # Windows consoles default to cp1252
        stream.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--app", required=True, choices=sorted(APPS))
    ap.add_argument("--out", required=True)
    ap.add_argument("--cli", help="skip download and install: only run the media checks with this CLI")
    a = ap.parse_args()
    cfg = APPS[a.app]
    out = Path(a.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="live-smoke-"))
    env = dict(os.environ)
    env["PHOTOCRAFT_CONFIG_DIR"] = str(work / "config")  # isolated profile
    summary = {"app": a.app, "os": os_name(), "started": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
    rc = 0
    try:
        if a.cli:
            exercise(cfg, Path(a.cli).resolve(), samples(work), out, "local", env)
        else:
            latest, files = download_live(a.app, work)
            summary["version"] = latest["version"]
            summary["files"] = [p.name for p in files.values()]
            RUNNERS[os_name()](cfg, latest, files, samples(work), out, env)
    except Fail as e:
        summary["failed"] = str(e)
        rc = 1
    except Exception as e:  # anything unexpected is a failure too, with its message
        summary["failed"] = f"{type(e).__name__}: {e}"
        print(f"FAIL unexpected: {summary['failed']}", flush=True)
        rc = 1
    summary["checks"] = RESULTS
    summary["passed"] = rc == 0
    (out / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    step = os.environ.get("GITHUB_STEP_SUMMARY")
    if step:
        with open(step, "a", encoding="utf-8") as fh:
            fh.write(f"### {cfg['name']} {summary.get('version', '?')} live smoke on {os_name()}: {'PASS' if rc == 0 else 'FAIL'}\n\n")
            for r in RESULTS:
                fh.write(f"- {'PASS' if r['ok'] else 'FAIL'} {r['check']}" + (f" ({r['detail'][:160]})" if r["detail"] else "") + "\n")
            if summary.get("failed"):
                fh.write(f"\n**Failed:** {summary['failed']}\n")
    print(f"\n{'PASS' if rc == 0 else 'FAIL'}: {sum(r['ok'] for r in RESULTS)} checks passed"
          + (f"; failed: {summary.get('failed')}" if rc else ""), flush=True)
    return rc


if __name__ == "__main__":
    sys.exit(main())
