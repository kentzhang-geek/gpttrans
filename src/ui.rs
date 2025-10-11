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

static OUTPUT_SENDER: Lazy<Mutex<Option<mpsc::Sender<UiMessage>>>> = Lazy::new(|| Mutex::new(None));
static LAST_TEXT: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));
static HAS_UPDATED: AtomicBool = AtomicBool::new(false);
static FONTS_SET: AtomicBool = AtomicBool::new(false);
static WINDOW_VISIBLE: AtomicBool = AtomicBool::new(false);
static CONFIG: Lazy<Mutex<Option<Arc<Mutex<Config>>>>> = Lazy::new(|| Mutex::new(None));

enum UiMessage {
    ShowText(String),
    OpenSettings,
    AppendText(String),  // For streaming updates
    SetTranslating(bool), // Show/hide loading indicator
}

fn ensure_output_thread() {
    let mut guard = OUTPUT_SENDER.lock().unwrap();
    if guard.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<UiMessage>();
    *guard = Some(tx);

    thread::spawn(move || {
        logger::log("Output UI thread: starting");
        let app = OutputApp { 
            text: String::new(), 
            rx, 
            need_focus: false, 
            show_settings: false,
            settings_api_key: String::new(),
            settings_model: String::new(),
            settings_lang: String::new(),
            is_translating: false,
        };
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
    if let Ok(guard) = OUTPUT_SENDER.lock() {
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(UiMessage::ShowText(text.clone()));
            logger::log("UI: sent translation to output window");
        }
    }
    if let Ok(mut lt) = LAST_TEXT.lock() { *lt = text.clone(); }
}

pub fn append_text(text: String) {
    ensure_output_thread();
    if let Ok(guard) = OUTPUT_SENDER.lock() {
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(UiMessage::AppendText(text));
        }
    }
}

pub fn set_translating(translating: bool) {
    ensure_output_thread();
    if let Ok(guard) = OUTPUT_SENDER.lock() {
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(UiMessage::SetTranslating(translating));
            if translating {
                logger::log("UI: showing translating indicator");
            }
        }
    }
}

pub fn show_settings() {
    ensure_output_thread();
    if let Ok(guard) = OUTPUT_SENDER.lock() {
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(UiMessage::OpenSettings);
            logger::log("UI: requested open settings");
        }
    }
}

pub fn show_translation_window() {
    ensure_output_thread();
    let text = { LAST_TEXT.lock().unwrap().clone() };
    if let Ok(guard) = OUTPUT_SENDER.lock() {
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(UiMessage::ShowText(text));
            logger::log("UI: requested show translation window");
        }
    }
}

pub fn has_ever_updated() -> bool {
    HAS_UPDATED.load(Ordering::Relaxed)
}

pub fn set_config(cfg: Arc<Mutex<Config>>) {
    if let Ok(mut config_guard) = CONFIG.lock() {
        *config_guard = Some(cfg);
    }
}

struct OutputApp {
    text: String,
    rx: mpsc::Receiver<UiMessage>,
    need_focus: bool,
    show_settings: bool,
    settings_api_key: String,
    settings_model: String,
    settings_lang: String,
    is_translating: bool,
}

impl eframe::App for OutputApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Wake up periodically so we can poll the channel even without user events
        ctx.request_repaint_after(Duration::from_millis(120));
        if !HAS_UPDATED.swap(true, Ordering::Relaxed) {
            logger::log("Output window: update entered");
        }
        
        // Handle ESC key to hide window
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            // Move window off-screen instead of hiding it to keep event loop running
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(-10000.0, -10000.0)));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1.0, 1.0)));
            WINDOW_VISIBLE.store(false, Ordering::Relaxed);
            logger::log("Output window: hidden by ESC key (moved off-screen)");
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
        
        // Drain any pending messages
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                UiMessage::ShowText(new_text) => {
                    self.text = new_text;
                    self.need_focus = true;
                    self.show_settings = false;
                    self.is_translating = false;
                    logger::log("UI: ShowText message received, will show window");
                }
                UiMessage::AppendText(chunk) => {
                    self.text.push_str(&chunk);
                    if let Ok(mut lt) = LAST_TEXT.lock() { *lt = self.text.clone(); }
                }
                UiMessage::SetTranslating(translating) => {
                    self.is_translating = translating;
                    if translating {
                        self.text = String::from("🔄 Translating...");
                        self.need_focus = true;
                        self.show_settings = false;
                    }
                }
                UiMessage::OpenSettings => {
                    self.show_settings = true;
                    self.need_focus = true;
                    logger::log("UI: OpenSettings message received, will show window");
                    // Load current config
                    if let Ok(cfg_guard) = CONFIG.lock() {
                        if let Some(cfg_arc) = cfg_guard.as_ref() {
                            if let Ok(cfg) = cfg_arc.lock() {
                                self.settings_api_key = cfg.openai_api_key.clone();
                                self.settings_model = cfg.openai_model.clone();
                                self.settings_lang = cfg.target_lang.clone();
                            }
                        }
                    }
                }
            }
        }

        // Show and focus window when needed
        if self.need_focus {
            let was_visible = WINDOW_VISIBLE.load(Ordering::Relaxed);
            logger::log(&format!("UI: Showing window (was_visible={})", was_visible));
            
            // Restore window size and position to center of screen
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(800.0, 560.0)));
            // Move to center (approximate - let OS position it)
            if let Some(monitor_size) = ctx.input(|i| i.viewport().monitor_size) {
                let x = (monitor_size.x - 800.0) / 2.0;
                let y = (monitor_size.y - 560.0) / 2.0;
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(x.max(0.0), y.max(0.0))));
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            
            WINDOW_VISIBLE.store(true, Ordering::Relaxed);
            self.need_focus = false;
        }

        if self.show_settings {
            self.show_settings_ui(ctx);
        } else {
            self.show_translation_ui(ctx);
        }
    }
}

impl OutputApp {
    fn show_translation_ui(&mut self, ctx: &egui::Context) {
        // Custom frameless window with rounded corners
        egui::CentralPanel::default()
            .frame(egui::Frame::none()
                .fill(egui::Color32::from_rgb(28, 31, 38))
                .rounding(egui::Rounding::same(12.0))
                .inner_margin(egui::Margin::same(0.0)))
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    // Custom title bar with drag area
                    let title_bar_height = 48.0;
                    let title_bar_rect = {
                        let mut rect = ui.available_rect_before_wrap();
                        rect.max.y = rect.min.y + title_bar_height;
                        rect
                    };
                    
                    let title_bar_response = ui.allocate_rect(title_bar_rect, egui::Sense::click());
                    if title_bar_response.clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    
                    // Draw custom title bar
                    ui.painter().rect_filled(
                        title_bar_rect,
                        egui::Rounding { nw: 12.0, ne: 12.0, sw: 0.0, se: 0.0 },
                        egui::Color32::from_rgb(35, 39, 46),
                    );
                    
                    ui.allocate_ui_at_rect(title_bar_rect, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(16.0);
                            ui.vertical_centered(|ui| {
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new("📝 GPTTrans")
                                    .size(18.0)
                                    .color(egui::Color32::from_rgb(138, 180, 248)));
                            });
                            
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_space(8.0);
                                
                                // Close button
                                let close_btn = ui.add_sized(
                                    [36.0, 36.0],
                                    egui::Button::new(egui::RichText::new("✕").size(16.0))
                                        .fill(egui::Color32::TRANSPARENT)
                                        .stroke(egui::Stroke::NONE)
                                );
                                if close_btn.clicked() {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(-10000.0, -10000.0)));
                                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1.0, 1.0)));
                                    WINDOW_VISIBLE.store(false, Ordering::Relaxed);
                                    logger::log("Output window: hidden by user (moved off-screen)");
                                }
                                
                                // Settings button
                                let settings_btn = ui.add_sized(
                                    [36.0, 36.0],
                                    egui::Button::new(egui::RichText::new("⚙").size(16.0))
                                        .fill(egui::Color32::TRANSPARENT)
                                        .stroke(egui::Stroke::NONE)
                                );
                                if settings_btn.clicked() {
                                    self.show_settings = true;
                                    if let Ok(cfg_guard) = CONFIG.lock() {
                                        if let Some(cfg_arc) = cfg_guard.as_ref() {
                                            if let Ok(cfg) = cfg_arc.lock() {
                                                self.settings_api_key = cfg.openai_api_key.clone();
                                                self.settings_model = cfg.openai_model.clone();
                                                self.settings_lang = cfg.target_lang.clone();
                                            }
                                        }
                                    }
                                }
                                
                                // Copy button
                                let copy_btn = ui.add_sized(
                                    [36.0, 36.0],
                                    egui::Button::new(egui::RichText::new("📋").size(16.0))
                                        .fill(egui::Color32::TRANSPARENT)
                                        .stroke(egui::Stroke::NONE)
                                );
                                if copy_btn.clicked() {
                                    let _ = write_clipboard_string(&self.text);
                                    logger::log("Text copied to clipboard");
                                }
                            });
                        });
                    });
                    
                    ui.add_space(8.0);
                    
                    // Content area with padding
                    egui::Frame::none()
                        .inner_margin(egui::Margin::symmetric(16.0, 8.0))
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::TextEdit::multiline(&mut self.text)
                                            .desired_rows(20)
                                            .desired_width(f32::INFINITY)
                                            .font(egui::TextStyle::Body)
                                            .frame(true)
                                    );
                                });
                        });
                    
                    ui.add_space(8.0);
                });
            });
    }

    fn show_settings_ui(&mut self, ctx: &egui::Context) {
        // Modern settings panel with same styling
        egui::CentralPanel::default()
            .frame(egui::Frame::none()
                .fill(egui::Color32::from_rgb(28, 31, 38))
                .rounding(egui::Rounding::same(12.0))
                .inner_margin(egui::Margin::same(0.0)))
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    // Custom title bar
                    let title_bar_height = 48.0;
                    let title_bar_rect = {
                        let mut rect = ui.available_rect_before_wrap();
                        rect.max.y = rect.min.y + title_bar_height;
                        rect
                    };
                    
                    let title_bar_response = ui.allocate_rect(title_bar_rect, egui::Sense::click());
                    if title_bar_response.clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    
                    ui.painter().rect_filled(
                        title_bar_rect,
                        egui::Rounding { nw: 12.0, ne: 12.0, sw: 0.0, se: 0.0 },
                        egui::Color32::from_rgb(35, 39, 46),
                    );
                    
                    ui.allocate_ui_at_rect(title_bar_rect, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(16.0);
                            ui.vertical_centered(|ui| {
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new("⚙ Settings")
                                    .size(18.0)
                                    .color(egui::Color32::from_rgb(138, 180, 248)));
                            });
                            
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_space(8.0);
                                let close_btn = ui.add_sized(
                                    [36.0, 36.0],
                                    egui::Button::new(egui::RichText::new("✕").size(16.0))
                                        .fill(egui::Color32::TRANSPARENT)
                                        .stroke(egui::Stroke::NONE)
                                );
                                if close_btn.clicked() {
                                    self.show_settings = false;
                                }
                            });
                        });
                    });
                    
                    ui.add_space(16.0);
                    
                    // Settings content
                    egui::Frame::none()
                        .inner_margin(egui::Margin::symmetric(24.0, 0.0))
                        .show(ui, |ui| {
                            ui.add_space(8.0);
                            
                            // API Key
                            ui.label(egui::RichText::new("OpenAI API Key")
                                .size(14.0)
                                .color(egui::Color32::from_rgb(180, 190, 210)));
                            ui.add_space(4.0);
                            ui.add(egui::TextEdit::singleline(&mut self.settings_api_key)
                                .password(true)
                                .desired_width(f32::INFINITY)
                                .hint_text("sk-..."));
                            
                            ui.add_space(16.0);
                            
                            // Model
                            ui.label(egui::RichText::new("Model")
                                .size(14.0)
                                .color(egui::Color32::from_rgb(180, 190, 210)));
                            ui.add_space(4.0);
                            ui.add(egui::TextEdit::singleline(&mut self.settings_model)
                                .desired_width(f32::INFINITY)
                                .hint_text("gpt-4o-mini"));
                            
                            ui.add_space(16.0);
                            
                            // Target Language
                            ui.label(egui::RichText::new("Target Language")
                                .size(14.0)
                                .color(egui::Color32::from_rgb(180, 190, 210)));
                            ui.add_space(4.0);
                            ui.add(egui::TextEdit::singleline(&mut self.settings_lang)
                                .desired_width(f32::INFINITY)
                                .hint_text("English"));
                            
                            ui.add_space(24.0);
                            
                            // Buttons
                            ui.horizontal(|ui| {
                                let save_btn = ui.add_sized(
                                    [100.0, 36.0],
                                    egui::Button::new(egui::RichText::new("💾 Save").size(14.0))
                                        .fill(egui::Color32::from_rgb(67, 97, 238))
                                );
                                if save_btn.clicked() {
                                    if let Ok(cfg_guard) = CONFIG.lock() {
                                        if let Some(cfg_arc) = cfg_guard.as_ref() {
                                            if let Ok(mut cfg) = cfg_arc.lock() {
                                                cfg.openai_api_key = self.settings_api_key.clone();
                                                cfg.openai_model = self.settings_model.clone();
                                                cfg.target_lang = self.settings_lang.clone();
                                                
                                                match cfg.save() {
                                                    Ok(_) => {
                                                        logger::log("Settings saved to config.json");
                                                        crate::toast("GPTTrans", "Settings saved!");
                                                        self.show_settings = false;
                                                    }
                                                    Err(e) => {
                                                        logger::log(&format!("Failed to save: {}", e));
                                                        crate::toast("GPTTrans", &format!("Failed: {}", e));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                
                                let cancel_btn = ui.add_sized(
                                    [100.0, 36.0],
                                    egui::Button::new(egui::RichText::new("Cancel").size(14.0))
                                        .fill(egui::Color32::from_rgb(55, 60, 70))
                                );
                                if cancel_btn.clicked() {
                                    self.show_settings = false;
                                }
                            });
                            
                            ui.add_space(20.0);
                            ui.separator();
                            ui.add_space(12.0);
                            
                            ui.label(egui::RichText::new("Config file:")
                                .size(12.0)
                                .color(egui::Color32::from_rgb(130, 140, 160)));
                            let config_path = Config::path();
                            ui.label(egui::RichText::new(config_path.display().to_string())
                                .size(11.0)
                                .color(egui::Color32::from_rgb(100, 110, 130)));
                        });
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
    let (tx, rx) = mpsc::channel::<UiMessage>();
    *guard = Some(tx);
    drop(guard);

    logger::log("Main UI: starting event loop");
    let app = OutputApp { 
        text: String::new(), 
        rx, 
        need_focus: false,
        show_settings: false,
        settings_api_key: String::new(),
        settings_model: String::new(),
        settings_lang: String::new(),
        is_translating: false,
    };
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("GPTTrans")
            .with_inner_size([800.0, 600.0])
            .with_position(egui::pos2(-10000.0, -10000.0))  // Start off-screen
            .with_always_on_top()
            .with_taskbar(false)  // Don't show in taskbar
            .with_decorations(false)  // Frameless window
            .with_visible(true)  // Keep visible to egui (but off-screen)
            .with_transparent(true),  // Allow transparency for rounded corners
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
