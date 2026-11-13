use crate::{
    config::{exe_dir, games_dir, Msg, Notifier},
    discord::Game,
};
use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[derive(Clone)]
pub struct BadgeCfg {
    pub batch: usize,
    pub hold: u64,
    pub limit: usize,
    pub skip_history: bool,
    pub lite: bool,
}

impl Default for BadgeCfg {
    fn default() -> Self {
        Self {
            batch: 8,
            hold: 15,
            limit: 100,
            skip_history: true,
            lite: true,
        }
    }
}

#[derive(Default)]
pub struct BadgeShared {
    pub cancel: AtomicBool,
    pub running: AtomicBool,
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    pub current: Mutex<Vec<String>>,
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

fn spawn_one(
    me: &Path,
    root: &Path,
    slot: usize,
    g: &Game,
    lite: bool,
    ttl: u64,
) -> Result<Child, String> {
    let rel = crate::discord::normalize_rel(&g.exes[0]);
    if !rel_is_safe(&rel) {
        return Err(format!("unsafe path: {rel}"));
    }
    let target = root.join(slot.to_string()).join(&rel);
    if let Some(p) = target.parent() {
        fs::create_dir_all(p).map_err(|e| format!("{}: {e}", g.name))?;
    }
    if fs::hard_link(me, &target).is_err() {
        fs::copy(me, &target).map_err(|e| format!("{}: copy failed: {e}", g.name))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&target, fs::Permissions::from_mode(0o755));
    }
    let mut c = Command::new(&target);
    if lite {
        c.arg("--sleep-lite").arg(ttl.to_string()).arg(&g.name);
    } else {
        c.arg("--sleep").arg(&g.name);
    }
    c.spawn().map_err(|e| format!("{}: {e}", g.name))
}

fn run(games: Vec<Game>, cfg: BadgeCfg, sh: &BadgeShared, n: &Notifier) {
    let me = match env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            n.send(Msg::Log(format!("Badge: current_exe error: {e}")));
            return;
        }
    };

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
            match spawn_one(&me, &root, slot, g, cfg.lite, ttl) {
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

pub fn start(
    games: Vec<Game>,
    cfg: BadgeCfg,
    sh: Arc<BadgeShared>,
    n: Notifier,
) -> JoinHandle<()> {
    sh.cancel.store(false, Ordering::Relaxed);
    sh.running.store(true, Ordering::Relaxed);
    sh.done.store(0, Ordering::Relaxed);
    sh.total.store(0, Ordering::Relaxed);
    thread::spawn(move || {
        run(games, cfg, &sh, &n);
        if let Ok(mut cur) = sh.current.lock() {
            cur.clear();
        }
        sh.running.store(false, Ordering::Relaxed);
        n.send(Msg::Log(String::new()));
    })
}

#[cfg(windows)]
pub fn run_lite_window(title: String, ttl: u64) {
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
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            name.as_ptr(),
            0x00CF_0000,
            use_default,
            use_default,
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
        ShowWindow(hwnd, 7);
        let mut msg: MsgW = std::mem::zeroed();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(not(windows))]
pub fn run_lite_window(_title: String, ttl: u64) {
    thread::sleep(Duration::from_secs(ttl));
}