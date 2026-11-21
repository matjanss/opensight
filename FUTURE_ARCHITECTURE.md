# Future architecture: electronic target system

Where this test bed is heading. The current code (ESP32 BLE sensor + Rust app) is step 1
of the plan at the end.

## The system

- **Target:** an ESP32 plus electronics that measure where a shot hits. It reports each
  shot.
- **Viewer:** shows the shooter where they hit, on an SVG target card. It is either a
  dedicated Pi at the post or the shooter's own phone.
- **Management:** a central app for:
  - uploading target cards (one per discipline);
  - managing lanes, posts and the fleet of targets and viewers;
  - updates.

## Scale

| Stage | Setup |
|---|---|
| Prototype | 1 target, 1 viewer |
| Small range | 3 lanes, 25 posts: 10 m with 10 posts, 25 m with 10 posts, 100 m with 5 posts |
| Later | Bigger ranges, several ranges, a cloud tier |

Viewers per lane:
- **25 m:** a dedicated Pi at every post.
- **10 m and 100 m:** shooters' phones.

Power: battery for now, PoE or mains later.

## Principles

1. **One range is one local system.** It works without internet. A cloud tier is
   optional, on top, and syncs when it's online.
2. **The target is the source of truth for shots.** Every shot has a sequence number.
   The target keeps it in flash until it's acknowledged and resends it after a link drop.
   Receivers drop duplicates by `(target_id, seq)`. No shot is ever lost.
3. **Shots are physical.** A target reports x/y **in mm from the centre**, a timestamp and
   its raw measurements. It knows nothing about cards or scoring. The viewer maps the shot
   onto the card and scores it.
4. **Scoring never parses the SVG.** A card is the SVG (only the picture) plus JSON with
   the scoring geometry.
5. **Settings live in the management app, behind an admin login.** Viewers only show shots
   and have no settings. Config changes are signed by the server, and the target verifies
   them before applying them.

## Overview

```
 Target (ESP32, battery, Wi-Fi)  ──MQTT──►  Range server (Pi 5 / mini PC)
   BLE: setup + diagnostics only              - MQTT broker (Mosquitto)
                                              - range service (Rust): shots → SQLite,
                                                cards, fleet, updates, WebSocket
                                              - serves the viewer as a web app
                                                   │ WebSocket
                          ┌────────────────────────┼─────────────────────┐
                    Pi at 25 m post         Shooter's phone         Management UI
                    (kiosk, same viewer)    (scans QR on post)      (browser, admin login)
```

Why shots go over the network rather than BLE:
- Web pages on iPhones can't use Bluetooth.
- Asking every shooter to install an app is a barrier.
- BLE doesn't reach 100 m.

BLE stays for setup (entering Wi-Fi details, claiming a target, diagnostics) and as a
fallback.

## Components

### Target

- **Hardware:** an ESP32 with an external antenna for the 100 m lane (WROOM-32**U**).
  Ethernet/PoE boards (Olimex ESP32-POE, WT32-ETH01) can be used later.
- **Topics it publishes over MQTT:**
  - `target/<id>/shot`
  - `target/<id>/status`: heartbeat, battery, link quality, temperature, errors,
    firmware/hardware version
- **Topic it listens on:** `target/<id>/cmd`, with signed commands.
- **Identity:** an ID from the chip (eFuse MAC), plus a key of its own. A QR code on the
  case is used to claim the target in the management app.
- **Time:** SNTP from the range server.
- **Calibration data** is kept apart from the firmware, so updates never wipe it.
- **Battery:**
  - The target always listens for impacts, so it can't sleep deeply.
  - Use Wi-Fi power saving (modem sleep) and size the battery for a full range day.
  - The battery level is reported in the heartbeat.

### Range server

- A Pi 5, a mini PC, or a Docker container or VM on any machine that qualifies, running
  Mosquitto and one Rust service (for example axum + SQLite). It can start as only the
  server for the viewer (serving the wasm app and relaying shots) and grow into the
  central range management server.
- **Range model:** Range → Lane (distance, disciplines) → Post → assigned target and,
  optionally, a fixed viewer.
- **Jobs:**
  - stores shots;
  - forwards each target's shots to the viewers of its post over WebSocket;
  - serves the viewer and management web apps;
  - holds cards and update packages.
- **Single point of failure:** keep a spare Pi and back up the database. If the server is
  down, targets keep their shots until it's back.

### Viewer

- **One codebase:** the Rust/egui app, built twice:
  - as WebAssembly for phones and for the Pis (Chromium in kiosk mode);
  - as a native app for development.
- **Transport:** shots arrive through a `Transport` interface, implemented over BLE
  (native only) and over WebSocket (native and web).
- **Phones:**
  - A QR code at each post opens `…/post/25m-3`. A second QR code can join the range
    Wi-Fi.
  - The shooter claims the post for a session.
- **Pis:** read-only OS, kiosk browser, fixed to their post.

### Management

- A web app on the range server: cards, lanes and posts, fleet health, assignments,
  updates.
- Requires an admin login.

## Network per lane

| Lane | Target → network | Viewer |
|---|---|---|
| 10 m | Wi-Fi (indoor access point) | Phones |
| 25 m | Wi-Fi now, PoE later | Pi at each post (Ethernet if cabling is easy, else Wi-Fi) |
| 100 m | Outdoor access point near the targets, behind protection, with a cable or wireless bridge to the server. External antenna on the ESP32. | Phones |

Things to watch:
- **Phones may drop the range Wi-Fi.** Phones on a network without internet often switch
  to mobile data or warn about it. Provide an internet uplink, or a captive portal that
  opens the viewer.
- **HTTPS on the local network.** Browsers need it for installable web apps and some
  APIs. Use a real domain (`range.example.com`) that points to the server's local
  address, with a Let's Encrypt certificate obtained through a DNS challenge.
- **2.4 GHz is crowded** with many targets plus spectators' phones. Plan the channels,
  and use 5 GHz for the viewers where possible (the ESP32 only does 2.4 GHz).

## Updates

| What | How |
|---|---|
| ESP firmware | Over Wi-Fi from the range server (BLE as a last resort). Two app partitions with automatic rollback: the new image only becomes permanent after its self-test passes. Images are signed. Rollout goes one post first, then the rest. **Blocked while a post is live** or the battery is below about 50 %. The server only offers firmware whose protocol version the viewers support. |
| Viewer | Served by the range server, so a server update updates every viewer. The Pis' OS is read-only and rarely updated; RAUC or Mender can do A/B OS updates later. |
| Target cards | Uploaded in the management UI. Each card is a package: a sanitised SVG, rendered with a static renderer (resvg, which runs no scripts), plus JSON with ring diameters in mm, the scoring rule, card size and discipline. Named by content hash and cached by viewers. Cards are assigned to lanes or posts by discipline. |
| Range server | One container image or package for the whole server. Back up the database before each update. |

## Steps

Several lanes, with shooters' own phones and tablets as viewers, is the aim once the
single target measures well (user, 2026-11-21). The native app stays, for development and
as the admin tool.

1. **Prototype on BLE (now):**
   - Add a shot message to the protocol (sequence number, x/y in mm, timestamp).
   - The target produces test shots until the measuring hardware exists.
   - The current app shows them on an SVG card and scores them.
2. **Prototype on the network:**
   - Enter Wi-Fi details over BLE from the app.
   - The target publishes over MQTT to a broker on a laptop or Pi.
   - Add a minimal range service, the `Transport` interface in the viewer, and the WASM
     build.
3. **Small range:**
   - Lanes and posts, claiming posts by QR code.
   - The management UI and target cards.
   - Firmware updates over Wi-Fi.
   - The outdoor access point for 100 m.
4. **Later:** PoE or mains power, the cloud tier, results and shooter accounts.

Step 1 carries over unchanged to step 2: the shot message and the card view stay the same,
and only the transport changes.

## Open questions

- ~~How impacts are measured~~: acoustic, by time differences of arrival, and the target
  solves the position itself (PLAN.md D4, D12). Raw data goes along with every shot.
- Which disciplines and cards are needed first.
- Whether shooters need accounts and results history, and whether that data is shared
  across ranges.
