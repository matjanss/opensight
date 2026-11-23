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
| C2 | The holes: finding them, matching them to the shots | planned |
| C3 | The upload: QR code, the phone page, storage | planned |
| C4 | In the app: the photo under the shots, the ground-truth panel | planned |
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

Builds (in `cardphoto`):

- **Guided:** for each measured shot, a search window of ±3 mm around it in the rectified
  card (the card's placement is not known yet); a 4.5 mm circle fitted to the hole's edge
  (the contrast between the paper and what is behind it; in the black area the other way
  round); overlapping holes fitted together, each started from its shot.
- **The match:** the card's placement (a shift, a small angle, and the quarter turn: all four
  tried, the best one kept) fitted between the shots and the holes; per shot, the difference
  after that fit; shots with no hole and holes no shot explains are flagged.
- **Unguided** (no shots, the test cards): every hole on the card, found by its shape.
- **The results:** a small file per card (registration, hole centres with uncertainties,
  per-shot differences, the placement, the diagnostics, the serial), a patch per hole (8 × 8
  mm at 40 px/mm, greyscale), a low-resolution rectified card.

**Tests (synthetic, automated):** holes at known places (single, cutting ring lines, in the
black, two or three overlapping), the card turned by 0, 90, 180 and 270° plus a small angle,
shots given with a known placement and 0.1 mm of noise. Expected: hole centres within
**0.05 mm**; the quarter turn always right; the placement within 0.03 mm and 0.02°; a missing
hole and an extra hole flagged.

## C3. The upload

Builds (in the app):

- A small HTTP server (`tiny_http`) on the local network while the dialog is open; the QR
  code (`qrcode`) holds `http://<this computer>:<port>/<token>`; the token is random, single
  use, and names the series.
- The page: one "Take photo" button (`<input type="file" accept="image/*"
  capture="environment">`), the upload, a confirmation.
- Storage next to the event logs: the original (JPEG metadata such as GPS stripped from the
  file's segments, the image itself untouched), named by the series; the results files.
  Originals older than 30 days are removed (unless marked for evaluation).

**Tests:** an upload by HTTP from this computer (a real JPEG with GPS metadata: stored, the
metadata gone, the pixels identical); a wrong or used token refused; a too-large file
refused; the clean-up of old originals. The phone itself: by the user, once.

## C4. In the app

Builds: "Card photo" on the series card, the QR dialog, the processing in the background,
the photo under the shots on the card view (a fade slider), and a ground-truth panel (the
fit's quality, the per-shot differences, the placement). Until real shots exist, a
synthetic photo and dummy shots exercise it.

**Tests:** the processing on the synthetic photos from C1/C2 through the app's code path;
the UI by a look (the user, when convenient).

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
| 2026-11-23 | C1 | Synthetic photos (`tests/register.rs`), 2400 × 1800 px, the card at 9 px/mm, 3 × 3 samples per pixel, blur σ 1 px, run by Claude | **Pass**, 7 tests, 12 s. Worst error over the scoring area (r ≤ 78 mm): easy (10° turn, tilts 5°/3°, light ±20 %) 0.014 mm; hard (137°, tilts 12°/−9°, off-centre, near camera, k1 0.08, k2 −0.02, 9 holes, three overlapping, light ±30 %, noise 3 %) 0.016 mm, k1 found 0.0797; light table (0.72 against the paper's 0.93) 0.014 mm; barrel k1 −0.12 with 15°/10° tilt 0.013 mm, k1 −0.1213. The edge points' rms 0.006–0.010 mm; 92–96 % of them kept. The outline fixed the rotation in every case. A card bent 2 mm at its edges: rms 0.019 mm against 0.006 flat (so a bend shows), and up to 0.24 mm wrong: flatness matters. No card in view, and a card at 3 px/mm: an error each. |
