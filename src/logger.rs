use once_cell::sync::Lazy;
use std::fs::{OpenOptions, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_FILE: Lazy<Mutex<Option<File>>> = Lazy::new(|| Mutex::new(None));

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn init() {
    let path = exe_dir().join("log.txt");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "===== GPTTrans start =====");
        let mut guard = LOG_FILE.lock().unwrap();
        *guard = Some(f);
    }
}

fn ts() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    format!("{}.{:03}", now.as_secs(), now.subsec_millis())
}

pub fn log(msg: &str) {
    if let Ok(mut guard) = LOG_FILE.lock() {
        if let Some(f) = guard.as_mut() {
            let _ = writeln!(f, "[{}] {}", ts(), msg);
            let _ = f.flush();
            return;
        }
    }
    // Fallback: try to open lazily if init wasn't called yet
    let path = exe_dir().join("log.txt");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{}] {}", ts(), msg);
        let _ = f.flush();
    }
}
