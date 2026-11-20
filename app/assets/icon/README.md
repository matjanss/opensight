# OpenSights app icon

"OS" on a dark tile. The O is a ring with a dot in the app's last-shot colour (`LAST_SHOT_COLOR`, #E86034), so it reads as a hit in the centre of a target. The S is IBM Plex Sans Bold (the app's font family), converted to outlines, so no font is needed. The tile is the dark panel grey (#1A1A1A) with a thin #3C3C3C border, so it stays visible on dark taskbars.

## Files

| File | Use |
|---|---|
| `opensights-icon.svg` | Master, full-bleed tile (Linux, Windows, the window icon) |
| `opensights-icon-macos.svg` | Master on Apple's icon grid: the tile is inset 100 px on a 1024 px canvas |
| `png/opensights-icon-<n>.png` | 16, 24, 32, 48, 64, 128, 256, 512 and 1024 px |
| `opensights.ico` | Windows, 16–256 px |
| `opensights.icns` | macOS, made from the inset master |
| `make.py` | Regenerates everything (needs fontTools, cairosvg, Pillow, and fonts-ibm-plex for IBMPlexSans-Bold.ttf) |

## Hooking it up in the app (for Claude Code)

Suggested place: `app/assets/icon/`.

- **Window and taskbar icon (all platforms):** in `main.rs`, add `.with_icon(...)` to the main `ViewportBuilder`. Use `eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon/png/opensights-icon-256.png"))`, which returns a `Result`; unwrap it or fall back to no icon.
- **macOS bundle:**
  - In `.github/workflows/build.yml`, copy `opensights.icns` to `OpenSights.app/Contents/Resources/` before `codesign`.
  - In `app/macos/Info.plist`, add `CFBundleIconFile` = `opensights`.
- **Windows .exe icon:** add a `build.rs` that embeds `opensights.ico` with the `winresource` crate, as a Windows-only build dependency. Without it, Explorer shows the default icon, although the window icon still works.
- **Linux:** the window icon covers the taskbar. A `.desktop` file with the 256 or 512 px PNG is only needed for a menu entry.
