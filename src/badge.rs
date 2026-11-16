use crate::{
    config::{exe_dir, games_dir, now_ts, Msg, Notifier},
    discord::Game,
    process::RunningEntry,
};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const HOURS_MAX: usize = 50;
pub const HOURS_MAX_UNLIMITED: usize = 250;
pub const HOURS_MAX_TIME: u64 = 168;

pub const MODE_GAMES: usize = 0;
pub const MODE_HOURS: usize = 1;

#[derive(Clone)]
pub struct BadgeCfg {
    pub batch: usize,
    pub hold: u64,
    pub limit: usize,
    pub skip_history: bool,
}

impl Default for BadgeCfg {
    fn default() -> Self {
        Self {
            batch: 8,
            hold: 15,
            limit: 100,
            skip_history: true,
        }
    }
}

#[derive(Clone)]
pub struct HoursCfg {
    pub count: usize,
    pub hours: u64,
    pub unlimited: bool,
}

impl Default for HoursCfg {
    fn default() -> Self {
        Self {
            count: 5,
            hours: 1,
            unlimited: false,
        }
    }
}

#[derive(Default)]
pub struct BadgeShared {
    pub cancel: AtomicBool,
    pub running: AtomicBool,
    pub mode: AtomicUsize,
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    pub start_ts: AtomicU64,
    pub duration: AtomicU64,
    pub current: Mutex<Vec<String>>,
}

trait NoConsole {
    fn no_console(&mut self) -> &mut Self;
}

#[cfg(windows)]
impl NoConsole for Command {
    fn no_console(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(0x0800_0040)
    }
}

#[cfg(not(windows))]
impl NoConsole for Command {
    fn no_console(&mut self) -> &mut Self {
        self
    }
}

fn history_path() -> PathBuf {
    exe_dir().join("quester_badge_history.json")
}

fn history_load() -> HashSet<String> {
    fs::read_to_string(history_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn history_save(h: &HashSet<String>) {
    if let Ok(s) = serde_json::to_string(h) {
        let _ = fs::write(history_path(), s);
    }
}

pub fn history_clear() {
    let _ = fs::remove_file(history_path());
}

fn badge_root() -> PathBuf {
    games_dir().join("_badge")
}

fn hours_root() -> PathBuf {
    games_dir().join("_hours")
}

fn rel_is_safe(rel: &str) -> bool {
    let p = Path::new(rel);
    !rel.is_empty()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

fn cleanup(root: &Path) {
    for _ in 0..20 {
        if !root.exists() || fs::remove_dir_all(root).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn shuffle<T>(v: &mut [T]) {
    let mut s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
        | 1;
    for i in (1..v.len()).rev() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let j = (s % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
}

fn dir_key(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .take(40)
        .collect();
    let mut h: u32 = 0x811C_9DC5;
    for b in name.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{clean}-{h:08x}")
}

fn spawn_one(root: &Path, key: &str, g: &Game, ttl: u64) -> Result<Child, String> {
    let rel = crate::discord::normalize_rel(&g.exes[0]);
    if !rel_is_safe(&rel) {
        return Err(format!("unsafe path: {rel}"));
    }
    let target = root.join(key).join(&rel);
    if !crate::process::sleeper_present(&target) {
        if let Some(p) = target.parent() {
            fs::create_dir_all(p).map_err(|e| format!("{}: {e}", g.name))?;
        }
        crate::process::ensure_sleeper(&target)
            .map_err(|e| format!("{}: write sleeper failed: {e}", g.name))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&target, fs::Permissions::from_mode(0o755));
        }
    }
    Command::new(&target)
        .args(crate::process::sleeper_args(ttl, true, &g.name))
        .no_console()
        .spawn()
        .map_err(|e| format!("{}: {e}", g.name))
}

fn run(games: Vec<Game>, cfg: BadgeCfg, sh: &BadgeShared, n: &Notifier) {
    crate::process::wait_orphan_scan();
    crate::process::ensure_games_marker(true);
    let mut hist = history_load();
    let mut pool: Vec<Game> = games
        .into_iter()
        .filter(|g| {
            !g.exes.is_empty()
                && !g.name.is_empty()
                && (!cfg.skip_history || !hist.contains(&g.name))
        })
        .collect();
    if cfg.limit > 0 {
        pool.truncate(cfg.limit);
    }
    sh.total.store(pool.len(), Ordering::Relaxed);
    if pool.is_empty() {
        n.send(Msg::Log("Badge: nothing to do (all games in history?)".into()));
        return;
    }
    n.send(Msg::Log(format!("Badge: started, {} games", pool.len())));

    let root = badge_root();
    let batch = cfg.batch.clamp(1, 20);
    let hold_ms = cfg.hold.max(1) * 1000;
    let ttl = cfg.hold + 30;
    let mut done = 0usize;

    for chunk in pool.chunks(batch) {
        if sh.cancel.load(Ordering::Relaxed) {
            break;
        }
        cleanup(&root);

        let mut kids: Vec<Child> = vec![];
        let mut names: Vec<String> = vec![];
        for (slot, g) in chunk.iter().enumerate() {
            match spawn_one(&root, &slot.to_string(), g, ttl) {
                Ok(c) => {
                    kids.push(c);
                    names.push(g.name.clone());
                }
                Err(e) => n.send(Msg::Log(format!("Badge: {e}"))),
            }
        }
        if let Ok(mut cur) = sh.current.lock() {
            *cur = names.clone();
        }

        let mut waited = 0u64;
        while !kids.is_empty() && waited < hold_ms && !sh.cancel.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));
            waited += 100;
        }
        let completed = !kids.is_empty() && waited >= hold_ms;

        for c in kids.iter_mut() {
            let _ = c.kill();
            let _ = c.wait();
        }
        cleanup(&root);

        if completed {
            done += names.len();
            for nm in names {
                hist.insert(nm);
            }
            history_save(&hist);
            sh.done.store(done, Ordering::Relaxed);
            n.send(Msg::Log(format!("Badge: {done} / {}", pool.len())));
        }
    }
    cleanup(&root);
    let msg = if sh.cancel.load(Ordering::Relaxed) {
        "Badge: stopped"
    } else {
        "Badge: finished"
    };
    n.send(Msg::Log(msg.into()));
}

type Spawned = Result<(String, Child), String>;

fn spawn_parallel(root: &Path, pool: &[Game], ttl: u64, sh: &BadgeShared) -> Vec<Spawned> {
    let cpus = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 8);
    let workers = pool.len().min(cpus).max(1);
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, Spawned)>> = Mutex::new(Vec::new());
    thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                if sh.cancel.load(Ordering::Relaxed) {
                    break;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= pool.len() {
                    break;
                }
                let g = &pool[i];
                let r = spawn_one(root, &dir_key(&g.name), g, ttl).map(|c| (g.name.clone(), c));
                if let Ok(mut v) = results.lock() {
                    v.push((i, r));
                }
            });
        }
    });
    let mut v = results.into_inner().unwrap_or_default();
    v.sort_by_key(|x| x.0);
    v.into_iter().map(|x| x.1).collect()
}

fn run_hours(games: Vec<Game>, cfg: HoursCfg, sh: &BadgeShared, n: &Notifier) {
    crate::process::ensure_games_marker(true);
    let mut pool: Vec<Game> = games
        .into_iter()
        .filter(|g| {
            !g.exes.is_empty()
                && !g.name.is_empty()
                && rel_is_safe(&crate::discord::normalize_rel(&g.exes[0]))
                && !g.adult
        })
        .collect();
    shuffle(&mut pool);
    let present: HashSet<String> = fs::read_dir(hours_root())
        .map(|d| {
            d.flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    let have = |g: &Game| present.contains(&dir_key(&g.name));
    pool.sort_by_cached_key(|g| !have(g));
    let max = if cfg.unlimited {
        HOURS_MAX_UNLIMITED
    } else {
        HOURS_MAX
    };
    pool.truncate(cfg.count.clamp(1, max));
    if pool.is_empty() {
        n.send(Msg::Log("Hours: no games to run".into()));
        return;
    }
    let existing = pool.iter().filter(|g| have(g)).count();
    n.send(Msg::Log(format!(
        "Hours: {existing} existing, {} new",
        pool.len() - existing
    )));

    let secs = cfg.hours.clamp(1, HOURS_MAX_TIME) * 3600;
    let ttl = secs + 60;
    let root = hours_root();
    let _ = fs::create_dir_all(&root);
    crate::process::wait_orphan_scan();
    crate::process::kill_hours_orphans(now_ts());

    let mut kids: Vec<Child> = vec![];
    let mut names: Vec<String> = vec![];
    let t0 = Instant::now();
    let spawned = spawn_parallel(&root, &pool, ttl, sh);
    n.send(Msg::Log(format!(
        "Hours: launched in {} ms",
        t0.elapsed().as_millis()
    )));
    for r in spawned {
        match r {
            Ok((nm, c)) => {
                names.push(nm);
                kids.push(c);
            }
            Err(e) => n.send(Msg::Log(format!("Hours: {e}"))),
        }
    }
    let pids: Vec<u32> = kids.iter().map(|c| c.id()).collect();
    let started = now_ts();
    crate::process::running_add_hours(
        pids.iter()
            .zip(names.iter())
            .map(|(pid, nm)| RunningEntry {
                pid: *pid,
                app_name: nm.clone(),
                start_ts: started,
                hours: true,
            })
            .collect(),
    );

    if !kids.is_empty() && !sh.cancel.load(Ordering::Relaxed) {
        sh.total.store(kids.len(), Ordering::Relaxed);
        sh.start_ts.store(now_ts(), Ordering::Relaxed);
        sh.duration.store(secs, Ordering::Relaxed);
        if let Ok(mut cur) = sh.current.lock() {
            *cur = names;
        }
        n.send(Msg::Log(format!(
            "Hours: {} games for {} h",
            kids.len(),
            secs / 3600
        )));

        let deadline = Instant::now() + Duration::from_secs(secs);
        let mut last_check = Instant::now();
        while Instant::now() < deadline && !sh.cancel.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(200));
            if last_check.elapsed() >= Duration::from_secs(2) {
                last_check = Instant::now();
                kids.retain_mut(|c| matches!(c.try_wait(), Ok(None)));
                sh.total.store(kids.len(), Ordering::Relaxed);
                if kids.is_empty() {
                    n.send(Msg::Log("Hours: all windows were closed".into()));
                    break;
                }
            }
        }
    }

    for c in kids.iter_mut() {
        let _ = c.kill();
        let _ = c.wait();
    }
    crate::process::running_remove_pids(&pids);
    let msg = if sh.cancel.load(Ordering::Relaxed) {
        "Hours: stopped"
    } else {
        "Hours: finished"
    };
    n.send(Msg::Log(msg.into()));
}

fn begin(sh: &BadgeShared, mode: usize) {
    sh.cancel.store(false, Ordering::Relaxed);
    sh.running.store(true, Ordering::Relaxed);
    sh.mode.store(mode, Ordering::Relaxed);
    sh.done.store(0, Ordering::Relaxed);
    sh.total.store(0, Ordering::Relaxed);
    sh.start_ts.store(0, Ordering::Relaxed);
    sh.duration.store(0, Ordering::Relaxed);
}

fn finish(sh: &BadgeShared, n: &Notifier) {
    if let Ok(mut cur) = sh.current.lock() {
        cur.clear();
    }
    sh.running.store(false, Ordering::Relaxed);
    n.send(Msg::Log(String::new()));
}

pub fn start(
    games: Vec<Game>,
    cfg: BadgeCfg,
    sh: Arc<BadgeShared>,
    n: Notifier,
) -> JoinHandle<()> {
    begin(&sh, MODE_GAMES);
    thread::spawn(move || {
        run(games, cfg, &sh, &n);
        finish(&sh, &n);
    })
}

pub fn start_hours(
    games: Vec<Game>,
    cfg: HoursCfg,
    sh: Arc<BadgeShared>,
    n: Notifier,
) -> JoinHandle<()> {
    begin(&sh, MODE_HOURS);
    thread::spawn(move || {
        run_hours(games, cfg, &sh, &n);
        finish(&sh, &n);
    })
}

#[cfg(windows)]
pub fn run_lite_window(title: String, ttl: u64, hidden: bool) {
    use std::ffi::c_void;

    #[repr(C)]
    struct WndClassW {
        style: u32,
        wnd_proc: unsafe extern "system" fn(isize, u32, usize, isize) -> isize,
        cls_extra: i32,
        wnd_extra: i32,
        instance: isize,
        icon: isize,
        cursor: isize,
        background: isize,
        menu_name: *const u16,
        class_name: *const u16,
    }

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct MsgW {
        hwnd: isize,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt: Point,
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassW(c: *const WndClassW) -> u16;
        fn CreateWindowExW(
            ex: u32,
            class: *const u16,
            name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: isize,
            menu: isize,
            instance: isize,
            param: *const c_void,
        ) -> isize;
        fn ShowWindow(h: isize, cmd: i32) -> i32;
        fn DefWindowProcW(h: isize, m: u32, w: usize, l: isize) -> isize;
        fn GetMessageW(msg: *mut MsgW, h: isize, min: u32, max: u32) -> i32;
        fn TranslateMessage(msg: *const MsgW) -> i32;
        fn DispatchMessageW(msg: *const MsgW) -> isize;
        fn PostQuitMessage(code: i32);
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> isize;
    }

    unsafe extern "system" fn wnd_proc(h: isize, m: u32, w: usize, l: isize) -> isize {
        if m == 0x0002 {
            PostQuitMessage(0);
            return 0;
        }
        DefWindowProcW(h, m, w, l)
    }

    thread::spawn(move || {
        thread::sleep(Duration::from_secs(ttl));
        std::process::exit(0);
    });

    unsafe {
        let inst = GetModuleHandleW(std::ptr::null());
        let class: Vec<u16> = "QuesterLite\0".encode_utf16().collect();
        let name: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let wc = WndClassW {
            style: 0,
            wnd_proc,
            cls_extra: 0,
            wnd_extra: 0,
            instance: inst,
            icon: 0,
            cursor: 0,
            background: 0,
            menu_name: std::ptr::null(),
            class_name: class.as_ptr(),
        };
        if RegisterClassW(&wc) == 0 {
            return;
        }
        let use_default = 0x8000_0000u32 as i32;
        let (ex, pos) = if hidden {
            (0x0000_0080u32 | 0x0800_0000u32, -32000)
        } else {
            (0, use_default)
        };
        let hwnd = CreateWindowExW(
            ex,
            class.as_ptr(),
            name.as_ptr(),
            0x00CF_0000,
            pos,
            pos,
            320,
            120,
            0,
            0,
            inst,
            std::ptr::null(),
        );
        if hwnd == 0 {
            return;
        }
        ShowWindow(hwnd, if hidden { 4 } else { 7 });
        let mut msg: MsgW = std::mem::zeroed();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(not(windows))]
pub fn run_lite_window(_title: String, ttl: u64, _hidden: bool) {
    #[cfg(target_os = "linux")]
    if crate::process::is_tiling_wm() {
        unsafe {
            libc::prctl(1, libc::SIGKILL as libc::c_ulong, 0, 0, 0);
        }
    }
    thread::sleep(Duration::from_secs(ttl));
}