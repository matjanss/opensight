# OpenSights BLE protocol, version 3

The single source of truth for the link between the firmware (`firmware/`) and the
desktop app (`app/`). A change here must be made on both sides in the same commit,
and bumps `PROTOCOL_VERSION` if an old app could misread the new firmware or the reverse.

Mirrored in code by:
- firmware: `firmware/src/protocol.h` (constants) and `firmware/lib/proto/` (the messages,
  tested on the PC against this document's examples: `firmware/test/test_proto`)
- app: `app/src/protocol.rs` (its tests check the same examples), and `app/src/eventlog.rs`
  (the event log)

Version 3 (2026-11-23, measurement step 5) added the raw event of every capture, the
sensor's status, calibration mode, and reading and writing the calibration and settings
(PLAN D4, D5, D7, D9). The same messages go over MQTT later, one topic per characteristic.

## Roles

- The sensor is a BLE **peripheral** (GATT server). It accepts one connection at a time.
- The app is a BLE **central** (GATT client).
- All multi-byte integers are **little-endian**. Floats (`f32`) are IEEE 754 single, little-endian:
  the same bits the firmware keeps, so the app writes the firmware's log lines exactly.
- **ATT MTU:** version 3 needs 183 or more (the 180-byte event plus 3). BlueZ, Windows and
  macOS all negotiate that with this firmware (NimBLE prefers 255). On a smaller MTU the sensor
  sends no events (Status flag) and, under 35, no shots either.

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

Example, version 3 in pairing mode: `FF FF 03 01` (company ID little-endian, then the two bytes).

The app finds sensors by the service UUID, and reads the pairing flag to decide
whether to offer "Pair" (flag set) or tell the user to enter pairing mode first.

## GATT service

Service UUID: `5631f100-5314-4f02-b637-0654ff6d409d`

All characteristic UUIDs share the service UUID's base and differ only in the
4th hex group of the first field (`5631f1xx`):

| UUID suffix | Name      | Properties           | Value                                         |
|-------------|-----------|----------------------|-----------------------------------------------|
| `f101`      | Heartbeat | read, notify         | `u32` milliseconds since the sensor booted (`millis()`) at the last heartbeat |
| `f103`      | Name      | read, write, notify  | Sensor name, 1–20 bytes of printable ASCII (0x20–0x7E), no terminator |
| `f104`      | Version   | read                 | `u8` `PROTOCOL_VERSION`                        |
| `f105`      | PIN       | write                | `u32` new pairing passkey, 0–999 999 (6 digits) |
| `f106`      | Shot      | write, indicate      | Indicated: one hit (32 bytes). Written: a shot request (8 bytes) or a clear request (9 bytes) |
| `f107`      | Event     | write, indicate      | Indicated: one event (180 bytes). Written: an event request (8 bytes) |
| `f108`      | Status    | read, notify         | The sensor's state (52 bytes) |
| `f109`      | Control   | write                | A command (1–9 bytes): calibration mode, template point |
| `f10a`      | Calibration | read, write        | The calibration record (213 bytes: GATT long read and write) |
| `f10b`      | Settings  | read, write          | The solver's policy and the capture settings (36 bytes) |

`f102` was the LED switch in version 1. It is gone (the LED only shows the sensor's
status, see below) and its UUID is not reused.

Full UUIDs:
- Heartbeat `5631f101-5314-4f02-b637-0654ff6d409d`
- Name `5631f103-5314-4f02-b637-0654ff6d409d`
- Version `5631f104-5314-4f02-b637-0654ff6d409d`
- PIN `5631f105-5314-4f02-b637-0654ff6d409d`
- Shot `5631f106-5314-4f02-b637-0654ff6d409d`
- Event `5631f107-5314-4f02-b637-0654ff6d409d`
- Status `5631f108-5314-4f02-b637-0654ff6d409d`
- Control `5631f109-5314-4f02-b637-0654ff6d409d`
- Calibration `5631f10a-5314-4f02-b637-0654ff6d409d`
- Settings `5631f10b-5314-4f02-b637-0654ff6d409d`

### Behaviour

- **Heartbeat** is notified every 2000 ms while a client is subscribed. It wraps after
  about 49.7 days, like `millis()`.
  Example: 12 345 ms → `39 30 00 00`.
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
- **Shot**, **Event**, **Status**, **Control**, **Calibration**, **Settings**: see below. The
  version 3 app needs all of them.

### Shots

Each hit is **indicated**, so the app's Bluetooth stack confirms every one. The sensor sends
one at a time, oldest first, and moves on only after the confirmation. A hit that wasn't
confirmed is sent again. A waiting shot always goes before events.

A **hit** is anything the shooter's series should show: a measured shot, or one with no
position that the shooter (or range officer) resolves on the paper card.

Shot (sensor → app, 32 bytes):

| Bytes | Type  | Field   | Meaning                                                      |
|-------|-------|---------|--------------------------------------------------------------|
| 0–3   | `u32` | seq     | Hit number, from 1 for every boot of the sensor; every kind counts |
| 4–7   | `u32` | boot    | Random per boot, never 0: tells which boot `seq` belongs to  |
| 8–11  | `u32` | time    | The sensor's `millis()` at the impact                        |
| 12–15 | `i32` | x       | Hundredths of a mm from the target centre, positive to the right; 0 for `hole` and `missed` |
| 16–19 | `i32` | y       | Hundredths of a mm from the target centre, positive upwards  |
| 20–23 | `u32` | event   | The Event message it came from (its `seq`); 0 for none (the BOOT button's test shot) |
| 24–27 | `u32` | cal     | The calibration ID it was measured with; 0 if there is no calibration record |
| 28–29 | `u16` | sigma   | The position's uncertainty: 1σ along the error ellipse's long axis, 0.01 mm; `0xFFFF` unknown |
| 30    | `u8`  | kind    | 0 `shot` (has a position), 1 `hole` (a curtain dip with no sound: a shot through an existing hole), 2 `missed` (sound in the dead time while the last event was read: a second shot, not measured) |
| 31    | `u8`  | flags   | bit 0: a channel was dropped (drop-one); bit 1: the air was partly assumed; bit 2: the calibration changed since it was made (stale); bit 3: the curtain confirmed it (a dip in its window) |

Directions are as seen by the shooter. The sensor knows nothing about target cards or
scoring: the app maps the position onto the card and scores it. Only an event with verdict
`ok` (and not in calibration mode) becomes a `shot`; a rejected event is in the Event stream
only. `hole` and `missed` are never scored by themselves.

Example: shot 3 of boot `0x12345678`, at 12 345 ms, x = 1.25 mm, y = −2.5 mm, from event 18,
calibration `0x1A2B3C4D`, 1σ 0.08 mm, confirmed by the curtain →
`03 00 00 00 78 56 34 12 39 30 00 00 7D 00 00 00 06 FF FF FF 12 00 00 00 4D 3C 2B 1A 08 00 00 08`.

Example: hit 4, a hole at 13 000 ms, from event 19 →
`04 00 00 00 78 56 34 12 C8 32 00 00 00 00 00 00 00 00 00 00 13 00 00 00 4D 3C 2B 1A FF FF 01 00`.

Shot request (app → sensor, 8 bytes): `u32` boot, `u32` next seq. "Send your shots from
`next seq` of `boot` on."
- The sensor keeps the last **64 hits** of the current boot (in RAM).
- When it subscribes, an app gets only hits made after that. Right after subscribing,
  the app writes a request with the boot and seq of the last hit it has, plus 1.
- If `boot` is not the sensor's current boot (for example 0, because the app has no hits
  yet), the sensor sends every hit it still has.
- The app drops duplicates by (boot, seq). A gap in `seq` within one boot means hits
  were lost: more than 64 were made while the app was away.
- A request of the wrong length is ignored.

Example: the app has hit 3 of boot `0x12345678` → `78 56 34 12 04 00 00 00`.

Clear request (app → sensor, 9 bytes): `u8` `0x01`, `u32` boot, `u32` last seq. "Forget your
hits of `boot` up to and including `last seq`." The app sends it when the user clears the
series, with the newest hit it has, so the cleared hits are not sent again on a later
connect (for example after the app restarts).
- Hits after `last seq` are kept: a hit made while the request was on its way still
  reaches the app.
- If `boot` is not the sensor's current boot, the request is ignored (the sensor has none of
  those hits any more). `last seq` beyond the newest hit means all hits.
- Numbering is not reset: the next hit keeps counting from the last one.
- A cleared hit is never sent again, also not when a shot request asks for it.

Example: clear up to hit 3 of boot `0x12345678` → `01 78 56 34 12 03 00 00 00`.

### Events

One message per event the sensor saw, whatever its verdict: every acoustic event (also
rejected ones and calibration sparks), every hole and every missed shot. It has everything
the firmware's serial `rec` line has (`firmware/MODEL.md` "Log lines"), so the app writes the
same line, and `firmware/tools/replay` reads an app's log like a serial one (PLAN D4, D7).

**Delivery:** like hits: indicated one at a time, oldest first, after any waiting hit. The
sensor keeps the last **32 events** in RAM. Event request (app → sensor, 8 bytes): `u32` boot,
`u32` next seq, as the shot request. There is no clear request: old events drop out of the
ring, and a gap in `seq` shows how many. A new subscription gets new events only.

Event (sensor → app, 180 bytes):

| Bytes | Type | Field | Meaning |
|---|---|---|---|
| 0 | `u8` | kind | 0 acoustic event, 1 hole (a dip only), 2 missed |
| 1 | `u8` | verdict | `ok` 0, `few` 1, `together` 2, `nofit` 3, `residual` 4, `lost` 5, `outside` 6, `curtain` 7 (`firmware/MODEL.md` "Validity"); `0xFF` for kinds 1, 2 |
| 2 | `u8` | acoustic | the verdict without the curtain (PLAN D9 counts each defence apart) |
| 3 | `i8` | curtain | 1 a dip in the window, 0 none, −1 not judged |
| 4–7 | `u32` | boot | |
| 8–11 | `u32` | seq | the event stream's number, from 1 per boot (requests use it; a hit's `event` points at it) |
| 12–15 | `u32` | n | the capture's event number (the `rec` line's `n=`); for a missed shot, the event being read; 0 for a hole |
| 16–19 | `u32` | time | `millis()` at the event |
| 20–27 | `u64` | t | the first edge (a hole: the dip's rise), ns since the capture started (the `rec` line's `t=`) |
| 28–31 | `u32` | cfg | the `cfg` ID it was solved with |
| 32–35 | `u32` | cal | the calibration ID; 0 if there is no record |
| 36 | `u8` | flags | bit 0 calibration mode; bit 1 calibration stale; bit 2 has a true point (`pt`); bit 3 the curtain was on; bit 4 solved (the fit ran); bit 5 sent as a hit |
| 37 | `u8` | channels | bit c: CH(c+1) was in the event |
| 38 | `u8` | fitted | bit c: in the final fit |
| 39 | `i8` | dropped | the channel drop-one left out (0–7), −1 none |
| 40–43 | `i32` | x | the fit's position, 0.01 mm (also for a rejected event, if solved) |
| 44–47 | `i32` | y | |
| 48–51 | `f32` | rms | the fit's rms residual, ns (NaN if not solved) |
| 52–55 | `f32` | sigma | the position's 1σ (long axis), mm (NaN if not solved) |
| 56–59 | `f32` | T | the air, °C |
| 60–63 | `f32` | RH | % |
| 64–67 | `f32` | p | hPa |
| 68–71 | `f32` | c | the speed of sound used, m/s |
| 72 | `u8` | measured | bit 0 T, bit 1 RH, bit 2 p measured (else assumed) |
| 73 | `u8` | dips | how many curtain dips follow (0–4) |
| 74–75 | | | 0 |
| 76–107 | `i32` ×8 | t1 | OUT_L's first rise per channel CH1–CH8, ns from the first edge; `0x80000000` no edge |
| 108–139 | `i32` ×8 | t2 | OUT_H's |
| 140–155 | `i32` ×4 | dip | each dip's rise, ns from the first edge (0 for a hole's own dip); unused 0 |
| 156–171 | `i32` ×4 | dip length | ns; `0x80000000` its fall not seen; unused 0 |
| 172–175 | `f32` | pt x | the true point in calibration mode (the template point) or the simulator's, mm |
| 176–179 | `f32` | pt y | |

The app writes each event as a log line: an acoustic event as the firmware's `rec` line, a
hole as a `dip` line, a missed shot as a `MISSED` line (`app/src/eventlog.rs`).

Example: event 21 of the 4b bench log (`rec n=21 t=30.9781094 cfg=4A87309E T=18.00 RH=50.0
p=1013.25 meas=--- c=342.739716 CH3=320400,321500 CH4=214100,215000 CH5=5800,6200
CH6=214100,215000 CH7=167100,167900 CH8=0,500 pt=30.0000,30.0000 cal=- ir=-160725/27075`),
solved `ok` at (29.99, 29.99) mm with the curtain confirming it, sent, as event seq 18 of boot
`0x12345678` at 12 345 ms:

```
00 00 00 01 78 56 34 12 12 00 00 00 15 00 00 00 39 30 00 00 D8 6F 70 36 07 00 00 00 9E 30 87 4A
00 00 00 00 3C FC FC FF B7 0B 00 00 B7 0B 00 00 E0 B7 98 41 FF F5 2F 3C 00 00 90 41 00 00 48 42
00 50 7D 44 AF 5E AB 43 00 01 00 00 00 00 00 80 00 00 00 80 90 E3 04 00 54 44 03 00 A8 16 00 00
54 44 03 00 BC 8C 02 00 00 00 00 00 00 00 00 80 00 00 00 80 DC E7 04 00 D8 47 03 00 38 18 00 00
D8 47 03 00 DC 8F 02 00 F4 01 00 00 2B 8C FD FF 00 00 00 00 00 00 00 00 00 00 00 00 C3 69 00 00
00 00 00 00 00 00 00 00 00 00 00 00 00 00 F0 41 00 00 F0 41
```

(The fit's floats, `rms` and `sigma`, can differ in their last bits between processors with and
without fused multiply-add; the tests compare those within a tolerance.)

### Status

Read, and notified when it changes: a new `cfg` or calibration ID, calibration mode or the
template point, the air at each reading, the counters, the MTU flag. The sensor checks every
0.5 s and notifies only a change.

| Bytes | Type | Field |
|---|---|---|
| 0–3 | `u32` | boot |
| 4–7 | `u32` | the `cfg` ID now |
| 8–11 | `u32` | the calibration ID now (0: no record) |
| 12 | `u8` | flags: bit 0 calibration mode; bit 1 calibration stale; bit 2 a template point is set; bit 3 the curtain's gate is on; bit 4 the MTU is too small for events; bit 5 an air sensor was found |
| 13 | `u8` | measured: bit 0 T, bit 1 RH, bit 2 p |
| 14 | `u8` | mics: bit c, CH(c+1) has a mic |
| 15 | `u8` | 0 |
| 16–19 | `f32` | the template point's x, mm (0 if none) |
| 20–23 | `f32` | its y |
| 24–27 | `f32` | T, °C |
| 28–31 | `f32` | RH, % |
| 32–35 | `f32` | p, hPa |
| 36–39 | `f32` | c, m/s |
| 40–43 | `u32` | events since boot |
| 44–47 | `u32` | missed shots since boot |
| 48–51 | `u32` | hits since boot |

Example: boot `0x12345678`, cfg `4A87309E`, no calibration record, the curtain on, mics on
CH3–CH8, 18 °C, 50 % RH, 1013.25 hPa (assumed), 342.739716 m/s, 21 events, 21 hits →
`78 56 34 12 9E 30 87 4A 00 00 00 00 08 00 FC 00 00 00 00 00 00 00 00 00 00 00 90 41 00 00 48 42
00 50 7D 44 AF 5E AB 43 15 00 00 00 00 00 00 00 15 00 00 00`.

### Control

Write only. Byte 0 is the command. A wrong length, an unknown command or a point beyond
±1000 mm is ignored. An accepted command changes Status, which is notified. The serial
console shows each one, as it does for its own `cal` commands.

| Command | Bytes | Meaning |
|---|---|---|
| `0x01` calibration mode | `01`, `u8` 0 off / 1 on | As serial `cal on|off`: no event becomes a hit; events carry the template point |
| `0x02` template point | `02`, `f32` x, `f32` y (mm, card coordinates) | As `cal pt x y`: the point the next sparks are at |
| `0x03` no template point | `03` | As `cal pt -` |

Examples: calibration mode on → `01 01`; template point (50, −30) mm →
`02 00 00 48 42 00 00 F0 C1`; no point → `03`.

### Calibration

The calibration record (PLAN D5): each channel's mic and arrival-time calibration, and where
they came from. Read and written whole: 213 bytes, more than one packet, so GATT's long read
and long write (both standard; the OS splits it).

| Bytes | Type | Field |
|---|---|---|
| 0 | `u8` | layout, 1 |
| 1–4 | `u32` | the date, yyyymmdd (from the app; the sensor has no clock); 0 if there is no record |
| 5–8 | `f32` | the fit's rms residual, ns |
| 9–10 | `u16` | template points used |
| 11–12 | `u16` | sparks used |
| 13–212 | 8 × 25 | per channel CH1–CH8: `u8` mic (0 none, 1 a mic); `f32` x, y, h (mm); `f32` k; `f32` offset (ns); `f32` sigma (ns) |

- **On a write** the sensor checks the whole record: layout 1; a mic on 4 channels or more;
  per channel |x|, |y| ≤ 1000 mm, h 0–100 mm, k 0–2, |offset| ≤ 100 µs, sigma 1–100 000 ns
  (the serial commands' limits). Anything else is rejected whole: nothing changes. Accepted, it
  is stored (NVS `"cal"`), becomes the calibration record (not stale), and the `cfg` line is
  printed; Status is notified with the new IDs. The app reads it back to confirm.
- **The calibration ID** is the sensor's: the CRC-32 of the channel fields as the `cfg` line
  prints them (`measure::calibrationId`). The date, residual and counts don't change it. It
  comes in Status and with every hit and event.

Example: no mic on CH1 and CH2 (k 1/3, sigma 50 ns), the 4b bench layout on CH3–CH8 (h
10.5 mm, k 0.33333, offset 0, sigma 50 ns), of 2026-11-23, residual 42.5 ns, 20 points, 160
sparks; its calibration ID is `746AAA08`:

```
01 03 29 35 01 00 00 2A 42 14 00 A0 00 00 00 00 00 00 00 00 00 00 00 00 28 41 3B AA AA 3E 00 00
00 00 00 00 48 42 00 00 00 00 00 00 00 00 00 00 00 28 41 3B AA AA 3E 00 00 00 00 00 00 48 42 01
00 00 D2 C2 00 00 D2 C2 00 00 28 41 3B AA AA 3E 00 00 00 00 00 00 48 42 01 00 00 D2 42 00 00 D2
C2 00 00 28 41 3B AA AA 3E 00 00 00 00 00 00 48 42 01 00 00 82 42 00 00 D2 42 00 00 28 41 3B AA
AA 3E 00 00 00 00 00 00 48 42 01 00 00 D2 C2 00 00 D2 42 00 00 28 41 3B AA AA 3E 00 00 00 00 00
00 48 42 01 00 00 00 00 00 00 D2 C2 00 00 28 41 3B AA AA 3E 00 00 00 00 00 00 48 42 01 00 00 D2
42 00 00 00 00 00 00 28 41 3B AA AA 3E 00 00 00 00 00 00 48 42
```

### Settings

The solver's policy and the capture settings: the `cfg` line apart from its channels, plus the
mic span, dead time and channel mask. Read and written whole, 36 bytes; checked, then stored,
the `cfg` line printed, Status notified. Rejected whole if out of range, or if the mask changes
while the self-test or the simulator runs.

| Bytes | Type | Field | Serial | Range |
|---|---|---|---|---|
| 0 | `u8` | layout, 1 | | |
| 1 | `u8` | drop-one, 0/1 | `dropone` | |
| 2 | `i8` | mic channels that may be lost, −1 off | `valid lost` | −1, 0–4 |
| 3 | `u8` | capture channel mask | `mask` | |
| 4–7 | `f32` | residual limit, ns | `reslimit` | > 0, ≤ 100 000 |
| 8–11 | `f32` | area, ± mm; 0 off | `valid area` | 0, 10–1000 |
| 12–15 | `f32` | together, ns; 0 off | `valid together` | 0, ≤ 100 000 |
| 16–19 | `f32` | pulse max, ns; 0 off | `valid pulse` | 0, ≤ 100 000 |
| 20–23 | `f32` | the curtain's window low, ns | `curtain window` | 0, 0 (off), or 0 ≤ low < high |
| 24–27 | `f32` | its high, ns | | ≤ 2 000 000 |
| 28–29 | `u16` | mic span, mm | `span` | 50 to the ring's maximum |
| 30–31 | `u16` | dead time, ms | `deadtime` | 1–1000 |
| 32–35 | | 0 | | |

Example: the defaults with the curtain on (45–100 µs), mask `3F`, span 300 mm, dead time
30 ms → `01 01 01 3F 00 80 BB 44 00 00 D2 42 00 40 9C 46 00 40 1C 46 00 C8 2F 47 00 50 C3 47 2C
01 1E 00 00 00 00 00`.

### After a firmware update

A computer that has bonded with the sensor caches its GATT layout. After an update that
changed it, the sensor tells each bonded computer to read it again ("services changed"), on
that computer's next secured connection; the list is kept in NVS until then. BlueZ (Linux)
doesn't act on that with this firmware: NimBLE keeps a subscription to "services changed" only
if it was made on an encrypted link, BlueZ makes it before, and BlueZ ignored the indication
also when it was sent on the live link (bench, 2026-11-23). There, pairing again clears the
cache: hold BOOT 2 s, then Pair in the app. The app waits 8 s for a new layout first, then says
so. (NimBLE's GATT caching, a database hash BlueZ would check by itself, doesn't build in this
NimBLE version.)

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
| Serial `pairing`        | As BOOT held 2 s (development: the serial port means the board is in hand) |
| LED blinking fast       | Pairing mode                                  |
| LED on for 1.5 s, then blinking | The PIN reset was done                |

## Default name

On first boot the sensor names itself `OpenSights-xyz`, where `xyz` is 3 random lowercase
hex digits, and stores that name. Renaming over BLE replaces it.
