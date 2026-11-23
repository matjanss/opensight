# Card photos: steps, tests and results

A phone photo of the shot card, matched to the measured shots (PLAN D13): first to measure
the system's accuracy on real shots (PLAN D11), later the shooter's view. This file is to
the card photo what `firmware/MEASUREMENT.md` is to the measurement: each step with what it
builds, its tests (with the numbers to expect), its status, and every result, failures
included. Claude runs the tests; the user provides what can't be made here (real photos)
and reviews the results.

## Steps

| # | Step | Status |
|---|---|---|
| C1 | Registration from the rings (`app/cardphoto/`) | synthetic tests pass (2026-11-23); real photos in C5 |
| C2 | The holes: finding them, matching them to the shots | synthetic tests pass (2026-11-23); real holes in C5 |
| C3 | The upload: QR code, the phone page, storage | tests over HTTP pass (2026-11-23); the phone: the user, once |
| C4 | In the app: the photo under the shots, the ground-truth panel | the processing passes through the app's path (2026-11-23); the UI: a look by the user |
| C5 | Real photos: registration (no holes), then holes | waiting for the photos |
| C6 | The serial number from the photo | later |

## The card and its coordinates

- The built-in card: ISSF 10 m air pistol (`assets/cards/issf-10m-air-pistol.json`): 170 ×
  170 mm, centre (85, 85), rings 1–10 of diameter 155.5 to 11.5 mm in steps of 16 mm, the
  inner ten 5.0 mm; the black aiming area covers rings 7–10 (59.5 mm).
- **Card coordinates:** mm from the centre, x right, y up, as printed (the numbers upright).
  **Frame coordinates:** the sensor's (x right, y up as seen by the shooter). The two differ
  by the card's placement: a shift, a small angle and a quarter turn (0, 90, 180 or 270°).

## C1. Registration from the rings

Builds, in a library crate `app/cardphoto/` (pure Rust, workspace member, `cargo test`):

1. **The image:** decoded (`image`), greyscale, scaled so the card is about 2000–3000 px.
2. **Coarse:** the black aiming disc (the largest dark, round blob): its centre, size and
   ellipse give a first homography (a similarity plus tilt).
3. **Ring edges:** intensity profiles from the centre outwards at about 720 angles (through
   the coarse homography). Along each, the ring lines (dark on white, light in the black) and
   the black disc's edge, each to a fraction of a pixel, assigned to their ring by the coarse
   radius.
4. **The fit:** image point = distort(H · card point). H is a homography (8 values), the
   distortion radial about the image centre (k1, k2); each circle's radius is its stated one
   plus an offset per kind of line (d_dark for the lines on the white, d_light in the black:
   where on the printed line the stated diameter sits; the disc's edge is the reference).
   **The print's scale isn't fitted:** a photo has no absolute scale (a larger card looks like
   a nearer one), so the rings set it to their ISSF size, which is what scoring uses; paper
   that changed size between the shot and the photo shows in C2, as a scale between the shots
   and the holes. Robust (Huber weights from the residuals' MAD), so hole edges, dust and the
   printed numbers drop out; Levenberg-Marquardt (`nalgebra`), three rounds of edge points
   with narrower windows (±2, 0.8, 0.5 mm). Circles fix everything but the card's rotation
   about its centre: that is fixed here by the card's square outline where it is in the photo
   (else arbitrary), and settled in C2 by the shots.
5. **Diagnostics:** the rms residual of the edge points, in mm on the card; how many points,
   per ring; a warning above a limit (to be set from real photos).
6. **Rectified images** from the fit: any region of the card in card mm at a chosen
   resolution (the hole patches, the low-resolution card, the overlay).

**Tests (synthetic, automated):** the app's own card render (resvg) at about 20 px/mm,
warped by a known homography (tilts up to 15°, any rotation), radial distortion (k1 up to
±0.1 over the half-diagonal), defocus blur (σ 1 px), uneven light (±30 %), noise (σ 3 % of
the range), a table texture around the card, and holes (dark or light discs of 4.5 mm).
Expected: every card point mapped within **0.02 mm** of the truth over the scoring area (up to
the quarter turn); k1 within 0.01; the reported residual small (under 0.02 mm) and clearly
larger for a bent card (a bend added on purpose). Also: no card in the image, or a card too
small, gives an error, not a wrong fit. `cargo test -p cardphoto` (`tests/register.rs`; the
synthetic photos are 2400 × 1800 at 9 px/mm, about half a phone's resolution).

## C2. The holes

Builds (in `cardphoto`, `src/holes.rs`):

- **In colour.** A hole shows whatever is under the card: a dark table hides a hole in the
  black, a white one a hole in the white, in grey values. In colour, a table that is neither
  black nor white-like shows in both.
- **The expected card, from the photo itself** (not from the SVG, which the real print won't
  match in its font, logos and serial): the light as smooth surfaces (the paper's and the
  ink's colour across the card, quadratic per channel, fitted robustly to the plain bands
  between the printed circles), so every colour is normalised (0 ink, 1 paper); and the print
  as one profile by radius, the median over the full circle (a hole is a small part of a
  circle from 1.5 mm out; inside 2 mm the card is plain black). The difference from it is
  high in a hole.
- **Each hole is a circle** fitted to the normalised colours: inside, the table's colour
  (one unknown colour); outside, the card as expected at each point; a soft edge; the radius
  shared within a group; holes closer than about a diameter fitted together; robust (Huber).
  This holds for any table colour and for a hole across the black disc's edge. The centre's
  uncertainty from the fit (a lower bound: neighbouring pixels are correlated).
- **Guided by the shots:** the quarter turn (all four tried: where the holes show best near
  the shots, by a matched filter), coarse holes near each shot (±4 mm), the placement (shift
  and angle; a scale too with 5 or more shots spread over more than 20 mm), the holes fitted
  from the placed shots, the placement again (twice, robustly). Per shot, the hole minus the
  placed shot is the sensor's error. A shot with no clear, round hole near it is reported as
  such (a false shot, or a hole not visible); holes no shot explains are found as below.
- **Unguided** (no shots: the test cards; and the holes no shot explains): blobs where the
  photo differs from the card, at least half a hole's area (the numbers' strokes are
  smaller); per blob, the count of holes from its area, then one or two more if they clearly
  fit better (cost 5 % lower, centres at least 1 mm apart).

**Tests (synthetic, automated, `tests/holes.rs`):** holes in the black, in the white, across
ring lines and the disc's edge, near the centre, three overlapping; the card's placement in
the frame turned by each quarter turn plus 0.4°, shifted about 1 mm; shots with 0.1 mm of
noise; tables blue, grey and dark. Expected: hole centres within **0.05 mm**; the quarter
turn always right; the placement as good as the shots' noise allows (with 9 shots of 0.1 mm
noise and 4 placement values about 0.07 mm rms at the shots); a false shot and an unmatched
hole flagged; a clean card with no holes found (the printed numbers aren't holes); the
unguided search finding every hole; on a table as dark as the ink, holes in the black not
found, but none placed wrong.

**For the photos (C5 and the shooter's guide):** put the card on a mid-grey or coloured
surface (a coloured folder, a cutting mat), not black and not white: a black table hides the
holes in the black disc.

## C3. The upload

Builds (in the app, `src/upload.rs`; the stripping in `cardphoto`, `src/meta.rs`):

- **The server** (`tiny_http`), while the dialog is open: on every interface, port 8710 (so a
  firewall rule can name it; any free port if 8710 is taken). The link is
  `http://<this computer's address>:<port>/p/<token>`, the token 128 random bits (hex). It
  names the series and works once; a new link ends the old one; anything else gets a 404.
  The address is the one the default route leads to (the range's Wi-Fi): the phone must be on
  the same network.
- **The QR code** (`qrcode`): the modules, for the app to paint.
- **The page:** a few lines on how to take the photo (flat on a grey or coloured surface,
  phone straight above, the whole card, no shadow), "Take photo" (`capture=environment`, the
  camera at once) and "Choose a photo", a progress bar, the answer. The photo is sent as it
  is (the body of a POST): no resizing on the phone.
- **The checks:** at most 40 MB (refused by its length before reading, or while reading);
  JPEG or PNG only (an iPhone set to HEIC: its file input sends JPEG anyway; if not, the
  answer says how to switch); a refused upload leaves the link working.
- **The metadata out** (`cardphoto::meta::strip`), without re-encoding: JPEG keeps its
  image segments, JFIF, an ICC profile and Adobe's segment; it drops APP1 (EXIF with GPS,
  the make, the time; XMP), the other APPn (MPF, maker data), comments, and what follows the
  end of the image (phones append depth maps and second images). PNG drops eXIf, tEXt,
  zTXt, iTXt and tIME. The EXIF orientation is read first and kept in the JSON.
- **Storage:** `photos/` next to `logs/` in the app's data folder,
  `YYYY-MM-DD_HHMMSS_<series>.jpg` (UTC) and a JSON beside it: series, time, file, format,
  sizes sent and stored, orientation, what was dropped, `keep`, `removed`.
- **Clean-up** (`clean_up`, run by the app at start and when the dialog opens, C4): an
  original more than 30 days old and not marked to keep is removed; its JSON stays (marked
  `removed`), and so do results files.

**Tests (automated, `cargo test` in `app/`; HTTP over TCP from this computer, as a browser
sends it):**

1. A phone-like JPEG (EXIF with orientation 6, the make and a GPS IFD; XMP with a GPS
   position; MPF; a comment; 400 bytes after the end) uploaded: 200, stored; "Exif", "GPS",
   the make and the XMP gone from the file; the decoded pixels identical to the original's;
   orientation 6 in the JSON; the JSON on disk equal to what the app got; the page served
   with the camera button and the series' name.
2. The same link again: 404 for the page and the upload (used).
3. An old link (replaced), a made-up token, `/p/`, `/`, `/favicon.ico`: 404, nothing stored.
4. Too large: by its stated length (413, refused before reading) and chunked without a
   length (413 after the limit); not a photo (415, the answer names JPEG); nothing stored,
   and the link still works afterwards.
5. The clean-up: of three photos 31, 40 (marked to keep) and 29 days old, only the first's
   image goes; its JSON stays, marked `removed`; a results file stays; a second run removes
   nothing.
6. Two photos of one series in the same second get different names; the UTC time is right.
7. The QR code of a link: the right size, the finder patterns there; tokens random.
8. In `cardphoto`: the stripping as in 1, stripping twice changes nothing, PNG text chunks
   dropped (the rest byte for byte), other formats and cut-short files refused.
9. Over this computer's network address (`cargo test lan -- --ignored`): the page and an
   upload, as a phone would reach them.

**The phone (the user, once):** in `app/`, `PHONE_WAIT=120 cargo test lan -- --ignored
--nocapture` prints a QR code and the link and waits 2 minutes. Scan it with the phone (on
the same Wi-Fi), take a photo. Expected: the page opens (it shows the tips and two buttons),
"Take photo" opens the camera, the upload shows progress and "Received", and the test prints
the stored file's JSON (orientation, what was dropped) and the folder. Worth trying both an
Android phone and an iPhone if at hand. A firewall may block port 8710 (Windows asks once).

## C4. In the app

Builds (`src/photo.rs`, the window in `main.rs`):

- **"Card photo"** under the series (next to "Clear series"; also with no shots yet, for a
  test card) opens the photo window: the old originals cleaned up, the server started, the
  QR code, the link, the server's news ("waiting", "the phone opened the page",
  "received", a refusal's reason), "New link". A photo file dropped on the window goes the
  same way (stored, then processed). With `OPENSIGHTS_DEMO_PHOTO=1` set, a button makes a
  synthetic photo of the series (holes 0.1 mm off the shots at random, the card turned
  90.5° in the holder and shifted 0.8 mm) to try it all without a real card.
- **The processing** on a thread of its own, against the series' shots as they are when
  the photo arrives: the registration (C1), the holes and the placement (C2), or without
  shots every hole (C2 unguided); the photo resampled onto the card's own grid (10 px per mm,
  through the placement, so it lies under the shots as drawn); the results in
  `<photo>.result.json` beside the photo (the registration, the placement, per shot the
  hole, the difference and its uncertainty, the holes no shot explains). The clean-up
  leaves results files alone.
- **On the card view:** the photo over the printed card, faded by a Card–Photo bar at the
  top right (0.6 when the photo arrives); the holes found as thin rings, also with the photo
  faded out, so the sensor's markers and the real holes can be compared at any zoom.
  Cleared with the series, or by "Remove from the card".
- **The ground-truth panel** in the photo window: the rms of the sensor against the holes,
  the worst shot; warnings (the card small in the photo, the rings fitting badly, shots with
  no hole, holes no shot explains, the series grown since the photo); per shot the shot,
  hole − shot (the sensor's axes), the distance; the photo's resolution, the rings' rms,
  the lens; the card's centre and turn in the holder (and the print's scale when fitted);
  "Keep the photo for evaluation"; "Take another photo", "Open the folder".
- Without shots the card's quarter turn is unknown (C2): the photo may show turned by a
  multiple of 90° until C6 reads the print.

**Tests:**

1. Automated (`cargo test`, `photo::tests`): the demo photo (3000 × 2250 px, 11 px/mm, a
   blue table, 8 shots, holes 0.1 mm off at random) stored as an upload and processed:
   every hole found; the card's turn within 0.2° and its shift within 0.15 mm of the truth;
   the rms between 0.04 and 0.2 mm (the holes' 0.1 mm per axis, less what the placement takes
   up); no warnings; on the card's grid the table shows through at every shot (the photo
   lies under the shots), paper away from them; the results file there and the clean-up
   leaving it.
2. Where the photo lies on the card's grid, by the black disc's centre (a circle fitted to
   its edge): on the printed card itself within 0.03 mm of the centre (the measurement);
   the photo without the placement within 0.05 mm of the printed card's centre (the
   registration); with the placement within 0.05 mm of where the placement and the truth put
   the card's centre in the frame (the demo: 0.8 mm off the sensor's centre).
2a. The same photo without shots: all 8 holes found.
3. A photo with no card: refused, "no card found".
4. The UI by a look (the user, when convenient): `OPENSIGHTS_DEMO_PHOTO=1 cargo run` in
   `app/`, connect, make a few shots (BOOT, or `sim` on the serial console), "Card photo",
   then "Demo: a synthetic photo of this series". Expected: the spinner for a few seconds; the
   ground-truth panel with about 0.1 mm rms and a row per shot; on the card the photo under
   the shots, a ring around each marker, the Card–Photo bar fading between them. Then the
   QR code with the phone (as C3's phone test, but into the app): the photo processed the
   same way (with no holes in it: every shot "no hole found").

## C5. Real photos

From the user: about 10 photos of a card without holes, and about 10 of a card with holes
(some in the black, some in the white, some cutting a ring line, two or three overlapping):
flat on a table, card filling 60–90 % of the frame, angles up to about 15°, daylight and lamp
light, a dark and a light table; the original files (not sent through a chat app), and which
phone.

**Tests:** registration on every photo (the residual; the warnings); the same card's scale
and the lens's distortion consistent across photos of one phone; then the holes: the same
hole found at the same card position in every photo (its spread is the photo method's own
precision; expected under 0.05 mm).

## C6. The serial number

Later: where and how the serial is printed (a close-up from the user), then reading its 6
digits from the rectified card.

## Results

Newest first.

| Date | Step | Setup | Result |
|---|---|---|---|
| 2026-11-23 | C4 | The user's look at the demo: the photo's rings beside the printed card's, about 0.8 mm apart, its numbers turned 90°; then test 2 added, run by Claude | **The photo is placed right; the view mixes two frames.** The disc's centre: the printed card (+0.007, −0.011) mm (the measurement), the photo without the placement (−0.010, +0.015) (the registration), with it (+0.388, +0.707) against the demo's card centre (+0.406, +0.696). The demo puts the card 0.8 mm off the sensor's centre and turned 90.5° in the holder; the card view draws the printed card centred and upright in the sensor's frame, and the photo where the real card hung. So the rings can't coincide, and the numbers show turned. Test 1's check (the table under each shot) would have passed a 2 mm error: test 2 now measures the photo's place. Open: which frame the view should use (to decide with the user). |
| 2026-11-23 | C4 | `cargo test` in `app/` (`photo::tests`), run by Claude; the UI not looked at (no screen capture or input on this Wayland desktop) | **Pass**, tests 1–3. The demo photo: 3.0 s (dev build), the rings 0.008 mm rms over 7509 of 7621 points, 11.0 px/mm, the card's turn 0.44° (true 0.5°), its shift 0.85 mm (true 0.81 mm), the sensor against the holes 0.086 mm rms, the worst 0.14 mm. **Found on the way:** the placement's quarter turn is relative to the registration's own axes (the outline fixes those only modulo 90°), not to the print: the test compared it with the print's and failed; the photo on the grid was right all along (it goes through the same placement). |
| 2026-11-23 | C3 | `cargo test` in `app/` (HTTP over TCP to 127.0.0.1, and once over the LAN address 192.168.2.201:8710), run by Claude | **Pass**, tests 1–9. The phone-like JPEG: 5 segments dropped (APP1 Exif, APP1 XMP, APP2 MPF, COM, 400 trailing bytes), pixels identical, orientation 6 kept. **Found on the way:** the size limit's message said "0 MB" for limits under 1 MB (now rounded up). The phone itself not yet tried (the user). |
| 2026-11-23 | C2 | Synthetic photos in colour (`tests/holes.rs`), as C1, 9 holes (two in the black near the centre, one across ring 6's line, one across the disc's edge, two in the white, three overlapping), run by Claude | **Pass**, 6 tests, 28 s. Hole centres within 0.019 mm (all four quarter turns, blue table), 0.022 mm (grey table), 0.026 mm (with a false shot and an extra hole: both flagged); the quarter turn right every time; the placement's angle 0.40–0.49° against 0.4°, its rms at the shots 0.05–0.07 mm (the shots' noise); clean card: no holes; unguided: 9 of 9, within 0.022 mm. Dark table (0.1 against the ink's 0.07): 5 of 9 not found (those in the black), none misplaced. **Found and changed on the way:** (1) an expected card per 5° sector failed: out to 52 mm a hole covers a sector's whole width; now the light as smooth surfaces and the print as one profile over the full circle. (2) A hole with one contrast inside was biased 0.06 mm across the disc's edge (a blue table differs more from paper than from ink); the model is now the table's colour inside and the expected card outside: 0.001 mm. (3) The matched filter missed overlapping holes in the unguided search (their neighbours are in its ring); it now starts from blobs and their area. |
| 2026-11-23 | C1 | Synthetic photos (`tests/register.rs`), 2400 × 1800 px, the card at 9 px/mm, 3 × 3 samples per pixel, blur σ 1 px, run by Claude | **Pass**, 7 tests, 12 s. Worst error over the scoring area (r ≤ 78 mm): easy (10° turn, tilts 5°/3°, light ±20 %) 0.014 mm; hard (137°, tilts 12°/−9°, off-centre, near camera, k1 0.08, k2 −0.02, 9 holes, three overlapping, light ±30 %, noise 3 %) 0.016 mm, k1 found 0.0797; light table (0.72 against the paper's 0.93) 0.014 mm; barrel k1 −0.12 with 15°/10° tilt 0.013 mm, k1 −0.1213. The edge points' rms 0.006–0.010 mm; 92–96 % of them kept. The outline fixed the rotation in every case. A card bent 2 mm at its edges: rms 0.019 mm against 0.006 flat (so a bend shows), and up to 0.24 mm wrong: flatness matters. No card in view, and a card at 3 px/mm: an error each. |
