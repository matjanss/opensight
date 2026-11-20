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

## Where it is used

- **Window and taskbar icon (all platforms):** `app_icon()` in `src/main.rs` loads `png/opensights-icon-256.png` (compiled in) for the main window and the settings window. If the PNG can't be decoded the windows get the system's default icon; a test checks that it decodes.
- **macOS bundle:** `.github/workflows/build.yml` copies `opensights.icns` to `OpenSights.app/Contents/Resources/` before `codesign`; `macos/Info.plist` has `CFBundleIconFile` = `opensights`.
- **Windows .exe icon:** `build.rs` embeds `opensights.ico` with `winresource` (a Windows-only build dependency), so Explorer and shortcuts show it.
- **Linux:** the window icon covers X11. Wayland ignores window icons: the dock shows the icon of the `.desktop` file named after the window's app id (`APP_ID` = `opensights-app` in `main.rs`, which is also eframe's storage name, so don't change it lightly). `linux/opensights-app.desktop` is that file; the Linux release archive contains it and the 256 px PNG. To install for the current user:

  ```sh
  install -Dm644 opensights-app.desktop ~/.local/share/applications/opensights-app.desktop
  install -Dm644 opensights-icon-256.png ~/.local/share/icons/hicolor/256x256/apps/opensights-app.png
  ```

  `Exec=opensights-app` expects the binary on `PATH` (e.g. `~/.local/bin`); for a build from source, point it at `app/target/release/opensights-app`.

After changing the icon, run `make.py`; the app picks up the new PNG and .ico on the next build.
