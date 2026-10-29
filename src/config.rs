use crate::discord::Game;
use eframe::egui;
use std::{
    env, fs,
    path::PathBuf,
    sync::mpsc::Sender,
    time::{SystemTime, UNIX_EPOCH},
};

pub const APP_NAME: &str = "Quester";

pub const VERSION: &str = "v1.1.0";

pub const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
pub const DISCORD_DB_URL: &str = "https://discord.com/api/v9/applications/detectable";
pub const GITHUB_DB_URL: &str =
    "https://raw.githubusercontent.com/pyosek/quester/refs/heads/main/custom_db.json";

pub fn exe_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn games_dir() -> PathBuf {
    exe_dir().join("games")
}

#[link(name = "shell32")]
extern "system" {
    fn SetCurrentProcessExplicitAppUserModelID(app_id: *const u16) -> i32;
}
pub fn set_app_id() {
    let id: Vec<u16> = "Quester.App".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        SetCurrentProcessExplicitAppUserModelID(id.as_ptr());
    }
}

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

pub enum Msg {
    DiscordGames(Vec<Game>),
    GithubGames(Vec<Game>),
    Alive(Option<Vec<u32>>),
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