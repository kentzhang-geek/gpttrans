#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use once_cell::sync::Lazy;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

mod config;
mod ui;

#[cfg(windows)]
mod win_hotkey {
    use std::thread;

    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging as wm;
    use windows::Win32::UI::Input::KeyboardAndMouse as km;

    pub const HOTKEY_ID: i32 = 1;

    pub fn spawn_hotkey_listener(tx: std::sync::mpsc::Sender<()>) {
        thread::spawn(move || unsafe {
            // Register on this thread so WM_HOTKEY messages are posted here
            let modifiers = km::HOT_KEY_MODIFIERS(km::MOD_ALT.0 as u32);
            if km::RegisterHotKey(HWND(std::ptr::null_mut()), HOTKEY_ID, modifiers, km::VK_F3.0 as u32).is_err() {
                // best effort notification
                crate::toast("GPTTrans", "Failed to register Alt+F3 hotkey (in use?)");
            }
            loop {
                let mut msg = wm::MSG::default();
                let got = wm::GetMessageW(&mut msg, HWND(std::ptr::null_mut()), 0, 0);
                if got.0 == -1 {
                    break;
                }
                if msg.message == wm::WM_HOTKEY {
                    let _ = tx.send(());
                }
                wm::TranslateMessage(&msg);
                wm::DispatchMessageW(&msg);
            }
            let _ = km::UnregisterHotKey(HWND(std::ptr::null_mut()), HOTKEY_ID);
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
    use tray_icon::menu::{Menu, MenuItem, MenuEvent};
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
            let settings = MenuItem::new("Settings…", true, None);
            let quit = MenuItem::new("Quit", true, None);
            menu.append_items(&[&settings, &quit])?;

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
                    let _ = self.action_tx.send(TrayAction::Quit);
                } else if id == self.settings_item.id() {
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
    // Channels
    let (hotkey_tx, hotkey_rx) = mpsc::channel::<()>();
    let (tray_tx, tray_rx) = mpsc::channel::<tray::TrayAction>();

    // Hotkey listener thread (Windows)
    win_hotkey::spawn_hotkey_listener(hotkey_tx);

    // Tray icon
    let tray: Option<tray::TrayHandle> = match tray::TrayHandle::new(tray_tx.clone()) {
        Ok(t) => Some(t),
        Err(e) => {
            toast("GPTTrans", &format!("Tray failed: {}", e));
            None
        }
    };

    // Background runtime for API calls
    let rt = tokio::runtime::Runtime::new().expect("tokio rt");

    // Config: load from config.json (next to exe). Env vars still override if present.
    let mut cfg = config::Config::load();
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
        // Pump tray events (non-blocking)
        // If tray creation failed earlier, this is a no-op via shadowing
        if let Some(ref tray) = tray {
            tray.pump();
        }

        // Non-blocking check for events
        let mut did_something = false;

        if let Ok(act) = tray_rx.try_recv() {
            match act {
                tray::TrayAction::Quit => break,
                tray::TrayAction::OpenSettings => {
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
            } else if let Some(text) = read_clipboard_string() {
                if text.trim().is_empty() {
                    toast("GPTTrans", "Clipboard is empty.");
                } else {
                    toast("GPTTrans", "Translating…");
                    let res = rt.block_on(async move { translate_via_openai(&text, &target_lang, &api_key, &model).await });
                    match res {
                        Ok(out) => {
                            let ok = write_clipboard_string(&out);
                            if ok {
                                toast("GPTTrans", "Translation copied to clipboard.");
                            } else {
                                toast("GPTTrans", "Translated. Failed to write clipboard.");
                            }
                            // Show egui output window (scrollable)
                            ui::spawn_output_window(out);
                        }
                        Err(e) => {
                            toast("GPTTrans", &format!("Error: {}", e));
                        }
                    }
                }
            } else {
                toast("GPTTrans", "Failed to read clipboard.");
            }
        }

        if !did_something {
            // Sleep briefly to avoid busy loop; tray menu uses its own events
            thread::sleep(Duration::from_millis(25));
        }
    }
}
