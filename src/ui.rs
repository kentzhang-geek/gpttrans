use crate::config::Config;
use crate::logger;
use crate::write_clipboard_string;
use eframe::egui;
use std::sync::{Arc, Mutex};
use std::thread;

pub fn spawn_output_window(text: String) {
    thread::spawn(move || {
        logger::log("Output window: launching");
        let app = OutputApp { text, first: true };
        let native_options = eframe::NativeOptions::default();
        match eframe::run_native(
            "GPTTrans - Translation",
            native_options,
            Box::new(|_cc| Box::new(app)),
        ) {
            Ok(_) => logger::log("Output window: closed"),
            Err(e) => logger::log(&format!("Output window error: {}", e)),
        }
    });
}

pub fn spawn_settings_window(cfg: Arc<Mutex<Config>>) {
    thread::spawn(move || {
        logger::log("Settings window: launching");
        let app = SettingsApp { cfg, first: true };
        let native_options = eframe::NativeOptions::default();
        match eframe::run_native(
            "GPTTrans - Settings",
            native_options,
            Box::new(|_cc| Box::new(app)),
        ) {
            Ok(_) => logger::log("Settings window: closed"),
            Err(e) => logger::log(&format!("Settings window error: {}", e)),
        }
    });
}

struct OutputApp {
    text: String,
    first: bool,
}

impl eframe::App for OutputApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.first {
            self.first = false;
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

struct SettingsApp {
    cfg: Arc<Mutex<Config>>,
    first: bool,
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.first {
            self.first = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            logger::log("Settings window: shown (focused)");
        }
        let mut tmp = self.cfg.lock().unwrap().clone();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("GPTTrans Settings");
            ui.separator();
            ui.label("OpenAI API Key");
            ui.add(egui::TextEdit::singleline(&mut tmp.openai_api_key).password(true).hint_text("sk-..."));
            ui.separator();
            ui.label("Model");
            ui.add(egui::TextEdit::singleline(&mut tmp.openai_model));
            ui.label("Target Language");
            ui.add(egui::TextEdit::singleline(&mut tmp.target_lang));

            ui.separator();
            if ui.button("Save").clicked() {
                if let Ok(mut g) = self.cfg.lock() {
                    *g = tmp.clone();
                    let _ = g.save();
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.add_space(8.0);
            if ui.button("Cancel").clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }
}

