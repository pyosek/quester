use crate::{
    config::{exe_dir, log_crash, now_ts, Msg, Notifier, APP_NAME, VERSION},
    discord::{self, Game},
    github,
    process::{
        alive_pids, cleanup_manifest, kill_pid, launch_fake, pid_is_ours, running_load,
        running_save, RunningEntry,
    },
};
use eframe::egui::{self, Color32, Margin, Rounding, Stroke};
use std::{
    fs,
    sync::mpsc::{channel, Receiver},
    thread,
    time::{Duration, Instant},
};

const LOGO_SMALL: &[u8] = include_bytes!("../assets/logo_64.png");
const LOGO_FULL: &[u8] = include_bytes!("../assets/logo.png");

const ROW_CONTENT_H: f32 = 36.0;
const ROW_H: f32 = ROW_CONTENT_H + 12.0;

const BG: Color32 = Color32::from_rgb(0x08, 0x09, 0x0a);
const CARD: Color32 = Color32::from_rgb(0x0a, 0x0a, 0x0b);
const ROW: Color32 = Color32::from_rgb(0x15, 0x15, 0x17);
const BORDER: Color32 = Color32::from_rgb(150, 150, 158);
const BORDER_DIM: Color32 = Color32::from_rgb(60, 60, 66);
const TEXT: Color32 = Color32::from_rgb(230, 230, 234);
const DIM: Color32 = Color32::from_rgb(110, 110, 118);
const GLOW: Color32 = Color32::from_rgba_premultiplied(30, 30, 32, 110);

fn setup_fonts(ctx: &egui::Context) {
    if let Ok(bytes) = fs::read(exe_dir().join("Skranji-Regular.ttf")) {
        let mut fonts = egui::FontDefinitions::default();
        fonts
            .font_data
            .insert("skranji".to_owned(), egui::FontData::from_owned(bytes).into());
        if let Some(f) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            f.insert(0, "skranji".to_owned());
        }
        ctx.set_fonts(fonts);
    }
}

fn apply_theme(ctx: &egui::Context) {
    setup_fonts(ctx);
    ctx.options_mut(|o| o.zoom_with_keyboard = false);

    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = CARD;
    v.extreme_bg_color = CARD;
    v.faint_bg_color = ROW;
    v.override_text_color = None;
    v.hyperlink_color = TEXT;
    v.selection.bg_fill = Color32::from_rgb(0x4E, 0x4B, 0x58);
    v.selection.stroke = Stroke::new(1.0, TEXT);

    let r = Rounding::same(6.0);
    v.widgets.noninteractive.bg_fill = CARD;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER_DIM);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.noninteractive.rounding = r;

    v.widgets.inactive.bg_fill = ROW;
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER_DIM);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.rounding = r;

    v.widgets.hovered.bg_fill = ROW;
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(30, 30, 34);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.hovered.rounding = r;

    v.widgets.active.bg_fill = ROW;
    v.widgets.active.weak_bg_fill = Color32::from_rgb(40, 40, 46);
    v.widgets.active.bg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.active.rounding = r;

    v.widgets.open.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.open.rounding = r;

    v.window_rounding = Rounding::same(20.0);
    v.menu_rounding = Rounding::same(10.0);

    ctx.set_visuals(v);
    ctx.style_mut(|s| {
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

fn window_viewport(w: f32, h: f32) -> egui::ViewportBuilder {
    let mut vb = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_inner_size([w, h]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(LOGO_FULL) {
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
        .stroke(Stroke::new(2.0, BORDER))
        .rounding(Rounding::same(20.0))
        .inner_margin(Margin::same(12.0))
        .shadow(egui::epaint::Shadow {
            offset: egui::vec2(0.0, 0.0),
            blur: 30.0,
            spread: 2.0,
            color: GLOW,
        })
}

fn fake_card_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(2.0, BORDER))
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
    let mut job = egui::text::LayoutJob::single_section(
        name.to_owned(),
        egui::TextFormat {
            font_id: egui::FontId::proportional(13.5),
            color: TEXT,
            ..Default::default()
        },
    );
    job.wrap.max_rows = 2;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    ui.add(egui::Label::new(job).wrap()).on_hover_text(name);
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
    GithubDb,
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
    github_games: Vec<Game>,
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
}

impl App {
    fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = channel();

        let mut entries = running_load();
        if let Some(alive) = alive_pids() {
            entries.retain(|e| alive.contains(&e.pid));
        }
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
            logo: None,
            discord_games: vec![],
            github_games: vec![],
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
        self.push_log("Refreshing Discord DB and GitHub DB...");
        discord::spawn_refresh(&self.notifier);
        github::spawn_refresh(&self.notifier);
    }

    fn is_running(&self, name: &str) -> bool {
        self.running.iter().any(|g| g.app_name == name)
    }

    fn run_fake_discord(&mut self, g: Game) {
        if self.is_running(&g.name) {
            return;
        }
        let (pid, lines) = launch_fake(&g);
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
            || self.last_check.elapsed() < Duration::from_secs(3)
        {
            return;
        }
        self.last_check = Instant::now();
        self.alive_inflight = true;
        let n = self.notifier.clone();
        thread::spawn(move || n.send(Msg::Alive(alive_pids())));
    }

    fn drain_messages(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::DiscordGames(g) => {
                    self.push_log(format!("Discord DB loaded: {} games", g.len()));
                    self.discord_games = g;
                    self.filter_dirty = true;
                }
                Msg::GithubGames(g) => {
                    self.push_log(format!("GitHub DB loaded: {} games", g.len()));
                    self.github_games = g;
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
                Msg::Log(s) => self.push_log(s),
            }
        }
    }

    fn refilter_if_needed(&mut self) {
        if !self.filter_dirty && self.search == self.last_search && self.tab == self.last_tab {
            return;
        }
        let q = self.search.to_lowercase();
        let src = match self.tab {
            Tab::DiscordDb => &self.discord_games,
            Tab::GithubDb => &self.github_games,
        };
        self.filtered = src
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
                ui.label(egui::RichText::new(VERSION).small().color(DIM));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if pill(ui, "clean").clicked() {
                    let (cleaned, skipped, lines) = cleanup_manifest();
                    for l in lines {
                        self.push_log(l);
                    }
                    self.push_log(format!("Cleanup: removed {cleaned}, skipped {skipped}"));
                }
                if pill(ui, "refresh").clicked() {
                    self.load_games();
                }
            });
        });

        ui.horizontal_wrapped(|ui| {
            let d = format!("Discord ({})", self.discord_games.len());
            let c = format!("GitHub ({})", self.github_games.len());
            for (tab, label) in [(Tab::DiscordDb, d), (Tab::GithubDb, c)] {
                if tab_button(ui, &label, self.tab == tab).clicked() {
                    self.tab = tab;
                }
            }
        });

        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("search game...")
                .desired_width(f32::INFINITY)
                .margin(egui::vec2(10.0, 6.0)),
        );
    }

    fn ui_games(&mut self, ui: &mut egui::Ui) {
        let source = match self.tab {
            Tab::DiscordDb => &self.discord_games,
            Tab::GithubDb => &self.github_games,
        };
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

        if let Some(gi) = to_run {
            if let Some(g) = source.get(gi).cloned() {
                self.run_fake_discord(g);
            }
        }
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
            self.logo = load_logo(ctx);
            self.style_set = true;
        }
        if !self.apps_requested {
            self.apps_requested = true;
            self.load_games();
        }

        self.drain_messages();
        self.poll_alive();
        self.refilter_if_needed();

        if !self.running.is_empty() {
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
                card_frame().show(ui, |ui| self.ui_games(ui));
            });

        self.window_running(ctx);
        self.window_logs(ctx);
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
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        vsync: true,
        persist_window: false,
        viewport: window_viewport(360.0, 200.0)
            .with_title(&title)
            .with_resizable(false)
            .with_maximize_button(false)
            .with_maximized(false)
            .with_min_inner_size([360.0, 200.0])
            .with_max_inner_size([360.0, 200.0]),
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