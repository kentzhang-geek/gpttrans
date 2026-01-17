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
use serde::{Deserialize, Serialize};
use windows::{
    Win32::{
        Foundation::POINT,
        Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTOPRIMARY, GetMonitorInfoW, MONITORINFO},
        UI::{WindowsAndMessaging::GetCursorPos, HiDpi::{GetDpiForMonitor, MONITOR_DPI_TYPE}},
    },
};

static OUTPUT_SENDER: Lazy<Mutex<Option<mpsc::Sender<UiMessage>>>> = Lazy::new(|| Mutex::new(None));
static LAST_TEXT: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));
static HAS_UPDATED: AtomicBool = AtomicBool::new(false);
static FONTS_SET: AtomicBool = AtomicBool::new(false);
static WINDOW_VISIBLE: AtomicBool = AtomicBool::new(false);
static CONFIG: Lazy<Mutex<Option<Arc<Mutex<Config>>>>> = Lazy::new(|| Mutex::new(None));

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OllamaModel {
    name: String,
    size: Option<u64>,
    modified_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OllamaModelsResponse {
    models: Vec<OllamaModel>,
}

/// Fetch available models from local Ollama installation
async fn fetch_ollama_models(api_base: &str) -> Result<Vec<OllamaModel>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;
    
    let url = format!("{}/api/tags", api_base);
    logger::log(&format!("Fetching Ollama models from: {}", url));
    
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to connect to Ollama: {}", e))?;
    
    if !response.status().is_success() {
        return Err(format!("Ollama API returned status: {}", response.status()));
    }
    
    let models_response: OllamaModelsResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Ollama response: {}", e))?;
    
    logger::log(&format!("Found {} Ollama models", models_response.models.len()));
    Ok(models_response.models)
}

/// Get the monitor dimensions and position where the mouse cursor is currently located
fn get_mouse_monitor_info() -> Option<(f32, f32, f32, f32)> {
    unsafe {
        let mut cursor_pos = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut cursor_pos).is_ok() {
            let monitor = MonitorFromPoint(cursor_pos, MONITOR_DEFAULTTOPRIMARY);
            if !monitor.is_invalid() {
                let mut monitor_info = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                let result = GetMonitorInfoW(monitor, &mut monitor_info);
                if result.as_bool() {
                    // Use full monitor bounds (rcMonitor) instead of work area (rcWork) for proper centering
                    let width = monitor_info.rcMonitor.right - monitor_info.rcMonitor.left;
                    let height = monitor_info.rcMonitor.bottom - monitor_info.rcMonitor.top;
                    let x_offset = monitor_info.rcMonitor.left;
                    let y_offset = monitor_info.rcMonitor.top;
                    
                    // Simple fix: assume 125% scaling and divide coordinates by 1.25
                    // This converts from logical pixels (Windows API) to egui pixels
                    let scale_factor = 1.25;
                    let egui_width = width as f32 / scale_factor;
                    let egui_height = height as f32 / scale_factor;
                    let egui_x_offset = x_offset as f32 / scale_factor;
                    let egui_y_offset = y_offset as f32 / scale_factor;
                    
                    return Some((egui_width, egui_height, egui_x_offset, egui_y_offset));
                }
            }
        }
    }
    None
}

enum UiMessage {
    ShowText(String),
    OpenSettings,
    AppendText(String),  // For streaming updates
    SetTranslating(bool), // Show/hide loading indicator
    OllamaModelsLoaded(Vec<OllamaModel>), // Ollama models fetched successfully
    OllamaModelsError(String), // Error fetching Ollama models
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
        let mut app = OutputApp { 
            text: String::new(), 
            rx, 
            need_focus: false, 
            show_settings: false,
            settings_api_key: String::new(),
            settings_model: String::new(),
            settings_lang: String::new(),
            settings_hotkey: String::new(),
            settings_api_type: String::new(),
            settings_api_base: String::new(),
            is_translating: false,
            selected_api_type: 0,
            selected_model: 0,
            ollama_models: Vec::new(),
            ollama_models_loading: false,
            ollama_models_error: None,
        };
        // Initialize with current config
        app.sync_from_config();
        
        let native_options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("Echo - Translation")
                .with_inner_size([800.0, 560.0])
                .with_always_on_top()
                .with_visible(false),
            ..Default::default()
        };
        match eframe::run_native(
            "Echo - Translation",
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
    settings_hotkey: String,
    settings_api_type: String,
    settings_api_base: String,
    is_translating: bool,
    // Dropdown selections
    selected_api_type: usize,
    selected_model: usize,
    // Dynamic Ollama models
    ollama_models: Vec<OllamaModel>,
    ollama_models_loading: bool,
    ollama_models_error: Option<String>,
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
                // Add Phosphor icons first so they don't get overwritten
                egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);

                fonts.font_data.insert("cjk".to_owned(), egui::FontData::from_owned(bytes));
                fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "cjk".to_owned());
                fonts.families.entry(egui::FontFamily::Monospace).or_default().insert(0, "cjk".to_owned());
                ctx.set_fonts(fonts);
                logger::log("Applied CJK font and Phosphor icons to egui");
            } else {
                // Even if no CJK font, we still want icons
                let mut fonts = egui::FontDefinitions::default();
                egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
                ctx.set_fonts(fonts);
                logger::log("Applied Phosphor icons (no CJK font found)");
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
                    self.sync_from_config();
                }
                UiMessage::OllamaModelsLoaded(models) => {
                    self.ollama_models = models;
                    self.ollama_models_loading = false;
                    self.ollama_models_error = None;
                    logger::log(&format!("UI: Loaded {} Ollama models", self.ollama_models.len()));
                    
                    // Try to find the currently selected model in the loaded models
                    if let Ok(cfg_guard) = CONFIG.lock() {
                        if let Some(cfg_arc) = cfg_guard.as_ref() {
                            if let Ok(cfg) = cfg_arc.lock() {
                                if let Some(index) = self.ollama_models.iter().position(|m| m.name == cfg.openai_model) {
                                    self.selected_model = index;
                                }
                            }
                        }
                    }
                }
                UiMessage::OllamaModelsError(error) => {
                    self.ollama_models_loading = false;
                    self.ollama_models_error = Some(error);
                    logger::log("UI: Failed to load Ollama models");
                }
            }
        }

        // Show and focus window when needed
        if self.need_focus {
            let was_visible = WINDOW_VISIBLE.load(Ordering::Relaxed);
            logger::log(&format!("UI: Showing window (was_visible={})", was_visible));
            
            // Restore window size and position to center of the monitor where mouse cursor is
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(800.0, 560.0)));
            // Move to center of the monitor where the mouse cursor is located
            if let Some((monitor_width, monitor_height, x_offset, y_offset)) = get_mouse_monitor_info() {
                // Calculate center position with safety margins to prevent clipping
                let window_width = 800.0;
                let window_height = 560.0;
                let center_x = x_offset + (monitor_width - window_width) / 2.0;
                let center_y = y_offset + (monitor_height - window_height) / 2.0;
                
                // Ensure the window doesn't go outside the monitor bounds
                let final_x = center_x.max(x_offset).min(x_offset + monitor_width - window_width);
                let final_y = center_y.max(y_offset).min(y_offset + monitor_height - window_height);
                
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(final_x, final_y)));
                logger::log(&format!("Positioning window on mouse monitor: {}x{} offset({},{}) center({},{}) final({},{}) [Scaled for 125%]", monitor_width, monitor_height, x_offset, y_offset, center_x, center_y, final_x, final_y));
                
                // Force a repaint to ensure the position change is applied
                ctx.request_repaint();
            } else {
                // Fallback to default monitor if we can't detect mouse monitor
                if let Some(monitor_size) = ctx.input(|i| i.viewport().monitor_size) {
                    let x = (monitor_size.x - 800.0) / 2.0;
                    let y = (monitor_size.y - 560.0) / 2.0;
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(x.max(0.0), y.max(0.0))));
                    logger::log("Positioning window on default monitor (fallback)");
                }
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
    fn sync_from_config(&mut self) {
        if let Ok(cfg_guard) = CONFIG.lock() {
            if let Some(cfg_arc) = cfg_guard.as_ref() {
                if let Ok(cfg) = cfg_arc.lock() {
                    self.settings_api_key = cfg.openai_api_key.clone();
                    self.settings_model = cfg.openai_model.clone();
                    self.settings_lang = cfg.target_lang.clone();
                    self.settings_hotkey = cfg.hotkey.clone();
                    self.settings_api_type = cfg.api_type.clone();
                    self.settings_api_base = cfg.api_base.clone();
                    
                    self.selected_api_type = match cfg.api_type.as_str() {
                        "openai" => 0,
                        "ollama" => 1,
                        "google_free" => 2,
                        _ => 0,
                    };
                    
                    // Load Ollama models if API type is Ollama
                    if self.selected_api_type == 1 {
                        self.load_ollama_models();
                    }
                    
                    // Set model selection based on config
                    if self.selected_api_type == 0 {
                        self.selected_model = 0; // OpenAI always has one model
                    } else if self.selected_api_type == 1 {
                        // For Ollama, try to match the current model string to an index
                        if !self.ollama_models.is_empty() {
                            if let Some(index) = self.ollama_models.iter().position(|m| m.name == cfg.openai_model) {
                                self.selected_model = index;
                            }
                        }
                    }
                }
            }
        }
    }

    fn save_to_config(&mut self) {
        if let Ok(cfg_guard) = CONFIG.lock() {
            if let Some(cfg_arc) = cfg_guard.as_ref() {
                if let Ok(mut cfg) = cfg_arc.lock() {
                    let new_api_type = match self.selected_api_type {
                        0 => "openai",
                        1 => "ollama",
                        2 => "google_free",
                        _ => "openai",
                    };
                    
                    cfg.api_type = new_api_type.to_string();
                    self.settings_api_type = cfg.api_type.clone();
                    
                    // Auto-configure base URL and Model
                    if self.selected_api_type == 1 { // Ollama
                        if cfg.api_base.is_empty() || cfg.api_base == "https://api.openai.com/v1" {
                             cfg.api_base = "http://localhost:11434".to_string();
                             self.settings_api_base = cfg.api_base.clone();
                        }
                        // If current model is OpenAI default, switch to a default Ollama model
                        // We can't rely on loaded models yet as they might be loading async
                        if cfg.openai_model == "gpt-4o-mini" {
                            cfg.openai_model = "gemma3:1b".to_string();
                            self.settings_model = cfg.openai_model.clone();
                        } else {
                            // If we have a selected model index and it's valid, use it
                            if !self.ollama_models.is_empty() && self.selected_model < self.ollama_models.len() {
                                cfg.openai_model = self.ollama_models[self.selected_model].name.clone();
                                self.settings_model = cfg.openai_model.clone();
                            }
                        }
                    } else if self.selected_api_type == 0 { // OpenAI
                        if cfg.api_base.is_empty() || cfg.api_base == "http://localhost:11434" {
                             cfg.api_base = "https://api.openai.com/v1".to_string();
                             self.settings_api_base = cfg.api_base.clone();
                        }
                        // Always ensure OpenAI model is valid
                        cfg.openai_model = "gpt-4o-mini".to_string();
                        self.settings_model = cfg.openai_model.clone();
                    }

                    match cfg.save() {
                        Ok(_) => logger::log("Settings saved from title bar"),
                        Err(e) => logger::log(&format!("Failed to save settings: {}", e)),
                    }
                }
            }
        }
    }

    fn load_ollama_models(&mut self) {
        if self.ollama_models_loading {
            return; // Already loading
        }
        
        self.ollama_models_loading = true;
        self.ollama_models_error = None;
        
        let api_base = self.settings_api_base.clone();
        let rt = tokio::runtime::Runtime::new().unwrap();
        
        // Spawn async task to fetch models
        std::thread::spawn(move || {
            let models = rt.block_on(async {
                fetch_ollama_models(&api_base).await
            });
            
            // Send result back to UI thread
            if let Ok(guard) = OUTPUT_SENDER.lock() {
                if let Some(tx) = guard.as_ref() {
                    match models {
                        Ok(models) => {
                            let _ = tx.send(UiMessage::OllamaModelsLoaded(models));
                        }
                        Err(error) => {
                            let _ = tx.send(UiMessage::OllamaModelsError(error));
                        }
                    }
                }
            }
        });
    }

    fn show_translation_ui(&mut self, ctx: &egui::Context) {
        // Set custom style for better text rendering
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 12.0);
        style.spacing.button_padding = egui::vec2(12.0, 8.0);
        style.spacing.window_margin = egui::Margin::same(0.0);
        style.visuals.window_rounding = egui::Rounding::same(12.0);
        style.visuals.window_shadow = egui::epaint::Shadow {
            offset: egui::vec2(0.0, 8.0),
            blur: 24.0,
            spread: 0.0,
            color: egui::Color32::from_black_alpha(100),
        };
        ctx.set_style(style);
        
        // Custom frameless window with rounded corners and gradient
        egui::CentralPanel::default()
            .frame(egui::Frame::none()
                .fill(egui::Color32::from_rgb(32, 35, 42))
                .rounding(egui::Rounding::same(12.0))
                .inner_margin(egui::Margin::same(0.0))
                .shadow(egui::epaint::Shadow {
                    offset: egui::vec2(0.0, 8.0),
                    blur: 32.0,
                    spread: 0.0,
                    color: egui::Color32::from_black_alpha(80),
                }))
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
                    
                    // Draw custom title bar with gradient effect
                    ui.painter().rect_filled(
                        title_bar_rect,
                        egui::Rounding { nw: 12.0, ne: 12.0, sw: 0.0, se: 0.0 },
                        egui::Color32::from_rgb(42, 46, 54),
                    );
                    
                    // Add subtle gradient line at bottom of title bar
                    let line_rect = egui::Rect::from_min_max(
                        egui::pos2(title_bar_rect.min.x + 12.0, title_bar_rect.max.y - 1.0),
                        egui::pos2(title_bar_rect.max.x - 12.0, title_bar_rect.max.y),
                    );
                    ui.painter().rect_filled(
                        line_rect,
                        egui::Rounding::ZERO,
                        egui::Color32::from_rgb(60, 65, 75),
                    );
                    
                    ui.allocate_ui_at_rect(title_bar_rect, |ui| {
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add_space(16.0);
                            
                            // Left-aligned Title
                            ui.label(egui::RichText::new("📝 Echo")
                                .size(18.0)
                                .color(egui::Color32::from_rgb(138, 180, 248)));
                            
                            // Right-aligned controls
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_space(8.0);
                                
                                // Close button
                                let close_btn_size = egui::vec2(36.0, 36.0);
                                let (close_rect, close_resp) = ui.allocate_at_least(close_btn_size, egui::Sense::click());
                                if close_resp.hovered() {
                                    ui.painter().rect_filled(
                                        close_rect,
                                        egui::Rounding::same(6.0),
                                        egui::Color32::from_rgb(239, 68, 68),
                                    );
                                }
                                ui.painter().text(
                                    close_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    egui_phosphor::regular::X,
                                    egui::FontId::proportional(16.0),
                                    if close_resp.hovered() { egui::Color32::WHITE } else { egui::Color32::from_rgb(200, 200, 210) },
                                );
                                if close_resp.clicked() {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(-10000.0, -10000.0)));
                                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1.0, 1.0)));
                                    WINDOW_VISIBLE.store(false, Ordering::Relaxed);
                                    logger::log("Output window: hidden by user (moved off-screen)");
                                }
                                
                                // Settings button
                                let settings_btn_size = egui::vec2(36.0, 36.0);
                                let (settings_rect, settings_resp) = ui.allocate_at_least(settings_btn_size, egui::Sense::click());
                                if settings_resp.hovered() {
                                    ui.painter().rect_filled(
                                        settings_rect,
                                        egui::Rounding::same(6.0),
                                        egui::Color32::from_rgb(55, 60, 70),
                                    );
                                }
                                ui.painter().text(
                                    settings_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    egui_phosphor::regular::GEAR,
                                    egui::FontId::proportional(16.0),
                                    if settings_resp.hovered() { egui::Color32::WHITE } else { egui::Color32::from_rgb(200, 200, 210) },
                                );
                                if settings_resp.clicked() {
                                    self.show_settings = true;
                                    self.sync_from_config();
                                }
                                
                                // Copy button
                                let copy_btn_size = egui::vec2(36.0, 36.0);
                                let (copy_rect, copy_resp) = ui.allocate_at_least(copy_btn_size, egui::Sense::click());
                                if copy_resp.hovered() {
                                    ui.painter().rect_filled(
                                        copy_rect,
                                        egui::Rounding::same(6.0),
                                        egui::Color32::from_rgb(55, 60, 70),
                                    );
                                }
                                ui.painter().text(
                                    copy_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    egui_phosphor::regular::CLIPBOARD_TEXT,
                                    egui::FontId::proportional(16.0),
                                    if copy_resp.hovered() { egui::Color32::WHITE } else { egui::Color32::from_rgb(200, 200, 210) },
                                );
                                if copy_resp.clicked() {
                                    let _ = write_clipboard_string(&self.text);
                                    logger::log("Text copied to clipboard");
                                }

                                ui.add_space(8.0);
                                ui.separator();
                                ui.add_space(8.0);

                                // API Switcher Combo Box
                                let combo_width = 110.0;
                                ui.allocate_ui_with_layout(
                                    egui::vec2(combo_width, 36.0),
                                    egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                                    |ui| {
                                        egui::ComboBox::from_id_source("title_api_switcher")
                                            .width(combo_width)
                                            .selected_text(match self.selected_api_type {
                                                0 => "OpenAI",
                                                1 => "Ollama",
                                                2 => "Google",
                                                _ => "OpenAI"
                                            })
                                            .show_ui(ui, |ui| {
                                                let mut changed = false;
                                                if ui.selectable_value(&mut self.selected_api_type, 0, "OpenAI").clicked() { changed = true; }
                                                if ui.selectable_value(&mut self.selected_api_type, 1, "Ollama").clicked() { changed = true; }
                                                if ui.selectable_value(&mut self.selected_api_type, 2, "Google").clicked() { changed = true; }
                                                
                                                if changed {
                                                    if self.selected_api_type == 1 {
                                                        self.load_ollama_models();
                                                    }
                                                    self.save_to_config();
                                                }
                                            });
                                    }
                                );
                            });
                        });
                    });

                    ui.add_space(8.0);
                    
                    // Content area with padding and better styling
                    egui::Frame::none()
                        .fill(egui::Color32::from_rgb(40, 43, 50))
                        .inner_margin(egui::Margin::symmetric(20.0, 16.0))
                        .rounding(egui::Rounding { nw: 0.0, ne: 0.0, sw: 12.0, se: 12.0 })
                        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                                    // Custom text style with better line spacing
                                    let mut layout_job = egui::text::LayoutJob::default();
                                    layout_job.text = self.text.clone();
                                    layout_job.wrap = egui::text::TextWrapping {
                                        max_width: ui.available_width() - 8.0,
                                        max_rows: 1000,
                                        break_anywhere: false,
                                        overflow_character: Some('…'),
                                    };
                                    
                                    // Add all text with custom styling
                                    layout_job.sections.push(egui::text::LayoutSection {
                                        leading_space: 0.0,
                                        byte_range: 0..layout_job.text.len(),
                                        format: egui::TextFormat {
                                            font_id: egui::FontId::proportional(16.0),
                                            color: egui::Color32::from_rgb(220, 225, 235),
                                            background: egui::Color32::TRANSPARENT,
                                            italics: false,
                                            underline: egui::Stroke::NONE,
                                            strikethrough: egui::Stroke::NONE,
                                            valign: egui::Align::BOTTOM,
                                            ..Default::default()
                                        },
                                    });
                                    
                    ui.add(
                        egui::TextEdit::multiline(&mut self.text)
                            .desired_rows(20)
                                            .desired_width(f32::INFINITY)
                                            .font(egui::FontId::proportional(16.0))
                                            .frame(false)
                                            .text_color(egui::Color32::from_rgb(220, 225, 235))
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
                                    egui::Button::new(egui::RichText::new(egui_phosphor::regular::X).size(16.0))
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
                    
                    // Settings content - scrollable
                    egui::ScrollArea::vertical()
                        .max_height(ui.available_height() - 120.0) // Leave space for buttons and config path
                        .show(ui, |ui| {
                            egui::Frame::none()
                                .inner_margin(egui::Margin::symmetric(24.0, 0.0))
                                .show(ui, |ui| {
                            ui.add_space(8.0);
                            
                            // API Type dropdown
                            ui.label(egui::RichText::new("API Type")
                                .size(14.0)
                                .color(egui::Color32::from_rgb(180, 190, 210)));
                            ui.add_space(4.0);
                            egui::ComboBox::from_id_source("api_type")
                                .selected_text(match self.selected_api_type {
                                    0 => "OpenAI",
                                    1 => "Ollama (Free)",
                                    2 => "Google (Free)",
                                    _ => "OpenAI"
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut self.selected_api_type, 0, "OpenAI");
                                    ui.selectable_value(&mut self.selected_api_type, 1, "Ollama (Free)");
                                    ui.selectable_value(&mut self.selected_api_type, 2, "Google (Free)");
                                });
                            
                            // Update API type and base URL when selection changes
                            let new_api_type = match self.selected_api_type {
                                0 => "openai",
                                1 => "ollama",
                                2 => "google_free",
                                _ => "openai",
                            };

                            if self.settings_api_type != new_api_type {
                                self.settings_api_type = new_api_type.to_string();
                                self.settings_api_base = match self.selected_api_type {
                                    0 => "https://api.openai.com/v1".to_string(),
                                    1 => "http://localhost:11434".to_string(),
                                    _ => String::new(), // Not used for google_free
                                };
                                // Reset model selection when API type changes
                                self.selected_model = 0;
                                
                                // Load Ollama models if switching to Ollama
                                if self.selected_api_type == 1 {
                                    self.load_ollama_models();
                                }
                            }
                            
                            ui.add_space(16.0);

                            if self.selected_api_type == 2 {
                                // Google Free - No extra settings needed
                                ui.label(egui::RichText::new("ℹ No API key required for Google Translate")
                                    .size(13.0)
                                    .color(egui::Color32::from_rgb(138, 180, 248)));
                            } else {
                                // API Key (Only for OpenAI)
                                if self.selected_api_type == 0 {
                                    ui.label(egui::RichText::new("OpenAI API Key")
                                        .size(14.0)
                                        .color(egui::Color32::from_rgb(180, 190, 210)));
                                    ui.add_space(4.0);
                                    ui.add(egui::TextEdit::singleline(&mut self.settings_api_key)
                                        .password(true)
                                        .desired_width(f32::INFINITY)
                                        .hint_text("sk-..."));
                                    
                                    ui.add_space(16.0);
                                }
                                
                                // Model dropdown
                                ui.label(egui::RichText::new("Model")
                                    .size(14.0)
                                    .color(egui::Color32::from_rgb(180, 190, 210)));
                                ui.add_space(4.0);
                                
                                if self.selected_api_type == 0 {
                                    // OpenAI models (static)
                                    egui::ComboBox::from_id_source("model")
                                        .selected_text("GPT-4o Mini")
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(&mut self.selected_model, 0, "GPT-4o Mini");
                                    });
                            } else {
                                // Ollama models (dynamic)
                                if self.ollama_models_loading {
                                    ui.label(egui::RichText::new("Loading models...")
                                        .color(egui::Color32::from_rgb(150, 160, 180)));
                                } else if let Some(ref error) = self.ollama_models_error {
                                    ui.label(egui::RichText::new(&format!("Error: {}", error))
                                        .color(egui::Color32::from_rgb(255, 100, 100)));
                                    if ui.button("🔄 Retry").clicked() {
                                        self.load_ollama_models();
                                    }
                                } else if self.ollama_models.is_empty() {
                                    ui.label(egui::RichText::new("No models found")
                                        .color(egui::Color32::from_rgb(150, 160, 180)));
                                    if ui.button("🔄 Refresh").clicked() {
                                        self.load_ollama_models();
                                    }
                                } else {
                                    let selected_text = if self.selected_model < self.ollama_models.len() {
                                        &self.ollama_models[self.selected_model].name
                                    } else {
                                        "Select a model"
                                    };
                                    
                                    egui::ComboBox::from_id_source("model")
                                        .selected_text(selected_text)
                                        .show_ui(ui, |ui| {
                                            for (i, model) in self.ollama_models.iter().enumerate() {
                                                ui.selectable_value(&mut self.selected_model, i, &model.name);
                                            }
                                        });
                                }
                            }
                            
                            }
                            
                            // Update model when selection changes
                            let new_model = if self.selected_api_type == 0 {
                                "gpt-4o-mini".to_string()
                            } else {
                                if self.selected_model < self.ollama_models.len() {
                                    self.ollama_models[self.selected_model].name.clone()
                                } else {
                                    "gemma3:1b".to_string() // fallback
                                }
                            };
                            if self.settings_model != new_model {
                                self.settings_model = new_model;
                            }
                            
                            ui.add_space(16.0);
                            
                            // API Base URL (read-only, auto-configured)
                            ui.label(egui::RichText::new("API Base URL (Auto-configured)")
                                .size(14.0)
                                .color(egui::Color32::from_rgb(180, 190, 210)));
                            ui.add_space(4.0);
                            ui.add(egui::TextEdit::singleline(&mut self.settings_api_base)
                                .desired_width(f32::INFINITY)
                                .interactive(false)
                                .hint_text("Automatically set based on API Type"));
                            
                            ui.add_space(16.0);
                            
                            // Target Language
                            ui.label(egui::RichText::new("Target Language")
                                .size(14.0)
                                .color(egui::Color32::from_rgb(180, 190, 210)));
                            ui.add_space(4.0);
                            ui.add(egui::TextEdit::singleline(&mut self.settings_lang)
                                .desired_width(f32::INFINITY)
                                .hint_text("English"));
                            
                            ui.add_space(16.0);
                            
                            // Hotkey
                            ui.label(egui::RichText::new("Hotkey (requires restart)")
                                .size(14.0)
                                .color(egui::Color32::from_rgb(180, 190, 210)));
                            ui.add_space(4.0);
                            ui.add(egui::TextEdit::singleline(&mut self.settings_hotkey)
                                .desired_width(f32::INFINITY)
                                .hint_text("Alt+F3"));
                            ui.label(egui::RichText::new("Examples: Alt+F3, Ctrl+Shift+T, Win+Q")
                                .size(11.0)
                                .color(egui::Color32::from_rgb(120, 130, 150)));
                            
                            ui.add_space(24.0);
                            });
                        });
                    
                    // Fixed buttons at bottom (always visible)
                    egui::Frame::none()
                        .inner_margin(egui::Margin::symmetric(24.0, 0.0))
                        .show(ui, |ui| {
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
                                                cfg.hotkey = self.settings_hotkey.clone();
                                                cfg.api_type = self.settings_api_type.clone();
                                                cfg.api_base = self.settings_api_base.clone();
                                                
                                                match cfg.save() {
                                                    Ok(_) => {
                                                        logger::log("Settings saved to config.json (restart to apply changes)");
                                                        crate::toast("GPTTrans", "Saved! Restart to apply changes.");
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
        settings_hotkey: String::new(),
        settings_api_type: String::new(),
        settings_api_base: String::new(),
        is_translating: false,
        selected_api_type: 0,
        selected_model: 0,
        ollama_models: Vec::new(),
        ollama_models_loading: false,
        ollama_models_error: None,
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
