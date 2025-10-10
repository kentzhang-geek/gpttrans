#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use once_cell::sync::Lazy;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

mod config;
mod ui;
mod logger;

// Pump Windows messages on the thread that owns the tray icon, while preserving WM_HOTKEY for explicit handling
#[cfg(windows)]
fn process_windows_messages_nonblocking(hotkey_tx: &std::sync::mpsc::Sender<()>) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging as wm;
    unsafe {
        // First, drain any WM_HOTKEY and forward to our channel
        let mut hot = wm::MSG::default();
        while wm::PeekMessageW(&mut hot, HWND(std::ptr::null_mut()), wm::WM_HOTKEY, wm::WM_HOTKEY, wm::PM_REMOVE).into() {
            let _ = hotkey_tx.send(());
        }

        // Then process all other messages (two ranges to skip WM_HOTKEY)
        let mut msg = wm::MSG::default();
        while wm::PeekMessageW(&mut msg, HWND(std::ptr::null_mut()), 0, wm::WM_HOTKEY - 1, wm::PM_REMOVE).into() {
            let _ = wm::TranslateMessage(&msg);
            wm::DispatchMessageW(&msg);
        }
        let mut msg2 = wm::MSG::default();
        while wm::PeekMessageW(&mut msg2, HWND(std::ptr::null_mut()), wm::WM_HOTKEY + 1, u32::MAX, wm::PM_REMOVE).into() {
            let _ = wm::TranslateMessage(&msg2);
            wm::DispatchMessageW(&msg2);
        }
    }
}

#[cfg(not(windows))]
fn process_windows_messages_nonblocking(_hotkey_tx: &std::sync::mpsc::Sender<()>) {}

#[cfg(windows)]
mod win_hotkey {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging as wm;
    use windows::Win32::UI::Input::KeyboardAndMouse as km;

    pub const HOTKEY_ID: i32 = 1;

    pub fn register_on_current_thread() -> bool {
        unsafe {
            // Ensure message queue exists for this thread
            let mut dummy = wm::MSG::default();
            let _ = wm::PeekMessageW(&mut dummy, HWND(std::ptr::null_mut()), 0, 0, wm::PM_NOREMOVE);

            let modifiers = km::HOT_KEY_MODIFIERS(km::MOD_ALT.0 as u32);
            if km::RegisterHotKey(HWND(std::ptr::null_mut()), HOTKEY_ID, modifiers, km::VK_F3.0 as u32).is_ok() {
                crate::logger::log("RegisterHotKey Alt+F3 OK (main thread)");
                true
            } else {
                crate::logger::log("RegisterHotKey Alt+F3 FAILED (main thread)");
                false
            }
        }
    }

    pub fn drain_hotkey_messages(tx: &std::sync::mpsc::Sender<()>) {
        unsafe {
            let mut msg = wm::MSG::default();
            while wm::PeekMessageW(&mut msg, HWND(std::ptr::null_mut()), wm::WM_HOTKEY, wm::WM_HOTKEY, wm::PM_REMOVE).into() {
                crate::logger::log("WM_HOTKEY received (Alt+F3) [main]");
                let _ = tx.send(());
            }
        }
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
    use crossbeam_channel::Receiver;

    pub struct TrayHandle {
        #[allow(dead_code)]
        tray: TrayIcon,
        menu_event_rx: Receiver<MenuEvent>,
        quit_item: MenuItem,
        settings_item: MenuItem,
        action_tx: Sender<TrayAction>,
    }

    #[derive(Clone, Debug)]
    pub enum TrayAction {
        Quit,
        OpenSettings,
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

            Ok(Self { tray, menu_event_rx, quit_item: quit, settings_item: settings, action_tx })
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
    let system = format!(
        "You are a professional translator. Translate the user's text into {}. Preserve meaning, tone, and formatting. Only output the translation.",
        target_lang
    );
    let req = ChatRequest {
        model,
        messages: vec![
            ChatMessage { role: "system", content: &system },
            ChatMessage { role: "user", content: input },
        ],
        temperature: 0.2,
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

fn main() {
    // Init logger first
    logger::init();
    logger::log("App starting");
    // Channels
    let (hotkey_tx, hotkey_rx) = mpsc::channel::<()>();
    let (tray_tx, tray_rx) = mpsc::channel::<tray::TrayAction>();

    // Register hotkey on main thread so our message pump sees it
    if !win_hotkey::register_on_current_thread() {
        toast("GPTTrans", "Failed to register Alt+F3 hotkey (in use?)");
    }

    // Tray icon
    let tray: Option<tray::TrayHandle> = match tray::TrayHandle::new(tray_tx.clone()) {
        Ok(t) => Some(t),
        Err(e) => {
            toast("GPTTrans", &format!("Tray failed: {}", e));
            logger::log(&format!("Tray failed: {}", e));
            None
        }
    };

    // Background runtime for API calls
    let rt = tokio::runtime::Runtime::new().expect("tokio rt");

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

    // Main pump loop
    loop {
        // Ensure the main thread processes Windows messages; capture WM_HOTKEY explicitly
        process_windows_messages_nonblocking(&hotkey_tx);
        // Pump tray events (non-blocking)
        // If tray creation failed earlier, this is a no-op via shadowing
        if let Some(ref tray) = tray {
            tray.pump();
        }

        // WM_HOTKEY already drained above

        // Non-blocking check for events
        let mut did_something = false;

        if let Ok(act) = tray_rx.try_recv() {
            match act {
                tray::TrayAction::Quit => { logger::log("Quit action received"); break },
                tray::TrayAction::OpenSettings => {
                    logger::log("OpenSettings action received");
                    ui::spawn_settings_window(cfg.clone());
                }
            }
        }

        if let Ok(()) = hotkey_rx.try_recv() {
            did_something = true;
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
                            // Show egui output window (scrollable)
                            ui::spawn_output_window(out);
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

        if !did_something {
            // Sleep briefly to avoid busy loop; tray menu uses its own events
            thread::sleep(Duration::from_millis(25));
        }
    }
}
