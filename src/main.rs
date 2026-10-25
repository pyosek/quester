#![windows_subsystem = "windows"]

use eframe::egui;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    env, fs,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{channel, Receiver, Sender},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
const APP_TITLE: &str = "Yarok Launcher";
const MAX_RESULTS: usize = 150;
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Clone, Serialize, Deserialize)]
struct ManifestEntry {
    kind: String,
    path: String,
    quest_app: String,
    created_at: u64,
    #[serde(default)]
    size: u64,
}

fn manifest_path() -> PathBuf {
    exe_dir().join("yarok_launcher_manifest.json")
}

fn exe_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn games_dir() -> PathBuf {
    exe_dir().join("games")
}

fn manifest_load() -> Vec<ManifestEntry> {
    fs::read_to_string(manifest_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn manifest_save(entries: &[ManifestEntry]) {
    if let Ok(s) = serde_json::to_string_pretty(entries) {
        let _ = fs::write(manifest_path(), s);
    }
}

fn manifest_append(entry: ManifestEntry) {
    let mut v = manifest_load();
    v.push(entry);
    manifest_save(&v);
}

fn now_ts() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn inside_games(p: &Path) -> bool {
    match (fs::canonicalize(p), fs::canonicalize(games_dir())) {
        (Ok(a), Ok(b)) => a.starts_with(&b),
        _ => false,
    }
}

fn rel_is_safe(rel: &str) -> bool {
    let p = Path::new(rel);
    !rel.is_empty()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

fn norm_path(s: &str) -> String {
    s.replace('/', "\\")
        .to_lowercase()
        .trim_end_matches('\\')
        .to_string()
}

fn manifest_file_entry(path: &Path) -> Option<ManifestEntry> {
    let want = norm_path(&path.to_string_lossy());
    manifest_load()
        .into_iter()
        .find(|e| e.kind == "file" && norm_path(&e.path) == want)
}

fn create_dirs_tracked(dir: &Path, quest_app: &str) {
    let mut missing: Vec<PathBuf> = vec![];
    let mut cur = Some(dir);
    while let Some(d) = cur {
        if d.exists() {
            break;
        }
        missing.push(d.to_path_buf());
        cur = d.parent();
    }
    if fs::create_dir_all(dir).is_ok() {
        for d in missing {
            manifest_append(ManifestEntry {
                kind: "dir".to_string(),
                path: d.to_string_lossy().to_string(),
                quest_app: quest_app.to_string(),
                created_at: now_ts(),
                size: 0,
            });
        }
    }
}

fn cleanup_manifest() -> (usize, usize, Vec<String>) {
    let entries = manifest_load();
    let mut cleaned = 0usize;
    let mut skipped = 0usize;
    let mut log = vec![];
    let mut remaining = vec![];
    let cur_exe_len = env::current_exe()
        .ok()
        .and_then(|p| fs::metadata(p).ok())
        .map(|m| m.len())
        .unwrap_or(0);
    let (files, mut dirs): (Vec<_>, Vec<_>) = entries.into_iter().partition(|e| e.kind == "file");

    for e in files {
        let p = PathBuf::from(&e.path);
        if !p.exists() {
            cleaned += 1;
            continue;
        }
        if !inside_games(&p) {
            skipped += 1;
            log.push(format!(
                "Skipped file (outside games folder, NOT touching it): {}",
                e.path
            ));
            continue;
        }
        let len = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        let ours = e.size == 0 || len == e.size || len == cur_exe_len;
        if !ours {
            skipped += 1;
            log.push(format!(
                "Skipped file (changed since creation, NOT touching it): {}",
                e.path
            ));
            continue;
        }
        match fs::remove_file(&p) {
            Ok(_) => {
                cleaned += 1;
                log.push(format!("Removed file: {}", e.path));
            }
            Err(err) => {
                skipped += 1;
                log.push(format!("Could not remove file {}: {err}", e.path));
                remaining.push(e);
            }
        }
    }

    dirs.sort_by_key(|e| std::cmp::Reverse(PathBuf::from(&e.path).components().count()));

    for e in dirs {
        let p = PathBuf::from(&e.path);
        if !p.exists() {
            cleaned += 1;
            continue;
        }
        if !inside_games(&p) {
            skipped += 1;
            log.push(format!(
                "Skipped folder (outside games folder, NOT touching it): {}",
                e.path
            ));
            continue;
        }
        let is_empty = fs::read_dir(&p)
            .map(|mut it| it.next().is_none())
            .unwrap_or(false);
        if !is_empty {
            skipped += 1;
            log.push(format!("Skipped folder (not empty, leaving it): {}", e.path));
            remaining.push(e);
            continue;
        }
        match fs::remove_dir(&p) {
            Ok(_) => {
                cleaned += 1;
                log.push(format!("Removed folder: {}", e.path));
            }
            Err(err) => {
                skipped += 1;
                log.push(format!("Could not remove folder {}: {err}", e.path));
                remaining.push(e);
            }
        }
    }

    manifest_save(&remaining);
    (cleaned, skipped, log)
}

#[derive(Clone, Serialize, Deserialize)]
struct RunningEntry {
    pid: u32,
    app_name: String,
    start_ts: u64,
}

fn running_path() -> PathBuf {
    exe_dir().join("yarok_running.json")
}

fn running_load() -> Vec<RunningEntry> {
    fs::read_to_string(running_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn running_save(entries: &[RunningEntry]) {
    if let Ok(s) = serde_json::to_string_pretty(entries) {
        let _ = fs::write(running_path(), s);
    }
}

fn pid_alive(pid: u32) -> bool {
    let out = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match out {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout);
            s.lines().any(|l| l.trim_start().starts_with('"'))
        }
        Err(_) => false,
    }
}

fn pid_is_ours(pid: u32) -> bool {
    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("(Get-Process -Id {pid} -ErrorAction SilentlyContinue).Path"),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let path = match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => return false,
    };
    if path.is_empty() {
        return false;
    }
    match (fs::canonicalize(&path), fs::canonicalize(games_dir())) {
        (Ok(p), Ok(g)) => p.starts_with(&g),
        _ => false,
    }
}

fn kill_pid(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

#[derive(Clone, Serialize, Deserialize)]
struct Game {
    name: String,
    exes: Vec<String>,
}

struct RunningGame {
    app_name: String,
    start_ts: u64,
    pid: u32,
}

enum Msg {
    DiscordGames(Vec<Game>),
    Log(String),
}

fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    let primary = egui::Color32::from_rgb(0x4E, 0x4B, 0x58);
    let surface = egui::Color32::from_rgb(0x1C, 0x1B, 0x1F);
    let surface_variant = egui::Color32::from_rgb(0x2B, 0x28, 0x30);
    let white = egui::Color32::WHITE;

    visuals.panel_fill = surface;
    visuals.window_fill = surface;
    visuals.extreme_bg_color = surface_variant;
    visuals.faint_bg_color = surface_variant;
    visuals.override_text_color = Some(white);
    visuals.selection.bg_fill = primary;
    visuals.selection.stroke.color = white;
    visuals.hyperlink_color = white;

    visuals.widgets.noninteractive.bg_fill = surface;
    visuals.widgets.noninteractive.fg_stroke.color = white;
    visuals.widgets.noninteractive.rounding = egui::Rounding::same(14.0);

    visuals.widgets.inactive.bg_fill = surface_variant;
    visuals.widgets.inactive.fg_stroke.color = white;
    visuals.widgets.inactive.rounding = egui::Rounding::same(14.0);

    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(0x65, 0x60, 0x75);
    visuals.widgets.hovered.fg_stroke.color = white;
    visuals.widgets.hovered.rounding = egui::Rounding::same(14.0);

    visuals.widgets.active.bg_fill = egui::Color32::from_rgb(0x38, 0x35, 0x42);
    visuals.widgets.active.fg_stroke.color = white;
    visuals.widgets.active.rounding = egui::Rounding::same(14.0);

    visuals.widgets.open.fg_stroke.color = white;
    visuals.widgets.open.rounding = egui::Rounding::same(14.0);

    visuals.window_rounding = egui::Rounding::same(20.0);
    visuals.menu_rounding = egui::Rounding::same(14.0);

    ctx.set_visuals(visuals);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(10.0, 10.0);
        s.spacing.button_padding = egui::vec2(14.0, 8.0);
        s.spacing.window_margin = egui::Margin::same(16.0);
    });
}

fn exes_for_app(app: &Value) -> Vec<String> {
    let list = match app["executables"].as_array() {
        Some(l) => l,
        None => return vec![],
    };
    let mut c: Vec<(bool, String)> = list
        .iter()
        .filter(|e| e["os"] == "win32")
        .filter_map(|e| {
            let n = e["name"]
                .as_str()?
                .trim_start_matches('>')
                .trim_start_matches(['/', '\\'])
                .to_string();
            if n.is_empty() || n.contains("..") || n.contains(':') {
                return None;
            }
            Some((e["is_launcher"] == true, n))
        })
        .collect();
    c.sort();
    c.dedup_by(|a, b| a.1 == b.1);
    let non: Vec<String> = c.iter().filter(|x| !x.0).map(|x| x.1.clone()).collect();
    (if non.is_empty() {
        c.into_iter().map(|x| x.1).collect()
    } else {
        non
    })
    .into_iter()
    .take(3)
    .collect()
}

fn build_discord_games(apps: &[Value]) -> Vec<Game> {
    let mut out: Vec<Game> = apps
        .iter()
        .filter_map(|a| {
            let name = a["name"].as_str()?.to_string();
            let exes = exes_for_app(a);
            if exes.is_empty() {
                return None;
            }
            Some(Game { name, exes })
        })
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

fn safe_json_parse(resp: reqwest::blocking::Response) -> Result<Value, String> {
    let status = resp.status();
    let text = resp.text().map_err(|e| format!("read body: {e}"))?;
    let trimmed = text.trim_start();

    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        let preview: String = trimmed.chars().take(160).collect();
        return Err(format!(
            "HTTP {status}, unexpected body (likely HTML/Cloudflare): {preview}"
        ));
    }
    serde_json::from_str(&text).map_err(|e| format!("json parse: {e}"))
}

struct App {
    style_set: bool,
    apps_requested: bool,

    discord_games: Vec<Game>,

    search: String,
    log: Vec<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    running: Vec<RunningGame>,
    last_check: Instant,
}

impl App {
    fn new() -> Self {
        let (tx, rx) = channel();

        let mut entries = running_load();
        entries.retain(|e| pid_is_ours(e.pid));
        running_save(&entries);
        let running = entries
            .into_iter()
            .map(|e| RunningGame {
                app_name: e.app_name,
                start_ts: e.start_ts,
                pid: e.pid,
            })
            .collect();

        Self {
            style_set: false,
            apps_requested: false,
            discord_games: vec![],
            search: String::new(),
            log: vec!["Loading game databases...".to_string()],
            tx,
            rx,
            running,
            last_check: Instant::now(),
        }
    }

    fn push_log(&mut self, s: impl Into<String>) {
        self.log.push(s.into());
        if self.log.len() > 200 {
            self.log.remove(0);
        }
    }

    fn save_running(&self) {
        let v: Vec<RunningEntry> = self
            .running
            .iter()
            .map(|g| RunningEntry {
                pid: g.pid,
                app_name: g.app_name.clone(),
                start_ts: g.start_ts,
            })
            .collect();
        running_save(&v);
    }

    fn load_games(&mut self) {
        self.push_log("Refreshing Discord DB...");
        let tx = self.tx.clone();
        thread::spawn(move || {
            let client = match reqwest::blocking::Client::builder()
                .user_agent(UA)
                .timeout(Duration::from_secs(20))
                .build()
            {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(Msg::Log(format!("Client build error: {e}")));
                    return;
                }
            };

            match client
                .get("https://discord.com/api/v9/applications/detectable")
                .send()
            {
                Ok(r) => {
                    if !r.status().is_success() {
                        let _ = tx.send(Msg::Log(format!(
                            "Discord DB HTTP error: {}",
                            r.status()
                        )));
                        return;
                    }
                    match safe_json_parse(r) {
                        Ok(val) => match val.as_array() {
                            Some(apps) => {
                                let games = build_discord_games(apps);
                                let _ = tx.send(Msg::DiscordGames(games));
                            }
                            None => {
                                let _ =
                                    tx.send(Msg::Log("Discord DB: invalid JSON".to_string()));
                            }
                        },
                        Err(e) => {
                            let _ = tx.send(Msg::Log(format!("Discord DB error: {e}")));
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Msg::Log(format!("No network (Discord): {e}")));
                }
            }
        });
    }

    fn is_running(&self, name: &str) -> bool {
        self.running.iter().any(|g| g.app_name == name)
    }

    fn run_fake_discord(&mut self, g: Game) {
        if self.is_running(&g.name) {
            return;
        }
        let me = env::current_exe().unwrap();
        let base = games_dir();
        for rel in &g.exes {
            if !rel_is_safe(rel) {
                self.push_log(format!("Refused unsafe path: {rel}"));
                continue;
            }
            let target = base.join(rel);
            let entry = manifest_file_entry(&target);

            if target.exists() {
                match &entry {
                    None => {
                        self.push_log(format!(
                            "Refused: {rel} already exists and was not created by launcher"
                        ));
                        continue;
                    }
                    Some(e) => {
                        let len = fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
                        if e.size > 0 && len != e.size {
                            self.push_log(format!(
                                "Refused: {rel} changed since creation, not overwriting"
                            ));
                            continue;
                        }
                    }
                }
            }

            if let Some(p) = target.parent() {
                create_dirs_tracked(p, &g.name);
                if !inside_games(p) {
                    self.push_log(format!("Refused: {rel} resolves outside games folder"));
                    continue;
                }
            }
            if fs::copy(&me, &target).is_err() && !target.exists() {
                self.push_log(format!("Failed to copy {rel}"));
                continue;
            }
            if entry.is_none() {
                let size = fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
                manifest_append(ManifestEntry {
                    kind: "file".to_string(),
                    path: target.to_string_lossy().to_string(),
                    quest_app: g.name.clone(),
                    created_at: now_ts(),
                    size,
                });
            }
            match Command::new(&target).arg("--sleep").arg(&g.name).spawn() {
                Ok(c) => {
                    let pid = c.id();
                    self.push_log(format!("Launched: {rel} (PID {pid})"));
                    self.running.push(RunningGame {
                        app_name: g.name.clone(),
                        start_ts: now_ts(),
                        pid,
                    });
                    self.save_running();
                }
                Err(e) => self.push_log(format!("Failed to launch {rel}: {e}")),
            }
            break;
        }
    }

    fn stop_game(&mut self, idx: usize) {
        if idx >= self.running.len() {
            return;
        }
        let g = self.running.remove(idx);
        if pid_is_ours(g.pid) {
            kill_pid(g.pid);
            self.push_log(format!("Stopped: {}", g.app_name));
        } else {
            self.push_log(format!(
                "PID {} is not our fake game anymore, not killing it: {}",
                g.pid, g.app_name
            ));
        }
        self.save_running();
    }

    fn recheck_running(&mut self, ctx: &egui::Context) {
        if self.running.is_empty() || self.last_check.elapsed() < Duration::from_secs(3) {
            return;
        }
        self.last_check = Instant::now();
        let before = self.running.len();
        self.running.retain(|g| pid_alive(g.pid));
        if self.running.len() != before {
            self.push_log("Some games were closed — removed from list.");
            self.save_running();
        }
        ctx.request_repaint();
    }

}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if !self.style_set {
            apply_theme(ctx);
            self.style_set = true;
        }
        if !self.apps_requested {
            self.apps_requested = true;
            self.load_games();
        }

        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::DiscordGames(g) => {
                    self.push_log(format!("Discord DB loaded: {} games", g.len()));
                    self.discord_games = g;
                }
                Msg::Log(s) => self.push_log(s),
            }
        }

        self.recheck_running(ctx);
        ctx.request_repaint_after(Duration::from_millis(300));

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().button_padding = egui::vec2(8.0, 4.0);
                ui.spacing_mut().item_spacing.x = 6.0;
                let h = ui.heading(APP_TITLE).rect.height();
                let size = egui::vec2(0.0, h);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::Button::new("Refresh").min_size(size))
                        .on_hover_text("Refresh Discord DB")
                        .clicked()
                    {
                        self.load_games();
                    }
                    if ui
                        .add(egui::Button::new("Clean").min_size(size))
                        .on_hover_text("Clean games")
                        .clicked()
                    {
                        let (cleaned, skipped, lines) = cleanup_manifest();
                        for l in lines {
                            self.push_log(l);
                        }
                        self.push_log(format!("Cleanup: removed {cleaned}, skipped {skipped}"));
                    }
                });
            });

            ui.add_space(6.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("search game...")
                    .desired_width(f32::INFINITY),
            );

            if !self.running.is_empty() {
                ui.add_space(6.0);
                let mut to_stop = None;
                egui::ScrollArea::vertical()
                    .id_salt("running_scroll")
                    .max_height(130.0)
                    .show(ui, |ui| {
                        for (i, g) in self.running.iter().enumerate() {
                            let m = (now_ts().saturating_sub(g.start_ts)) / 60;
                            egui::Frame::none()
                                .fill(ui.visuals().widgets.inactive.bg_fill)
                                .rounding(egui::Rounding::same(14.0))
                                .inner_margin(egui::Margin::symmetric(12.0, 6.0))
                                .show(ui, |ui| {
                                    egui::Sides::new().show(
                                        ui,
                                        |ui| {
                                            ui.add(
                                                egui::Label::new(format!(
                                                    "▶ {} — {} min",
                                                    g.app_name, m
                                                ))
                                                .truncate(),
                                            );
                                        },
                                        |ui| {
                                            if ui.button("Stop").clicked() {
                                                to_stop = Some(i);
                                            }
                                        },
                                    );
                                });
                            ui.add_space(3.0);
                        }
                    });
                if let Some(i) = to_stop {
                    self.stop_game(i);
                }
            }

            ui.add_space(4.0);
        });

        egui::TopBottomPanel::bottom("log")
            .min_height(110.0)
            .show(ctx, |ui| {
                ui.label("Log:");
                egui::ScrollArea::vertical()
                    .id_salt("log_scroll")
                    .auto_shrink([false, false])
                    .max_height(90.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for l in &self.log {
                            ui.small(l);
                        }
                    });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let search = self.search.to_lowercase();

            let filtered: Vec<Game> = self
                .discord_games
                .iter()
                .filter(|g| search.is_empty() || g.name.to_lowercase().contains(&search))
                .take(MAX_RESULTS)
                .cloned()
                .collect();

            if filtered.len() == MAX_RESULTS {
                ui.small(format!("Showing first {MAX_RESULTS}, refine your search"));
            }

            egui::ScrollArea::vertical()
                .id_salt("discord_games_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if self.discord_games.is_empty() {
                        ui.small("Loading Discord DB...");
                    }
                    for g in filtered {
                        let already = self.is_running(&g.name);
                        egui::Frame::none()
                            .fill(ui.visuals().extreme_bg_color)
                            .rounding(egui::Rounding::same(14.0))
                            .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                            .show(ui, |ui| {
                                egui::Sides::new().show(
                                    ui,
                                    |ui| {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&g.name).strong(),
                                            )
                                            .truncate(),
                                        );
                                    },
                                    |ui| {
                                        let btn = egui::Button::new(if already {
                                            "Running"
                                        } else {
                                            "Play"
                                        });
                                        if ui.add_enabled(!already, btn).clicked() {
                                            self.run_fake_discord(g.clone());
                                        }
                                    },
                                );
                            });
                        ui.add_space(4.0);
                    }
                });
        });
    }
}

struct FakeWindowApp {
    title: String,
    style_set: bool,
}

impl eframe::App for FakeWindowApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if !self.style_set {
            apply_theme(ctx);
            self.style_set = true;
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(30.0);
                ui.heading(&self.title);
                ui.label("keep this window open, don't close it");
            });
        });
        ctx.request_repaint_after(Duration::from_secs(30));
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.get(1).map(String::as_str) == Some("--sleep") {
        let title = if args.len() > 2 {
            args[2..].join(" ")
        } else {
            "Game".to_string()
        };
        let window_title = title.clone();
        let options = eframe::NativeOptions {
            renderer: eframe::Renderer::Glow,
            multisampling: 0,
            depth_buffer: 0,
            stencil_buffer: 0,
            vsync: true,
            persist_window: false,
            viewport: egui::ViewportBuilder::default()
                .with_title(&window_title)
                .with_inner_size([360.0, 200.0])
                .with_resizable(false),
            ..Default::default()
        };
        eframe::run_native(
            &window_title,
            options,
            Box::new(move |_| Ok(Box::new(FakeWindowApp { title, style_set: false }))),
        )
        .unwrap();
        return;
    }

    eframe::run_native(
        APP_TITLE,
        eframe::NativeOptions {
            renderer: eframe::Renderer::Glow,
            persist_window: false,
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([520.0, 640.0])
                .with_min_inner_size([340.0, 480.0]),
            ..Default::default()
        },
        Box::new(|_| Ok(Box::new(App::new()))),
    )
    .unwrap();
}