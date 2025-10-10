use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub openai_api_key: String,
    pub openai_model: String,
    pub target_lang: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            openai_api_key: String::new(),
            openai_model: "gpt-4o-mini".to_string(),
            target_lang: "English".to_string(),
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
}

