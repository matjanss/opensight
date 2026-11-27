//! Counting cards (`CARDPHOTO.md` C7): a card photo apart from any series, only to score
//! the card. Count mode in the main window: one link for the phone takes photo after photo (up to
//! `COUNT_MAX`), a worker thread counts them in turn (the holes found without shots, each
//! scored on the printed card), and the app shows each card's photo with its holes as
//! rings, its score, and the total over the cards. The photos are stored as card photos are
//! (`photos/`, removed after 30 days); the count keeps only the card it shows in memory.

use std::path::PathBuf;
use std::sync::mpsc;

use eframe::egui::{self, Align, Color32, Layout, RichText, Stroke, Ui, Vec2};

use crate::card::{Card, CardTexture, Score};
use crate::{
    BAR_BUTTON, BAR_PAD, CardView, LAST_SHOT_COLOR, PhotoJob, ZOOM_LABEL, card as panel, dashed_circle,
    fade_control, notice, open_folder, paint_quad, photo, qr_code, review_detail, review_notice, score_chart,
    upload, zoom_controls,
};

/// The most cards a count holds.
pub const COUNT_MAX: usize = 20;
/// The series name the photos are stored under.
const SERIES: &str = "count";
/// Below this width the card and the list stack.
const STACK_BELOW: f32 = 760.0;
const SIDE_WIDTH: f32 = 360.0;

/// What count mode needs from the app's settings.
pub struct CountEnv<'a> {
    pub card: &'a Card,
    /// Decimal scores (the setting, on a card that has them).
    pub decimal: bool,
    pub zoom_controls: bool,
    /// The close-call margin, mm, when the notice is on.
    pub close_call_mm: Option<f32>,
    pub photos_dir: Option<PathBuf>,
}

/// One hole on a counted card: mm from the card's centre (y up), its score.
#[derive(Clone, Debug)]
struct Hole {
    at: (f64, f64),
    score: Score,
    overlap: bool,
    by: By,
}

/// Where a hole came from: the count found it, or the shooter added it by hand (one the
/// count missed: a black hole on a dark background); a found one can be removed by hand (not
/// a hole, or one hole counted twice) and restored. A removed hole doesn't count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum By {
    Found,
    Added,
    Removed,
}

/// Adding a hole by hand on the shown card: its place (mm, once clicked), and the grab
/// offset while it is dragged.
struct Placing {
    card: u32,
    at: Option<(f64, f64)>,
    grab: Option<Vec2>,
}

/// A card counted.
struct Count {
    /// Best first.
    holes: Vec<Hole>,
    warnings: Vec<String>,
    details: String,
    /// The photo on the card's grid (`photo::save_grid`), or why there is none.
    grid: Result<PathBuf, String>,
}

impl Count {
    fn from(p: photo::Processed, grid: Result<PathBuf, String>, card: &Card) -> Count {
        let mut holes: Vec<Hole> = p
            .holes
            .iter()
            .enumerate()
            .map(|(i, &at)| Hole {
                at,
                score: card.score(at.0 as f32, at.1 as f32),
                overlap: p.hole_overlap.get(i).copied().unwrap_or(false),
                by: By::Found,
            })
            .collect();
        sort_holes(&mut holes);
        let details = format!(
            "{:.1} px per mm; the rings fit to {:.3} mm rms ({} of {} edge points); lens k1 {:+.3}. Taken {:.0}° \
             from straight above: the holes expected to within about {:.2} mm (rms). Processed in {:.1} s. The \
             photo: {}.",
            p.px_per_mm,
            p.reg_rms_mm,
            p.reg_points.0,
            p.reg_points.1,
            p.k1,
            p.camera_deg,
            p.expected_mm,
            p.seconds,
            p.photo.meta.image
        );
        Count {
            holes,
            warnings: p.warnings_with(false),
            details,
            grid,
        }
    }

    /// The holes that count: found or added, not removed.
    fn counted(&self) -> impl Iterator<Item = &Hole> {
        self.holes.iter().filter(|h| h.by != By::Removed)
    }

    fn n_holes(&self) -> usize {
        self.counted().count()
    }

    fn rings(&self) -> u32 {
        self.counted().map(|h| u32::from(h.score.ring)).sum()
    }

    fn tenths(&self) -> u32 {
        self.counted().map(|h| u32::from(h.score.tenths)).sum()
    }

    /// A hole added by hand at `at`, scored on the card. Returns its place in the list.
    fn add(&mut self, at: (f64, f64), card: &Card) -> usize {
        self.holes.push(Hole {
            at,
            score: card.score(at.0 as f32, at.1 as f32),
            overlap: false,
            by: By::Added,
        });
        let new = self.holes.len() - 1;
        let key = |h: &Hole| (h.at.0.to_bits(), h.at.1.to_bits(), h.by);
        let k = key(&self.holes[new]);
        sort_holes(&mut self.holes);
        self.holes.iter().rposition(|h| key(h) == k).unwrap_or(new)
    }

    /// Hole `i` removed: an added one goes, a found one stays listed as removed.
    fn remove(&mut self, i: usize) {
        match self.holes.get(i).map(|h| h.by) {
            Some(By::Added) => {
                self.holes.remove(i);
            }
            Some(By::Found) => self.holes[i].by = By::Removed,
            _ => {}
        }
    }

    fn restore(&mut self, i: usize) {
        if let Some(h) = self.holes.get_mut(i).filter(|h| h.by == By::Removed) {
            h.by = By::Found;
        }
    }

    fn total(&self, decimal: bool) -> String {
        total_text(self.rings(), self.tenths(), decimal)
    }
}

/// Best first; equal scores the nearer to the centre first.
fn sort_holes(holes: &mut [Hole]) {
    holes.sort_by(|a, b| {
        (b.score.tenths, b.score.inner_ten)
            .cmp(&(a.score.tenths, a.score.inner_ten))
            .then(a.at.0.hypot(a.at.1).total_cmp(&b.at.0.hypot(b.at.1)))
    });
}

fn total_text(rings: u32, tenths: u32, decimal: bool) -> String {
    if decimal {
        format!("{}.{}", tenths / 10, tenths % 10)
    } else {
        rings.to_string()
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

enum State {
    Waiting,
    Counting,
    Done(Box<Count>),
    /// Not counted: why (the card not found in the photo, mostly).
    Failed(String),
}

struct Counted {
    /// Numbered as they arrive, never renumbered (the phone shows the same numbers).
    n: u32,
    state: State,
}

impl Counted {
    /// The line in the list and on the phone.
    fn line(&self, decimal: bool) -> String {
        match &self.state {
            State::Waiting => format!("Card {}: waiting", self.n),
            State::Counting => format!("Card {}: counting…", self.n),
            State::Done(c) => format!(
                "Card {}: {}, {}",
                self.n,
                c.total(decimal),
                plural(c.n_holes(), "hole", "holes")
            ),
            State::Failed(e) => format!("Card {}: not counted, {e}. Take it again.", self.n),
        }
    }
}

/// A change by hand to the shown card's holes (the side panel's buttons, the card's keys).
enum HoleEdit {
    StartAdd,
    Confirm,
    Cancel,
    Remove(usize),
    Restore(usize),
}

/// What the worker tells the count.
enum News {
    Started(u32),
    Done(
        u32,
        Result<(Box<photo::Processed>, Result<PathBuf, String>), String>,
    ),
}

/// The count, from the first switch to count mode until the app closes.
pub struct CountTask {
    server: Result<upload::UploadServer, String>,
    /// The phone pressed "Finished": the server stopped until "New QR code".
    phone_done: bool,
    /// For a new server: its folder, and the context it wakes.
    dir: PathBuf,
    ctx: egui::Context,
    url: Option<String>,
    qr: Option<(usize, Vec<bool>)>,
    status: String,
    show_link: bool,
    cards: Vec<Counted>,
    next_n: u32,
    /// The card shown; `follow`: the newest counted one, until the user picks one.
    selected: Option<u32>,
    follow: bool,
    jobs: mpsc::Sender<(u32, PhotoJob)>,
    news: mpsc::Receiver<News>,
    /// The shown card's photo, and the card it is of.
    texture: Option<(u32, egui::TextureHandle)>,
    card_texture: CardTexture,
    view: CardView,
    fade: f32,
    selected_hole: Option<usize>,
    /// Adding a hole by hand on the shown card, while on.
    placing: Option<Placing>,
    /// Where the card was drawn last (the UI test places holes by it).
    card_rect: egui::Rect,
    /// The lines the phone was given last.
    lines: Vec<String>,
    error: Option<String>,
    /// "Start over" clicked once: the second click (on "Remove all") clears the cards.
    confirm_start_over: bool,
}

impl CountTask {
    /// The server started with a session link, the worker waiting for photos.
    pub fn open(ctx: &egui::Context, dir: PathBuf) -> CountTask {
        let c = ctx.clone();
        let server = upload::UploadServer::start_lan(dir.clone(), move || c.request_repaint());
        let dir_for_worker = dir.clone();
        let (url, qr, status) = match &server {
            Ok(s) => {
                let url = s.offer_session(SERIES, COUNT_MAX);
                let qr = upload::qr(&url).ok();
                (Some(url), qr, "Waiting for the phone…".to_string())
            }
            Err(_) => (None, None, String::new()),
        };
        let (jobs, rx) = mpsc::channel::<(u32, PhotoJob)>();
        let (tx, news) = mpsc::channel();
        let c = ctx.clone();
        std::thread::Builder::new()
            .name("count-cards".into())
            .spawn(move || worker(dir_for_worker, rx, tx, c))
            .expect("no thread for counting cards");
        CountTask {
            server,
            phone_done: false,
            dir,
            ctx: ctx.clone(),
            url,
            qr,
            status,
            show_link: true,
            cards: Vec::new(),
            next_n: 1,
            selected: None,
            follow: true,
            jobs,
            news,
            texture: None,
            card_texture: CardTexture::default(),
            view: CardView::default(),
            fade: 0.6,
            selected_hole: None,
            placing: None,
            card_rect: egui::Rect::NOTHING,
            lines: Vec::new(),
            confirm_start_over: false,
            error: None,
        }
    }

    /// A photo in: a place in the list, a job for the worker.
    fn add(&mut self, job: PhotoJob) {
        let n = self.next_n;
        self.next_n += 1;
        self.cards.push(Counted {
            n,
            state: State::Waiting,
        });
        let _ = self.jobs.send((n, job));
        self.show_link = false;
    }

    /// The server's news, the worker's, and the phone's lines.
    pub fn poll(&mut self, env: &CountEnv) {
        let mut received = Vec::new();
        let mut finished = false;
        if let Ok(s) = &self.server {
            while let Some(e) = s.try_recv() {
                match e {
                    upload::UploadEvent::Opened { .. } => self.status = "The phone opened the page.".into(),
                    upload::UploadEvent::Refused { reason, .. } => self.status = format!("Refused: {reason}"),
                    upload::UploadEvent::Received(p) => {
                        self.status = format!("Received a photo ({:.1} MB).", p.meta.bytes as f64 / 1e6);
                        received.push(p);
                    }
                    upload::UploadEvent::Finished => finished = true,
                }
            }
        }
        if finished {
            // The server stops (dropped); the photos it took are still counted.
            self.server = Err("the phone finished".into());
            self.phone_done = true;
            self.show_link = false;
            self.url = None;
            self.qr = None;
            self.status = "The phone finished: the link is closed.".into();
        }
        for p in received {
            self.add(PhotoJob::Stored(p));
        }
        while let Ok(news) = self.news.try_recv() {
            match news {
                News::Started(n) => {
                    if let Some(c) = self.cards.iter_mut().find(|c| c.n == n) {
                        c.state = State::Counting;
                    }
                }
                News::Done(n, r) => {
                    // Gone when removed meanwhile.
                    let Some(c) = self.cards.iter_mut().find(|c| c.n == n) else {
                        continue;
                    };
                    c.state = match r {
                        Ok((p, grid)) => State::Done(Box::new(Count::from(*p, grid, env.card))),
                        Err(e) => State::Failed(e),
                    };
                    if self.follow {
                        self.select(Some(n));
                    }
                }
            }
        }
        let mut lines: Vec<String> = self.cards.iter().map(|c| c.line(env.decimal)).collect();
        if self.done().count() >= 2 {
            // Last: the phone shows the newest first, so the total on top.
            lines.push(format!("All: {}", self.total_line(env.decimal)));
        }
        if lines != self.lines {
            if let Ok(s) = &self.server {
                s.set_lines(lines.clone());
            }
            self.lines = lines;
        }
    }

    fn done(&self) -> impl Iterator<Item = &Count> {
        self.cards.iter().filter_map(|c| match &c.state {
            State::Done(d) => Some(d.as_ref()),
            _ => None,
        })
    }

    /// "548, 6 cards, 58 holes".
    fn total_line(&self, decimal: bool) -> String {
        let (rings, tenths, holes, cards) = self.done().fold((0, 0, 0, 0), |a, c| {
            (a.0 + c.rings(), a.1 + c.tenths(), a.2 + c.n_holes(), a.3 + 1)
        });
        format!(
            "{}, {}, {}",
            total_text(rings, tenths, decimal),
            plural(cards, "card", "cards"),
            plural(holes, "hole", "holes")
        )
    }

    fn select(&mut self, n: Option<u32>) {
        if n != self.selected {
            self.selected_hole = None;
            self.placing = None;
        }
        self.selected = n;
    }

    /// The shown card's count, to change it.
    fn shown_mut(&mut self) -> Option<&mut Count> {
        let n = self.selected?;
        self.cards.iter_mut().find(|c| c.n == n).and_then(|c| match &mut c.state {
            State::Done(d) => Some(d.as_mut()),
            _ => None,
        })
    }

    /// A change by hand to the shown card's holes.
    fn edit(&mut self, e: HoleEdit, card: &Card) {
        match e {
            HoleEdit::StartAdd => {
                if let Some(n) = self.selected {
                    self.placing = Some(Placing { card: n, at: None, grab: None });
                    self.selected_hole = None;
                }
            }
            HoleEdit::Cancel => self.placing = None,
            HoleEdit::Confirm => {
                let at = self.placing.as_ref().and_then(|p| p.at);
                if let (Some(at), Some(d)) = (at, self.shown_mut()) {
                    let i = d.add(at, card);
                    self.selected_hole = Some(i);
                    self.placing = None;
                }
            }
            HoleEdit::Remove(i) => {
                let added = self
                    .shown_mut()
                    .is_some_and(|d| d.holes.get(i).is_some_and(|h| h.by == By::Added));
                if let Some(d) = self.shown_mut() {
                    d.remove(i);
                }
                if added && self.selected_hole.is_some_and(|s| s >= i) {
                    self.selected_hole = None; // the list moved up
                }
            }
            HoleEdit::Restore(i) => {
                if let Some(d) = self.shown_mut() {
                    d.restore(i);
                }
            }
        }
    }

    /// All cards gone, a new link for the phone (the old one stops). Photos the worker is
    /// still counting are dropped when they come back (their card is gone).
    fn start_over(&mut self) {
        self.cards.clear();
        self.select(None);
        self.follow = true;
        self.texture = None;
        self.error = None;
        self.show_link = true;
        if self.phone_done {
            self.new_link();
        } else if let Ok(s) = &self.server {
            let url = s.offer_session(SERIES, COUNT_MAX);
            self.qr = upload::qr(&url).ok();
            self.url = Some(url);
            self.status = "Waiting for the phone…".into();
        }
        self.lines.clear();
    }

    /// After the phone's "Finished": the server again, a session with room for the cards
    /// the count can still take.
    fn new_link(&mut self) {
        let c = self.ctx.clone();
        self.server = upload::UploadServer::start_lan(self.dir.clone(), move || c.request_repaint());
        self.phone_done = false;
        self.show_link = true;
        self.lines.clear();
        match &self.server {
            Ok(s) => {
                let url = s.offer_session(SERIES, COUNT_MAX.saturating_sub(self.cards.len()));
                self.qr = upload::qr(&url).ok();
                self.url = Some(url);
                self.status = "Waiting for the phone…".into();
            }
            Err(_) => {
                self.url = None;
                self.qr = None;
            }
        }
    }

    fn remove(&mut self, n: u32) {
        self.cards.retain(|c| c.n != n);
        if let Ok(s) = &self.server {
            s.free_slot();
        }
        if self.selected == Some(n) {
            self.select(None);
            self.follow = true;
        }
        if self.texture.as_ref().is_some_and(|t| t.0 == n) {
            self.texture = None;
        }
    }

    /// Count mode in the main window: the card, and the side column with `top` above the
    /// list (the app's mode buttons).
    pub fn ui(&mut self, ui: &mut Ui, env: &CountEnv, top: impl FnOnce(&mut Ui)) {
        // Photo files dropped on the window: counted like the phone's, within the limit.
        let dropped: Vec<Vec<u8>> = ui.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.bytes().ok())
                .collect()
        });
        for b in dropped {
            let room = match &self.server {
                Ok(s) => s.take_slot(),
                Err(_) => self.cards.len() < COUNT_MAX,
            };
            if room {
                self.add(PhotoJob::Bytes(b));
            } else {
                self.error = Some(format!(
                    "A count holds {COUNT_MAX} cards, its most. Remove one, or start over."
                ));
            }
        }
        // Up and down (or left and right) flip through the cards.
        let step = ui.input(|i| {
            if i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::ArrowRight) {
                1
            } else if i.key_pressed(egui::Key::ArrowUp) || i.key_pressed(egui::Key::ArrowLeft) {
                -1
            } else {
                0
            }
        });
        if step != 0 && !self.cards.is_empty() && self.placing.is_none() {
            let at = self.cards.iter().position(|c| Some(c.n) == self.selected);
            let i = match at {
                Some(i) => (i as i64 + step).clamp(0, self.cards.len() as i64 - 1) as usize,
                None => self.cards.len() - 1,
            };
            self.follow = false;
            self.select(Some(self.cards[i].n));
        }

        let avail = ui.available_size();
        let gap = 16.0;
        let aspect = env.card.size_mm().y / env.card.size_mm().x;
        if avail.x >= STACK_BELOW {
            let card_w = (avail.x - SIDE_WIDTH - gap).min(avail.y / aspect).max(160.0);
            let side_w = (avail.x - card_w - gap).max(SIDE_WIDTH);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                self.card_view(ui, card_w, env);
                ui.vertical(|ui| {
                    ui.set_width(side_w);
                    top(ui);
                    egui::ScrollArea::vertical()
                        .id_salt("count_side")
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.side(ui, env));
                });
            });
        } else {
            egui::ScrollArea::vertical()
                .id_salt("count_stacked")
                .show(ui, |ui| {
                    let width = ui.available_width();
                    self.card_view(ui, width, env);
                    ui.add_space(6.0);
                    top(ui);
                    self.side(ui, env);
                });
        }
    }

    /// The list of cards, the total, the link, and the shown card's holes.
    fn side(&mut self, ui: &mut Ui, env: &CountEnv) {
        if let Some(e) = self.error.clone()
            && notice(ui, &e)
        {
            self.error = None;
        }
        let decimal = env.decimal;
        let (mut pick, mut remove, mut toggle_link) = (None, None, false);
        let (mut start_over, mut confirm) = (false, self.confirm_start_over);
        panel(ui, "Cards", |ui| {
            if self.cards.is_empty() {
                ui.label(RichText::new("No cards yet.").weak());
            }
            for c in &self.cards {
                ui.horizontal(|ui| {
                    let busy = matches!(c.state, State::Waiting | State::Counting);
                    if busy {
                        ui.spinner();
                    }
                    let mut text = RichText::new(c.line(decimal));
                    if matches!(c.state, State::Failed(_)) {
                        text = text.color(ui.visuals().error_fg_color);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new("×").frame(false))
                            .on_hover_text("Remove this card (the photo file stays)")
                            .clicked()
                        {
                            remove = Some(c.n);
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            let r = ui.add(egui::Button::selectable(self.selected == Some(c.n), text).wrap());
                            if r.clicked() {
                                pick = Some(c.n);
                            }
                        });
                    });
                });
            }
            if self.done().count() > 0 {
                ui.separator();
                let (rings, tenths) = self
                    .done()
                    .fold((0, 0), |a, c| (a.0 + c.rings(), a.1 + c.tenths()));
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(total_text(rings, tenths, decimal))
                            .size(28.0)
                            .strong(),
                    );
                    let holes: usize = self.done().map(|c| c.n_holes()).sum();
                    let mut detail = format!(
                        "{} · {}",
                        plural(self.done().count(), "card", "cards"),
                        plural(holes, "hole", "holes")
                    );
                    let x: usize = self
                        .done()
                        .map(|c| c.counted().filter(|h| h.score.inner_ten).count())
                        .sum();
                    if decimal {
                        detail.push_str(&format!(" · {rings} in whole rings"));
                    } else if x > 0 {
                        detail.push_str(&format!(" · {x}X"));
                    }
                    ui.label(RichText::new(detail).weak());
                });
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let label = if self.show_link || self.cards.is_empty() {
                    "Hide the code"
                } else {
                    "Show the code"
                };
                if !self.cards.is_empty() && !self.phone_done {
                    toggle_link = ui.add(egui::Button::new(label).frame(false)).clicked();
                }
                if let Some(dir) = &env.photos_dir
                    && ui
                        .add(egui::Button::new("Open the folder").frame(false))
                        .clicked()
                {
                    open_folder(dir);
                }
                if !self.cards.is_empty() && !confirm {
                    confirm = ui
                        .add(egui::Button::new("Start over").frame(false))
                        .on_hover_text("Remove all the cards and start a new count (the photo files stay)")
                        .clicked();
                }
            });
            if confirm && !self.cards.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Remove all {}?", plural(self.cards.len(), "card", "cards")));
                    start_over = ui.button("Remove all").clicked();
                    if ui.button("Keep them").clicked() {
                        confirm = false;
                    }
                });
            }
        });
        self.confirm_start_over = confirm && !start_over;
        if start_over {
            self.start_over();
            return;
        }
        let mut new_link = false;
        if self.phone_done {
            panel(ui, "From the phone", |ui| {
                ui.add(egui::Label::new("The phone pressed \"Finished\": the link is closed.").wrap());
                new_link = ui
                    .button("New QR code")
                    .on_hover_text("Start the link again, for more cards from the phone")
                    .clicked();
                ui.label(RichText::new("Or drop photo files on this window.").weak());
            });
        } else if self.show_link || self.cards.is_empty() {
            panel(ui, "From the phone", |ui| {
                match &self.server {
                    Err(e) => {
                        ui.label(format!("No link for the phone: {e}."));
                    }
                    Ok(_) => {
                        ui.add(
                            egui::Label::new(format!(
                                "Scan the code with the phone's camera, then take a photo of each card in turn \
                                 (\"Upload another card\"); up to {COUNT_MAX}. The phone must be on the same Wi-Fi \
                                 as this computer."
                            ))
                            .wrap(),
                        );
                        ui.add_space(8.0);
                        if let Some((w, m)) = &self.qr {
                            qr_code(ui, *w, m, 230.0);
                        }
                        if let Some(u) = &self.url {
                            ui.label(RichText::new(u).small().weak());
                        }
                    }
                }
                ui.add_space(4.0);
                ui.label(RichText::new("Or drop photo files on this window.").weak());
            });
        }
        if !self.status.is_empty() {
            ui.add(egui::Label::new(RichText::new(&self.status).small().weak()).wrap());
        }
        if toggle_link {
            self.show_link = !self.show_link;
        }
        if new_link {
            self.new_link();
        }
        if let Some(n) = pick {
            self.follow = false;
            self.select(Some(n));
        }
        if let Some(n) = remove {
            self.remove(n);
        }

        // The shown card.
        let Some(c) = self.selected.and_then(|n| self.cards.iter().find(|c| c.n == n)) else {
            return;
        };
        let mut hole_pick = None;
        let mut edit = None;
        let placing = &mut self.placing;
        panel(ui, &format!("Card {}", c.n), |ui| match &c.state {
            State::Waiting | State::Counting => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Finding the rings and the holes (a few seconds)…");
                });
            }
            State::Failed(e) => {
                ui.add(egui::Label::new(format!("Not counted: {e}.")).wrap());
                ui.add(
                    egui::Label::new(RichText::new("Remove it with × and take the photo again.").weak())
                        .wrap(),
                );
            }
            State::Done(d) => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(d.total(decimal)).size(28.0).strong());
                    let mut detail = plural(d.n_holes(), "hole", "holes");
                    let by_hand = d.holes.iter().filter(|h| h.by != By::Found).count();
                    if by_hand > 0 {
                        detail.push_str(&format!(" · {by_hand} by hand"));
                    }
                    let x = d.counted().filter(|h| h.score.inner_ten).count();
                    if !decimal && x > 0 {
                        detail.push_str(&format!(" · {x}X"));
                    }
                    ui.label(RichText::new(detail).weak());
                });
                for w in &d.warnings {
                    ui.add(egui::Label::new(format!("⚠ {w}")).wrap());
                }
                if let Err(e) = &d.grid {
                    ui.add(egui::Label::new(format!("⚠ The photo can't be shown: {e}")).wrap());
                }
                if !d.holes.is_empty() {
                    ui.separator();
                    egui::Grid::new(("count_holes", c.n))
                        .num_columns(5)
                        .spacing([18.0, 4.0])
                        .striped(true)
                        .show(ui, |ui| {
                            for t in ["Score", "x mm", "y mm", "", ""] {
                                ui.label(RichText::new(t).small().weak());
                            }
                            ui.end_row();
                            for (i, h) in d.holes.iter().enumerate() {
                                let removed = h.by == By::Removed;
                                let mut s = RichText::new(h.score.text(decimal)).strong();
                                if removed {
                                    s = RichText::new(h.score.text(decimal)).weak().strikethrough();
                                }
                                if self.selected_hole == Some(i) {
                                    s = s.underline();
                                }
                                if ui
                                    .add(egui::Label::new(s).sense(egui::Sense::click()))
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    hole_pick = Some(i);
                                }
                                let coord = |v: f64| {
                                    let t = RichText::new(format!("{v:+.2}"));
                                    if removed { t.weak().strikethrough() } else { t }
                                };
                                ui.label(coord(h.at.0));
                                ui.label(coord(h.at.1));
                                match h.by {
                                    By::Removed => {
                                        ui.label(RichText::new("removed by hand").small().weak());
                                    }
                                    By::Added => {
                                        ui.label(RichText::new("by hand").small().weak());
                                    }
                                    By::Found if h.overlap => {
                                        ui.label("⚠ overlaps").on_hover_text(
                                            "Fitted together with the hole it overlaps: less certain than a single hole.",
                                        );
                                    }
                                    By::Found => {
                                        ui.label("");
                                    }
                                }
                                if removed {
                                    if ui
                                        .add(egui::Button::new("Restore").small().frame(false))
                                        .on_hover_text("Count this hole again")
                                        .clicked()
                                    {
                                        edit = Some(HoleEdit::Restore(i));
                                    }
                                } else if ui
                                    .add(egui::Button::new("×").frame(false))
                                    .on_hover_text("Remove this hole: not a hole, or one hole counted twice")
                                    .clicked()
                                {
                                    edit = Some(HoleEdit::Remove(i));
                                }
                                ui.end_row();
                            }
                        });
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(if decimal {
                            "Holes per score"
                        } else {
                            "Holes per ring"
                        })
                        .small()
                        .weak(),
                    );
                    ui.allocate_ui(egui::vec2(ui.available_width(), 130.0), |ui| {
                        let scores: Vec<Score> = d.counted().map(|h| h.score).collect();
                        score_chart(ui, &scores, decimal);
                    });
                }
                // Adding a hole the count missed.
                ui.separator();
                match placing.as_mut().filter(|p| p.card == c.n) {
                    None => {
                        if ui
                            .button("Add a hole…")
                            .on_hover_text("A hole the count missed: place it on the card yourself.")
                            .clicked()
                        {
                            edit = Some(HoleEdit::StartAdd);
                        }
                    }
                    Some(p) => {
                        ui.label(RichText::new("Add a hole").strong());
                        ui.add(
                            egui::Label::new(
                                RichText::new(
                                    "Click the hole on the card, then drag the ring until it sits on the hole's \
                                     edge. The arrow keys move it 0.05 mm (Shift: 0.5 mm). Zoom in for a close fit.",
                                )
                                .weak(),
                            )
                            .wrap(),
                        );
                        match &mut p.at {
                            Some(at) => {
                                ui.horizontal(|ui| {
                                    ui.label("x");
                                    ui.add(egui::DragValue::new(&mut at.0).speed(0.01).range(-200.0..=200.0).suffix(" mm"));
                                    ui.label("y");
                                    ui.add(egui::DragValue::new(&mut at.1).speed(0.01).range(-200.0..=200.0).suffix(" mm"));
                                });
                                let sc = env.card.score(at.0 as f32, at.1 as f32);
                                ui.label(format!("Score {}", sc.text(decimal)));
                            }
                            None => {
                                ui.label(RichText::new("Not placed yet.").weak());
                            }
                        }
                        ui.horizontal(|ui| {
                            if ui.add_enabled(p.at.is_some(), egui::Button::new("Confirm")).clicked() {
                                edit = Some(HoleEdit::Confirm);
                            }
                            if ui.button("Cancel").clicked() {
                                edit = Some(HoleEdit::Cancel);
                            }
                        });
                    }
                }
                egui::CollapsingHeader::new(RichText::new("The photo's details").small().weak())
                    .id_salt(("count_details", c.n))
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(&d.details).small().weak()).wrap());
                    });
            }
        });
        if let Some(i) = hole_pick {
            self.selected_hole = if self.selected_hole == Some(i) {
                None
            } else {
                Some(i)
            };
        }
        if let Some(e) = edit {
            self.edit(e, env.card);
        }
    }

    /// The shown card's photo under the printed card, its holes as rings; zoom and pan as on
    /// the main card, a hole clicked shows its score.
    fn card_view(&mut self, ui: &mut Ui, width: f32, env: &CountEnv) {
        let card = env.card;
        let card_mm = card.size_mm();
        let base = width / card_mm.x;
        let (rect, response) = ui.allocate_exact_size(card_mm * base, egui::Sense::click_and_drag());
        self.card_rect = rect;
        self.view.clamp(card_mm);
        let view_before = self.view;
        if response.hovered() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = pinch * (scroll / 200.0).exp();
            if factor != 1.0 {
                let anchor = response.hover_pos().map_or(Vec2::ZERO, |p| p - rect.center());
                self.view.zoom_by(factor, anchor, base, card_mm);
            }
            ui.input_mut(|i| i.smooth_scroll_delta.y = 0.0);
        }
        // Adding a hole by hand: a drag that starts on the placed ring moves it, following the
        // pointer where it was grabbed (as on the main card); anywhere else it moves the card.
        let shown_n = self
            .selected
            .filter(|n| self.cards.iter().any(|c| c.n == *n && matches!(c.state, State::Done(_))));
        if self.placing.as_ref().is_some_and(|p| Some(p.card) != shown_n) {
            self.placing = None;
        }
        {
            let scale = base * self.view.zoom;
            let origin = rect.center() - self.view.centre_mm * scale;
            let centre = card.to_card(0.0, 0.0);
            let follow = |pl: &mut Placing, ptr: egui::Pos2, grab: Vec2| {
                let m = (ptr + grab - origin) / scale;
                pl.at = Some(((m.x - centre.x) as f64, (centre.y - m.y) as f64));
            };
            if response.drag_started()
                && let Some(pl) = self.placing.as_mut()
            {
                let reach = (card.pellet_diameter_mm / 2.0 * scale).max(10.0);
                pl.grab = match (pl.at, ui.input(|i| i.pointer.press_origin())) {
                    (Some(a), Some(press)) => {
                        let at = origin + card.to_card(a.0 as f32, a.1 as f32) * scale;
                        (at.distance(press) <= reach).then(|| at - press)
                    }
                    _ => None,
                };
            }
            if response.drag_stopped()
                && let Some(pl) = &mut self.placing
                && let Some(grab) = pl.grab.take()
                && let Some(ptr) = response.interact_pointer_pos().or(ui.input(|i| i.pointer.latest_pos()))
            {
                follow(pl, ptr, grab);
            }
            if let Some(pl) = &mut self.placing
                && let Some(grab) = pl.grab
                && let Some(ptr) = response.interact_pointer_pos()
            {
                follow(pl, ptr, grab);
            }
        }
        let moving = self.placing.as_ref().is_some_and(|p| p.grab.is_some());
        if response.dragged() && !moving {
            self.view.pan(response.drag_delta(), base, card_mm);
        }
        let scale = base * self.view.zoom;
        let origin = rect.center() - self.view.centre_mm * scale;
        let to_screen = |mm: Vec2| origin + mm * scale;
        let centre_mm = card.to_card(0.0, 0.0);
        let to_mm = |p: egui::Pos2| {
            let m = (p - origin) / scale;
            ((m.x - centre_mm.x) as f64, (centre_mm.y - m.y) as f64)
        };
        let painter = ui.painter_at(rect);
        let ppp = ui.ctx().pixels_per_point();
        let quad = |mm: egui::Rect| {
            [mm.left_top(), mm.right_top(), mm.right_bottom(), mm.left_bottom()]
                .map(|c| to_screen(c.to_vec2()))
        };
        if let Some(tex) = self.card_texture.get(ui.ctx(), card, width * ppp) {
            paint_quad(
                &painter,
                tex.id(),
                quad(egui::Rect::from_min_size(egui::Pos2::ZERO, card_mm)),
            );
        }
        if self.view.zoom > 1.0 {
            let visible = egui::Rect::from_center_size(self.view.centre_mm.to_pos2(), rect.size() / scale);
            let px = [
                (visible.width() * scale * ppp).round() as u32,
                (visible.height() * scale * ppp).round() as u32,
            ];
            let settled = self.view == view_before && !response.dragged();
            if let Some((tex, mm)) = self.card_texture.detail(ui.ctx(), card, visible, px, settled) {
                paint_quad(&painter, tex.id(), quad(mm));
            }
            if !settled {
                ui.ctx().request_repaint();
            }
        }

        // The shown card's photo, loaded when it changes (one in memory at a time).
        let shown = self.selected.and_then(|n| {
            self.cards.iter().find(|c| c.n == n).and_then(|c| match &c.state {
                State::Done(d) => Some((n, d.as_ref())),
                _ => None,
            })
        });
        if let Some((n, d)) = shown
            && self.texture.as_ref().is_none_or(|t| t.0 != n)
        {
            self.texture = None;
            if let Ok(path) = &d.grid {
                match photo::load_grid(path) {
                    Ok(img) => {
                        let tex = ui
                            .ctx()
                            .load_texture("count-photo", img, egui::TextureOptions::LINEAR);
                        self.texture = Some((n, tex));
                    }
                    Err(e) => self.error = Some(format!("Card {n}'s photo can't be shown: {e}.")),
                }
            }
        }
        let radius = card.pellet_diameter_mm / 2.0 * scale;
        let holes: &[Hole] = shown.map_or(&[], |(_, d)| &d.holes);
        if let Some((n, tex)) = &self.texture
            && shown.is_some_and(|s| s.0 == *n)
            && self.fade > 0.0
        {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(
                tex.id(),
                egui::Rect::from_min_size(origin, card_mm * scale),
                uv,
                Color32::WHITE.gamma_multiply(self.fade),
            );
        }
        let pos = |h: &Hole| to_screen(card.to_card(h.at.0 as f32, h.at.1 as f32));
        for h in holes {
            let c = pos(h);
            match h.by {
                By::Found => {
                    painter.circle_stroke(c, radius, Stroke::new(3.0, Color32::from_black_alpha(150)));
                    painter.circle_stroke(c, radius, Stroke::new(1.2, Color32::WHITE));
                }
                // Added by hand: white dashes on a dark ring (as the main card's hits by hand).
                By::Added => {
                    painter.circle_stroke(c, radius, Stroke::new(3.0, Color32::from_gray(20)));
                    dashed_circle(&painter, c, radius, Stroke::new(1.5, Color32::WHITE));
                }
                // Removed: faint dashes and a cross, not counted.
                By::Removed => {
                    dashed_circle(&painter, c, radius, Stroke::new(1.2, Color32::from_white_alpha(150)));
                    let arm = radius * 0.6;
                    for d in [egui::vec2(arm, arm), egui::vec2(arm, -arm)] {
                        painter.line_segment([c - d, c + d], Stroke::new(1.2, Color32::from_white_alpha(150)));
                    }
                }
            }
        }
        // The hole being added: translucent, a cross through its centre to line it up.
        if let Some(a) = self.placing.as_ref().and_then(|p| p.at) {
            let c = to_screen(card.to_card(a.0 as f32, a.1 as f32));
            painter.circle_filled(c, radius, LAST_SHOT_COLOR.gamma_multiply(0.45));
            painter.circle_stroke(c, radius, Stroke::new(3.0, Color32::from_black_alpha(160)));
            painter.circle_stroke(c, radius, Stroke::new(1.2, Color32::WHITE));
            let arm = radius * 1.6 + 6.0;
            for d in [egui::vec2(arm, 0.0), egui::vec2(0.0, arm)] {
                painter.line_segment([c - d, c + d], Stroke::new(2.5, Color32::from_black_alpha(160)));
                painter.line_segment([c - d, c + d], Stroke::new(1.0, Color32::WHITE));
            }
        }

        let hit_r = radius.max(6.0);
        let hole_at = |p: egui::Pos2| {
            holes
                .iter()
                .enumerate()
                .map(|(i, h)| (i, pos(h).distance(p)))
                .filter(|&(_, d)| d <= hit_r)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        };
        let placing = self.placing.is_some();
        let on_placed = |p: egui::Pos2| {
            self.placing
                .as_ref()
                .and_then(|pl| pl.at)
                .is_some_and(|a| to_screen(card.to_card(a.0 as f32, a.1 as f32)).distance(p) <= radius.max(10.0))
        };
        if moving || (response.dragged() && self.view.zoom > 1.0) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if placing && response.hover_pos().is_some_and(on_placed) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        } else if placing && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        } else if response.hover_pos().and_then(hole_at).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        } else if response.hovered() && self.view.zoom > 1.0 {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        let mut hole_pick = self.selected_hole;
        let mut remove_hole = None;
        if response.clicked() {
            if let Some(pl) = self.placing.as_mut() {
                // Placing: the click puts the hole there (again: moves it there).
                if let Some(p) = response.interact_pointer_pos() {
                    pl.at = Some(to_mm(p));
                }
            } else {
                let hit = response.interact_pointer_pos().and_then(hole_at);
                hole_pick = if hit == self.selected_hole { None } else { hit };
            }
        }
        let no_focus = ui.memory(|m| m.focused().is_none());
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.placing.is_some() {
                self.placing = None;
            } else {
                hole_pick = None;
            }
        }
        // Placing: the arrow keys move the hole 0.05 mm, with Shift 0.5 mm.
        if no_focus
            && let Some(pl) = self.placing.as_mut()
            && let Some(at) = pl.at.as_mut()
        {
            let (dx, dy, shift) = ui.input(|i| {
                let k = |key| i.key_pressed(key) as i32 as f64;
                (
                    k(egui::Key::ArrowRight) - k(egui::Key::ArrowLeft),
                    k(egui::Key::ArrowUp) - k(egui::Key::ArrowDown),
                    i.modifiers.shift,
                )
            });
            let step = if shift { 0.5 } else { 0.05 };
            at.0 += dx * step;
            at.1 += dy * step;
        }
        // Delete (or Backspace) removes the selected hole.
        if no_focus
            && self.placing.is_none()
            && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
            && let Some(i) = hole_pick.filter(|&i| holes.get(i).is_some_and(|h| h.by != By::Removed))
        {
            remove_hole = Some(i);
        }

        // Zoom bottom right, the photo's fade top right (as on the main card).
        let margin = 10.0;
        let height = BAR_BUTTON + 2.0 * BAR_PAD;
        if env.zoom_controls {
            let zoom_w = 2.0 * BAR_BUTTON + ZOOM_LABEL + 2.0 * BAR_PAD;
            let bar = egui::Rect::from_min_size(
                egui::pos2(rect.right() - margin - zoom_w, rect.bottom() - margin - height),
                egui::vec2(zoom_w, height),
            );
            if let Some(zoom) = zoom_controls(ui, &painter, bar, self.view.zoom) {
                self.view.zoom = zoom;
                self.view.clamp(card_mm);
                ui.ctx().request_repaint();
            }
        }
        if self.texture.is_some() && shown.is_some() {
            let w = 210.0;
            let bar = egui::Rect::from_min_size(
                egui::pos2(rect.right() - margin - w, rect.top() + margin),
                egui::vec2(w, height),
            );
            if let Some(f) = fade_control(ui, &painter, bar, self.fade) {
                self.fade = f;
                ui.ctx().request_repaint();
            }
        }

        // The close calls: holes that nearly scored higher, worth a look at the paper.
        if let Some(margin_mm) = env.close_call_mm {
            let close: Vec<(usize, crate::card::Higher)> = holes
                .iter()
                .enumerate()
                .filter(|(_, h)| h.by != By::Removed)
                .filter_map(|(i, h)| Some((i, card.higher(h.at.0 as f32, h.at.1 as f32)?)))
                .filter(|(_, h)| h.gap_mm <= margin_mm)
                .collect();
            if !close.is_empty() {
                let lines: Vec<String> = close
                    .iter()
                    .map(|&(i, h)| format!("{} · {}", holes[i].score.text(env.decimal), review_detail(h)))
                    .collect();
                let (clicked, _) = review_notice(
                    ui,
                    &painter,
                    rect,
                    "Close Call - Manual review recommended",
                    "Close Call",
                    0.0,
                    "count_close_calls",
                    &lines,
                    Some("Show this hole"),
                );
                if let Some(line) = clicked {
                    hole_pick = Some(close[line].0);
                }
            }
        }

        // The picked hole: marked, its score beside it.
        let picked = hole_pick.and_then(|i| holes.get(i).map(|h| (i, h)));
        let mut restore_hole = None;
        if let Some((i, h)) = picked {
            let p = pos(h);
            if rect.contains(p) {
                painter.circle_stroke(p, radius + 3.0, Stroke::new(3.0, Color32::BLACK));
                painter.circle_stroke(p, radius + 3.0, Stroke::new(1.5, Color32::WHITE));
                let gap = radius + 8.0;
                let (offset, pivot) = if p.x > rect.center().x {
                    (egui::vec2(-gap, -gap), egui::Align2::RIGHT_BOTTOM)
                } else {
                    (egui::vec2(gap, -gap), egui::Align2::LEFT_BOTTOM)
                };
                egui::Area::new(egui::Id::unique("count_hole_info"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(p + offset)
                    .pivot(pivot)
                    .constrain(true)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.horizontal(|ui| {
                                ui.label("Score:");
                                ui.label(RichText::new(h.score.text(env.decimal)).size(20.0).strong());
                            });
                            ui.label(format!("x {:+.2} mm · y {:+.2} mm", h.at.0, h.at.1));
                            let off = h.at.0.hypot(h.at.1);
                            ui.label(RichText::new(format!("{off:.2} mm from the centre")).weak());
                            match h.by {
                                By::Added => {
                                    ui.label(RichText::new("Added by hand").small().weak());
                                }
                                By::Removed => {
                                    ui.label(RichText::new("Removed by hand: not counted").small());
                                }
                                By::Found if h.overlap => {
                                    ui.label(RichText::new("Overlaps another hole: less certain.").small());
                                }
                                By::Found => {}
                            }
                            if let Some(hi) = card
                                .higher(h.at.0 as f32, h.at.1 as f32)
                                .filter(|hi| env.close_call_mm.is_some_and(|m| hi.gap_mm <= m))
                            {
                                let text =
                                    format!("⚠ Close call: {}, manual review recommended", review_detail(hi));
                                ui.label(RichText::new(text).small());
                            }
                            // Not a hole, or one counted twice: away with it (also the
                            // Delete key, and × in the list).
                            if h.by == By::Removed {
                                restore_hole = ui.button("Restore this hole").clicked().then_some(i);
                            } else if ui
                                .button("Remove this hole")
                                .on_hover_text("Not a hole, or one hole counted twice (Delete)")
                                .clicked()
                            {
                                remove_hole = Some(i);
                            }
                        });
                    });
            }
        }
        self.selected_hole = hole_pick;
        if let Some(i) = remove_hole {
            self.edit(HoleEdit::Remove(i), card);
        }
        if let Some(i) = restore_hole {
            self.edit(HoleEdit::Restore(i), card);
        }
    }
}

/// Counts the photos one at a time, in the order they came: stored (a dropped file), the
/// holes found and placed on the printed card, the photo on the card's grid saved beside it.
fn worker(dir: PathBuf, jobs: mpsc::Receiver<(u32, PhotoJob)>, news: mpsc::Sender<News>, ctx: egui::Context) {
    let card = crate::card::air_pistol();
    // Ends when the app closes (the sender dropped).
    while let Ok((n, job)) = jobs.recv() {
        if news.send(News::Started(n)).is_err() {
            return;
        }
        ctx.request_repaint();
        let run = || -> Result<(Box<photo::Processed>, Result<PathBuf, String>), String> {
            let stored = match job {
                PhotoJob::Stored(p) => p,
                PhotoJob::Bytes(b) => upload::store(&dir, SERIES, &b, upload::now_unix())?,
                PhotoJob::Demo => return Err("no demo here".into()),
            };
            let mut p = photo::process(stored, &card, &[])?;
            let grid = match p.image.take() {
                Some(img) => photo::save_grid(&img, &p.photo.image),
                None => Err("no image".into()),
            };
            Ok((Box::new(p), grid))
        };
        if news.send(News::Done(n, run())).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cardphoto::holes::Placement;
    use egui_kittest::kittest::Queryable;

    /// A synthetic photo (dropped, as a file) through the worker: every hole found where it
    /// is on the card, scored as the card scores it; the photo on the card's grid saved and
    /// loaded again; the phone's lines and the total.
    #[test]
    fn counts_a_dropped_photo() {
        let card = crate::card::air_pistol();
        let dir = std::env::temp_dir().join(format!("opensights-count-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // The shots as demo_photo places them on the card (its placement, no error).
        let shots = [
            (1.0, 2.0),
            (-9.0, 14.0),
            (22.0, -6.0),
            (-30.0, -25.0),
            (40.0, 35.0),
            (5.0, -48.0),
        ];
        let place = Placement {
            angle: 90.5f64.to_radians(),
            quarter: 1,
            shift: (0.7, -0.4),
            scale: 1.0,
        };
        let truth: Vec<(f64, f64)> = shots.iter().map(|s| place.apply(*s)).collect();
        let bytes = photo::demo_photo(&card, &shots, 0.0, 3).unwrap();
        let (jobs, rx) = mpsc::channel();
        let (tx, news) = mpsc::channel();
        jobs.send((1, PhotoJob::Bytes(bytes))).unwrap();
        drop(jobs);
        worker(dir.clone(), rx, tx, egui::Context::default());
        assert!(matches!(news.recv().unwrap(), News::Started(1)));
        let News::Done(1, r) = news.recv().unwrap() else {
            panic!("no result")
        };
        let (p, grid) = r.unwrap();
        let grid_file = grid.clone().unwrap();
        let img = photo::load_grid(&grid_file).unwrap();
        assert!(img.size[0] > 1000, "{:?}", img.size);
        let c = Count::from(*p, grid, &card);
        assert_eq!(
            c.holes.len(),
            truth.len(),
            "{:?}",
            c.holes.iter().map(|h| h.at).collect::<Vec<_>>()
        );
        let mut want: u32 = 0;
        for t in &truth {
            let h = c
                .holes
                .iter()
                .min_by(|a, b| {
                    let d = |h: &Hole| (h.at.0 - t.0).hypot(h.at.1 - t.1);
                    d(a).total_cmp(&d(b))
                })
                .unwrap();
            let off = (h.at.0 - t.0).hypot(h.at.1 - t.1);
            println!("{t:?}: off {off:.3} mm, score {}", h.score.text(true));
            assert!(off < 0.1, "{t:?}: {off}");
            assert_eq!(h.score, card.score(t.0 as f32, t.1 as f32), "{t:?}");
            want += u32::from(h.score.ring);
        }
        assert_eq!(c.rings(), want);
        // Best first.
        assert!(c.holes.windows(2).all(|w| w[0].score.tenths >= w[1].score.tenths));
        let line = Counted {
            n: 1,
            state: State::Done(Box::new(c)),
        }
        .line(false);
        assert_eq!(line, format!("Card 1: {want}, 6 holes"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Holes changed by hand (the window driven as the shooter would, a counted card with a
    /// false hole and a missed one): the false one removed by its ×, restored, removed again;
    /// the missed one added (a click, a drag, an arrow key, Confirm), scored and marked; the
    /// added one deleted by the Delete key; Escape stops adding. The totals and the phone's
    /// line follow. `UI_SHOTS=<dir>` saves an image of each step.
    #[test]
    fn holes_changed_by_hand() {
        let card: &'static Card = Box::leak(Box::new(crate::card::air_pistol()));
        let found = [(0.5, 0.3), (-12.0, 4.0), (30.0, -30.0), (60.0, 60.0)];
        let holes = found
            .iter()
            .map(|&at| Hole {
                at,
                score: card.score(at.0 as f32, at.1 as f32),
                overlap: false,
                by: By::Found,
            })
            .collect::<Vec<_>>();
        let mut count = Count {
            holes,
            warnings: Vec::new(),
            details: String::new(),
            grid: Err("a test card, no photo".into()),
        };
        sort_holes(&mut count.holes);
        let rings0 = count.rings();
        let (jobs, _rx) = mpsc::channel();
        let (_tx, news) = mpsc::channel();
        let task = CountTask {
            server: Err("no server in the test".into()),
            phone_done: false,
            dir: std::env::temp_dir(),
            ctx: egui::Context::default(),
            url: None,
            qr: None,
            status: String::new(),
            show_link: false,
            cards: vec![Counted { n: 1, state: State::Done(Box::new(count)) }],
            next_n: 2,
            selected: Some(1),
            follow: false,
            jobs,
            news,
            texture: None,
            card_texture: CardTexture::default(),
            view: CardView::default(),
            fade: 0.6,
            selected_hole: None,
            placing: None,
            card_rect: egui::Rect::NOTHING,
            lines: Vec::new(),
            confirm_start_over: false,
            error: None,
        };
        let env = move || CountEnv {
            card,
            decimal: false,
            zoom_controls: true,
            close_call_mm: Some(0.5),
            photos_dir: None,
        };
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1200.0, 900.0))
            .build_ui_state(
                move |ui, t: &mut CountTask| {
                    crate::install_fonts(ui.ctx());
                    t.poll(&env());
                    t.ui(ui, &env(), |_| {});
                },
                task,
            );
        let shots_dir = std::env::var_os("UI_SHOTS").map(std::path::PathBuf::from);
        let step = |h: &mut egui_kittest::Harness<CountTask>, name: &str| {
            h.run_steps(4);
            if let Some(dir) = &shots_dir {
                h.render().expect("render").save(dir.join(format!("count-{name}.png"))).unwrap();
            }
        };
        step(&mut h, "0-start");
        let d = |t: &CountTask| match &t.cards[0].state {
            State::Done(d) => d.holes.clone(),
            _ => panic!("not counted"),
        };
        let rings = |t: &CountTask| match &t.cards[0].state {
            State::Done(c) => (c.rings(), c.n_holes()),
            _ => panic!(),
        };
        assert_eq!(rings(h.state()), (rings0, 4));
        let at = |t: &CountTask, x: f32, y: f32| {
            let r = t.card_rect;
            r.min + card.to_card(x, y) * (r.width() / card.size_mm().x)
        };
        let click = |h: &mut egui_kittest::Harness<CountTask>, p: egui::Pos2| {
            h.hover_at(p);
            h.run_steps(1);
            h.drag_at(p);
            h.run_steps(1);
            h.drop_at(p);
            h.run_steps(2);
        };
        let false_score = card.score(60.0, 60.0);
        // The false hole, the worst, is the last row: its × (the card list's × comes first).
        let crosses = h.get_all_by_label("×").count();
        assert_eq!(crosses, 1 + 4);
        h.get_all_by_label("×").last().unwrap().click();
        step(&mut h, "1-removed");
        assert_eq!(d(h.state()).last().unwrap().by, By::Removed);
        assert_eq!(rings(h.state()), (rings0 - u32::from(false_score.ring), 3));
        assert!(h.state().lines[0].ends_with(", 3 holes"), "{:?}", h.state().lines);
        h.get_by_label("Restore").click();
        h.run_steps(3);
        assert_eq!(rings(h.state()), (rings0, 4));
        h.get_all_by_label("×").last().unwrap().click();
        h.run_steps(3);
        let base = rings(h.state()).0;

        // The same from the card: a click on the hole shows its popup, with the button.
        let spot = d(h.state()).last().unwrap().at;
        let p = at(h.state(), spot.0 as f32, spot.1 as f32);
        click(&mut h, p);
        step(&mut h, "1b-popup");
        h.get_by_label("Restore this hole").click();
        h.run_steps(3);
        assert_eq!(rings(h.state()), (rings0, 4));
        h.get_by_label("Remove this hole").click();
        h.run_steps(3);
        assert_eq!(d(h.state()).last().unwrap().by, By::Removed);
        assert_eq!(rings(h.state()).0, base);

        // The missed hole at (-20, 15): placed by a click near it, dragged, nudged, confirmed.
        h.get_by_label("Add a hole…").click();
        h.run_steps(3);
        assert!(h.state().placing.is_some());
        let p = at(h.state(), -21.0, 15.0);
        click(&mut h, p);
        let placed = h.state().placing.as_ref().unwrap().at.unwrap();
        let mm_per_pt = card.size_mm().x as f64 / h.state().card_rect.width() as f64;
        assert!((placed.0 + 21.0).abs() < mm_per_pt && (placed.1 - 15.0).abs() < mm_per_pt, "{placed:?}");
        // Dragged from the ring 1 mm right: it follows the pointer, the card stays.
        let (from, to) = (at(h.state(), placed.0 as f32, placed.1 as f32), at(h.state(), placed.0 as f32 + 1.0, placed.1 as f32));
        h.hover_at(from);
        h.run_steps(1);
        h.drag_at(from);
        h.run_steps(1);
        for k in 1..=10 {
            h.hover_at(from + (to - from) * (k as f32 / 10.0));
            h.run_steps(1);
        }
        h.drop_at(to);
        h.run_steps(2);
        let dragged = h.state().placing.as_ref().unwrap().at.unwrap();
        assert!((dragged.0 - placed.0 - 1.0).abs() < 2.0 * mm_per_pt, "{dragged:?}");
        assert!(h.state().view.zoom == 1.0);
        // Up 0.05 mm by the arrow key (the cards don't flip while adding).
        h.key_press(egui::Key::ArrowUp);
        h.run_steps(2);
        let nudged = h.state().placing.as_ref().unwrap().at.unwrap();
        assert!((nudged.1 - dragged.1 - 0.05).abs() < 1e-9, "{nudged:?}");
        assert_eq!(h.state().selected, Some(1));
        step(&mut h, "2-placing");
        h.get_by_label("Confirm").click();
        step(&mut h, "3-added");
        assert!(h.state().placing.is_none());
        let added: Vec<Hole> = d(h.state()).into_iter().filter(|h| h.by == By::Added).collect();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].at, nudged);
        let score = card.score(nudged.0 as f32, nudged.1 as f32);
        assert_eq!(added[0].score, score);
        assert_eq!(rings(h.state()), (base + u32::from(score.ring), 4));
        assert!(h.state().lines[0].ends_with(", 4 holes"), "{:?}", h.state().lines);
        h.get_by_label("by hand");
        // It is selected: Delete takes it away (an added hole goes, not "removed").
        let sel = h.state().selected_hole.unwrap();
        assert_eq!(d(h.state())[sel].by, By::Added);
        h.key_press(egui::Key::Delete);
        h.run_steps(3);
        assert!(d(h.state()).iter().all(|h| h.by != By::Added));
        assert_eq!(rings(h.state()), (base, 3));

        // Escape stops adding.
        h.get_by_label("Add a hole…").click();
        h.run_steps(2);
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(h.state().placing.is_none());
    }
}

/// The real photos (`COUNT_PHOTOS=<folder>`, every JPEG and PNG in it, its subfolders too):
/// each counted as the window counts it, a line per photo. Ignored: run by hand.
#[cfg(test)]
mod real {
    use super::*;

    #[test]
    #[ignore]
    fn counts_real_photos() {
        let Some(dir) = std::env::var_os("COUNT_PHOTOS") else {
            return;
        };
        let mut files = Vec::new();
        let mut todo = vec![PathBuf::from(dir)];
        while let Some(d) = todo.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    todo.push(p);
                } else if p.extension().is_some_and(|x| {
                    ["jpg", "jpeg", "png"].contains(&x.to_ascii_lowercase().to_str().unwrap_or(""))
                }) {
                    files.push(p);
                }
            }
        }
        files.sort();
        let card = crate::card::air_pistol();
        // COUNT_OUT=<folder>: keep the cards' images there (`<photo>.grid.png`) to look at.
        let keep = std::env::var_os("COUNT_OUT").map(std::path::PathBuf::from);
        let out = keep
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join(format!("opensights-count-real-{}", std::process::id())));
        std::fs::create_dir_all(&out).unwrap();
        let (jobs, rx) = mpsc::channel();
        let (tx, news) = mpsc::channel();
        for (i, f) in files.iter().enumerate() {
            jobs.send((i as u32 + 1, PhotoJob::Bytes(std::fs::read(f).unwrap())))
                .unwrap();
        }
        drop(jobs);
        let t0 = std::time::Instant::now();
        worker(out.clone(), rx, tx, egui::Context::default());
        for news in news.try_iter() {
            if let News::Done(n, r) = news {
                let name = files[n as usize - 1]
                    .strip_prefix(std::env::var_os("COUNT_PHOTOS").unwrap())
                    .unwrap();
                let state = match r {
                    Ok((p, grid)) => {
                        if std::env::var_os("COUNT_DEBUG").is_some() {
                            println!(
                                "  print quarter {:?}, margin {:.3}",
                                p.print_quarter, p.print_margin
                            );
                        }
                        State::Done(Box::new(Count::from(*p, grid, &card)))
                    }
                    Err(e) => State::Failed(e),
                };
                let c = Counted { n, state };
                let extra = match &c.state {
                    State::Done(d) => format!(
                        " · {} · {}",
                        d.holes
                            .iter()
                            .map(|h| h.score.text(false))
                            .collect::<Vec<_>>()
                            .join(" "),
                        d.warnings.len()
                    ),
                    _ => String::new(),
                };
                println!("{}: {}{extra}", name.display(), c.line(false));
                if std::env::var_os("COUNT_DEBUG").is_some()
                    && let State::Done(d) = &c.state
                {
                    for w in &d.warnings {
                        println!("    ⚠ {w}");
                    }
                    for h in &d.holes {
                        if let Some(hi) = card
                            .higher(h.at.0 as f32, h.at.1 as f32)
                            .filter(|hi| hi.gap_mm < 0.3)
                        {
                            println!(
                                "    {} at ({:+.2}, {:+.2}): {:.3} mm from {}",
                                h.score.text(false),
                                h.at.0,
                                h.at.1,
                                hi.gap_mm,
                                hi.ring
                            );
                        }
                    }
                }
            }
        }
        println!("{} photos in {:.0} s", files.len(), t0.elapsed().as_secs_f64());
        if keep.is_none() {
            std::fs::remove_dir_all(&out).unwrap();
        }
    }
}

