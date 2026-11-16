use crate::discord::Game;
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::PathBuf,
    sync::mpsc::Sender,
    time::{SystemTime, UNIX_EPOCH},
};

pub const APP_NAME: &str = "Quester";

pub const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

pub const RELEASES_API: &str = "https://api.github.com/repos/pyosek/quester/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/pyosek/quester/releases";


#[cfg(windows)]
pub const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
#[cfg(not(windows))]
pub const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
pub const DISCORD_DB_URL: &str = "https://discord.com/api/v9/applications/detectable";

pub fn exe_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn games_dir() -> PathBuf {
    exe_dir().join("games")
}

#[cfg(windows)]
#[link(name = "shell32")]
extern "system" {
    fn SetCurrentProcessExplicitAppUserModelID(app_id: *const u16) -> i32;
}

#[cfg(windows)]
pub fn set_app_id() {
    let id: Vec<u16> = "Quester.App"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        SetCurrentProcessExplicitAppUserModelID(id.as_ptr());
    }
}

#[cfg(not(windows))]
pub fn set_app_id() {}

pub fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn install_crash_log() {
    std::panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let text = format!("[{}] {info}\n{bt}\n\n", now_ts());
        let path = exe_dir().join("quester_crash.log");
        let old = fs::read_to_string(&path).unwrap_or_default();
        let _ = fs::write(path, format!("{old}{text}"));
    }));
}

pub fn log_crash(msg: &str) {
    let path = exe_dir().join("quester_crash.log");
    let old = fs::read_to_string(&path).unwrap_or_default();
    let _ = fs::write(path, format!("{old}[{}] {msg}\n\n", now_ts()));
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub lite: bool,
    pub hide_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lite: true,
            hide_updates: false,
        }
    }
}

fn settings_path() -> PathBuf {
    exe_dir().join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        fs::read_to_string(settings_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = fs::write(settings_path(), s);
        }
    }
}

pub enum Msg {
    UpdateChecked(Result<Option<(String, String)>, String>),
    DiscordGames(Vec<Game>),
    Alive(Option<Vec<u32>>),
    Fonts(Box<egui::FontDefinitions>),
    Log(String),
}

#[derive(Clone)]
pub struct Notifier {
    tx: Sender<Msg>,
    ctx: egui::Context,
}

impl Notifier {
    pub fn new(tx: Sender<Msg>, ctx: egui::Context) -> Self {
        Self { tx, ctx }
    }

    pub fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
        self.ctx.request_repaint();
    }
}