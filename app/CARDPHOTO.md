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
| C3 | The upload: QR code, the phone page, storage | tests over HTTP pass (2026-11-23); the phone test passes (the user, 2026-11-24) |
| C4 | In the app: the photo under the shots, the ground-truth panel | the processing passes through the app's path (2026-11-23); the UI looked at and accepted (the user, 2026-11-24) |
| C5 | Real photos: registration (no holes), then holes | first set (2026-11-25): registration passes after a fix; the holes with the gauge (torn holes): every hole found, 0.04–0.06 mm from straight above; tilted photos, overlapping holes and the app's use still open (see C5) |
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
   printed numbers drop out; Levenberg-Marquardt (`nalgebra`). The edge points are found
   from the disc outwards (C5): the circles out to 38 mm, then to 54 mm, both with the lens
   held, each in a ±2 mm window about the last fit; then all circles with the lens free, in
   windows of ±2, 0.8 and 0.5 mm. (From the disc alone, the outer rings of a close, tilted
   photo can be off by more than the window, or by a whole ring.) Circles fix everything but the card's rotation
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
- **The fit's arithmetic** (2026-11-24): the derivatives worked out directly, each pixel
  with only the circles near it (beyond 36 edge widths a circle's share is below a double's
  resolution), the normal equations built pixel by pixel; big groups in chunks on the cores,
  the groups, the quarter turns and the blobs each on a core. The same cost, the same steps:
  the same holes as before to 0.00001 mm, in a fraction of the time (a 41-shot card 85 s →
  1.4 s). Each centre stays within 4.5 mm of its shot and moves at most 1 mm a step: deep in
  a shot-out cluster a circle hidden by its neighbours has no say in any pixel, and used to
  run off by metres, dragging the group's shared radius down to its limit.
- **The quarter turn** is judged on 10 shots spread over the series (the outer ones), on
  all of them when the runner-up scores 80 % of the best or more, or when a shot then finds
  no hole or pairs are left out as outliers (the turn that matches more holes wins). Each
  turn counts only the coarse holes that agree on one placement within 2 mm: turned 90°, a
  tight group around the centre lands on itself and finds holes everywhere, but scattered.
- **Doubtful groups fitted again: tried and dropped** (2026-11-24). Groups with a hole that
  did not count, two holes on one, or a high residual were fitted again from other starts
  (the matched filter's peaks, close holes pushed apart) and with a radius per hole (torn
  edges); the better fit won. On the bench cases it helped nowhere and made dense torn
  clusters worse (20 shots at 6 mm sd: the worst hole 0.39 → 1.65 mm; 40 at 10 mm: 0.53 →
  3.92 mm): where the paper is shot out, the other starts found other wrong answers.
- **A hole counts as ground truth** only when its fit pins it down: its centre known to
  0.25 mm, its radius within 0.2 mm of the photo's median. Otherwise it is reported as not
  found: a wrong hole is worse than a missing one.
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
found, but none placed wrong. Added 2026-11-24: pairs 0.5–4 mm apart and triples 1.5 mm
apart, with the shots' noise at 0.1 and 0.3 mm, every hole on its own shot's hole within
**0.25 mm**; torn edges (0.1 and 0.25 mm rms) within 0.12 and 0.35 mm; a tight group (20
shots, 6 mm sd) with no hole off by 0.2 mm or more (its shot-out middle not found); the
41-shot demo within 0.06 mm and 30 s.

**Comparing two versions:** `cargo run --release -p cardphoto --example holes_bench --
[--jobs N] [--only a,b] [--photos <the app's photos/>] > out.jsonl` runs a fixed set of
cases (the tests' cards, series of 20–60 shots, pairs, torn holes, and the stored photos
with their shots), one JSON line each: the holes, the placement, the time, and for the
synthetic ones each shot's error against its own hole.

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
- **On the card view, all in the sensor's frame** (the shots and groups never turn): the
  shots where the sensor put them; the photo where the real card hung, so its holes lie
  under the markers (a marker with no hole under it is a shot measured in the wrong place);
  the app's printed card drawn where the real card hung too (turned and shifted in the
  holder: `photo::CardPose`), so its rings and numbers lie on the photo's. A Card–Photo bar
  at the top right fades between the two (0.6 when the photo arrives); the holes found as
  thin rings, also with the photo faded out. Cleared with the series, or by "Remove from
  the card".
- **The score on paper** (the series list): with a photo of the series processed, a
  "Paper" column beside the sensor's score. Each shot's hole is taken back through the pose
  to the printed card's mm and scored by the same rule as the sensor's shots (the card's
  pellet diameter, not the hole's measured size; ring scoring needs only the distance from
  the centre, so an unknown print quarter changes nothing). Equal scores are faint; a score
  that differs is as strong as the sensor's, with a tooltip. "–": no hole found for the
  shot; "·": a shot after the photo. The header line adds the paper's total and how many
  differ. The sensor's scores, total and chart stay the sensor's. The results file holds
  each shot's `paper_mm`, `paper_score` and `sensor_score`.
- **Which way up the card was printed** (needed to draw the app's card turned like the real
  one): the photo's luminance in strips along the four axes, 31–80 mm out, where the numbers
  are, against the app's card at each quarter turn (normalised cross-correlation; the rings
  are the same at every turn, the numbers are not). The best turn must win by 0.1, else a
  warning and the card drawn upright. Without shots the view is the print's frame: the photo
  with its numbers upright, the holes in printed mm.
- **The ground-truth panel** in the photo window: the rms of the sensor against the holes,
  the worst shot; warnings (the card small in the photo, the rings fitting badly, shots with
  no hole, holes no shot explains, the series grown since the photo); per shot the shot,
  hole − shot (the sensor's axes), the distance; the photo's resolution, the rings' rms,
  the lens; the card's centre and turn in the holder (and the print's scale when fitted);
  "Keep the photo for evaluation"; "Take another photo", "Open the folder".

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
   the card's centre in the frame (the demo: 0.8 mm off the sensor's centre). The print's
   quarter turn found by the numbers; the printed card's pose within 0.2° and 0.05 mm of the
   truth (turned −90.5°); the demo's holes (printed mm) through the pose within 0.1 mm of
   their shots. The paper: each shot's `paper` within 0.1 mm of where the demo punched its
   hole in printed mm, and its score the score of that punched point.
2a. The same photo without shots: all 8 holes found, within 0.05 mm of where the demo
   punched them in printed mm (the numbers upright, the printed card not moved).
3. A photo with no card: refused, "no card found".
4. The UI by a look (the user, when convenient): `OPENSIGHTS_DEMO_PHOTO=1 cargo run` in
   `app/`, connect, make a few shots (BOOT, or `sim` on the serial console), "Card photo",
   then "Demo: a synthetic photo of this series". Expected: the spinner for a few seconds; the
   ground-truth panel with about 0.1 mm rms and a row per shot; on the card the photo under
   the shots, a ring around each marker, the Card–Photo bar fading between them. Then the
   QR code with the phone (as C3's phone test, but into the app): the photo processed the
   same way (with no holes in it: every shot "no hole found").
   The series list: a "Paper" column; with the demo's card 0.8 mm off in the holder, in
   decimal most shots differ by a tenth or a few (faint where equal), the header line with
   "paper …" and "(n differ)".

## C5. Real photos

From the user (2026-11-25: one clean card and nine with 3–5 holes each, so more cards than first asked):
10 photos of the clean card, 10 of the holed card with the most varied holes (in the black,
in the white, cutting a ring line, overlapping), 3–4 of each of the other eight; one folder
per card (`~/opensights-photos/clean/`, `card1/`–`card9/`). Flat on a grey or coloured
surface (not black), nothing behind the paper, no flash or shadow; card filling 60–90 % of
the frame; tilts from straight down to 30°; some with the phone's 2× or 3× lens from further
away; daylight and lamp light; the original files (not sent through a chat app), and which
phone; the card's thickness if a calliper is at hand. A 600 dpi scan of each holed card if a
scanner is at hand (the accuracy, not only the spread).

**The card's thickness (not in the model yet):** a hole is a short tube through the card, not
a circle on its surface. Seen at an angle θ from the camera, the surface seen through it is
where the top and bottom openings overlap: narrower by about t·tan θ, its centre about
t·tan θ / 2 off the hole's centre on the surface, toward the camera (the bottom opening
seen through the top one shifts toward the camera by t·tan θ; first written here as away). With t = 0.4 mm that is 0.05 mm at 15°,
0.09 mm at 25°, 0.12 mm at 30°. Even a photo from straight above sees holes off the lens's
axis at an angle: with a phone's main lens (about 70° across) close enough for the card to
fill 70 % of the frame, a hole 78 mm from the centre is seen at about 24°. The tilted photos
and the tele lens's photos measure this; if it shows, the hole model takes the camera's
position (from the registration) and the card's thickness.

**Tests:** registration on every photo (the residual; the warnings); the same card's scale
and the lens's distortion consistent across photos of one phone; then the holes: the same
hole found at the same card position in every photo (its spread is the photo method's own
precision; expected under 0.05 mm), and that spread against the angle each photo saw the hole
at (the thickness above); every hole of every card found, none false.

**The tool:** `cargo run --release -p cardphoto --example c5 -- <photo>... > out.jsonl` in
`app/`: per file the registration (rms, points per circle, the lens, the homography), every
hole the old unguided search finds (`holes`: the openings) and the gauge's circles
(`gauges`, below; `C5_CAL=<mm>` the calibre, 4.5 by default), one JSON line each;
`C5_NOHOLES=1` the registration alone, `C5_RECT=<dir>` each photo on the card's grid (6 px/mm)
with the openings in red and the gauge's circles in blue. The analysis (the camera's pose from
the homography, the holes' spread across photos) was done with scripts outside the repo; the
numbers are in Results.

**What the first set showed (2026-11-25):**

- **Torn holes.** These pellets tore the card rather than punching a clean disc. In the white:
  a dark opening about 3 mm across (the table in the card's shadow), often off-centre, torn
  paper and flaps around it, and a dark grey rim (the pellet's smear) whose outer edge is a
  clean circle. In the black: the opening, and a white halo where the ink flaked off. The old
  hole model (the table's colour inside a circle, the card outside) fits the opening: r
  1.35–1.6 mm in the white, then rejected (its filter wants 2.25 ± 0.5 mm), often as two
  circles. Hence the gauge, below.
- **A light wooden table** (close to the cream paper). The user (2026-11-25): no coloured
  surface for the next set, since shooters won't always use one; the hole model has to work
  on a light table. The gauge does (the rim, not the table, makes the edge). In the app the
  series' shots also say where each hole should be, which the unguided search doesn't have.
- **The tilt.** The scatter of a hole across photos grows with the photo's tilt and with the
  hole's distance from the centre: from straight above (under 10°) 0.04–0.06 mm, at 17–31°
  0.15–0.35 mm for holes 50–80 mm out (the correlation with the photo's ring-fit rms: 0.66).
  The hole fits themselves are good in those photos (edge points within 0.09–0.12 mm of the
  circle), so the mapping is off there: most likely the card isn't flat (a card 0.5 mm off
  the table, seen at 27°, appears 0.25 mm off; from above, hardly at all), which the plane
  homography can't follow. A shift of each white hole by s·tan θ toward the camera, fitted
  over all photos, gave s = −0.25 mm and only lowered their spread from 0.100 to 0.083 mm.
- **The card's thickness:** not seen (t fitted 0.11 ± 0.14 mm with the old model); the torn
  flaps and the card's shape matter more.
- **How the holes were made** (the user): 4.5 mm air pellets or .22 LR (5.6 mm), not known.
  The gauge's circles: r 2.62–2.86 mm in the white (the smear's outer edge, 5.2–5.7 mm across)
  and 2.32–2.45 mm in the black (the halo's): the white ones match .22 LR better than 4.5 mm.
  The scans of cards 1 and 3 will tell. The calibre has to be a setting either way (PLAN: air
  guns and subsonic .22).
- **The set:** 4 holed cards and 1 clean (enough for now, the user). Card 3 has 9 holes, not
  3–5. Still to come: scans of cards 1 and 3, the truth for the photos.

**How others do it (research, 2026-11-25):**

- Phone apps (TargetScan, TargetLog, TargetIQ, Target Scan ISSF, Shotalyze, a YOLO-on-iOS
  write-up at drobinin.com): most find holes with a trained detector (YOLO), take the centre
  from the box, and add the pellet's radius for scoring. Their claims are "within one ring";
  the one paper giving mm reports a mean error of 1.77 mm. Torn and overlapping holes are their
  known weak spot. A detector helps finding holes, not placing them to 0.05 mm.
- Scoring machines and scanner software (DISAG RM IV, scanResult, KreisMeister): light through
  the card or a scan at 600 dpi or more, the centroid of the hole's pixels, aiming at 1/10
  ring; a patent (DE 4207933) calls frayed holes the difficulty.
- US 2011/0218021 (visual image scoring): one fixed radius for all holes (from the card's
  holes, their mean), circles voted over the hole's edge, a sub-pixel refinement.
- The ISSF rule for badly torn or overlapping holes: a transparent overlay with a circle of the
  pellet's size, laid over the hole to rebuild where the pellet went. The plug gauge's spindle
  settles in the hole the same way.

**The gauge (`cardphoto/src/gauge.rs`, `holes::all_gauges`).** A hole as the gauge sees it:
the outer edge of the damage (the rim), not the opening. From a start, 360 rays; on each the
damage (`Expected::diff`, as for the old model) followed out to where it ends: the last drop
below 0.15 within 1 mm past the calibre's radius, with intact card for 0.4 mm after it (flaps
inside are passed over, a neighbour further out not reached), placed at half the rim's height
(halfway between the damage's peak within 0.6 mm inside and the card after it: a faint rim in
a scan and a strong one in a photo give the same edge). A circle fitted to the edge points
(Huber weights: a tear sticking out counts little), the rays cast again from its centre until
it settles. The unguided search: blobs down to 0.2 of the pellet's disc (a torn hole's
opening and rim can be separate small blobs), a gauge from each; kept if its radius is 0.8–1.4
of the calibre's (a ring's number gives 0.9–1.6 mm) and it is damaged inside; one per hole
(the smoother of two on the same damage). Overlapping holes are fitted again together: the
circles' outer arcs make the outline, each edge point on its nearest circle, the radius held
at the median of the single holes in the same zone (black or white).

**The gauge's results** (Results, 2026-11-25): the same count in every file of a card (0 on the
clean card in 14 files, 5, 4, 9, 3), the scans included; from straight above 0.041 mm per axis
in the white and 0.058 mm in the black; card 4's photos against its scan 0.08–0.16 mm (tilted
photos included). Synthetic torn holes (`tests/gauge.rs`: the opening off-centre in shadow,
torn paper, a smear or a halo, a light wooden table, 3° and 25° of tilt): all 8 found, centres
within 0.038 mm, radii within 0.03 mm, the ring numbers and the clean card not taken for holes.

**Not solved yet:**

- **Overlapping holes in the black** (card 3's pair): the outline is ragged (flaked ink, a tail
  into the white) and the pair's separation comes out 1.8–4.7 mm from photo to photo. With the
  series' shots (how many holes, about how far apart) the joint fit is better constrained; until
  then such holes don't count as ground truth.
- **Tilted photos:** either the app asks for a photo from straight above and checks the tilt
  (from the homography and the phone's focal length), or the registration fits the card's
  bend (only possible when tilted: heights show as parallax). The first is cheap.
- **The app** still uses the old hole model (`ground_truth`); the gauge goes in there next, with
  the calibre as a setting.

## C6. The serial number

Later: where and how the serial is printed (a close-up from the user), then reading its 6
digits from the rectified card.

## Results

Newest first.

| Date | Step | Setup | Result |
|---|---|---|---|
| 2026-11-25 | C5 | The gauge (torn holes) on the first set, as above; run by Claude with the `c5` example | **Partly.** Every hole found, the same count in every file of each card: clean 0 (14 files), card 1: 5 (12), card 2: 4 (10), card 3: 9 (12), card 4: 3 (9 and both scans); before: 0–8 per photo. The circles: r 2.62–2.86 mm in the white, 2.32–2.45 mm in the black (±0.01–0.06 per hole across photos); their edge points within 0.06–0.2 mm of the circle in the white, 0.2–0.5 in the black. Spread per hole across photos, each photo's rotation fitted to the matched holes: 0.03–0.08 mm per axis for holes under 45 mm from the centre, 0.10–0.17 mm for those 50–81 mm out; card 3's overlapping pair 0.5 mm (not solved). From straight above (12 photos under 10°): 0.041 mm in the white (54 hole positions), 0.058 mm in the black (15); at 12–33° (30 photos): 0.118 and 0.228 mm. Card 4's photos (their mean) against its scans: 0.08–0.11, 0.15–0.16 (its elongated hole), 0.12 mm. Synthetic torn holes (`tests/gauge.rs`): pass, worst centre 0.038 mm, radius 0.030 mm. `cargo test` passes (cardphoto 8 + 3 + 10 + 7, the app 47), clippy clean. |
| 2026-11-25 | C5 | The user's first set: an iPhone 14 Plus, main lens (26 mm, f/1.5), 4032 × 3024, no flash, a light wooden table, daylight; the clean card 13 photos and a 600 dpi scan; four holed cards (1: 012370, 12 photos; 2: 012373, 10; 3: 12; 4: 012371, 9 and a scan); the camera 160–370 mm above the card, tilts 2–35°, the card's corners seen at up to 53°. Two files damaged in the copy (IMG_3441, IMG_3449: a grey band at one side). Run by Claude with the `c5` example | **Registration: fail, fixed, then pass.** First: 13 photos with only 1,700–5,000 of about 7,900 edge points, rms 0.10–0.19 mm and an absurd lens (k1 up to +1.36, k2 −6.1): the outer rings lost, the first window placed from the disc alone. With the fit grown outwards (C1 4): every file 6,700–7,900 points (4,000–5,000 in two), rms 0.017–0.080 mm (the scans 0.016–0.018; about 0.02 mm from straight above, 0.05–0.08 at 20–33° of tilt), k1 −0.04 to −0.23 with k2 0.07–0.67 (the two trade off), the line offsets d_dark −0.09 to +0.03 and d_light about −0.09 mm. **Holes: partly.** The clean card: none found in 14 files (no false holes). The holes in the black found in every photo; those in the white mostly not (cards 2 and 4: 0–4 found, and those split): fitted at r 1.35–1.6 mm and rejected (see C5). Spread across photos, each photo's rotation fitted to the holes found in at least 80 % of them: card 3's three touching holes sd 0.043–0.078 mm (worst 0.17 mm, seen at 33°), card 1's two holes in the black 0.090 and 0.130 mm; without any alignment, card 1's two holes 34.353 mm apart with sd 0.095 mm, their distances from the centre sd 0.049 and 0.084 mm. Card thickness fitted 0.11 ± 0.14 mm (no effect seen). `cargo test` passes (cardphoto 10 + 7 + 5, the app 47). |
| 2026-11-24 | C2 | The user: a test of 50 shots in the app, from the `photo-speed` worktree | **Pass** (the user's report: "works out perfectly"); the same test from the old code had taken nearly 2 minutes. Merged to `main`, with C4's series panel. **Found on the merge:** `upload_stores_without_metadata` failed once in the full `cargo test` (the photo tests now keep every core busy): the server answered the phone before telling the app, and the test reads the app's event right after the answer. Now the app first; the full `cargo test` passes 3 times in a row. |
| 2026-11-24 | C2 | The holes made faster (the user: "the analysis takes a long time"), old and new compared by `holes_bench` on 38 cases (24 synthetic with their truth, 14 stored demo photos with their shots), run by Claude | **Pass.** Before: 3–5 s a photo, 85 s for 41 shots (a group of 16 touching holes fitted together, the cost growing as n³ with a numerical Jacobian), 4–56 minutes or an error for dense series. Now (release build, one case at a time): the stored photos 0.1–0.4 s (41 shots 1.2 s), the 41-shot demo 1.2 s (115 s), pairs 0.5–1.4 s (18–23 s), dense series of 20–60 shots 3–11 s. The same holes as before to 0.00001 mm on the tests' cards, the stored photos, the demo, the pairs and the torn nine. **Found on the way:** (1) in shot-out clusters, circles hidden by their neighbours ran off by metres and pulled the shared radius to 1.35 mm, both versions: now held to their shots. (2) Tight central groups could get the wrong quarter turn (rms 16–19 mm): now only holes agreeing on one placement count. (3) Holes in a shot-out middle were reported found but 1–4 mm off: now those not pinned down (sd over 0.25 mm, radius 0.2 mm off the median) are not found. (4) Fitting doubtful groups again (the user's idea of a slower, more robust fallback) made things worse (see C2): dropped; the quarter turn's fallback to all shots stays. Dense series against the old code: 20 shots at 6 mm sd 9 found, none off (12, 3 up to 1.9 mm off); 40 at 6 mm 6 found, none off (an error); torn pairs fewer wrong holes (13 → 8, the worst 0.56 → 0.27 mm; 15 → 13); 40 at 6 mm torn 23 found, 10 more than 0.2 mm off, the worst 3.3 mm (22, 9, 2.4 mm); the other dense series within a hole of the old counts. Shot-out fits are chaotic: a change in the last digits of a sum moves those results (the same code twice gives the same). 4 new tests (`overlapping_pairs`, `ragged_holes`, `dense_group_never_misplaced`, `demo_41_shots_in_time`); `cargo test` passes (cardphoto 10 + 7 + 5). |
| 2026-11-24 | C4 | The user's wishes: the photo window closes once the photo is processed (an error keeps it open); the results in the series panel as a table, columns #, OpenSights, Paper (both red where they differ), shot x, y, hole − shot, off, with the summary and warnings above and the photo's buttons below; a click on a series row opens the shot's popup on the card. `cargo test` run by Claude | **Built**, `cargo test` passes (68, 1 ignored: the phone); no warnings. The UI: to be looked at (the user), with `OPENSIGHTS_DEMO_PHOTO=1 cargo run` (the Card photo window's demo button). |
| 2026-11-24 | C3 | CI on Windows (GitHub runner), `cargo test` | **Fail, then fixed.** `too_large_and_not_a_photo_refused`: the chunked upload's 413 came back empty. The server answered after 1 MB and closed with the rest unread; Windows then resets the connection and the answer is lost (a phone would see a broken upload, not "too large"). Now a refused upload is read and dropped (up to 64 MB) before the answer. Linux: the test 30 times in a row, and the whole `cargo test`, pass; Windows: the next CI run. |
| 2026-11-24 | C3, C4 | The user: the phone test (`PHONE_WAIT=120 cargo test lan -- --ignored`), and a look at the UI with `OPENSIGHTS_DEMO_PHOTO=1 cargo run` | **Pass**, both (the user's report). Merged to `main`. C5 waits for real photos. |
| 2026-11-23 | C4 | The score on paper (the user's wish): the "Paper" column in the series list; `cargo test` run by Claude | **Pass**, tests 1–3 (2 extended): every shot's paper position within 0.1 mm of its punched hole, the same score as the punched point; 6 of the demo's 8 shots score differently in tenths on paper (the card 0.8 mm off in the holder). The UI: to be looked at (the user). |
| 2026-11-23 | C4 | The view decided with the user: everything in the sensor's frame, the app's card drawn where the real card hung; the print's way up from the numbers; `cargo test` run by Claude | **Pass**, tests 1–3 (2 and 2a extended). The numbers found the quarter turn with a margin of 0.18 (with shots) and 0.18 (without), against 0.1 needed; with the app's own font (the demo's card is the app's SVG): a real card's font will match less well, C5 will tell. The printed card's pose −90.44° (true −90.5°), its centre (+0.406, +0.695) mm (true +0.406, +0.696). The UI: to be looked at again (the user). |
| 2026-11-23 | C4 | The user's look at the demo: the photo's rings beside the printed card's, about 0.8 mm apart, its numbers turned 90°; then test 2 added, run by Claude | **The photo is placed right; the view mixes two frames.** The disc's centre: the printed card (+0.007, −0.011) mm (the measurement), the photo without the placement (−0.010, +0.015) (the registration), with it (+0.388, +0.707) against the demo's card centre (+0.406, +0.696). The demo puts the card 0.8 mm off the sensor's centre and turned 90.5° in the holder; the card view draws the printed card centred and upright in the sensor's frame, and the photo where the real card hung. So the rings can't coincide, and the numbers show turned. Test 1's check (the table under each shot) would have passed a 2 mm error: test 2 now measures the photo's place. Open: which frame the view should use (to decide with the user). |
| 2026-11-23 | C4 | `cargo test` in `app/` (`photo::tests`), run by Claude; the UI not looked at (no screen capture or input on this Wayland desktop) | **Pass**, tests 1–3. The demo photo: 3.0 s (dev build), the rings 0.008 mm rms over 7509 of 7621 points, 11.0 px/mm, the card's turn 0.44° (true 0.5°), its shift 0.85 mm (true 0.81 mm), the sensor against the holes 0.086 mm rms, the worst 0.14 mm. **Found on the way:** the placement's quarter turn is relative to the registration's own axes (the outline fixes those only modulo 90°), not to the print: the test compared it with the print's and failed; the photo on the grid was right all along (it goes through the same placement). |
| 2026-11-23 | C3 | `cargo test` in `app/` (HTTP over TCP to 127.0.0.1, and once over the LAN address 192.168.2.201:8710), run by Claude | **Pass**, tests 1–9. The phone-like JPEG: 5 segments dropped (APP1 Exif, APP1 XMP, APP2 MPF, COM, 400 trailing bytes), pixels identical, orientation 6 kept. **Found on the way:** the size limit's message said "0 MB" for limits under 1 MB (now rounded up). The phone itself not yet tried (the user). |
| 2026-11-23 | C2 | Synthetic photos in colour (`tests/holes.rs`), as C1, 9 holes (two in the black near the centre, one across ring 6's line, one across the disc's edge, two in the white, three overlapping), run by Claude | **Pass**, 6 tests, 28 s. Hole centres within 0.019 mm (all four quarter turns, blue table), 0.022 mm (grey table), 0.026 mm (with a false shot and an extra hole: both flagged); the quarter turn right every time; the placement's angle 0.40–0.49° against 0.4°, its rms at the shots 0.05–0.07 mm (the shots' noise); clean card: no holes; unguided: 9 of 9, within 0.022 mm. Dark table (0.1 against the ink's 0.07): 5 of 9 not found (those in the black), none misplaced. **Found and changed on the way:** (1) an expected card per 5° sector failed: out to 52 mm a hole covers a sector's whole width; now the light as smooth surfaces and the print as one profile over the full circle. (2) A hole with one contrast inside was biased 0.06 mm across the disc's edge (a blue table differs more from paper than from ink); the model is now the table's colour inside and the expected card outside: 0.001 mm. (3) The matched filter missed overlapping holes in the unguided search (their neighbours are in its ring); it now starts from blobs and their area. |
| 2026-11-23 | C1 | Synthetic photos (`tests/register.rs`), 2400 × 1800 px, the card at 9 px/mm, 3 × 3 samples per pixel, blur σ 1 px, run by Claude | **Pass**, 7 tests, 12 s. Worst error over the scoring area (r ≤ 78 mm): easy (10° turn, tilts 5°/3°, light ±20 %) 0.014 mm; hard (137°, tilts 12°/−9°, off-centre, near camera, k1 0.08, k2 −0.02, 9 holes, three overlapping, light ±30 %, noise 3 %) 0.016 mm, k1 found 0.0797; light table (0.72 against the paper's 0.93) 0.014 mm; barrel k1 −0.12 with 15°/10° tilt 0.013 mm, k1 −0.1213. The edge points' rms 0.006–0.010 mm; 92–96 % of them kept. The outline fixed the rotation in every case. A card bent 2 mm at its edges: rms 0.019 mm against 0.006 flat (so a bend shows), and up to 0.24 mm wrong: flatness matters. No card in view, and a card at 3 px/mm: an error each. |
