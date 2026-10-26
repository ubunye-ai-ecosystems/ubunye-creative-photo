# Installers with ThabangVision collaboration

One download per operating system installs Ubunye Photo together with `kopanod`, the small
background service that does the collaboration (inside the code it is the Kopano protocol;
people only ever see "ThabangVision collaboration").

| System | Download | Where kopanod goes | Start at sign-in | Removed by |
|---|---|---|---|---|
| Windows | `.msi` (and a portable `.zip`) | beside `photocraft.exe` | a value in the user's Run key | the MSI uninstall |
| macOS | `.dmg` | `PhotoCraft.app/Contents/MacOS/` (Finder shows it as Ubunye Photo) | a LaunchAgent | (drag to Trash: see below) |
| Linux | `.AppImage`, `.deb`, `.rpm`, `.tar.gz` | `/usr/bin` (AppImage: inside it, copied to `~/.local/share/kopano/bin` on first start) | a systemd user unit | `.deb` / `.rpm` removal |

Why this way: each one is the package Photo already ships, with one more program inside. No
second download, no second installer, nothing to set up.

## What happens on the first start

Photo does it, as the signed-in person, silently (`apps/photocraft/src/ubunye_collab.rs`, the
same steps as Ubunye Light):

1. finds `kopanod` beside itself (a developer build has none: nothing happens);
2. `kopanod init`: makes this computer's device key (into Windows Credential Manager or the
   macOS Keychain);
3. `kopanod autostart on`, unless start at sign-in already points at this `kopanod`;
4. starts `kopanod run` if nothing answers yet.

Until the person signs in with ThabangVision, `kopanod` only answers Photo on 127.0.0.1 and
stays offline, so Share & Collaborate shows the "Sign in with ThabangVision" button.

## Uninstall

- Windows: the MSI turns start at sign-in off for the person removing it and stops every
  `kopanod` running from the install folder (`windows/collab.wxs`). An upgrade only stops it.
- Linux `.deb` / `.rpm`: `linux/prerm.sh` stops it and removes each user's unit.
- macOS and AppImage have no uninstaller. The macOS LaunchAgent stays until removed under
  System Settings > General > Login Items.
- The service folder (`%LOCALAPPDATA%\Kopano`, `~/Library/Application Support/Kopano`,
  `~/.local/share/kopano`) stays, like Photo's own settings.

## Building

- Windows: `ubunye/packaging/windows/collab-package.ps1 -Arch x64 -KopanoLab <lab>`
- macOS: `ubunye/packaging/macos/collab-package.sh <lab> --arch universal`
- Linux: `ubunye/packaging/linux/collab-package.sh <lab>`

`<lab>` is `research/kopano/lab` of ubunye-creative-studio-tools. Each script builds `kopanod`
and then runs the normal `packaging/<os>/package.*` with `UBUNYE_EXTRA_BIN_DIR` set (a small
hook in those scripts; Windows also sets `UBUNYE_EXTRA_WXS` to `windows/collab.wxs`, and Linux
sets `UBUNYE_LINUX_PREREMOVE` to `linux/prerm.sh`). Without those variables the normal
packages are unchanged. CI: `.github/workflows/ubunye-collab-packages.yml` (the `kopano_ref`
input picks the studio-tools commit).

## Not done yet (needs the owner)

- Code signing. Without it Windows Smart App Control blocks the programs on new PCs and macOS
  asks people to allow the app. The workflow has a marked place for the secrets; the release
  signing in `release.yml` is unchanged.
- Store accounts (Microsoft Store, Apple Developer ID) are not set up.
