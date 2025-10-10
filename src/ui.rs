use crate::config::Config;
use crate::logger;
use crate::write_clipboard_string;
use eframe::egui;
use once_cell::sync::Lazy;
use std::sync::{mpsc, Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::thread;
use std::fs;

static OUTPUT_SENDER: Lazy<Mutex<Option<mpsc::Sender<String>>>> = Lazy::new(|| Mutex::new(None));
static LAST_TEXT: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));
static HAS_UPDATED: AtomicBool = AtomicBool::new(false);
static FONTS_SET: AtomicBool = AtomicBool::new(false);
static WINDOW_VISIBLE: AtomicBool = AtomicBool::new(false);

fn ensure_output_thread() {
    let mut guard = OUTPUT_SENDER.lock().unwrap();
    if guard.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<String>();
    *guard = Some(tx);

    thread::spawn(move || {
        logger::log("Output UI thread: starting");
        let app = OutputApp { text: String::new(), rx, need_focus: false, first: true, logged_init: false };
        let native_options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("GPTTrans - Translation")
                .with_inner_size([800.0, 560.0])
                .with_always_on_top()
                .with_visible(false),
            ..Default::default()
        };
        match eframe::run_native(
            "GPTTrans - Translation",
            native_options,
            Box::new(|_cc| Box::new(app)),
        ) {
            Ok(_) => logger::log("Output UI thread: stopped"),
            Err(e) => logger::log(&format!("Output window error: {}", e)),
        }
    });
}

pub fn show_output_text(text: String) {
    ensure_output_thread();
    if let Ok(mut guard) = OUTPUT_SENDER.lock() {
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(text.clone());
            logger::log("UI: sent translation to output window");
        }
    }
    if let Ok(mut lt) = LAST_TEXT.lock() { *lt = text.clone(); }
    // If the window is hidden (after user clicked Hide) and UI loop is alive, show a quick native box
    if HAS_UPDATED.load(Ordering::Relaxed) && !WINDOW_VISIBLE.load(Ordering::Relaxed) {
        crate::show_message_box("GPTTrans - Translation", &text);
    }
}

pub fn show_window() {
    ensure_output_thread();
    let text = { LAST_TEXT.lock().unwrap().clone() };
    if let Ok(guard) = OUTPUT_SENDER.lock() {
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(text);
            logger::log("UI: requested show (resent last text)");
        }
    }
}

pub fn has_ever_updated() -> bool {
    HAS_UPDATED.load(Ordering::Relaxed)
}

pub fn spawn_settings_window(_cfg: Arc<Mutex<Config>>) {
    // TODO: move Settings into the same UI thread to avoid winit EventLoop limitations
    logger::log("Settings window currently disabled to avoid EventLoop conflicts");
}

struct OutputApp {
    text: String,
    rx: mpsc::Receiver<String>,
    need_focus: bool,
    first: bool,
    logged_init: bool,
}

impl eframe::App for OutputApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Wake up periodically so we can poll the channel even without user events
        ctx.request_repaint_after(Duration::from_millis(120));
        if !HAS_UPDATED.swap(true, Ordering::Relaxed) {
            logger::log("Output window: update entered");
        }
        if !FONTS_SET.swap(true, Ordering::Relaxed) {
            let candidates = [
                r"C:\\Windows\\Fonts\\msyh.ttc",
                r"C:\\Windows\\Fonts\\msyh.ttf",
                r"C:\\Windows\\Fonts\\msyhbd.ttf",
                r"C:\\Windows\\Fonts\\simsun.ttc",
                r"C:\\Windows\\Fonts\\simhei.ttf",
            ];
            let mut loaded = None;
            for path in candidates {
                if let Ok(bytes) = fs::read(path) {
                    loaded = Some(bytes);
                    logger::log(&format!("Loaded CJK font: {}", path));
                    break;
                }
            }
            if let Some(bytes) = loaded {
                let mut fonts = egui::FontDefinitions::default();
                fonts.font_data.insert("cjk".to_owned(), egui::FontData::from_owned(bytes));
                fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "cjk".to_owned());
                fonts.families.entry(egui::FontFamily::Monospace).or_default().insert(0, "cjk".to_owned());
                ctx.set_fonts(fonts);
                logger::log("Applied CJK font to egui");
            } else {
                logger::log("No CJK font found; text may render as squares");
            }
        }
        // Drain any pending texts; keep only the latest
        while let Ok(new_text) = self.rx.try_recv() {
            self.text = new_text;
            self.need_focus = true;
        }

        if self.first || self.need_focus {
            self.first = false;
            self.need_focus = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            WINDOW_VISIBLE.store(true, Ordering::Relaxed);
            logger::log("Output window: shown (focused)");
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Translation");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Hide").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        WINDOW_VISIBLE.store(false, Ordering::Relaxed);
                    }
                    if ui.button("Copy").clicked() {
                        let _ = write_clipboard_string(&self.text);
                    }
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.text)
                            .desired_rows(20)
                            .desired_width(f32::INFINITY),
                    );
                });
        });
    }
}

// Run the UI event loop on the main thread (blocking)
pub fn run_ui_main_thread() {
    let mut guard = OUTPUT_SENDER.lock().unwrap();
    if guard.is_some() {
        logger::log("UI already running; run_ui_main_thread called twice");
        return;
    }
    let (tx, rx) = mpsc::channel::<String>();
    *guard = Some(tx);
    drop(guard);

    logger::log("Main UI: starting event loop");
    let app = OutputApp { text: String::new(), rx, need_focus: false, first: true, logged_init: false };
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("GPTTrans - Translation")
            .with_inner_size([800.0, 560.0])
            .with_always_on_top()
            .with_visible(true),
        ..Default::default()
    };
    match eframe::run_native(
        "GPTTrans - Translation",
        native_options,
        Box::new(|_cc| Box::new(app)),
    ) {
        Ok(_) => logger::log("Main UI: event loop exited"),
        Err(e) => logger::log(&format!("Main UI error: {}", e)),
    }
}
