use crate::{
    config::{exe_dir, games_dir, now_ts},
    discord::Game,
};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    thread,
    time::Duration,
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

trait QuietCommand {
    fn quiet(&mut self) -> &mut Self;
}

#[cfg(windows)]
impl QuietCommand for Command {
    fn quiet(&mut self) -> &mut Self {
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

#[cfg(not(windows))]
impl QuietCommand for Command {
    fn quiet(&mut self) -> &mut Self {
        self
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub kind: String,
    pub path: String,
    pub quest_app: String,
    pub created_at: u64,
    #[serde(default)]
    pub size: u64,
}

fn manifest_path() -> PathBuf {
    exe_dir().join("quester_manifest.json")
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

#[cfg(windows)]
fn norm_path(s: &str) -> String {
    s.replace('/', "\\")
        .to_lowercase()
        .trim_end_matches('\\')
        .to_string()
}

#[cfg(not(windows))]
fn norm_path(s: &str) -> String {
    s.trim_end_matches('/').to_string()
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

#[derive(Clone, Serialize, Deserialize)]
pub struct RunningEntry {
    pub pid: u32,
    pub app_name: String,
    pub start_ts: u64,
    #[serde(default)]
    pub hours: bool,
}

fn running_path() -> PathBuf {
    exe_dir().join("quester_running.json")
}

pub fn running_load() -> Vec<RunningEntry> {
    fs::read_to_string(running_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn running_save(entries: &[RunningEntry]) {
    if let Ok(s) = serde_json::to_string_pretty(entries) {
        let _ = fs::write(running_path(), s);
    }
}

static RUNNING_LOCK: Mutex<()> = Mutex::new(());

pub fn running_replace_normal(entries: &[RunningEntry]) {
    let _g = RUNNING_LOCK.lock();
    let mut v: Vec<RunningEntry> = running_load().into_iter().filter(|e| e.hours).collect();
    v.extend(entries.iter().cloned());
    running_save(&v);
}

pub fn running_add_hours(entries: Vec<RunningEntry>) {
    let _g = RUNNING_LOCK.lock();
    let mut v = running_load();
    v.extend(entries);
    running_save(&v);
}

pub fn running_remove_pids(pids: &[u32]) {
    let _g = RUNNING_LOCK.lock();
    let mut v = running_load();
    v.retain(|e| !pids.contains(&e.pid));
    running_save(&v);
}

pub fn kill_hours_orphans(before_ts: u64) -> usize {
    let _g = RUNNING_LOCK.lock();
    let all = running_load();
    let (old, keep): (Vec<RunningEntry>, Vec<RunningEntry>) = all
        .into_iter()
        .partition(|e| e.hours && e.start_ts < before_ts);
    let pids: Vec<u32> = old.iter().map(|e| e.pid).collect();
    let killed = kill_ours(&pids);
    running_save(&keep);
    killed
}

#[cfg(windows)]
pub fn alive_pids() -> Option<Vec<u32>> {
    let o = Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .quiet()
        .output()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&o.stdout);
    Some(
        s.lines()
            .filter_map(|l| l.split("\",\"").nth(1)?.trim_matches('"').parse().ok())
            .collect(),
    )
}

#[cfg(windows)]
pub fn alive_of(pids: &[u32]) -> Option<Vec<u32>> {
    let all = alive_pids()?;
    Some(pids.iter().copied().filter(|p| all.contains(p)).collect())
}

#[cfg(not(windows))]
pub fn alive_of(pids: &[u32]) -> Option<Vec<u32>> {
    Some(
        pids.iter()
            .copied()
            .filter(|pid| {
                fs::read_to_string(format!("/proc/{pid}/stat"))
                    .map(|s| !is_zombie(&s))
                    .unwrap_or(false)
            })
            .collect(),
    )
}

#[cfg(not(windows))]
fn is_zombie(stat: &str) -> bool {
    stat.rfind(')')
        .and_then(|i| stat[i + 1..].split_whitespace().next())
        .map(|s| s == "Z" || s == "X")
        .unwrap_or(false)
}

#[cfg(windows)]
pub fn pid_is_ours(pid: u32) -> bool {
    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("(Get-Process -Id {pid} -ErrorAction SilentlyContinue).Path"),
        ])
        .quiet()
        .output();
    let path = match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => return false,
    };
    if path.is_empty() {
        return false;
    }
    let Ok(p) = fs::canonicalize(&path) else {
        return false;
    };
    pid_path_is_ours(&p)
}

#[cfg(not(windows))]
pub fn pid_is_ours(pid: u32) -> bool {
    let Ok(link) = fs::read_link(format!("/proc/{pid}/exe")) else {
        return false;
    };
    let link = link.to_string_lossy();
    let link = link.strip_suffix(" (deleted)").unwrap_or(&link);
    let Ok(p) = fs::canonicalize(link) else {
        return false;
    };
    pid_path_is_ours(&p)
}

fn pid_path_is_ours(p: &Path) -> bool {
    if let Ok(g) = fs::canonicalize(games_dir()) {
        if p.starts_with(&g) {
            return true;
        }
    }
    false
}

#[cfg(windows)]
pub fn kill_pid(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .quiet()
        .output();
}

#[cfg(not(windows))]
pub fn kill_pid(pid: u32) {
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

#[cfg(windows)]
fn ours_among(pids: &[u32]) -> Vec<u32> {
    let mut res = vec![];
    for chunk in pids.chunks(100) {
        let list = chunk
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let script = format!(
            "Get-Process -Id {list} -ErrorAction SilentlyContinue | ForEach-Object {{ \"$($_.Id)|$($_.Path)\" }}"
        );
        let Ok(o) = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .quiet()
            .output()
        else {
            continue;
        };
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            let Some((id, path)) = line.trim().split_once('|') else {
                continue;
            };
            let (Ok(id), false) = (id.parse::<u32>(), path.is_empty()) else {
                continue;
            };
            if let Ok(p) = fs::canonicalize(path) {
                if pid_path_is_ours(&p) {
                    res.push(id);
                }
            }
        }
    }
    res
}

#[cfg(not(windows))]
fn ours_among(pids: &[u32]) -> Vec<u32> {
    pids.iter().copied().filter(|p| pid_is_ours(*p)).collect()
}

#[cfg(windows)]
fn kill_many(pids: &[u32]) {
    for chunk in pids.chunks(100) {
        let mut c = Command::new("taskkill");
        for p in chunk {
            c.arg("/PID").arg(p.to_string());
        }
        let _ = c.arg("/F").quiet().output();
    }
}

#[cfg(not(windows))]
fn kill_many(pids: &[u32]) {
    for p in pids {
        kill_pid(*p);
    }
}

static SCAN_DONE: AtomicBool = AtomicBool::new(true);

pub fn wait_orphan_scan() {
    for _ in 0..100 {
        if SCAN_DONE.load(Ordering::Relaxed) {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub fn spawn_orphan_cleanup(launch_ts: u64) {
    SCAN_DONE.store(false, Ordering::Relaxed);
    thread::spawn(move || {
        kill_hours_orphans(launch_ts);
        kill_unlisted_orphans();
        SCAN_DONE.store(true, Ordering::Relaxed);
    });
}

fn kill_unlisted_orphans() {
    let _g = RUNNING_LOCK.lock();
    let listed: Vec<u32> = running_load().iter().map(|e| e.pid).collect();
    let orphans: Vec<u32> = pids_in_games()
        .into_iter()
        .filter(|p| !listed.contains(p))
        .collect();
    kill_many(&orphans);
}

#[cfg(windows)]
fn pids_in_games() -> Vec<u32> {
    let base = games_dir().to_string_lossy().replace('\'', "''");
    let script = format!(
        "$p = '{base}\\'; Get-Process | Where-Object {{ $_.Path -and $_.Path.StartsWith($p, [System.StringComparison]::OrdinalIgnoreCase) }} | ForEach-Object {{ $_.Id }}"
    );
    let Ok(o) = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .quiet()
        .output()
    else {
        return vec![];
    };
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
}

#[cfg(not(windows))]
fn pids_in_games() -> Vec<u32> {
    let Ok(g) = fs::canonicalize(games_dir()) else {
        return vec![];
    };
    let Ok(rd) = fs::read_dir("/proc") else {
        return vec![];
    };
    rd.flatten()
        .filter_map(|e| {
            let pid = e.file_name().to_string_lossy().parse::<u32>().ok()?;
            let link = fs::read_link(e.path().join("exe")).ok()?;
            let link = link.to_string_lossy().to_string();
            let link = link.strip_suffix(" (deleted)").unwrap_or(&link).to_string();
            Path::new(&link).starts_with(&g).then_some(pid)
        })
        .collect()
}

pub fn kill_ours(pids: &[u32]) -> usize {
    let ours = ours_among(pids);
    kill_many(&ours);
    ours.len()
}

const MANUAL_TTL: u64 = 31_536_000;

static SLEEPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sleeper.bin"));

pub fn sleeper_template() -> Result<PathBuf, String> {
    env::current_exe().map_err(|e| format!("current_exe: {e}"))
}

pub fn write_sleeper(target: &Path) -> std::io::Result<()> {
    fs::write(target, SLEEPER)
}

pub fn sleeper_present(target: &Path) -> bool {
    fs::metadata(target)
        .map(|m| m.len() == SLEEPER.len() as u64)
        .unwrap_or(false)
}

fn sleeper_hash() -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for b in SLEEPER {
        h ^= *b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn sleeper_tpl() -> Option<&'static Path> {
    static TPL: OnceLock<Option<PathBuf>> = OnceLock::new();
    TPL.get_or_init(|| {
        let dir = games_dir().join("_tpl");
        fs::create_dir_all(&dir).ok()?;
        let ext = if cfg!(windows) { ".exe" } else { "" };
        let p = dir.join(format!("sleeper-{:08x}{ext}", sleeper_hash()));
        if !sleeper_present(&p) {
            let _ = fs::remove_file(&p);
            fs::write(&p, SLEEPER).ok()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&p, fs::Permissions::from_mode(0o755));
            }
        }
        Some(p)
    })
    .as_deref()
}

pub fn ensure_sleeper(target: &Path) -> std::io::Result<()> {
    if sleeper_present(target) {
        return Ok(());
    }
    let _ = fs::remove_file(target);
    if let Some(tpl) = sleeper_tpl() {
        if fs::hard_link(tpl, target).is_ok() || fs::copy(tpl, target).is_ok() {
            return Ok(());
        }
    }
    fs::write(target, SLEEPER)
}

pub fn sleeper_args(ttl: u64, hidden: bool, title: &str) -> Vec<String> {
    vec![
        ttl.to_string(),
        (hidden as u8).to_string(),
        (is_tiling_wm() as u8).to_string(),
        title.to_string(),
    ]
}

#[cfg(target_os = "linux")]
pub fn is_tiling_wm() -> bool {
    const ENV: &[&str] = &["NIRI_SOCKET", "HYPRLAND_INSTANCE_SIGNATURE", "SWAYSOCK", "I3SOCK"];
    if ENV.iter().any(|k| env::var_os(k).is_some()) {
        return true;
    }
    const NAMES: &[&str] = &[
        "niri", "hyprland", "sway", "i3", "bspwm", "river", "dwl", "awesome", "qtile", "xmonad",
        "dwm", "herbstluftwm", "leftwm", "scrollwm", "mango", "labwc-tiling",
    ];
    ["XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP", "DESKTOP_SESSION"]
        .iter()
        .filter_map(|k| env::var(k).ok())
        .flat_map(|v| {
            v.split(|c| c == ':' || c == ';')
                .map(|s| s.trim().to_lowercase())
                .collect::<Vec<_>>()
        })
        .any(|d| NAMES.contains(&d.as_str()))
}

#[cfg(not(target_os = "linux"))]
pub fn is_tiling_wm() -> bool {
    false
}

pub fn ensure_games_marker(create: bool) {
    let dir = games_dir();
    if !dir.exists() {
        if !create || fs::create_dir_all(&dir).is_err() {
            return;
        }
    }
    if create && !running_path().exists() {
        let _ = fs::write(running_path(), "[]");
    }
}

pub fn is_fake_copy() -> bool {
    let Ok(me) = env::current_exe() else {
        return false;
    };
    for dir in me.ancestors().skip(1).take(12) {
        let is_games = dir
            .file_name()
            .map(|n| n.to_string_lossy().eq_ignore_ascii_case("games"))
            .unwrap_or(false);
        if is_games {
            if let Some(p) = dir.parent() {
                if [
                    "quester_manifest.json",
                    "quester_running.json",
                    "settings.json",
                    "quester_badge_history.json",
                ]
                .iter()
                .any(|f| p.join(f).exists())
                {
                    return true;
                }
            }
        }
    }
    false
}

pub fn launch_fake(g: &Game, lite: bool) -> (Option<u32>, Vec<String>) {
    wait_orphan_scan();
    let lite = lite || is_tiling_wm();
    let mut log: Vec<String> = vec![];
    ensure_games_marker(true);

    let me = match env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            log.push(format!("current_exe error: {e}"));
            return (None, log);
        }
    };

    let base = games_dir();

    for rel in &g.exes {
        let rel = crate::discord::normalize_rel(rel);
        if !rel_is_safe(&rel) {
            log.push(format!("Refused unsafe path: {rel}"));
            continue;
        }
        let target = base.join(&rel);
        let entry = manifest_file_entry(&target);

        if target.exists() {
            match &entry {
                None => {
                    log.push(format!(
                        "Refused: {rel} already exists and was not created by Quester"
                    ));
                    continue;
                }
                Some(e) => {
                    let len = fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
                    if e.size > 0 && len != e.size {
                        log.push(format!(
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
                log.push(format!("Refused: {rel} resolves outside allowed folder"));
                continue;
            }
        }

        if !target.exists() {
            let ok = if lite {
                ensure_sleeper(&target).is_ok()
            } else {
                fs::hard_link(&me, &target).is_ok() || fs::copy(&me, &target).is_ok()
            };
            if !ok {
                log.push(format!("Failed to copy {rel}"));
                continue;
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&target, fs::Permissions::from_mode(0o755));
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
        let mut cmd = Command::new(&target);
        if lite {
            cmd.args(sleeper_args(MANUAL_TTL, false, &g.name));
        } else {
            cmd.arg("--sleep").arg(&g.name);
        }
        return match cmd.quiet().spawn() {
            Ok(mut c) => {
                let pid = c.id();
                thread::spawn(move || {
                    let _ = c.wait();
                });
                log.push(format!("Launched: {rel} (PID {pid})"));
                (Some(pid), log)
            }
            Err(e) => {
                log.push(format!("Failed to launch {rel}: {e}"));
                (None, log)
            }
        };
    }
    (None, log)
}