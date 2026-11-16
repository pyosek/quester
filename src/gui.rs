use crate::{
    badge::{
        self, BadgeCfg, BadgeShared, HoursCfg, HOURS_MAX, HOURS_MAX_TIME, HOURS_MAX_UNLIMITED,
        MODE_GAMES, MODE_HOURS,
    },
    config::{
        exe_dir, games_dir, log_crash, now_ts, Msg, Notifier, Settings, APP_NAME, VERSION,
    },
    discord::{self, Game},
    update,
    process::{
        alive_of, ensure_games_marker, is_tiling_wm, kill_pid, launch_fake,
        pid_is_ours, running_load, running_replace_normal, running_save, spawn_orphan_cleanup,
        RunningEntry,
    },
};
use eframe::egui::{self, Color32, Margin, Rounding, Stroke};
use std::{
    fs,
    sync::{
        atomic::Ordering,
        mpsc::{channel, Receiver},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const LOGO_SMALL: &[u8] = include_bytes!("../assets/logo_64.png");
const LOGO_FULL: &[u8] = include_bytes!("../assets/logo.png");

const RAM_PER_GAME_MB: f32 = 0.1;
const RAM_PER_HOURS_MB: f32 = 0.1;

fn ram_label(count: usize, per_mb: f32) -> String {
    let mb = count as f32 * per_mb;
    if mb >= 1024.0 {
        format!("~{:.1} GB RAM", mb / 1024.0)
    } else {
        format!("~{:.0} MB RAM", mb)
    }
}

const ROW_CONTENT_H: f32 = 36.0;
const ROW_H: f32 = ROW_CONTENT_H + 12.0;

const BG: Color32 = Color32::from_rgb(0x08, 0x09, 0x0a);
const CARD: Color32 = Color32::from_rgb(0x0a, 0x0a, 0x0b);
const ROW: Color32 = Color32::from_rgb(0x15, 0x15, 0x17);
const BORDER: Color32 = Color32::from_rgb(150, 150, 158);
const BORDER_DIM: Color32 = Color32::from_rgb(60, 60, 66);
const TEXT: Color32 = Color32::from_rgb(230, 230, 234);
const DIM: Color32 = Color32::from_rgb(110, 110, 118);
const GREEN: Color32 = Color32::from_rgb(0x4a, 0xde, 0x80);
const RED: Color32 = Color32::from_rgb(0xff, 0x4d, 0x4d);

fn fonts_base() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();

    if let Some(data) = map_font(&exe_dir().join("Skranji-Regular.ttf"), 0) {
        fonts.font_data.insert("skranji".to_owned(), data);
        if let Some(f) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            f.insert(0, "skranji".to_owned());
        }
    }

    fonts
}

fn add_system_fonts(fonts: &mut egui::FontDefinitions) {
    for (key, path, index) in system_fonts() {
        push_font(fonts, &key, &path, index);
    }
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = fonts_base();
    add_system_fonts(&mut fonts);
    ctx.set_fonts(fonts);
}

fn spawn_fonts(n: &Notifier) {
    let n = n.clone();
    thread::spawn(move || {
        let mut fonts = fonts_base();
        add_system_fonts(&mut fonts);
        n.send(Msg::Fonts(Box::new(fonts)));
    });
}

fn map_font(path: &std::path::Path, index: u32) -> Option<egui::FontData> {
    let file = fs::File::open(path).ok()?;
    let map = unsafe { memmap2::Mmap::map(&file).ok()? };
    let bytes: &'static [u8] = Box::leak(Box::new(map));
    let mut data = egui::FontData::from_static(bytes);
    data.index = index;
    Some(data)
}

fn push_font(fonts: &mut egui::FontDefinitions, key: &str, path: &std::path::Path, index: u32) {
    if fonts.font_data.contains_key(key) {
        return;
    }
    let Some(data) = map_font(path, index) else {
        return;
    };
    fonts.font_data.insert(key.to_owned(), data);
    if let Some(f) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        f.push(key.to_owned());
    }
}

#[cfg(windows)]
fn system_fonts() -> Vec<(String, std::path::PathBuf, u32)> {
    let win = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
    let dir = std::path::Path::new(&win).join("Fonts");
    [
        "msyh.ttc",
        "msjh.ttc",
        "YuGothM.ttc",
        "meiryo.ttc",
        "malgun.ttf",
        "seguisym.ttf",
        "segoeui.ttf",
    ]
    .iter()
    .map(|f| (format!("sys_{f}"), dir.join(f), 0))
    .collect()
}

#[cfg(not(windows))]
fn fc_match(query: &str) -> Option<(std::path::PathBuf, u32)> {
    let out = std::process::Command::new("fc-match")
        .args(["-f", "%{file}|%{index}", query])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let (path, index) = text.rsplit_once('|')?;
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    let index = index.trim().parse::<u32>().unwrap_or(0) & 0xFFFF;
    Some((std::path::PathBuf::from(path), index))
}

#[cfg(not(windows))]
fn system_fonts() -> Vec<(String, std::path::PathBuf, u32)> {
    const STATIC: &[&str] = &[
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/OTF/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        "/usr/share/fonts/truetype/arphic/uming.ttc",
    ];
    let mut out: Vec<(String, std::path::PathBuf, u32)> = Vec::new();
    for (i, query) in [
        "sans-serif",
        ":lang=ja",
        ":lang=zh-cn",
        ":lang=zh-tw",
        ":lang=ko",
    ]
    .iter()
    .enumerate()
    {
        if let Some((path, index)) = fc_match(query) {
            if !out.iter().any(|(_, p, x)| *p == path && *x == index) {
                out.push((format!("sys_fc{i}"), path, index));
            }
        }
    }
    for (i, path) in STATIC.iter().enumerate() {
        let path = std::path::PathBuf::from(path);
        if path.is_file() && !out.iter().any(|(_, p, _)| *p == path) {
            out.push((format!("sys_static{i}"), path, 0));
        }
    }
    out
}

fn open_path(path: &std::path::Path) -> std::io::Result<std::process::Child> {
    #[cfg(windows)]
    {
        std::process::Command::new("explorer").arg(path).spawn()
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(path).spawn()
    }
}

fn open_url(url: &str) -> std::io::Result<std::process::Child> {
    open_path(std::path::Path::new(url))
}

fn apply_theme(ctx: &egui::Context) {
    ctx.set_fonts(fonts_base());
    ctx.options_mut(|o| o.zoom_with_keyboard = false);

    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = CARD;
    v.extreme_bg_color = CARD;
    v.faint_bg_color = ROW;
    v.override_text_color = None;
    v.hyperlink_color = TEXT;
    v.selection.bg_fill = Color32::from_rgb(0x4E, 0x4B, 0x58);
    v.selection.stroke = Stroke::new(1.0_f32, TEXT);

    let r = Rounding::same(6.0);
    v.widgets.noninteractive.bg_fill = CARD;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER_DIM);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.noninteractive.rounding = r;

    v.widgets.inactive.bg_fill = ROW;
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER_DIM);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.inactive.rounding = r;

    v.widgets.hovered.bg_fill = ROW;
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(30, 30, 34);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    v.widgets.hovered.rounding = r;

    v.widgets.active.bg_fill = ROW;
    v.widgets.active.weak_bg_fill = Color32::from_rgb(40, 40, 46);
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    v.widgets.active.rounding = r;

    v.widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.open.rounding = r;

    v.window_rounding = Rounding::same(20.0);
    v.menu_rounding = Rounding::same(10.0);

    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.animation_time = 0.0;
        s.spacing.item_spacing = egui::vec2(10.0, 8.0);
        s.spacing.button_padding = egui::vec2(8.0, 2.0);
        s.text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(24.0));
        s.text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        s.text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
    });
}

fn load_logo(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let icon = eframe::icon_data::from_png_bytes(LOGO_SMALL).ok()?;
    let img = egui::ColorImage::from_rgba_unmultiplied(
        [icon.width as usize, icon.height as usize],
        &icon.rgba,
    );
    Some(ctx.load_texture("quester_logo", img, egui::TextureOptions::LINEAR))
}

#[cfg(windows)]
fn hide_main_from_taskbar() {
    #[link(name = "user32")]
    extern "system" {
        fn FindWindowW(class: *const u16, title: *const u16) -> isize;
        fn ShowWindow(h: isize, cmd: i32) -> i32;
        #[cfg_attr(target_pointer_width = "64", link_name = "GetWindowLongPtrW")]
        #[cfg_attr(target_pointer_width = "32", link_name = "GetWindowLongW")]
        fn GetWindowLongPtr(h: isize, idx: i32) -> isize;
        #[cfg_attr(target_pointer_width = "64", link_name = "SetWindowLongPtrW")]
        #[cfg_attr(target_pointer_width = "32", link_name = "SetWindowLongW")]
        fn SetWindowLongPtr(h: isize, idx: i32, v: isize) -> isize;
    }
    let title: Vec<u16> = APP_NAME.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if hwnd == 0 {
            return;
        }
        const GWL_EXSTYLE: i32 = -20;
        const WS_EX_TOOLWINDOW: isize = 0x80;
        const WS_EX_APPWINDOW: isize = 0x0004_0000;
        let ex = GetWindowLongPtr(hwnd, GWL_EXSTYLE);
        ShowWindow(hwnd, 0);
        SetWindowLongPtr(hwnd, GWL_EXSTYLE, (ex | WS_EX_TOOLWINDOW) & !WS_EX_APPWINDOW);
        ShowWindow(hwnd, 4);
    }
}

fn icon_cached() -> Option<egui::IconData> {
    static ICON: std::sync::OnceLock<Option<egui::IconData>> = std::sync::OnceLock::new();
    ICON.get_or_init(|| eframe::icon_data::from_png_bytes(LOGO_FULL).ok())
        .clone()
}

fn window_viewport(w: f32, h: f32) -> egui::ViewportBuilder {
    let mut vb = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_inner_size([w, h]);
    if let Some(icon) = icon_cached() {
        vb = vb.with_icon(icon);
    }
    vb
}

fn tab_button(ui: &mut egui::Ui, text: &str, selected: bool) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).size(13.5))
            .selected(selected)
            .min_size(egui::vec2(80.0, 0.0)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn pill(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(text).size(13.0)).min_size(egui::vec2(46.0, 0.0)))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn card_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(2.0_f32, BORDER))
        .rounding(Rounding::same(20.0))
        .inner_margin(Margin::same(12.0))
}

fn fake_card_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(2.0_f32, BORDER))
        .rounding(Rounding::same(16.0))
        .inner_margin(Margin::same(12.0))
}

fn row_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(ROW)
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::symmetric(12.0, 6.0))
}

fn name_label(ui: &mut egui::Ui, name: &str) {
    ui.add(
        egui::Label::new(egui::RichText::new(name).size(13.5).color(TEXT))
            .truncate(),
    )
    .on_hover_text(name);
}

fn list_row(ui: &mut egui::Ui, name: &str, action: Option<&str>) -> bool {
    let mut clicked = false;
    row_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(ROW_CONTENT_H);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            match action {
                Some(a) => {
                    if pill(ui, a).clicked() {
                        clicked = true;
                    }
                }
                None => {
                    ui.label(egui::RichText::new("running").color(DIM));
                }
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                name_label(ui, name);
            });
        });
    });
    clicked
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    DiscordDb,
    Settings,
}

#[derive(Clone, Copy, PartialEq)]
enum FarmTab {
    Games,
    Hours,
}

enum FarmAction {
    None,
    StartGames,
    StartHours,
    Stop,
    ClearHistory,
}

fn fmt_left(secs: u64) -> String {
    let (h, m) = (secs / 3600, (secs % 3600) / 60);
    if h > 0 {
        format!("{h}h {m}m left")
    } else if secs >= 60 {
        format!("{m}m left")
    } else {
        format!("{secs}s left")
    }
}

struct RunningGame {
    app_name: String,
    start_ts: u64,
    pid: u32,
}

struct App {
    style_set: bool,
    apps_requested: bool,
    logo: Option<egui::TextureHandle>,

    discord_games: Vec<Game>,
    tab: Tab,

    search: String,
    filtered: Vec<usize>,
    last_search: String,
    last_tab: Tab,
    filter_dirty: bool,

    log: Vec<String>,
    notifier: Notifier,
    rx: Receiver<Msg>,

    running: Vec<RunningGame>,
    last_check: Instant,
    alive_inflight: bool,

    show_logs: bool,
    show_running: bool,

    show_badge: bool,
    badge_unlocked: bool,
    badge_ack: bool,
    badge_warn_since: Option<Instant>,
    badge_cfg: BadgeCfg,
    hours_cfg: HoursCfg,
    farm_tab: FarmTab,
    badge_shared: Arc<BadgeShared>,
    badge_handle: Option<JoinHandle<()>>,

    settings: Settings,
    update: Option<(String, String)>,
    update_status: String,
    update_inflight: bool,

    main_hidden: bool,
}

impl Drop for App {
    fn drop(&mut self) {
        self.badge_shared.cancel.store(true, Ordering::Relaxed);
        if let Some(h) = self.badge_handle.take() {
            let _ = h.join();
        }
        if is_tiling_wm() {
            for g in &self.running {
                if pid_is_ours(g.pid) {
                    kill_pid(g.pid);
                }
            }
            running_replace_normal(&[]);
        }
    }
}

impl App {
    fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        ensure_games_marker(false);

        let launch_ts = now_ts();
        let mut entries = running_load();
        let entry_pids: Vec<u32> = entries.iter().map(|e| e.pid).collect();
        if let Some(alive) = alive_of(&entry_pids) {
            entries.retain(|e| alive.contains(&e.pid));
        }
        entries.retain(|e| e.hours || pid_is_ours(e.pid));
        running_save(&entries);
        spawn_orphan_cleanup(launch_ts);
        let running = entries
            .into_iter()
            .filter(|e| !e.hours)
            .map(|e| RunningGame {
                app_name: e.app_name,
                start_ts: e.start_ts,
                pid: e.pid,
            })
            .collect();

        Self {
            style_set: false,
            apps_requested: false,
            logo: None,
            discord_games: vec![],
            tab: Tab::DiscordDb,
            search: String::new(),
            filtered: vec![],
            last_search: String::new(),
            last_tab: Tab::DiscordDb,
            filter_dirty: true,
            log: vec!["Loading game databases...".to_string()],
            notifier: Notifier::new(tx, ctx),
            rx,
            running,
            last_check: Instant::now(),
            alive_inflight: false,
            show_logs: false,
            show_running: false,
            show_badge: false,
            badge_unlocked: false,
            badge_ack: false,
            badge_warn_since: None,
            badge_cfg: BadgeCfg::default(),
            hours_cfg: HoursCfg::default(),
            farm_tab: FarmTab::Games,
            badge_shared: Arc::new(BadgeShared::default()),
            badge_handle: None,
            settings: Settings::load(),
            update: None,
            update_status: String::new(),
            update_inflight: false,
            main_hidden: false,
        }
    }

    fn check_updates(&mut self) {
        if self.settings.hide_updates || self.update_inflight {
            return;
        }
        self.update_inflight = true;
        self.update_status = "Checking...".to_string();
        update::spawn_check(&self.notifier);
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
                hours: false,
            })
            .collect();
        running_replace_normal(&v);
    }

    fn load_games(&mut self) {
        self.push_log("Refreshing Discord DB...");
        discord::spawn_refresh(&self.notifier);
    }

    fn is_running(&self, name: &str) -> bool {
        self.running.iter().any(|g| g.app_name == name)
    }

    fn window_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.main_hidden && self.show_badge {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                -32000.0, -32000.0,
            )));
            #[cfg(windows)]
            hide_main_from_taskbar();
            self.main_hidden = true;
        }
        if self.main_hidden && !self.show_badge {
            self.main_hidden = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn run_fake_discord(&mut self, g: Game) {
        if self.is_running(&g.name) {
            return;
        }
        let (pid, lines) = launch_fake(&g, self.settings.lite);
        for l in lines {
            self.push_log(l);
        }
        if let Some(pid) = pid {
            self.running.push(RunningGame {
                app_name: g.name.clone(),
                start_ts: now_ts(),
                pid,
            });
            self.save_running();
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

    fn poll_alive(&mut self) {
        if self.running.is_empty()
            || self.alive_inflight
            || self.last_check.elapsed() < Duration::from_secs(5)
        {
            return;
        }
        self.last_check = Instant::now();
        self.alive_inflight = true;
        let n = self.notifier.clone();
        let pids: Vec<u32> = self.running.iter().map(|g| g.pid).collect();
        thread::spawn(move || n.send(Msg::Alive(alive_of(&pids))));
    }

    fn drain_messages(&mut self, ctx: &egui::Context) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::DiscordGames(g) => {
                    self.push_log(format!("Discord DB loaded: {} games", g.len()));
                    self.discord_games = g;
                    self.filter_dirty = true;
                }
                Msg::Alive(list) => {
                    self.alive_inflight = false;
                    if let Some(alive) = list {
                        let before = self.running.len();
                        self.running.retain(|g| alive.contains(&g.pid));
                        if self.running.len() != before {
                            self.push_log("Some games were closed — removed from list.");
                            self.save_running();
                        }
                    }
                }
                Msg::UpdateChecked(r) => {
                    self.update_inflight = false;
                    if self.settings.hide_updates {
                        continue;
                    }
                    match r {
                        Ok(Some((tag, url))) => {
                            self.push_log(format!("New version available: {tag}"));
                            self.update_status = format!("New version: {tag}");
                            self.update = Some((tag, url));
                        }
                        Ok(None) => {
                            self.update_status = "You have the latest version".to_string();
                        }
                        Err(e) => {
                            self.update_status = "Check failed".to_string();
                            self.push_log(format!("Update check: {e}"));
                        }
                    }
                }
                Msg::Fonts(f) => ctx.set_fonts(*f),
                Msg::Log(s) => self.push_log(s),
            }
        }
    }

    fn refilter_if_needed(&mut self) {
        if !self.filter_dirty && self.search == self.last_search && self.tab == self.last_tab {
            return;
        }
        if self.tab == Tab::Settings {
            self.last_tab = self.tab;
            self.filter_dirty = false;
            return;
        }
        let q = self.search.to_lowercase();
        self.filtered = self
            .discord_games
            .iter()
            .enumerate()
            .filter(|(_, g)| q.is_empty() || g.lname.contains(&q))
            .map(|(i, _)| i)
            .collect();
        self.last_search = self.search.clone();
        self.last_tab = self.tab;
        self.filter_dirty = false;
    }

    fn ui_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(t) = &self.logo {
                ui.image((t.id(), egui::vec2(38.0, 38.0)));
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.heading(egui::RichText::new(APP_NAME).strong());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(egui::RichText::new(VERSION).small().color(DIM));
                    let shown = if self.settings.hide_updates {
                        None
                    } else {
                        self.update.clone()
                    };
                    if let Some((tag, url)) = shown {
                        let r = ui
                            .add(
                                egui::Label::new(
                                    egui::RichText::new(format!("new update {tag}"))
                                        .small()
                                        .color(GREEN),
                                )
                                .sense(egui::Sense::click()),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text("open release page");
                        if r.clicked() {
                            if let Err(e) = open_url(&url) {
                                self.push_log(format!("Could not open link: {e}"));
                            }
                        }
                    }
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if pill(ui, "refresh").clicked() {
                    self.load_games();
                }
                if pill(ui, "folder").clicked() {
                    let dir = games_dir();
                    let _ = fs::create_dir_all(&dir);
                    if let Err(e) = open_path(&dir) {
                        self.push_log(format!("Could not open folder: {e}"));
                    }
                }
            });
        });

        ui.horizontal_wrapped(|ui| {
            let d = format!("Discord ({})", self.discord_games.len());
            let st = "settings".to_string();
            for (tab, label) in [(Tab::DiscordDb, d), (Tab::Settings, st)] {
                if tab_button(ui, &label, self.tab == tab).clicked() {
                    self.tab = tab;
                }
            }
        });

        if self.tab != Tab::Settings {
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("search game...")
                    .desired_width(f32::INFINITY)
                    .margin(egui::vec2(10.0, 6.0)),
            );
        }
    }

    fn ui_settings(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        changed |= ui
            .checkbox(&mut self.settings.lite, "lite windows")
            .changed();
        ui.label(
            egui::RichText::new(
                "Games you run yourself use a tiny window and much less RAM. \
                 Farming always uses hidden lite windows, regardless of this option.",
            )
            .small()
            .color(DIM),
        );
        ui.add_space(8.0);
        let hide = ui
            .checkbox(&mut self.settings.hide_updates, "hide updates")
            .changed();
        ui.label(
            egui::RichText::new(
                "When on, Quester doesn't check for new versions and sends no request.",
            )
            .small()
            .color(DIM),
        );
        if hide {
            changed = true;
            if self.settings.hide_updates {
                self.update = None;
                self.update_status.clear();
            } else {
                self.check_updates();
            }
        }
        if !self.settings.hide_updates {
            ui.horizontal(|ui| {
                if pill(ui, "check now").clicked() {
                    self.check_updates();
                }
                if !self.update_status.is_empty() {
                    let color = if self.update.is_some() { GREEN } else { DIM };
                    ui.label(egui::RichText::new(&self.update_status).small().color(color));
                }
            });
            if let Some((_, url)) = self.update.clone() {
                if pill(ui, "open release page").clicked() {
                    if let Err(e) = open_url(&url) {
                        self.push_log(format!("Could not open link: {e}"));
                    }
                }
            }
        }
        if changed {
            self.settings.save();
        }
    }

    fn ui_games(&mut self, ui: &mut egui::Ui) {
        let source = &self.discord_games;
        let total = self.filtered.len();

        if source.is_empty() {
            ui.label(egui::RichText::new("Nothing loaded yet...").small().color(DIM));
        } else if total == 0 {
            ui.label(egui::RichText::new("No matches").small().color(DIM));
        }

        let mut to_run = None;
        egui::ScrollArea::vertical()
            .id_salt("games_scroll")
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show_rows(ui, ROW_H, total, |ui, range| {
                for i in range {
                    let Some(&gi) = self.filtered.get(i) else { break };
                    let Some(g) = source.get(gi) else { continue };
                    let running = self.running.iter().any(|r| r.app_name == g.name);
                    let action = if running { None } else { Some("run") };
                    if list_row(ui, &g.name, action) {
                        to_run = Some(gi);
                    }
                }
            });

        if let Some(g) = to_run.and_then(|gi| source.get(gi).cloned()) {
            self.run_fake_discord(g);
        }
    }

    fn badge_running(&self) -> bool {
        self.badge_shared.running.load(Ordering::Relaxed)
    }

    fn start_badge(&mut self) {
        if self.badge_running() {
            return;
        }
        if self.discord_games.is_empty() {
            self.push_log("Badge: Discord DB not loaded yet");
            return;
        }
        if let Some(h) = self.badge_handle.take() {
            let _ = h.join();
        }
        self.badge_handle = Some(badge::start(
            self.discord_games.clone(),
            self.badge_cfg.clone(),
            self.badge_shared.clone(),
            self.notifier.clone(),
        ));
    }

    fn start_hours(&mut self) {
        if self.badge_running() {
            return;
        }
        if self.discord_games.is_empty() {
            self.push_log("Hours: Discord DB not loaded yet");
            return;
        }
        if let Some(h) = self.badge_handle.take() {
            let _ = h.join();
        }
        self.badge_handle = Some(badge::start_hours(
            self.discord_games.clone(),
            self.hours_cfg.clone(),
            self.badge_shared.clone(),
            self.notifier.clone(),
        ));
    }

    fn ui_farm_games(&mut self, ui: &mut egui::Ui, running: bool, mine: bool) -> FarmAction {
        let mut action = FarmAction::None;
        ui.add_enabled_ui(!running, |ui| {
            egui::Grid::new("badge_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("batch size");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut self.badge_cfg.batch).range(1..=20));
                        ui.label(
                            egui::RichText::new(ram_label(self.badge_cfg.batch, RAM_PER_GAME_MB))
                                .small()
                                .color(DIM),
                        );
                    });
                    ui.end_row();
                    ui.label("hold, sec");
                    ui.add(egui::DragValue::new(&mut self.badge_cfg.hold).range(5..=7200));
                    ui.end_row();
                    ui.label("games (0 = all)");
                    ui.add(egui::DragValue::new(&mut self.badge_cfg.limit).range(0..=100000));
                    ui.end_row();
                });
            ui.checkbox(&mut self.badge_cfg.skip_history, "skip already done");
        });

        ui.horizontal(|ui| {
            if running {
                if pill(ui, "stop").clicked() {
                    action = FarmAction::Stop;
                }
            } else {
                if pill(ui, "start").clicked() {
                    action = FarmAction::StartGames;
                }
                if pill(ui, "clear history").clicked() {
                    action = FarmAction::ClearHistory;
                }
            }
        });

        if running && !mine {
            ui.label(
                egui::RichText::new("Farm hours is running. Stop it first.")
                    .small()
                    .color(DIM),
            );
            return action;
        }
        let done = self.badge_shared.done.load(Ordering::Relaxed);
        let total = self.badge_shared.total.load(Ordering::Relaxed);
        if mine && total > 0 {
            ui.label(format!("{done} / {total} games"));
            ui.add(egui::ProgressBar::new(done as f32 / total as f32));
        }
        let cur = self
            .badge_shared
            .current
            .lock()
            .map(|c| c.join(", "))
            .unwrap_or_default();
        if !cur.is_empty() {
            ui.label(egui::RichText::new(cur).small().color(DIM));
        }
        ui.label(
            egui::RichText::new(
                "Keep Quester open. Discord can take up to a day to update the badge.",
            )
            .small()
            .color(DIM),
        );
        action
    }

    fn ui_farm_hours(&mut self, ui: &mut egui::Ui, running: bool, mine: bool) -> FarmAction {
        let mut action = FarmAction::None;
        let max = if self.hours_cfg.unlimited {
            HOURS_MAX_UNLIMITED
        } else {
            HOURS_MAX
        };
        self.hours_cfg.count = self.hours_cfg.count.clamp(1, max);

        ui.add_enabled_ui(!running, |ui| {
            egui::Grid::new("hours_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("games at once");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut self.hours_cfg.count).range(1..=max));
                        ui.label(
                            egui::RichText::new(ram_label(self.hours_cfg.count, RAM_PER_HOURS_MB))
                                .small()
                                .color(DIM),
                        );
                    });
                    ui.end_row();
                    ui.label("hours");
                    ui.add(
                        egui::DragValue::new(&mut self.hours_cfg.hours).range(1..=HOURS_MAX_TIME),
                    );
                    ui.end_row();
                });
            ui.checkbox(
                &mut self.hours_cfg.unlimited,
                format!("no game limit (up to {HOURS_MAX_UNLIMITED})"),
            );
        });

        ui.horizontal(|ui| {
            if running {
                if pill(ui, "stop").clicked() {
                    action = FarmAction::Stop;
                }
            } else if pill(ui, "start").clicked() {
                action = FarmAction::StartHours;
            }
        });

        if running && !mine {
            ui.label(
                egui::RichText::new("Farm games is running. Stop it first.")
                    .small()
                    .color(DIM),
            );
            return action;
        }
        let dur = self.badge_shared.duration.load(Ordering::Relaxed);
        if running && dur > 0 {
            let start = self.badge_shared.start_ts.load(Ordering::Relaxed);
            let el = now_ts().saturating_sub(start).min(dur);
            let n = self.badge_shared.total.load(Ordering::Relaxed);
            ui.label(format!("{n} games, {}", fmt_left(dur - el)));
            ui.add(egui::ProgressBar::new(el as f32 / dur as f32));
        }
        ui.label(egui::RichText::new("PC may lag if you run many games").color(RED));
        ui.label(
            egui::RichText::new(
                "Keep Quester open. Discord can take up to a day to update the badge",
            )
            .small()
            .color(DIM),
        );
        action
    }

    fn window_badge(&mut self, ctx: &egui::Context) {
        if !self.show_badge {
            return;
        }
        let id = egui::ViewportId::from_hash_of("badge_window");
        let vb = window_viewport(320.0, 320.0)
            .with_title(format!("{APP_NAME} - farm badges"))
            .with_min_inner_size([320.0, 320.0]);
        ctx.show_viewport_immediate(id, vb, |ctx, _| {
            let running = self.badge_running();
            let mine_mode = if self.farm_tab == FarmTab::Games {
                MODE_GAMES
            } else {
                MODE_HOURS
            };
            let mine = self.badge_shared.mode.load(Ordering::Relaxed) == mine_mode;
            let mut action = FarmAction::None;
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(BG).inner_margin(Margin::same(12.0)))
                .show(ctx, |ui| {
                    if !self.badge_unlocked && !running {
                        ui.label(
                            egui::RichText::new("Warning")
                                .size(20.0)
                                .strong()
                                .color(Color32::from_rgb(0xe3, 0x26, 0x36)),
                        );
                        ui.label(
                            "This can get your Discord account banned. Faking game activity \
                             is against Discord's rules and the badge is not guaranteed. \
                             You use it at your own risk.",
                        );
                        ui.checkbox(&mut self.badge_ack, "I understand the risk");
                        let left = if self.badge_ack {
                            let since = *self.badge_warn_since.get_or_insert_with(Instant::now);
                            3u64.saturating_sub(since.elapsed().as_secs())
                        } else {
                            self.badge_warn_since = None;
                            3
                        };
                        let counting = self.badge_ack && left > 0;
                        let label = if counting {
                            format!("continue ({left})")
                        } else {
                            "continue".to_string()
                        };
                        let ready = self.badge_ack && left == 0;
                        if ui
                            .add_enabled(
                                ready,
                                egui::Button::new(egui::RichText::new(label).size(13.0))
                                    .min_size(egui::vec2(46.0, 0.0)),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.badge_unlocked = true;
                        }
                        if counting {
                            ctx.request_repaint_after(Duration::from_millis(250));
                        }
                        return;
                    }

                    ui.horizontal(|ui| {
                        for (t, label) in [
                            (FarmTab::Games, "farm games"),
                            (FarmTab::Hours, "farm hours"),
                        ] {
                            if tab_button(ui, label, self.farm_tab == t).clicked() {
                                self.farm_tab = t;
                            }
                        }
                    });
                    ui.add_space(4.0);

                    action = match self.farm_tab {
                        FarmTab::Games => self.ui_farm_games(ui, running, mine),
                        FarmTab::Hours => self.ui_farm_hours(ui, running, mine),
                    };
                });
            match action {
                FarmAction::StartGames => self.start_badge(),
                FarmAction::StartHours => self.start_hours(),
                FarmAction::Stop => self.badge_shared.cancel.store(true, Ordering::Relaxed),
                FarmAction::ClearHistory => {
                    badge::history_clear();
                    self.push_log("Badge history cleared");
                }
                FarmAction::None => {}
            }
            if ctx.input(|i| i.viewport().close_requested()) {
                self.show_badge = false;
            }
        });
    }

    fn window_logs(&mut self, ctx: &egui::Context) {
        if !self.show_logs {
            return;
        }
        let id = egui::ViewportId::from_hash_of("logs_window");
        let vb = window_viewport(560.0, 340.0)
            .with_title(format!("{APP_NAME} - logs"))
            .with_min_inner_size([300.0, 160.0]);
        ctx.show_viewport_immediate(id, vb, |ctx, _| {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(BG).inner_margin(Margin::same(12.0)))
                .show(ctx, |ui| self.ui_log(ui));
            if ctx.input(|i| i.viewport().close_requested()) {
                self.show_logs = false;
            }
        });
    }

    fn window_running(&mut self, ctx: &egui::Context) {
        if !self.show_running {
            return;
        }
        let id = egui::ViewportId::from_hash_of("running_window");
        let vb = window_viewport(440.0, 360.0)
            .with_title(format!("{APP_NAME} - running"))
            .with_min_inner_size([300.0, 160.0]);
        ctx.show_viewport_immediate(id, vb, |ctx, _| {
            let mut to_stop = None;
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(BG).inner_margin(Margin::same(12.0)))
                .show(ctx, |ui| {
                    if self.running.is_empty() {
                        ui.label(egui::RichText::new("No running apps").small().color(DIM));
                        return;
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("running_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for (i, g) in self.running.iter().enumerate() {
                                let m = now_ts().saturating_sub(g.start_ts) / 60;
                                let text = format!("{} - {} min", g.app_name, m);
                                if list_row(ui, &text, Some("stop")) {
                                    to_stop = Some(i);
                                }
                            }
                        });
                });
            if let Some(i) = to_stop {
                self.stop_game(i);
            }
            if ctx.input(|i| i.viewport().close_requested()) {
                self.show_running = false;
            }
        });
    }

    fn ui_log(&self, ui: &mut egui::Ui) {
        let h = ui.text_style_height(&egui::TextStyle::Small);
        egui::ScrollArea::vertical()
            .id_salt("log_scroll")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show_rows(ui, h, self.log.len(), |ui, range| {
                for i in range {
                    ui.label(
                        egui::RichText::new(format!("> {}", self.log[i]))
                            .small()
                            .color(DIM),
                    );
                }
            });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if !self.style_set {
            apply_theme(ctx);
            spawn_fonts(&self.notifier);
            self.logo = load_logo(ctx);
            self.style_set = true;
        }
        if !self.apps_requested {
            self.apps_requested = true;
            self.load_games();
            if !self.settings.hide_updates {
                self.check_updates();
            }
        }

        self.drain_messages(ctx);
        self.poll_alive();
        self.refilter_if_needed();

        if !self.running.is_empty() || self.badge_running() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }

        let screen = ctx.screen_rect();
        ctx.layer_painter(egui::LayerId::background())
            .rect_filled(screen, 0.0, BG);

        egui::TopBottomPanel::top("top")
            .frame(egui::Frame::none().inner_margin(Margin::symmetric(16.0, 12.0)))
            .show(ctx, |ui| self.ui_header(ui));
        self.refilter_if_needed();

        egui::TopBottomPanel::bottom("bar")
            .frame(egui::Frame::none().inner_margin(Margin::symmetric(20.0, 10.0)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if pill(ui, "farm badges").clicked() {
                        if self.show_badge {
                            ctx.send_viewport_cmd_to(
                                egui::ViewportId::from_hash_of("badge_window"),
                                egui::ViewportCommand::Focus,
                            );
                        } else if !self.badge_running() {
                            self.badge_unlocked = false;
                            self.badge_ack = false;
                            self.badge_warn_since = None;
                            self.farm_tab = FarmTab::Games;
                        } else {
                            self.farm_tab = if self.badge_shared.mode.load(Ordering::Relaxed)
                                == MODE_HOURS
                            {
                                FarmTab::Hours
                            } else {
                                FarmTab::Games
                            };
                        }
                        self.show_badge = true;
                    }
                    let label = format!("running ({})", self.running.len());
                    if pill(ui, &label).clicked() {
                        if self.show_running {
                            ctx.send_viewport_cmd_to(
                                egui::ViewportId::from_hash_of("running_window"),
                                egui::ViewportCommand::Focus,
                            );
                        }
                        self.show_running = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if pill(ui, "logs").clicked() {
                            if self.show_logs {
                                ctx.send_viewport_cmd_to(
                                    egui::ViewportId::from_hash_of("logs_window"),
                                    egui::ViewportCommand::Focus,
                                );
                            }
                            self.show_logs = true;
                        }
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().inner_margin(Margin::symmetric(16.0, 4.0)))
            .show(ctx, |ui| {
                card_frame().show(ui, |ui| {
                    if self.tab == Tab::Settings {
                        self.ui_settings(ui);
                    } else {
                        self.ui_games(ui);
                    }
                });
            });

        self.window_running(ctx);
        self.window_badge(ctx);
        self.window_logs(ctx);
        self.window_close(ctx);
    }
}

struct FakeWindowApp {
    title: String,
    style_set: bool,
    logo: Option<egui::TextureHandle>,
}

impl eframe::App for FakeWindowApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if !self.style_set {
            apply_theme(ctx);
            setup_fonts(ctx);
            self.logo = load_logo(ctx);
            self.style_set = true;
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(Margin::same(8.0)))
            .show(ctx, |ui| {
                fake_card_frame().show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    ui.vertical_centered(|ui| {
                        ui.add_space(((ui.available_height() - 110.0) / 2.0).max(0.0));
                        if let Some(t) = &self.logo {
                            ui.image((t.id(), egui::vec2(40.0, 40.0)));
                        }
                        ui.heading(egui::RichText::new(&self.title).strong());
                        ui.label(
                            egui::RichText::new("keep this window open, don't close it")
                                .small()
                                .color(DIM),
                        );
                    });
                });
            });
        ctx.request_repaint_after(Duration::from_secs(60));
    }
}

pub fn run_launcher() {
    if let Err(e) = eframe::run_native(
        APP_NAME,
        eframe::NativeOptions {
            renderer: eframe::Renderer::Glow,
            multisampling: 0,
            vsync: cfg!(windows),
            persist_window: false,
            viewport: window_viewport(350.0, 520.0).with_min_inner_size([300.0, 360.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(App::new(cc.egui_ctx.clone())))),
    ) {
        log_crash(&format!("run_native failed: {e}"));
    }
}

pub fn run_fake_window(title: String) {
    let viewport = window_viewport(360.0, 200.0)
        .with_title(&title)
        .with_resizable(false)
        .with_maximize_button(false)
        .with_maximized(false)
        .with_min_inner_size([360.0, 200.0])
        .with_max_inner_size([360.0, 200.0]);
    #[cfg(not(windows))]
    let viewport = viewport.with_app_id("Quester.Game");
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        vsync: true,
        persist_window: false,
        viewport,
        ..Default::default()
    };
    let window_title = title.clone();
    if let Err(e) = eframe::run_native(
        &window_title,
        options,
        Box::new(move |_| {
            Ok(Box::new(FakeWindowApp {
                title,
                style_set: false,
                logo: None,
            }))
        }),
    ) {
        log_crash(&format!("fake window failed: {e}"));
    }
}