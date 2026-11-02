# Ubunye Photo app icon

Ubunye AI Ecosystems artwork from the Ubunye Creative Studio brand system; the letters are an outline from DM Serif
Display (SIL OFL 1.1), so rendering needs no font. Licence: see `LICENSE.txt`.

This replaces upstream PhotoCraft's kitsune icon. The master lives at
`ubunye/brand/assets/icon.svg`; `python -I ubunye/brand/apply.py` copies it here and renders the rest.

## Geometry

A 512-unit tile (`viewBox="0 0 512 512"`), with its margin built in. The macOS render pads it
onto Apple's 824/1024 icon grid; Windows and Linux use it as is, and 32 px and below use
`icon-small.svg` (fewer, thicker rays, no glow) so the art reads at small sizes.

## Files

- `photocraft.svg`: copy of the master. `photocraft-small.svg`: copy of `icon-small.svg`, the 32 px and below art.
- `photocraft-1024.png`: 1024 px render on the macOS grid.
- `photocraft.icns`: macOS bundle icon (`CFBundleIconFile`).
- `photocraft.ico`: Windows icon, 16 to 256 px, embedded in the `.exe` by `apps/photocraft/build.rs`.
- `hicolor/<size>/apps/org.ubunye.creative.photo.png` (16 to 512) and `hicolor/scalable/apps/org.ubunye.creative.photo.svg`: Linux icon theme.

The file names `photocraft.*` stay as upstream has them, so build scripts need no change.
