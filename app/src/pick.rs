//! Photo files chosen in the system's file dialog (`rfd`; on Linux the desktop portal). Files
//! dropped on the window don't reach the app on Wayland (winit 0.30 has no drag and drop
//! there), so the dialog is the way in that works everywhere.
//!
//! The dialog runs on its own thread (it blocks until closed); the UI polls `take`.

use std::sync::mpsc;

/// The files read from the dialog's choice, or why one couldn't be read.
type Files = Vec<Result<Vec<u8>, String>>;

/// A file dialog opened from the UI, while it is open.
#[derive(Default)]
pub struct FilePick {
    rx: Option<mpsc::Receiver<Files>>,
}

impl FilePick {
    /// Opens the dialog for photo files (`many`: more than one), unless it is open already.
    pub fn start(&mut self, ctx: &egui::Context, title: &str, many: bool) {
        if self.rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let title = title.to_string();
        std::thread::spawn(move || {
            let dialog = rfd::FileDialog::new()
                .set_title(&title)
                .add_filter("Photos", &["jpg", "jpeg", "JPG", "JPEG", "png", "PNG"]);
            let paths = if many {
                dialog.pick_files().unwrap_or_default()
            } else {
                dialog.pick_file().into_iter().collect()
            };
            let files = paths
                .iter()
                .map(|p| std::fs::read(p).map_err(|e| format!("{}: {e}", p.display())))
                .collect();
            let _ = tx.send(files);
            ctx.request_repaint();
        });
        self.rx = Some(rx);
    }

    /// Whether the dialog is open.
    pub fn open(&self) -> bool {
        self.rx.is_some()
    }

    /// The files chosen, once the dialog has closed (none if it was cancelled).
    pub fn take(&mut self) -> Files {
        let Some(rx) = &self.rx else { return Vec::new() };
        match rx.try_recv() {
            Ok(files) => {
                self.rx = None;
                files
            }
            Err(mpsc::TryRecvError::Empty) => Vec::new(),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.rx = None;
                Vec::new()
            }
        }
    }
}

/// Whether files dropped on the window reach the app: not on Wayland (winit 0.30), where
/// winit runs whenever `WAYLAND_DISPLAY` is set.
pub fn drop_works() -> bool {
    !(cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()))
}

/// The "Choose photos…" button (greyed while the dialog is open), and where dropping works,
/// that files can be dropped too. True when clicked.
pub fn choose_button(ui: &mut egui::Ui, open: bool, label: &str, drop_hint: &str) -> bool {
    ui.horizontal_wrapped(|ui| {
        let clicked = ui
            .add_enabled(!open, egui::Button::new(label))
            .on_hover_text("Photo files on this computer")
            .clicked();
        if drop_works() {
            ui.label(egui::RichText::new(drop_hint).weak());
        }
        clicked
    })
    .inner
}
