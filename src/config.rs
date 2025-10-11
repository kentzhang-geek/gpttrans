use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub openai_api_key: String,
    pub openai_model: String,
    pub target_lang: String,
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
}

fn default_hotkey() -> String {
    "Alt+F3".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            openai_api_key: String::new(),
            openai_model: "gpt-4o-mini".to_string(),
            target_lang: "English".to_string(),
            hotkey: default_hotkey(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
        let dir = exe.parent().unwrap_or(Path::new("."));
        dir.join("config.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        match fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str::<Config>(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        let s = serde_json::to_string_pretty(self)?;
        fs::write(path, s)?;
        Ok(())
    }
    
    /// Parse hotkey string like "Alt+F3", "Ctrl+Shift+T", etc.
    /// Returns (modifiers, vk_code) for Windows
    #[cfg(windows)]
    pub fn parse_hotkey(&self) -> Option<(u32, u32)> {
        use windows::Win32::UI::Input::KeyboardAndMouse as km;
        
        let parts: Vec<&str> = self.hotkey.split('+').map(|s| s.trim()).collect();
        if parts.is_empty() {
            return None;
        }
        
        let mut modifiers = 0u32;
        let key = parts.last()?;
        
        for part in &parts[..parts.len() - 1] {
            match part.to_uppercase().as_str() {
                "CTRL" | "CONTROL" => modifiers |= km::MOD_CONTROL.0 as u32,
                "ALT" => modifiers |= km::MOD_ALT.0 as u32,
                "SHIFT" => modifiers |= km::MOD_SHIFT.0 as u32,
                "WIN" | "WINDOWS" => modifiers |= km::MOD_WIN.0 as u32,
                _ => {}
            }
        }
        
        let vk_code = match key.to_uppercase().as_str() {
            "F1" => km::VK_F1.0 as u32,
            "F2" => km::VK_F2.0 as u32,
            "F3" => km::VK_F3.0 as u32,
            "F4" => km::VK_F4.0 as u32,
            "F5" => km::VK_F5.0 as u32,
            "F6" => km::VK_F6.0 as u32,
            "F7" => km::VK_F7.0 as u32,
            "F8" => km::VK_F8.0 as u32,
            "F9" => km::VK_F9.0 as u32,
            "F10" => km::VK_F10.0 as u32,
            "F11" => km::VK_F11.0 as u32,
            "F12" => km::VK_F12.0 as u32,
            key if key.len() == 1 => {
                let ch = key.chars().next()?;
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_uppercase() as u32
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        
        Some((modifiers, vk_code))
    }
}

