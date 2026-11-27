//! The card photo's way from the phone to the app (`CARDPHOTO.md` C3, PLAN D13): while the
//! dialog is open, a small HTTP server on the local network; the app shows a QR code with a
//! link holding a random single-use token; the phone's page takes the photo and sends it
//! as it is. Counting cards (`CARDPHOTO.md` C7) uses a session link instead: it takes photo
//! after photo, up to the app's limit, and the page shows the app's word on each card. The photo is stored next to the event logs, `photos/`, with its metadata (EXIF,
//! GPS) dropped from the file (`cardphoto::meta`, the image untouched), and a JSON file
//! beside it. Originals older than 30 days are removed unless marked to keep; the JSON
//! stays.

use std::collections::HashMap;
use std::io::Read;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::eventlog::civil_from_days;

/// The largest photo taken (a 50 MP phone's JPEG is 10–25 MB).
pub const MAX_BYTES: usize = 40 << 20;
/// Originals older than this are removed (unless marked to keep).
pub const KEEP_DAYS: u64 = 30;
/// The port tried first (a firewall rule can name it); any free one if it is taken.
pub const PORT: u16 = 8710;

/// The JSON file beside each stored photo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhotoMeta {
    /// The series the link was made for.
    pub series: String,
    /// When it arrived: UTC "YYYY-MM-DD HH:MM:SS", and seconds since 1970.
    pub received: String,
    pub received_unix: u64,
    /// The image file's name (in the same folder), "jpeg" or "png", its size as stored and
    /// as sent.
    pub image: String,
    pub format: String,
    pub bytes: usize,
    pub sent_bytes: usize,
    /// The EXIF orientation the phone gave (1 upright, 6 and 8 turned, 3 upside down).
    pub orientation: Option<u16>,
    /// The metadata dropped from the file.
    pub dropped: Vec<String>,
    /// Marked to keep (for evaluation): never removed.
    #[serde(default)]
    pub keep: bool,
    /// The original was removed after `KEEP_DAYS` days.
    #[serde(default)]
    pub removed: bool,
}

/// A photo received and stored.
#[derive(Clone, Debug)]
pub struct Photo {
    pub image: PathBuf,
    pub meta_file: PathBuf,
    pub meta: PhotoMeta,
}

/// What the server tells the app.
#[derive(Debug)]
pub enum UploadEvent {
    /// The phone opened the page.
    Opened {
        series: String,
    },
    Received(Photo),
    /// An upload was refused (too large, not a photo, could not be stored); the link still
    /// works.
    Refused {
        series: String,
        reason: String,
    },
    /// The phone pressed "Finished": the link is gone; the app closes its server.
    Finished,
}

type Notify = Arc<dyn Fn() + Send + Sync>;

/// A live link.
struct Offer {
    series: String,
    /// A session link (counting cards): many photos, `room` more; else one.
    many: bool,
    room: usize,
    /// What the session's page shows, one line per card (the app's words).
    lines: Vec<String>,
}

/// What the phone hears when the session holds the most cards it may.
pub const SESSION_FULL: &str =
    "The app holds as many cards as it can. Remove one there, or start over there.";

struct Shared {
    dir: PathBuf,
    max_bytes: usize,
    /// Live tokens. At most one: a new link ends the old one.
    offers: Mutex<HashMap<String, Offer>>,
    events: Mutex<Sender<UploadEvent>>,
    notify: Notify,
}

impl Shared {
    fn send(&self, e: UploadEvent) {
        let _ = self.events.lock().unwrap().send(e);
        (self.notify)();
    }
}

/// The upload server: runs until dropped.
pub struct UploadServer {
    shared: Arc<Shared>,
    server: Arc<Server>,
    thread: Option<JoinHandle<()>>,
    /// The address in the links (this computer on the network) and the port.
    pub host: IpAddr,
    pub port: u16,
    events: Receiver<UploadEvent>,
}

impl UploadServer {
    /// On the local network, for the app: this computer's address, `PORT` or any free port.
    pub fn start_lan(
        dir: PathBuf,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Result<UploadServer, String> {
        let host = lan_ip().ok_or("this computer is on no network (no Wi-Fi or cable)")?;
        let any = IpAddr::from([0, 0, 0, 0]);
        let notify: Notify = Arc::new(notify);
        Self::start(
            dir.clone(),
            SocketAddr::new(any, PORT),
            host,
            MAX_BYTES,
            notify.clone(),
        )
        .or_else(|_| Self::start(dir, SocketAddr::new(any, 0), host, MAX_BYTES, notify))
    }

    /// Listens on `bind`; links name `host`. Photos go to `dir`.
    pub fn start(
        dir: PathBuf,
        bind: SocketAddr,
        host: IpAddr,
        max_bytes: usize,
        notify: Notify,
    ) -> Result<UploadServer, String> {
        std::fs::create_dir_all(&dir).map_err(|e| format!("no folder for the photos: {e}"))?;
        let server = Arc::new(Server::http(bind).map_err(|e| format!("could not listen on {bind}: {e}"))?);
        let port = server
            .server_addr()
            .to_ip()
            .map(|a| a.port())
            .ok_or("not an IP listener")?;
        let (tx, rx) = channel();
        let shared = Arc::new(Shared {
            dir,
            max_bytes,
            offers: Mutex::new(HashMap::new()),
            events: Mutex::new(tx),
            notify,
        });
        let thread = {
            let (server, shared) = (server.clone(), shared.clone());
            std::thread::Builder::new()
                .name("photo-upload".into())
                .spawn(move || {
                    // Ends when `unblock` (on drop) makes recv fail.
                    while let Ok(req) = server.recv() {
                        handle(&shared, req);
                    }
                })
                .map_err(|e| e.to_string())?
        };
        Ok(UploadServer {
            shared,
            server,
            thread: Some(thread),
            host,
            port,
            events: rx,
        })
    }

    /// A new link for a photo of `series`; any earlier link stops working.
    pub fn offer(&self, series: &str) -> String {
        let token = new_token();
        let mut offers = self.shared.offers.lock().unwrap();
        offers.clear();
        offers.insert(
            token.clone(),
            Offer {
                series: series.to_string(),
                many: false,
                room: 1,
                lines: Vec::new(),
            },
        );
        format!("http://{}/p/{token}", SocketAddr::new(self.host, self.port))
    }

    /// A session link (counting cards): it takes up to `room` photos, until dropped or
    /// replaced; any earlier link stops working.
    pub fn offer_session(&self, series: &str, room: usize) -> String {
        let token = new_token();
        let mut offers = self.shared.offers.lock().unwrap();
        offers.clear();
        offers.insert(
            token.clone(),
            Offer {
                series: series.to_string(),
                many: true,
                room,
                lines: Vec::new(),
            },
        );
        format!("http://{}/p/{token}", SocketAddr::new(self.host, self.port))
    }

    /// The session's lines for the phone's page.
    pub fn set_lines(&self, lines: Vec<String>) {
        for o in self.shared.offers.lock().unwrap().values_mut() {
            o.lines = lines.clone();
        }
    }

    /// Takes a place in the session for a photo that came another way (a dropped file):
    /// false when it is full (or there is no session).
    pub fn take_slot(&self) -> bool {
        let mut offers = self.shared.offers.lock().unwrap();
        let Some(o) = offers.values_mut().find(|o| o.many && o.room > 0) else {
            return false;
        };
        o.room -= 1;
        true
    }

    /// Gives a place back (a card removed in the app).
    pub fn free_slot(&self) {
        for o in self.shared.offers.lock().unwrap().values_mut().filter(|o| o.many) {
            o.room += 1;
        }
    }

    /// The next thing that happened, if any.
    pub fn try_recv(&self) -> Option<UploadEvent> {
        self.events.try_recv().ok()
    }
}

impl Drop for UploadServer {
    fn drop(&mut self) {
        self.server.unblock();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// 128 random bits, hex.
fn new_token() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("no random numbers from the system");
    b.iter().map(|v| format!("{v:02x}")).collect()
}

/// This computer's address on the network the default route leads to (no packet is sent:
/// a UDP socket's connect only picks the route).
pub fn lan_ip() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?; // TEST-NET-1: routed like any outside address
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

/// A QR code of `text`: its width and its modules, row by row, true = dark (the app paints
/// it, with a quiet zone of 4 modules around it).
pub fn qr(text: &str) -> Result<(usize, Vec<bool>), String> {
    let code = qrcode::QrCode::new(text.as_bytes()).map_err(|e| e.to_string())?;
    let w = code.width();
    let dark = code
        .to_colors()
        .into_iter()
        .map(|c| c == qrcode::Color::Dark)
        .collect();
    Ok((w, dark))
}

fn respond(req: Request, code: u16, kind: &str, body: impl Into<Vec<u8>>) {
    let r = Response::from_data(body.into())
        .with_status_code(code)
        .with_header(Header::from_bytes("Content-Type", kind).unwrap())
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
    let _ = req.respond(r);
}

const TEXT: &str = "text/plain; charset=utf-8";

/// How much of a refused upload is read and dropped before the answer goes out.
const DRAIN_MAX: u64 = 64 << 20;

/// Reads the rest of a refused upload, up to `DRAIN_MAX`, so the answer goes out on a
/// connection with nothing left unread. Windows resets a socket that is closed with data
/// still unread, and the reset throws away the answer before the phone reads it: the
/// phone would see a broken connection instead of "too large".
fn drain(req: &mut Request) {
    let _ = std::io::copy(&mut req.as_reader().take(DRAIN_MAX), &mut std::io::sink());
}

fn handle(shared: &Shared, mut req: Request) {
    let url = req.url().to_string();
    let path = url.strip_prefix("/p/").unwrap_or("");
    // A session's page asks for its lines at /p/<token>/cards; either page's "Finished"
    // posts to /p/<token>/finished.
    let (path, finished) = match path.strip_suffix("/finished") {
        Some(t) => (t, true),
        None => (path, false),
    };
    let (token, cards) = match path.strip_suffix("/cards") {
        Some(t) => (t, true),
        None => (path, false),
    };
    let found = shared
        .offers
        .lock()
        .unwrap()
        .get(token)
        .map(|o| (o.series.clone(), o.many, o.lines.clone()));
    let Some((series, many, lines)) = found.filter(|f| !token.is_empty() && (f.1 || !cards) && !(cards && finished))
    else {
        let msg = "This link has been used or has expired. Scan the code in the app again.";
        return if *req.method() == Method::Get && !cards && !finished {
            respond(req, 404, "text/html; charset=utf-8", page(Page::Gone, msg))
        } else {
            respond(req, 404, TEXT, msg)
        };
    };
    match req.method() {
        Method::Post if finished => {
            shared.offers.lock().unwrap().remove(token);
            shared.send(UploadEvent::Finished);
            respond(req, 200, TEXT, "Finished: the app has closed the link. You can close this page.");
        }
        _ if finished => respond(req, 405, TEXT, "Only POST here."),
        Method::Get if cards => {
            let body = serde_json::json!({ "lines": lines }).to_string();
            respond(req, 200, "application/json", body);
        }
        Method::Get => {
            shared.send(UploadEvent::Opened {
                series: series.clone(),
            });
            let kind = if many { Page::Session } else { Page::Single(&series) };
            respond(req, 200, "text/html; charset=utf-8", page(kind, ""));
        }
        Method::Post if cards => respond(req, 405, TEXT, "Only GET here."),
        Method::Post => {
            let refuse = |req: Request, code: u16, reason: String| {
                shared.send(UploadEvent::Refused {
                    series: series.clone(),
                    reason: reason.clone(),
                });
                respond(req, code, TEXT, reason);
            };
            let too_large = format!(
                "The photo is larger than {} MB. Take it again at a lower resolution.",
                shared.max_bytes.div_ceil(1 << 20)
            );
            if let Some(n) = req.body_length().filter(|&n| n > shared.max_bytes) {
                if n as u64 <= DRAIN_MAX {
                    drain(&mut req);
                }
                return refuse(req, 413, too_large);
            }
            let mut body = Vec::new();
            let read = req
                .as_reader()
                .take(shared.max_bytes as u64 + 1)
                .read_to_end(&mut body);
            if let Err(e) = read {
                return refuse(req, 400, format!("The upload broke off ({e}). Try again."));
            }
            if body.len() > shared.max_bytes {
                drain(&mut req);
                return refuse(req, 413, too_large);
            }
            // A place for it: a session may be full (another upload may have taken the last).
            {
                let mut offers = shared.offers.lock().unwrap();
                match offers.get_mut(token) {
                    Some(o) if o.room > 0 => o.room -= 1,
                    Some(_) => {
                        drop(offers);
                        return refuse(req, 409, SESSION_FULL.into());
                    }
                    None => {
                        drop(offers);
                        return respond(req, 404, TEXT, "This link has expired. Scan the code in the app again.");
                    }
                }
            }
            match store(&shared.dir, &series, &body, now_unix()) {
                Ok(photo) => {
                    if !many {
                        // Used: the link works once.
                        shared.offers.lock().unwrap().remove(token);
                    }
                    // The app first, then the phone: whoever hears "received" can count on
                    // the app having the photo (a test reads the event right after the answer).
                    shared.send(UploadEvent::Received(photo));
                    let answer = if many {
                        "Received."
                    } else {
                        "Received. You can close this page."
                    };
                    respond(req, 200, TEXT, answer);
                }
                Err(e) => {
                    if let Some(o) = shared.offers.lock().unwrap().get_mut(token) {
                        o.room += 1;
                    }
                    refuse(req, 415, e)
                }
            }
        }
        _ => respond(req, 405, TEXT, "Only GET and POST."),
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// UTC (date, time) of a Unix time: ("YYYY-MM-DD", "HH:MM:SS").
fn utc(t: u64) -> (String, String) {
    let (y, m, d) = civil_from_days((t / 86_400) as i64);
    let s = t % 86_400;
    (
        format!("{y:04}-{m:02}-{d:02}"),
        format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60),
    )
}

/// Letters, digits, '-' and '_' of a series name, for file names.
fn file_safe(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    if t.is_empty() { "series".into() } else { t }
}

/// Stores a sent photo: the metadata dropped, the image as sent; the JSON beside it.
pub fn store(dir: &Path, series: &str, sent: &[u8], now: u64) -> Result<Photo, String> {
    let (clean, info) = cardphoto::meta::strip(sent).map_err(|e| {
        format!("Not a photo the app can read ({e}). Send a JPEG (on an iPhone: Settings, Camera, Formats, Most Compatible).")
    })?;
    let (date, time) = utc(now);
    let ext = if info.format == "png" { "png" } else { "jpg" };
    let mut stem = format!("{date}_{}_{}", time.replace(':', ""), file_safe(series));
    let mut n = 1;
    while dir.join(format!("{stem}.json")).exists() {
        n += 1;
        stem = format!("{date}_{}_{}_{n}", time.replace(':', ""), file_safe(series));
    }
    let image = dir.join(format!("{stem}.{ext}"));
    let meta_file = dir.join(format!("{stem}.json"));
    let meta = PhotoMeta {
        series: series.to_string(),
        received: format!("{date} {time}"),
        received_unix: now,
        image: format!("{stem}.{ext}"),
        format: info.format.to_string(),
        bytes: clean.len(),
        sent_bytes: sent.len(),
        orientation: info.orientation,
        dropped: info.dropped,
        keep: false,
        removed: false,
    };
    let fail = |e: std::io::Error| format!("The app could not store the photo: {e}");
    write_atomic(&image, &clean).map_err(fail)?;
    write_meta(&meta_file, &meta).map_err(fail)?;
    Ok(Photo {
        image,
        meta_file,
        meta,
    })
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

fn write_meta(path: &Path, meta: &PhotoMeta) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(meta).map_err(std::io::Error::other)?;
    write_atomic(path, text.as_bytes())
}

/// Marks a stored photo to keep (for evaluation) or not.
pub fn set_keep(meta_file: &Path, keep: bool) -> std::io::Result<()> {
    let mut meta: PhotoMeta =
        serde_json::from_slice(&std::fs::read(meta_file)?).map_err(std::io::Error::other)?;
    meta.keep = keep;
    write_meta(meta_file, &meta)
}

/// Removes the originals received more than `KEEP_DAYS` days before `now` and not marked to
/// keep, with their images on the card's grid; their JSON stays (marked `removed`), and so do any results files. Returns the images
/// removed.
pub fn clean_up(dir: &Path, now: u64) -> std::io::Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(removed); // no photos yet
    };
    for e in entries {
        let path = e?.path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Ok(mut meta) = std::fs::read(&path)
            .map_err(|_| ())
            .and_then(|b| serde_json::from_slice::<PhotoMeta>(&b).map_err(|_| ()))
        else {
            continue; // a results file, or not ours
        };
        if meta.keep || meta.removed || now < meta.received_unix + KEEP_DAYS * 86_400 {
            continue;
        }
        let image = dir.join(&meta.image);
        // The photo on the card's grid (`photo::save_grid`), when there is one.
        let _ = std::fs::remove_file(image.with_extension("grid.png"));
        match std::fs::remove_file(&image) {
            Ok(()) => removed.push(image),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        meta.removed = true;
        write_meta(&path, &meta)?;
    }
    Ok(removed)
}

/// Which page the phone gets.
enum Page<'a> {
    /// The link is no good (the message says so).
    Gone,
    /// One photo of this series' card.
    Single(&'a str),
    /// Counting cards: photo after photo, the app's word on each below.
    Session,
}

/// How to take the photo, on both pages.
const TIPS: &str = r#"<ul>
<li>Lay the card flat.</li>
<li>Hold the phone straight above it; the whole card in the picture, as large as fits.</li>
<li>Even light, no shadow of the phone on the card.</li>
</ul>
<label class=btn><span id=cam-text>Take photo</span><input id=cam type=file accept="image/*" capture=environment hidden></label>
<label class="btn alt"><span id=pick-text>Choose a photo</span><input id=pick type=file accept="image/*" hidden></label>
<progress id=bar max=1 value=0 hidden></progress>
<p id=msg class=msg></p>
<button id=fin class="btn fin" type=button>Finished</button>"#;

/// The upload, on both pages; `done(ok)` after each.
const SEND_JS: &str = r#"const msg = document.getElementById('msg'), bar = document.getElementById('bar');
function send(f, done) {
  if (!f) return;
  const x = new XMLHttpRequest();
  x.open('POST', location.pathname);
  x.setRequestHeader('Content-Type', f.type || 'application/octet-stream');
  bar.hidden = false; bar.value = 0; msg.textContent = 'Sending...';
  x.upload.onprogress = e => { if (e.lengthComputable) bar.value = e.loaded / e.total; };
  x.onload = () => { bar.hidden = true; msg.textContent = x.responseText; done(x.status == 200); };
  x.onerror = () => { bar.hidden = true; msg.textContent = 'The upload failed. Is the phone on the same Wi-Fi as the app? Try again.'; done(false); };
  x.send(f);
}
for (const id of ['cam', 'pick'])
  document.getElementById(id).onchange = e => { send(e.target.files[0], ok => after(ok)); e.target.value = ''; };
// "Finished": the app closes its server; nothing more goes to it from this page.
document.getElementById('fin').onclick = () => {
  fetch(location.pathname + '/finished', { method: 'POST' })
    .then(r => r.text())
    .then(t => { finished(); msg.textContent = t; })
    .catch(() => { msg.textContent = 'The app could not be reached. Close this page.'; });
};
function finished() {
  if (typeof timer !== 'undefined') clearTimeout(timer);
  document.querySelectorAll('.btn').forEach(b => b.hidden = true);
  window.done = true;
}"#;

/// The phone's page.
fn page(kind: Page, msg: &str) -> String {
    let body = match kind {
        Page::Gone => format!("<p class=msg>{}</p>", html_escape(msg)),
        Page::Single(s) => format!(
            r#"<p>A photo of the card for <b>{}</b>.</p>
{TIPS}
<script>
{SEND_JS}
function after(ok) {{ if (ok) document.querySelectorAll('.btn').forEach(b => b.hidden = true); }}
</script>"#,
            html_escape(s)
        ),
        Page::Session => format!(
            r#"<p>Count cards: send a photo of each card. The app counts them in turn.</p>
{TIPS}
<ul id=cards class=cards></ul>
<script>
{SEND_JS}
function after(ok) {{
  if (!ok) return;
  document.getElementById('cam-text').textContent = 'Upload another card';
  document.getElementById('pick-text').textContent = 'Choose another photo';
  poll();
}}
const list = document.getElementById('cards');
let timer = null;
function poll() {{
  clearTimeout(timer);
  if (window.done) return;
  fetch(location.pathname + '/cards', {{ cache: 'no-store' }}).then(r => {{
    if (r.status == 404) {{
      document.querySelectorAll('.btn').forEach(b => b.hidden = true);
      msg.textContent = 'The app closed this session. Scan the code in the app again for a new one.';
      return null;
    }}
    return r.json();
  }}).then(j => {{
    if (!j) return;
    list.replaceChildren(...j.lines.slice().reverse().map(t => {{ const li = document.createElement('li'); li.textContent = t; return li; }}));
    timer = setTimeout(poll, 2000);
  }}).catch(() => {{ timer = setTimeout(poll, 4000); }});
}}
poll();
</script>"#
        ),
    };
    format!(
        r#"<!doctype html><html><head><meta charset=utf-8>
<meta name=viewport content="width=device-width, initial-scale=1">
<title>OpenSights card photo</title>
<style>
body {{ font: 17px/1.4 system-ui, sans-serif; margin: 0 auto; max-width: 30em; padding: 1em; color: #222; background: #fafafa; }}
h1 {{ font-size: 1.3em; }}
.btn {{ display: block; text-align: center; padding: .9em; margin: .7em 0; border-radius: .5em; background: #222; color: #fff; font-weight: 600; }}
.btn.alt {{ background: #ddd; color: #222; }}
.btn.fin {{ width: 100%; border: 0; font: inherit; font-weight: 600; background: #fff; color: #222; border: 2px solid #222; margin-top: 1.5em; }}
.btn[hidden] {{ display: none; }}
progress {{ width: 100%; height: 1.2em; }}
.msg {{ font-weight: 600; }}
.cards {{ padding-left: 1.2em; }}
.cards li {{ margin: .3em 0; }}
</style></head><body>
<h1>OpenSights card photo</h1>
{body}
</body></html>"#
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpStream;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("opensights-upload-{name}-{}", new_token()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A plain HTTP/1.1 request over a TCP stream, as a phone's browser sends it; the status
    /// and the body.
    fn http(
        port: u16,
        method: &str,
        path: &str,
        body: &[u8],
        content_length: Option<usize>,
    ) -> (u16, String) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut head = format!("{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n");
        if let Some(n) = content_length {
            head += &format!("Content-Type: image/jpeg\r\nContent-Length: {n}\r\n");
        }
        head += "\r\n";
        s.write_all(head.as_bytes()).unwrap();
        // A refusal may close the connection before the whole body is sent.
        let _ = s.write_all(body);
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        let text = String::from_utf8_lossy(&out).into_owned();
        let code = text.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        let body = text
            .split_once("\r\n\r\n")
            .map(|x| x.1.to_string())
            .unwrap_or_default();
        (code, body)
    }

    fn server(dir: &Path, max: usize) -> (UploadServer, Arc<AtomicUsize>) {
        let pokes = Arc::new(AtomicUsize::new(0));
        let p = pokes.clone();
        let s = UploadServer::start(
            dir.to_path_buf(),
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.1".parse().unwrap(),
            max,
            Arc::new(move || {
                p.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
        (s, pokes)
    }

    fn phone_photo() -> (Vec<u8>, image::RgbImage) {
        let img = image::RgbImage::from_fn(640, 480, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x * y) % 251) as u8])
        });
        (cardphoto::meta::testdata::phone_jpeg(&img, 6), img)
    }

    fn path_of(url: &str) -> String {
        url.split_once("/p/").map(|x| format!("/p/{}", x.1)).unwrap()
    }

    #[test]
    fn upload_stores_without_metadata() {
        let dir = temp_dir("store");
        let (srv, pokes) = server(&dir, MAX_BYTES);
        let url = srv.offer("Series 3");
        assert!(
            url.starts_with(&format!("http://127.0.0.1:{}/p/", srv.port)),
            "{url}"
        );
        let path = path_of(&url);

        let (code, page) = http(srv.port, "GET", &path, b"", None);
        assert_eq!(code, 200);
        assert!(page.contains("capture=environment") && page.contains("Series 3"));
        assert!(matches!(srv.try_recv(), Some(UploadEvent::Opened { series }) if series == "Series 3"));

        let (sent, _) = phone_photo();
        let (code, body) = http(srv.port, "POST", &path, &sent, Some(sent.len()));
        assert_eq!(code, 200, "{body}");
        let Some(UploadEvent::Received(photo)) = srv.try_recv() else {
            panic!("no photo")
        };
        println!("{:#?}", photo.meta);
        assert!(pokes.load(Ordering::SeqCst) >= 2);
        let stored = std::fs::read(&photo.image).unwrap();
        for needle in [&b"Exif"[..], b"GPS", b"PhoneMaker", b"xmpmeta"] {
            assert!(!stored.windows(needle.len()).any(|w| w == needle));
        }
        let a = image::load_from_memory(&sent).unwrap().to_rgb8();
        let b = image::load_from_memory(&stored).unwrap().to_rgb8();
        assert!(a.as_raw() == b.as_raw(), "pixels differ");
        assert_eq!(photo.meta.orientation, Some(6));
        assert_eq!(photo.meta.sent_bytes, sent.len());
        let on_disk: PhotoMeta = serde_json::from_slice(&std::fs::read(&photo.meta_file).unwrap()).unwrap();
        assert_eq!(on_disk, photo.meta);
        assert!(
            photo.meta.image.ends_with("_Series_3.jpg"),
            "{}",
            photo.meta.image
        );

        // Used: the link works once.
        let (code, _) = http(srv.port, "POST", &path, &sent, Some(sent.len()));
        assert_eq!(code, 404);
        let (code, page) = http(srv.port, "GET", &path, b"", None);
        assert_eq!(code, 404);
        assert!(page.contains("Scan the code"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// "Finished" on either page: the link gone, the app told; only by POST.
    #[test]
    fn finished_closes_the_link() {
        let dir = temp_dir("finished");
        let (srv, _) = server(&dir, MAX_BYTES);
        for session in [true, false] {
            // One at a time: a new link ends the old one.
            let path = path_of(&if session { srv.offer_session("count", 5) } else { srv.offer("Series 1") });
            let (code, page) = http(srv.port, "GET", &path, b"", None);
            assert_eq!(code, 200);
            assert!(page.contains(">Finished<") && page.contains("/finished"));
            assert!(matches!(srv.try_recv(), Some(UploadEvent::Opened { .. })));
            assert_eq!(http(srv.port, "GET", &format!("{path}/finished"), b"", None).0, 405);
            assert!(srv.try_recv().is_none());
            let (code, body) = http(srv.port, "POST", &format!("{path}/finished"), b"", Some(0));
            assert_eq!(code, 200, "{body}");
            assert!(body.starts_with("Finished"));
            assert!(matches!(srv.try_recv(), Some(UploadEvent::Finished)));
            assert_eq!(http(srv.port, "GET", &path, b"", None).0, 404);
            assert_eq!(http(srv.port, "POST", &format!("{path}/finished"), b"", Some(0)).0, 404);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A session link (counting cards): photo after photo up to its room, the page's lines,
    /// a place given back, and gone once replaced.
    #[test]
    fn session_takes_many_until_full() {
        let dir = temp_dir("session");
        let (srv, _) = server(&dir, MAX_BYTES);
        let path = path_of(&srv.offer_session("count", 2));
        let (code, page) = http(srv.port, "GET", &path, b"", None);
        assert_eq!(code, 200);
        assert!(page.contains("Upload another card") && page.contains("/cards"));
        assert!(matches!(srv.try_recv(), Some(UploadEvent::Opened { .. })));
        let (sent, _) = phone_photo();
        for _ in 0..2 {
            let (code, body) = http(srv.port, "POST", &path, &sent, Some(sent.len()));
            assert_eq!((code, body.as_str()), (200, "Received."));
            assert!(matches!(srv.try_recv(), Some(UploadEvent::Received(_))));
        }
        // Full: refused, the app told.
        let (code, body) = http(srv.port, "POST", &path, &sent, Some(sent.len()));
        assert_eq!((code, body.as_str()), (409, SESSION_FULL));
        assert!(matches!(srv.try_recv(), Some(UploadEvent::Refused { .. })));
        // A card removed in the app: one more fits; a dropped file takes the last place.
        srv.free_slot();
        srv.free_slot();
        let (code, _) = http(srv.port, "POST", &path, &sent, Some(sent.len()));
        assert_eq!(code, 200);
        assert!(srv.take_slot());
        assert!(!srv.take_slot());
        srv.set_lines(vec!["Card 1: 92, 10 holes".into(), "Card 2: counting…".into()]);
        let (code, body) = http(srv.port, "GET", &format!("{path}/cards"), b"", None);
        assert_eq!(code, 200);
        let j: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(j["lines"][1], "Card 2: counting…");
        // A single-use link has no lines; a replaced session is gone.
        let single = path_of(&srv.offer("A"));
        assert_eq!(http(srv.port, "GET", &format!("{single}/cards"), b"", None).0, 404);
        assert_eq!(http(srv.port, "GET", &format!("{path}/cards"), b"", None).0, 404);
        assert_eq!(http(srv.port, "POST", &path, &sent, Some(sent.len())).0, 404);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wrong_or_replaced_token_refused() {
        let dir = temp_dir("token");
        let (srv, _) = server(&dir, MAX_BYTES);
        let first = path_of(&srv.offer("A"));
        let second = path_of(&srv.offer("B"));
        let (sent, _) = phone_photo();
        for p in [
            first.as_str(),
            "/p/0123456789abcdef0123456789abcdef",
            "/p/",
            "/",
            "/favicon.ico",
        ] {
            let (code, _) = http(srv.port, "POST", p, &sent, Some(sent.len()));
            assert_eq!(code, 404, "{p}");
        }
        assert!(srv.try_recv().is_none());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let (code, _) = http(srv.port, "POST", &second, &sent, Some(sent.len()));
        assert_eq!(code, 200);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn too_large_and_not_a_photo_refused() {
        let dir = temp_dir("large");
        let (srv, _) = server(&dir, 1_000_000);
        let path = path_of(&srv.offer("S"));
        // Too large by its stated length: refused before reading.
        let big = vec![0xAB; 1_500_000];
        let (code, body) = http(srv.port, "POST", &path, &big, Some(big.len()));
        assert_eq!(code, 413, "{body}");
        match srv.try_recv() {
            Some(UploadEvent::Refused { series, reason }) => {
                assert_eq!(series, "S");
                assert!(reason.contains("larger than 1 MB"), "{reason}");
            }
            e => panic!("{e:?}"),
        }
        // Too large without a length (chunked): refused after the limit.
        let mut s = TcpStream::connect(("127.0.0.1", srv.port)).unwrap();
        let head = format!(
            "POST {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n"
        );
        s.write_all(head.as_bytes()).unwrap();
        for _ in 0..40 {
            let _ = s.write_all(format!("{:x}\r\n", 40_000).as_bytes());
            let _ = s.write_all(&[0xAB; 40_000]);
            let _ = s.write_all(b"\r\n");
        }
        let _ = s.write_all(b"0\r\n\r\n");
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        assert!(out.starts_with("HTTP/1.1 413"), "{}", &out[..out.len().min(80)]);
        // Not a photo.
        let (code, body) = http(srv.port, "POST", &path, b"GIF89a.....", Some(11));
        assert_eq!(code, 415);
        assert!(body.contains("JPEG"));
        // Nothing stored, and the link still works.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let (sent, _) = phone_photo();
        assert!(sent.len() < 1_000_000);
        let (code, _) = http(srv.port, "POST", &path, &sent, Some(sent.len()));
        assert_eq!(code, 200);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn old_originals_removed() {
        let dir = temp_dir("clean");
        let (sent, _) = phone_photo();
        let day = 86_400;
        let now = 1_795_000_000; // 2026-11-18
        let old = store(&dir, "old", &sent, now - 31 * day).unwrap();
        let kept = store(&dir, "kept", &sent, now - 40 * day).unwrap();
        set_keep(&kept.meta_file, true).unwrap();
        let young = store(&dir, "young", &sent, now - 29 * day).unwrap();
        std::fs::write(dir.join("results.json"), "{\"rms\": 0.1}").unwrap(); // not ours
        let removed = clean_up(&dir, now).unwrap();
        assert_eq!(removed, vec![old.image.clone()]);
        assert!(!old.image.exists() && old.meta_file.exists());
        let meta: PhotoMeta = serde_json::from_slice(&std::fs::read(&old.meta_file).unwrap()).unwrap();
        assert!(meta.removed);
        assert!(kept.image.exists() && young.image.exists());
        assert!(dir.join("results.json").exists());
        // Again: nothing more.
        assert!(clean_up(&dir, now).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn same_second_same_series_kept_apart() {
        let dir = temp_dir("names");
        let (sent, _) = phone_photo();
        let a = store(&dir, "s/1", &sent, 1_795_000_000).unwrap();
        let b = store(&dir, "s/1", &sent, 1_795_000_000).unwrap();
        assert_ne!(a.image, b.image);
        assert_eq!(a.meta.received, "2026-11-18 11:06:40");
        assert!(
            a.meta.image.starts_with("2026-11-18_110640_s_1"),
            "{}",
            a.meta.image
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn qr_of_a_link() {
        let (w, m) = qr("http://192.168.178.23:8710/p/0123456789abcdef0123456789abcdef").unwrap();
        assert_eq!(m.len(), w * w);
        assert!((21..=45).contains(&w), "{w}");
        // The three finder patterns: dark corners.
        assert!(m[0] && m[w - 1] && m[w * (w - 1)]);
    }

    /// Over this computer's network address, as a phone reaches it (needs a network:
    /// `cargo test lan -- --ignored`). With `PHONE_WAIT=<seconds>` set, it also prints a QR
    /// code and waits that long for a photo from a phone (the user's test, `CARDPHOTO.md` C3).
    #[test]
    #[ignore]
    fn lan_upload() {
        let dir = temp_dir("lan");
        let (tx, rx) = channel::<()>();
        let tx = Mutex::new(tx);
        let srv = UploadServer::start_lan(dir.clone(), move || {
            let _ = tx.lock().unwrap().send(());
        })
        .unwrap();
        println!("listening on {}:{}", srv.host, srv.port);
        let url = srv.offer("lan test");
        let path = path_of(&url);
        let ask = |method: &str, body: &[u8]| {
            let mut s = TcpStream::connect((srv.host, srv.port)).unwrap();
            let len = (method == "POST").then(|| format!("Content-Length: {}\r\n", body.len()));
            let head = format!(
                "{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n{}\r\n",
                len.unwrap_or_default()
            );
            s.write_all(head.as_bytes()).unwrap();
            s.write_all(body).unwrap();
            let mut out = String::new();
            let _ = s.read_to_string(&mut out);
            out
        };
        assert!(ask("GET", b"").starts_with("HTTP/1.1 200"));
        if let Some(secs) = std::env::var("PHONE_WAIT")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
        {
            let (w, m) = qr(&url).unwrap();
            let dark = |x: isize, y: isize| {
                x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < w && m[y as usize * w + x as usize]
            };
            for y in (-4..w as isize + 4).step_by(2) {
                let line: String = (-4..w as isize + 4)
                    .map(|x| match (dark(x, y), dark(x, y + 1)) {
                        (true, true) => ' ',
                        (true, false) => '\u{2584}',
                        (false, true) => '\u{2580}',
                        (false, false) => '\u{2588}',
                    })
                    .collect();
                println!("{line}");
            }
            println!("{url}\nwaiting {secs} s for the phone");
            let end = std::time::Instant::now() + std::time::Duration::from_secs(secs);
            while std::time::Instant::now() < end {
                let _ = rx.recv_timeout(std::time::Duration::from_millis(500));
                while let Some(e) = srv.try_recv() {
                    println!("{e:#?}");
                    if matches!(e, UploadEvent::Received(_)) {
                        println!("stored in {}", dir.display());
                        return;
                    }
                }
            }
            panic!("no photo from the phone");
        }
        let (sent, _) = phone_photo();
        assert!(ask("POST", &sent).starts_with("HTTP/1.1 200"));
        assert!(matches!(srv.try_recv(), Some(UploadEvent::Opened { .. })));
        assert!(matches!(srv.try_recv(), Some(UploadEvent::Received(_))));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tokens_are_random() {
        let a = new_token();
        assert_eq!(a.len(), 32);
        assert_ne!(a, new_token());
    }
}
