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

    pub fn spawn_hotkey_listener(tx: std::sync::mpsc::Sender<()>, modifiers: u32, vk_code: u32, hotkey_str: String) {
        thread::spawn(move || unsafe {
            let mods = km::HOT_KEY_MODIFIERS(modifiers);
            if km::RegisterHotKey(HWND(std::ptr::null_mut()), HOTKEY_ID, mods, vk_code).is_err() {
                crate::logger::log(&format!("RegisterHotKey {} FAILED (in use?)", hotkey_str));
                crate::toast("GPTTrans", &format!("Failed to register {} hotkey (in use?)", hotkey_str));
            } else {
                crate::logger::log(&format!("RegisterHotKey {} OK", hotkey_str));
            }
            loop {
                let mut msg = wm::MSG::default();
                let got = wm::GetMessageW(&mut msg, HWND(std::ptr::null_mut()), 0, 0);
                if got.0 == -1 {
                    crate::logger::log("GetMessageW returned -1, breaking hotkey loop");
                    break;
                }
                if msg.message == wm::WM_HOTKEY {
                    crate::logger::log(&format!("WM_HOTKEY received ({})", hotkey_str));
                    let _ = tx.send(());
                }
                let _ = wm::TranslateMessage(&msg);
                wm::DispatchMessageW(&msg);
            }
            let _ = km::UnregisterHotKey(HWND(std::ptr::null_mut()), HOTKEY_ID);
            crate::logger::log(&format!("UnregisterHotKey {}", hotkey_str));
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
#[serde(untagged)]
enum MessageContent<'a> {
    Text(&'a str),
    List(Vec<ContentPart<'a>>),
}

#[derive(serde::Serialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
enum ContentPart<'a> {
    Text { text: &'a str },
    ImageUrl { image_url: ImageUrl<'a> },
}

#[derive(serde::Serialize)]
struct ImageUrl<'a> {
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<&'a str>,
}

#[derive(serde::Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: MessageContent<'a>,
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
        use std::thread;
        use std::time::Duration;
        
        if !clipboard_win::is_format_avail(clipboard_win::formats::Unicode.into()) {
            return None;
        }

        for i in 0..3 {
            match clipboard_win::get_clipboard_string() {
                Ok(s) => return Some(s),
                Err(e) => {
                    let err_code = e.raw_code();
                    if err_code == 5 { // Access Denied
                        crate::logger::log(&format!("Try {}: Clipboard locked (Access Denied)", i+1));
                        thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    crate::logger::log(&format!("Try {}: Failed to read clipboard string: {} (code: {})", i+1, e, err_code));
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        None
    }
}

pub struct ImageData {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}

fn read_clipboard_image() -> Option<ImageData> {
    #[cfg(windows)]
    {
        use clipboard_win::{formats, get_clipboard, is_format_avail};
        use std::thread;
        use std::time::Duration;
        
        if !is_format_avail(formats::Bitmap.into()) {
            return None;
        }

        for i in 0..3 {
            match get_clipboard(formats::Bitmap) {
                Ok(buffer) => {
                    let buffer: Vec<u8> = buffer;
                    // formats::Bitmap in clipboard-win refers to CF_DIB (Device Independent Bitmap)
                    match load_dib(&buffer) {
                        Ok(img) => {
                            let mut png_bytes = std::io::Cursor::new(Vec::new());
                            if img.write_to(&mut png_bytes, image::ImageFormat::Png).is_ok() {
                                return Some(ImageData {
                                    bytes: png_bytes.into_inner(),
                                    mime_type: "image/png".to_string(),
                                });
                            }
                        }
                        Err(e) => {
                            crate::logger::log(&format!("Failed to load DIB from clipboard: {}", e));
                        }
                    }
                    break; // If we got a buffer but failed to parse, retrying likely won't help much
                }
                Err(e) => {
                    let err_code = e.raw_code();
                    if err_code == 5 { // Access Denied
                        crate::logger::log(&format!("Try {}: get_clipboard(Bitmap) locked (Access Denied)", i+1));
                        thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    crate::logger::log(&format!("Try {}: get_clipboard(Bitmap) failed: {} (code: {})", i+1, e, err_code));
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
fn load_dib(buffer: &[u8]) -> anyhow::Result<image::DynamicImage> {
    // DIB (Device Independent Bitmap) 
    // Usually it's BITMAPINFOHEADER followed by color table (optional) and then bits.
    // Actually, a DIB is essentially a BMP without the 14-byte File Header.
    
    if buffer.len() < 4 {
        anyhow::bail!("DIB too short ({} bytes)", buffer.len());
    }

    // Check if it's already a full BMP file (some apps or clipboard-win versions might return it this way)
    if buffer.starts_with(b"BM") {
        return Ok(image::load_from_memory_with_format(buffer, image::ImageFormat::Bmp)?);
    }

    let header_size = u32::from_le_bytes(buffer[0..4].try_into()?);
    
    // Validate header size. Standard sizes are 40 (V1/INFO), 108 (V4), 124 (V5), 12 (CORE)
    if header_size != 40 && header_size != 108 && header_size != 124 && header_size != 12 && header_size != 64 {
        let hex_prefix: String = buffer.iter().take(16).map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
        anyhow::bail!("Unsupported or invalid DIB header size: {} (Prefix: {})", header_size, hex_prefix);
    }

    let mut bmp_data = Vec::with_capacity(buffer.len() + 14);
    let total_size = (buffer.len() + 14) as u32;
    
    // BMP File Header
    bmp_data.extend_from_slice(b"BM");
    bmp_data.extend_from_slice(&total_size.to_le_bytes());
    bmp_data.extend_from_slice(&[0, 0, 0, 0]); // Reserved
    
    // Offset to pixel data. For DIB, it depends on the header size and color table.
    let mut offset = 14 + header_size;
    
    if header_size >= 16 {
        let bit_count = u16::from_le_bytes(buffer[14..16].try_into()?);
        if bit_count <= 8 {
            let clr_used = if header_size >= 36 {
                u32::from_le_bytes(buffer[32..36].try_into()?)
            } else { 0 };
            
            let num_colors = if clr_used == 0 {
                1 << bit_count
            } else {
                clr_used
            };
            offset += num_colors * 4;
        }
    } else if header_size == 12 { // BITMAPCOREHEADER
        let bit_count = u16::from_le_bytes(buffer[10..12].try_into()?);
        if bit_count <= 8 {
            offset += (1 << bit_count) * 3; // RGBTriple instead of RGBQuad
        }
    }

    bmp_data.extend_from_slice(&offset.to_le_bytes());
    bmp_data.extend_from_slice(buffer);
    
    Ok(image::load_from_memory_with_format(&bmp_data, image::ImageFormat::Bmp)?)
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

async fn translate_via_openai_stream<F>(
    input: &str, 
    image_data: Option<ImageData>,
    target_lang: &str, 
    api_key: &str, 
    model: &str, 
    api_base: &str,
    api_type: &str,
    mut on_chunk: F
) -> anyhow::Result<String>
where
    F: FnMut(String),
{
    use futures_util::StreamExt;
    use base64::{Engine as _, engine::general_purpose};
    
    // Optimized prompt for gemma3:270m translation
    let user_content = if target_lang.to_lowercase().contains("chinese") {
        if image_data.is_some() {
            "Translate the text in this image to Chinese, only output the translation, no other text".to_string()
        } else {
            format!("Translate '{}' to Chinese, only output the translation, no other text", input)
        }
    } else if target_lang.to_lowercase().contains("english") {
        if image_data.is_some() {
            "Translate the text in this image to English, only output the translation, no other text".to_string()
        } else {
            format!("Translate '{}' to English, only output the translation, no other text", input)
        }
    } else {
        if image_data.is_some() {
            format!("Translate the text in this image to {}, only output the translation, no other text", target_lang)
        } else {
            format!("Translate '{}' to {}, only output the translation, no other text", input, target_lang)
        }
    };

    let mut messages = Vec::new();
    if let Some(img) = image_data {
        let b64 = general_purpose::STANDARD.encode(&img.bytes);
        let data_url = format!("data:{};base64,{}", img.mime_type, b64);
        messages.push(ChatMessage {
            role: "user",
            content: MessageContent::List(vec![
                ContentPart::Text { text: &user_content },
                ContentPart::ImageUrl {
                    image_url: ImageUrl {
                        url: data_url,
                        detail: Some("high"),
                    },
                },
            ]),
        });
    } else {
        messages.push(ChatMessage {
            role: "user",
            content: MessageContent::Text(&user_content),
        });
    }

    let req = ChatRequest {
        model,
        messages,
        temperature: 0.1,
        max_tokens: Some(1024),
        stream: true,
    };

    // Build request with appropriate authentication and endpoint
    let (endpoint, request_body) = if api_type == "ollama" {
        // Use native Ollama API format
        let ollama_endpoint = format!("{}/api/generate", api_base);
        
        let mut ollama_req = serde_json::json!({
            "model": model,
            "prompt": user_content,
            "stream": true
        });

        if let MessageContent::List(ref list) = req.messages[0].content {
            for part in list {
                if let ContentPart::ImageUrl { image_url } = part {
                    // Extract base64 from data URL
                    if let Some(b64) = image_url.url.split(',').last() {
                        ollama_req["images"] = serde_json::json!([b64]);
                    }
                }
            }
        }

        (ollama_endpoint, ollama_req)
    } else {
        // Use OpenAI-compatible API format
        let openai_endpoint = format!("{}/chat/completions", api_base);
        let openai_req = serde_json::to_value(&req).unwrap();
        (openai_endpoint, openai_req)
    };
    
    let mut request_builder = CLIENT.post(&endpoint).json(&request_body);
    
    // Add authentication based on API type
    if api_type != "ollama" && !api_key.is_empty() {
        request_builder = request_builder.bearer_auth(api_key);
    }
    
    // Debug logging for Ollama requests
    if api_type == "ollama" {
        logger::log(&format!("Ollama native API request to: {}", endpoint));
        logger::log(&format!("Ollama model: {}", model));
        logger::log(&format!("User prompt: {}", user_content));
    }
    
    let resp = request_builder.send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        
        // Handle common Ollama error for non-vision models
        if api_type == "ollama" && status == 500 && text.contains("missing data required for image input") {
            anyhow::bail!("Ollama error: The model '{}' does not support images. Please use a vision model like 'llava'.", model);
        }
        
        anyhow::bail!("API error {}: {}", status, text);
    }

    let mut stream = resp.bytes_stream();
    let mut full_text = String::new();
    let mut buffer = Vec::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result?;
        buffer.extend_from_slice(&chunk);
        
        // Convert buffer to string and process
        let buffer_str = String::from_utf8_lossy(&buffer);
        let mut remaining = String::new();
        
        if api_type == "ollama" {
            // Native Ollama API format - each line is a JSON object
            for line in buffer_str.lines() {
                let line = line.trim();
                if !line.is_empty() {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(line) {
                        if let Some(content) = parsed["response"].as_str() {
                            full_text.push_str(content);
                            on_chunk(content.to_string());
                        }
                    }
                }
            }
        } else {
            // OpenAI-compatible API format - SSE format
            for line in buffer_str.lines() {
                let line = line.trim();
                
                if line.starts_with("data: ") {
                    let json_str = &line[6..];
                    if json_str == "[DONE]" {
                        break;
                    }
                    
                    // Parse the JSON chunk
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                        if let Some(content) = parsed["choices"][0]["delta"]["content"].as_str() {
                            full_text.push_str(content);
                            on_chunk(content.to_string());
                        }
                    }
                } else if !line.is_empty() && !line.starts_with("data:") {
                    remaining.push_str(line);
                    remaining.push('\n');
                }
            }
        }
        
        // Keep incomplete data in buffer
        buffer = remaining.into_bytes();
    }

    if full_text.is_empty() {
        anyhow::bail!("Empty response from OpenAI");
    }
    
    Ok(full_text)
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
    
    // Load config early to get hotkey
    let mut cfg = config::Config::load();
    logger::log("Config loaded from config.json");
    if let Ok(v) = std::env::var("OPENAI_API_KEY") { if !v.is_empty() { cfg.openai_api_key = v; } }
    if let Ok(v) = std::env::var("OPENAI_MODEL") { if !v.is_empty() { cfg.openai_model = v; } }
    if let Ok(v) = std::env::var("TARGET_LANG") { if !v.is_empty() { cfg.target_lang = v; } }
    
    // Channels
    let (hotkey_tx, hotkey_rx) = mpsc::channel::<()>();
    let (tray_tx, tray_rx) = mpsc::channel::<tray::TrayAction>();

    // Hotkey listener on worker thread with configurable hotkey
    logger::log("Spawning hotkey listener thread");
    #[cfg(windows)]
    {
        if let Some((modifiers, vk_code)) = cfg.parse_hotkey() {
            let hotkey_str = cfg.hotkey.clone();
            logger::log(&format!("Using hotkey: {}", hotkey_str));
            win_hotkey::spawn_hotkey_listener(hotkey_tx.clone(), modifiers, vk_code, hotkey_str);
        } else {
            logger::log(&format!("Invalid hotkey format: {}, using default Alt+F3", cfg.hotkey));
            win_hotkey::spawn_hotkey_listener(hotkey_tx.clone(), 0x0001, 0x72, "Alt+F3".to_string());
        }
    }
    #[cfg(not(windows))]
    {
        win_hotkey::spawn_hotkey_listener(hotkey_tx.clone());
    }

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

    // Wrap config in Arc<Mutex<>> for thread-safe sharing
    let cfg = Arc::new(Mutex::new(cfg));

    let hotkey_display = cfg.lock().unwrap().hotkey.clone();
    if cfg.lock().unwrap().openai_api_key.is_empty() {
        toast("GPTTrans", "Set OPENAI_API_KEY environment variable.");
    } else {
        toast("GPTTrans", &format!("Ready. Press {} to translate.", hotkey_display));
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
                let (api_key, model, target_lang, api_base, api_type) = {
                    let c = cfg.lock().unwrap().clone();
                    (c.openai_api_key, c.openai_model, c.target_lang, c.api_base, c.api_type)
                };
                
                // Check if API key is required (not needed for Ollama)
                if api_type != "ollama" && api_key.is_empty() {
                    toast("GPTTrans", "Missing API key. Configure in settings.");
                    logger::log("Hotkey: Missing API key");
                } else {
                    // Small delay to let the source application release the clipboard
                    // Especially important when triggered via hotkey
                    thread::sleep(Duration::from_millis(150));

                    let image = read_clipboard_image();
                    let text = read_clipboard_string();
                    
                    if image.is_none() && text.as_ref().map_or(true, |s| s.trim().is_empty()) {
                        toast("GPTTrans", "Clipboard is empty.");
                        logger::log("Hotkey: Clipboard empty");
                    } else {
                        // Show window immediately with loading indicator
                        ui::set_translating(true);
                        toast("GPTTrans", "Translating...");
                        
                        let input_text = text.unwrap_or_default();
                        let has_image = image.is_some();
                        
                        logger::log(&format!("Translating (image: {}, text len: {}) with {} ({}) to {}", 
                            has_image, input_text.len(), model, api_type, target_lang));
                        
                        let res = rt.block_on(async move {
                            // Clear text and start fresh
                            ui::show_output_text(String::new());
                            
                            translate_via_openai_stream(&input_text, image, &target_lang, &api_key, &model, &api_base, &api_type, |chunk| {
                                // Stream each chunk to the UI as it arrives
                                ui::append_text(chunk);
                            }).await
                        });
                        
                        match res {
                            Ok(out) => {
                                ui::set_translating(false);
                                let ok = write_clipboard_string(&out);
                                if ok {
                                    toast("GPTTrans", "Copied to clipboard!");
                                    logger::log("Translation success; copied to clipboard");
                                } else {
                                    toast("GPTTrans", "Translated (copy failed)");
                                    logger::log("Translation success; failed to write clipboard");
                                }
                            }
                            Err(e) => {
                                ui::set_translating(false);
                                toast("GPTTrans", &format!("Error: {}", e));
                                logger::log(&format!("Translation error: {}", e));
                                ui::show_output_text(format!("❌ Error: {}", e));
                            }
                        }
                    }
                }
            }
        });
    }

    // Run UI on main thread (blocks)
    ui::run_ui_main_thread();
}
