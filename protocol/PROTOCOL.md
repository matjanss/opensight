# ESP Sensor BLE protocol, version 1

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

Full UUIDs:
- Heartbeat `5631f101-5314-4f02-b637-0654ff6d409d`
- LED `5631f102-5314-4f02-b637-0654ff6d409d`
- Name `5631f103-5314-4f02-b637-0654ff6d409d`
- Version `5631f104-5314-4f02-b637-0654ff6d409d`
- PIN `5631f105-5314-4f02-b637-0654ff6d409d`

### Behaviour

- **Heartbeat** is notified every 2000 ms while a client is subscribed. It wraps after
  about 49.7 days, like `millis()`.
  Example: 12 345 ms → `39 30 00 00`.
- **LED**: writing `0` or `1` switches the onboard LED. Any other value or length is
  ignored. The sensor notifies the new state after every change, from the app or from
  the BOOT button, so the app does not need to assume its write succeeded.
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

## Security and pairing

- Every characteristic requires an **encrypted, authenticated (MITM) link**. Reading or
  subscribing on a link that is not yet secured makes the operating system start pairing.
- Pairing uses **passkey entry**: the sensor has no display, the user types its PIN into
  the computer. The PIN is 6 digits, **default `001234`**, and can be changed over BLE
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
- **Forgotten PIN**: holding BOOT for 10 s resets the PIN to `001234`, deletes all bonds
  (disconnecting the app) and opens pairing mode.

## Buttons and LED on the sensor

| Action                  | Result                                        |
|-------------------------|-----------------------------------------------|
| BOOT short press (< 2 s)| Toggle the LED                                |
| BOOT held 2 s           | Enter pairing mode for 60 s                   |
| BOOT held 10 s          | Reset the PIN to `001234`, delete all bonds, enter pairing mode |
| LED blinking fast       | Pairing mode (the LED state is restored after) |
| LED on for 1.5 s, then blinking | The PIN reset was done                |

## Default name

On first boot the sensor names itself `ESPSensor-xyz`, where `xyz` is 3 random lowercase
hex digits, and stores that name. Renaming over BLE replaces it.
