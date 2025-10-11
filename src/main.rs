#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use once_cell::sync::Lazy;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

mod config;
mod ui;
mod logger;

#[cfg(windows)]
mod win_hotkey {
    use std::thread;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging as wm;
    use windows::Win32::UI::Input::KeyboardAndMouse as km;

    pub const HOTKEY_ID: i32 = 1;

    pub fn spawn_hotkey_listener(tx: std::sync::mpsc::Sender<()>) {
        thread::spawn(move || unsafe {
            let modifiers = km::HOT_KEY_MODIFIERS(km::MOD_ALT.0 as u32);
            if km::RegisterHotKey(HWND(std::ptr::null_mut()), HOTKEY_ID, modifiers, km::VK_F3.0 as u32).is_err() {
                crate::logger::log("RegisterHotKey Alt+F3 FAILED (worker thread)");
                crate::toast("GPTTrans", "Failed to register Alt+F3 hotkey (in use?)");
            } else {
                crate::logger::log("RegisterHotKey Alt+F3 OK (worker thread)");
            }
            loop {
                let mut msg = wm::MSG::default();
                let got = wm::GetMessageW(&mut msg, HWND(std::ptr::null_mut()), 0, 0);
                if got.0 == -1 {
                    crate::logger::log("GetMessageW returned -1, breaking hotkey loop");
                    break;
                }
                if msg.message == wm::WM_HOTKEY {
                    crate::logger::log("WM_HOTKEY received (Alt+F3)");
                    let _ = tx.send(());
                }
                let _ = wm::TranslateMessage(&msg);
                wm::DispatchMessageW(&msg);
            }
            let _ = km::UnregisterHotKey(HWND(std::ptr::null_mut()), HOTKEY_ID);
            crate::logger::log("UnregisterHotKey Alt+F3");
        });
    }
}

#[cfg(not(windows))]
mod win_hotkey {
    pub fn spawn_hotkey_listener(_tx: std::sync::mpsc::Sender<()>) {
        // No-op on non-Windows for now
    }
}

mod tray {
    use std::sync::mpsc::Sender;

    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
    use tray_icon::menu::{Menu, MenuItem, MenuEvent, PredefinedMenuItem};
    use tray_icon as tri;
    use crossbeam_channel::Receiver;

    pub struct TrayHandle {
        #[allow(dead_code)]
        tray: TrayIcon,
        menu_event_rx: Receiver<MenuEvent>,
        tray_event_rx: Receiver<tri::TrayIconEvent>,
        quit_item: MenuItem,
        settings_item: MenuItem,
        action_tx: Sender<TrayAction>,
    }

    #[derive(Clone, Debug)]
    pub enum TrayAction {
        Quit,
        OpenSettings,
        ShowWindow,
    }

    impl TrayHandle {
        pub fn new(action_tx: Sender<TrayAction>) -> anyhow::Result<Self> {
            let menu = Menu::new();
            // Use plain ASCII labels to avoid any shell/encoding quirks
            let settings = MenuItem::new("Settings...", true, None);
            let quit = MenuItem::new("Quit", true, None);
            let sep = PredefinedMenuItem::separator();
            // Add a separator to improve reliability of menu rendering on some shells
            menu.append_items(&[&settings, &sep, &quit])?;

            // tiny 16x16 teal dot icon
            let (icon_w, icon_h) = (16, 16);
            let mut rgba = vec![0u8; icon_w * icon_h * 4];
            for y in 0..icon_h {
                for x in 0..icon_w {
                    let i = (y * icon_w + x) * 4;
                    rgba[i] = 0x14; // R
                    rgba[i + 1] = 0xB8; // G
                    rgba[i + 2] = 0xA6; // B
                    rgba[i + 3] = 0xFF; // A
                }
            }
            let icon = Icon::from_rgba(rgba, icon_w as u32, icon_h as u32)?;

            let tray = TrayIconBuilder::new()
                .with_tooltip("GPTTrans")
                .with_menu(Box::new(menu))
                .with_icon(icon)
                .build()?;

            let menu_event_rx = MenuEvent::receiver().clone();
            let tray_event_rx = tri::TrayIconEvent::receiver().clone();

            Ok(Self { tray, menu_event_rx, tray_event_rx, quit_item: quit, settings_item: settings, action_tx })
        }

        pub fn pump(&self) {
            // Non-blocking poll of tray menu events
            while let Ok(event) = self.menu_event_rx.try_recv() {
                let id = event.id;
                if id == self.quit_item.id() {
                    crate::logger::log("Tray: Quit clicked");
                    let _ = self.action_tx.send(TrayAction::Quit);
                } else if id == self.settings_item.id() {
                    crate::logger::log("Tray: Settings clicked");
                    let _ = self.action_tx.send(TrayAction::OpenSettings);
                }
            }
            // Non-blocking tray icon click events: show main window on left-click
            while let Ok(event) = self.tray_event_rx.try_recv() {
                #[allow(unused_variables)]
                let icon_id = event.id;
                match event.click_type {
                    tri::ClickType::Left | tri::ClickType::Double => {
                        let _ = self.action_tx.send(TrayAction::ShowWindow);
                        crate::logger::log("Tray: Left-click, showing window");
                    }
                    _ => {}
                }
            }
        }
    }
}

static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("failed to build client")
});

#[derive(serde::Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    temperature: f32,
    max_tokens: Option<u32>,
    stream: bool,
}

#[derive(serde::Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(serde::Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(serde::Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(serde::Deserialize)]
struct ChoiceMessage {
    content: String,
}

fn read_clipboard_string() -> Option<String> {
    #[cfg(windows)]
    {
        clipboard_win::get_clipboard_string().ok()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

pub(crate) fn write_clipboard_string(s: &str) -> bool {
    #[cfg(windows)]
    {
        clipboard_win::set_clipboard_string(s).is_ok()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

async fn translate_via_openai(input: &str, target_lang: &str, api_key: &str, model: &str) -> anyhow::Result<String> {
    // Shorter, optimized system prompt for faster responses
    let system = format!("Translate to {}. Output only translation.", target_lang);
    let req = ChatRequest {
        model,
        messages: vec![
            ChatMessage { role: "system", content: &system },
            ChatMessage { role: "user", content: input },
        ],
        temperature: 0.0,  // 0 for fastest, most deterministic responses
        max_tokens: Some(2048),  // Limit tokens for faster response
        stream: false,
    };

    let resp = CLIENT
        .post("https://api.openai.com/v1/chat/completions")
        .bearer_auth(api_key)
        .json(&req)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("OpenAI error {}: {}", status, text);
    }

    let parsed: ChatResponse = resp.json().await?;
    let out = parsed
        .choices
        .get(0)
        .map(|c| c.message.content.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Empty response"))?;
    Ok(out)
}

fn toast(title: &str, body: &str) {
    #[cfg(windows)]
    {
        let _ = winrt_notification::Toast::new("GPTTrans")
                .title(title)
                .text1(body)
                .show();
    }
}

#[cfg(windows)]
pub(crate) fn show_message_box(title: &str, text: &str) {
    use std::os::windows::ffi::OsStrExt;
    use std::ffi::OsStr;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging as wm;
    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }
    unsafe {
        let _ = wm::MessageBoxW(
            HWND(std::ptr::null_mut()),
            windows::core::PCWSTR(wide(text).as_ptr()),
            windows::core::PCWSTR(wide(title).as_ptr()),
            wm::MB_OK | wm::MB_TOPMOST | wm::MB_SETFOREGROUND,
        );
    }
}

#[cfg(not(windows))]
fn show_message_box(_title: &str, _text: &str) {}

fn main() {
    // Init logger first
    logger::init();
    logger::log("App starting");
    // Channels
    let (hotkey_tx, hotkey_rx) = mpsc::channel::<()>();
    let (tray_tx, tray_rx) = mpsc::channel::<tray::TrayAction>();

    // Hotkey listener on worker thread
    logger::log("Spawning hotkey listener thread");
    win_hotkey::spawn_hotkey_listener(hotkey_tx.clone());

    // Tray icon and pump on dedicated thread (keep non-Send types on one thread)
    {
        let tray_tx2 = tray_tx.clone();
        thread::spawn(move || {
            match tray::TrayHandle::new(tray_tx2) {
                Ok(tray) => {
                    logger::log("Tray created");
                    // Windows message pump on the tray thread so clicks/menus work
                    #[cfg(windows)]
                    {
                        use windows::Win32::Foundation::HWND;
                        use windows::Win32::UI::WindowsAndMessaging as wm;
                        loop {
                            unsafe {
                                let mut msg = wm::MSG::default();
                                while wm::PeekMessageW(&mut msg, HWND(std::ptr::null_mut()), 0, 0, wm::PM_REMOVE).into() {
                                    let _ = wm::TranslateMessage(&msg);
                                    wm::DispatchMessageW(&msg);
                                }
                            }
                            tray.pump();
                            thread::sleep(Duration::from_millis(25));
                        }
                    }
                    #[cfg(not(windows))]
                    loop {
                        tray.pump();
                        thread::sleep(Duration::from_millis(25));
                    }
                }
                Err(e) => {
                    toast("GPTTrans", &format!("Tray failed: {}", e));
                    logger::log(&format!("Tray failed: {}", e));
                }
            }
        });
    }

    // (tray pump handled in dedicated thread above)

    // Config: load from config.json (next to exe). Env vars still override if present.
    let mut cfg = config::Config::load();
    logger::log("Config loaded from config.json");
    if let Ok(v) = std::env::var("OPENAI_API_KEY") { if !v.is_empty() { cfg.openai_api_key = v; } }
    if let Ok(v) = std::env::var("OPENAI_MODEL") { if !v.is_empty() { cfg.openai_model = v; } }
    if let Ok(v) = std::env::var("TARGET_LANG") { if !v.is_empty() { cfg.target_lang = v; } }
    let cfg = Arc::new(Mutex::new(cfg));

    if cfg.lock().unwrap().openai_api_key.is_empty() {
        toast("GPTTrans", "Set OPENAI_API_KEY environment variable.");
    } else {
        toast("GPTTrans", "Ready. Press Alt+F3 to translate clipboard.");
    }

    // Pass config to UI module
    ui::set_config(Arc::clone(&cfg));

    // Background: tray actions
    {
        thread::spawn(move || {
            while let Ok(act) = tray_rx.recv() {
                match act {
                    tray::TrayAction::Quit => {
                        logger::log("Quit action received");
                        std::process::exit(0);
                    }
                    tray::TrayAction::OpenSettings => {
                        logger::log("OpenSettings action received");
                        ui::show_settings();
                    }
                    tray::TrayAction::ShowWindow => {
                        logger::log("ShowWindow action received");
                        ui::show_translation_window();
                    }
                }
            }
        });
    }

    // Background: hotkey translation worker
    {
        let cfg = Arc::clone(&cfg);
        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio rt");
            while let Ok(()) = hotkey_rx.recv() {
                let (api_key, model, target_lang) = {
                    let c = cfg.lock().unwrap().clone();
                    (c.openai_api_key, c.openai_model, c.target_lang)
                };
                if api_key.is_empty() {
                    toast("GPTTrans", "Missing OPENAI_API_KEY.");
                    logger::log("Hotkey: Missing OPENAI_API_KEY");
                } else if let Some(text) = read_clipboard_string() {
                    if text.trim().is_empty() {
                        toast("GPTTrans", "Clipboard is empty.");
                        logger::log("Hotkey: Clipboard empty");
                    } else {
                        toast("GPTTrans", "Translating...");
                        logger::log(&format!("Translating {} chars with model {} to {}", text.len(), model, target_lang));
                        let res = rt.block_on(async move { translate_via_openai(&text, &target_lang, &api_key, &model).await });
                        match res {
                            Ok(out) => {
                                let ok = write_clipboard_string(&out);
                                if ok {
                                    toast("GPTTrans", "Translation copied to clipboard.");
                                    logger::log("Translation success; copied to clipboard");
                                } else {
                                    toast("GPTTrans", "Translated. Failed to write clipboard.");
                                    logger::log("Translation success; failed to write clipboard");
                                }
                                ui::show_output_text(out.clone());
                                // Fallback: if UI hasn't updated, show a native message box with the translation
                                thread::spawn(move || {
                                    thread::sleep(Duration::from_millis(900));
                                    if !ui::has_ever_updated() {
                                        show_message_box("GPTTrans - Translation", &out);
                                    }
                                });
                            }
                            Err(e) => {
                                toast("GPTTrans", &format!("Error: {}", e));
                                logger::log(&format!("Translation error: {}", e));
                            }
                        }
                    }
                } else {
                    toast("GPTTrans", "Failed to read clipboard.");
                    logger::log("Hotkey: Failed to read clipboard");
                }
            }
        });
    }

    // Run UI on main thread (blocks)
    ui::run_ui_main_thread();
}
