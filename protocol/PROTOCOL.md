# OpenSights BLE protocol, version 1

The single source of truth for the link between the firmware (`firmware/`) and the
desktop app (`app/`). A change here must be made on both sides in the same commit,
and bumps `PROTOCOL_VERSION` if an old app could misread the new firmware or the reverse.

Mirrored in code by:
- firmware: `firmware/src/protocol.h`
- app: `app/src/protocol.rs`

## Roles

- The sensor is a BLE **peripheral** (GATT server). It accepts one connection at a time.
- The app is a BLE **central** (GATT client).
- All multi-byte integers are **little-endian**.

## Advertising

| Packet        | Contents                                                           |
|---------------|--------------------------------------------------------------------|
| Advertisement | Flags; complete list of 128-bit services = the sensor service UUID; manufacturer data (below) |
| Scan response | Complete local name = the sensor name                              |

Manufacturer data: company ID `0xFFFF` (reserved for testing), then 2 bytes:

| Byte | Meaning                                         |
|------|-------------------------------------------------|
| 0    | `PROTOCOL_VERSION`                              |
| 1    | Flags. Bit 0: sensor is in pairing mode. Other bits 0. |

Example, version 1 in pairing mode: `FF FF 01 01` (company ID little-endian, then the two bytes).

The app finds sensors by the service UUID, and reads the pairing flag to decide
whether to offer "Pair" (flag set) or tell the user to enter pairing mode first.

## GATT service

Service UUID: `5631f100-5314-4f02-b637-0654ff6d409d`

All characteristic UUIDs share the service UUID's base and differ only in the
4th hex group of the first field (`5631f1xx`):

| UUID suffix | Name      | Properties           | Value                                         |
|-------------|-----------|----------------------|-----------------------------------------------|
| `f101`      | Heartbeat | read, notify         | `u32` milliseconds since the sensor booted (`millis()`) at the last heartbeat |
| `f102`      | LED       | read, write, notify  | `u8`: 0 = off, 1 = on                          |
| `f103`      | Name      | read, write, notify  | Sensor name, 1–20 bytes of printable ASCII (0x20–0x7E), no terminator |
| `f104`      | Version   | read                 | `u8` `PROTOCOL_VERSION`                        |
| `f105`      | PIN       | write                | `u32` new pairing passkey, 0–999 999 (6 digits) |
| `f106`      | Shot      | write, indicate      | Indicated: one shot (20 bytes). Written: a shot request (8 bytes) |

Full UUIDs:
- Heartbeat `5631f101-5314-4f02-b637-0654ff6d409d`
- LED `5631f102-5314-4f02-b637-0654ff6d409d`
- Name `5631f103-5314-4f02-b637-0654ff6d409d`
- Version `5631f104-5314-4f02-b637-0654ff6d409d`
- PIN `5631f105-5314-4f02-b637-0654ff6d409d`
- Shot `5631f106-5314-4f02-b637-0654ff6d409d`

### Behaviour

- **Heartbeat** is notified every 2000 ms while a client is subscribed. It wraps after
  about 49.7 days, like `millis()`.
  Example: 12 345 ms → `39 30 00 00`.
- **LED**: writing `0` or `1` switches the onboard LED. Any other value or length is
  ignored. The sensor notifies the new state after every change, so the app does not
  need to assume its write succeeded.
- **Name**: writing a valid name stores it in flash, updates the advertised name and
  notifies the new name. An invalid write (empty, longer than 20 bytes, or any byte
  outside 0x20–0x7E) is ignored, and the sensor notifies the unchanged name, so the app
  always ends up showing the real one.
  Leading and trailing spaces are not trimmed by the sensor; the app trims before writing.
- **Version**: the app reads it right after connecting and disconnects with an error
  if it is not a version it supports.
- **PIN**: writing a value of 0–999 999 (exactly 4 bytes) stores it in flash as the passkey
  for future pairings, and **deletes every bond except the one of the writing central**:
  other computers must pair again with the new PIN. Any other value or length is ignored.
  It can't be read, so the PIN never leaves the sensor. The app shows PINs as 6 digits
  with leading zeros. Example: 123 456 → `40 E2 01 00`.
  Firmware from before this characteristic doesn't have it; the app treats it as optional.
- **Shot**: see below. Also optional for the app.

### Shots

Each shot is **indicated**, so the app's Bluetooth stack confirms every one. The sensor sends
one shot at a time, oldest first, and moves on only after the confirmation. A shot that
wasn't confirmed is sent again.

Shot (sensor → app, 20 bytes):

| Bytes | Type  | Field   | Meaning                                                      |
|-------|-------|---------|--------------------------------------------------------------|
| 0–3   | `u32` | seq     | Shot number, from 1 for every boot of the sensor             |
| 4–7   | `u32` | boot    | Random per boot, never 0: tells which boot `seq` belongs to  |
| 8–11  | `u32` | time    | The sensor's `millis()` at the impact                        |
| 12–15 | `i32` | x       | Hundredths of a mm from the target centre, positive to the right |
| 16–19 | `i32` | y       | Hundredths of a mm from the target centre, positive upwards  |

Directions are as seen by the shooter. The sensor knows nothing about target cards or
scoring: the app maps the position onto the card and scores it.

Example: shot 3 of boot `0x12345678`, at 12 345 ms, x = 1.25 mm, y = −2.5 mm →
`03 00 00 00 78 56 34 12 39 30 00 00 7D 00 00 00 06 FF FF FF`.

Shot request (app → sensor, 8 bytes): `u32` boot, `u32` next seq. "Send your shots from
`next seq` of `boot` on."
- The sensor keeps the last **64 shots** of the current boot (in RAM).
- When it subscribes, an app gets only shots made after that. Right after subscribing,
  the app writes a request with the boot and seq of the last shot it has, plus 1.
- If `boot` is not the sensor's current boot (for example 0, because the app has no shots
  yet), the sensor sends every shot it still has.
- The app drops duplicates by (boot, seq). A gap in `seq` within one boot means shots
  were lost: more than 64 were made while the app was away.
- A request of the wrong length is ignored.

Example: the app has shot 3 of boot `0x12345678` → `78 56 34 12 04 00 00 00`.

## Security and pairing

- Every characteristic requires an **encrypted, authenticated (MITM) link**. Reading or
  subscribing on a link that is not yet secured makes the operating system start pairing.
- Pairing uses **passkey entry**: the sensor has no display, the user types its PIN into
  the computer. The PIN is 6 digits, **default `000000`**, and can be changed over BLE
  (PIN characteristic). On Linux the app registers its own BlueZ pairing agent and asks
  for the PIN in its own window; elsewhere the operating system asks. Both sides **bond**,
  so later connections are encrypted without asking again.
- **Pairing mode**: holding BOOT for 2 s opens a 60 s pairing window (the LED blinks fast).
  It closes when a pairing succeeds or the time runs out.
- Outside pairing mode, the sensor disconnects any central it has **no bond with**,
  right after it connects. Bonded centrals can always connect.
- The sensor keeps up to 3 bonds (NimBLE default). If a bonded computer has deleted its
  keys and pairs again, the sensor replaces the old bond.
- A pairing that didn't use the passkey (no MITM) is rejected, and its bond deleted.
- **Forgotten PIN**: holding BOOT for 10 s resets the PIN to `000000`, deletes all bonds
  (disconnecting the app) and opens pairing mode.

## Buttons and LED on the sensor

| Action                  | Result                                        |
|-------------------------|-----------------------------------------------|
| BOOT short press (< 2 s)| Make a test shot, normally distributed around the centre (σ 15 mm), until there is measuring hardware. The LED flashes briefly |
| BOOT held 2 s           | Enter pairing mode for 60 s                   |
| BOOT held 10 s          | Reset the PIN to `000000`, delete all bonds, enter pairing mode |
| LED blinking fast       | Pairing mode (the LED state is restored after) |
| LED on for 1.5 s, then blinking | The PIN reset was done                |

## Default name

On first boot the sensor names itself `OpenSights-xyz`, where `xyz` is 3 random lowercase
hex digits, and stores that name. Renaming over BLE replaces it.
