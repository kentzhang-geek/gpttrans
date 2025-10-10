use crate::config::Config;
use crate::logger;
use crate::write_clipboard_string;
use eframe::egui;
use once_cell::sync::Lazy;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use std::thread;

static OUTPUT_SENDER: Lazy<Mutex<Option<mpsc::Sender<String>>>> = Lazy::new(|| Mutex::new(None));

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
                .with_visible(true),
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
            let _ = tx.send(text);
            logger::log("UI: sent translation to output window");
        }
    }
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
        if !self.logged_init {
            self.logged_init = true;
            logger::log("Output window: update entered");
        }
        // Drain any pending texts; keep only the latest
        while let Ok(new_text) = self.rx.try_recv() {
            self.text = new_text;
            self.need_focus = true;
        }

        if self.first || self.need_focus {
            self.first = false;
            self.need_focus = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            logger::log("Output window: shown (focused)");
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Translation");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
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
