use crate::{
    config::{exe_dir, games_dir, now_ts, MARKER_FILE, MARKER_TEXT},
    discord::Game,
};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
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

#[derive(Clone, Copy, PartialEq)]
pub enum SteamState {
    Invalid,
    NoDir,
    Ours,
    Foreign,
}

#[cfg(windows)]
fn reg_steam_path() -> Option<PathBuf> {
    let o = Command::new("reg")
        .args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"])
        .quiet()
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&o.stdout);
    for l in s.lines() {
        if let Some(i) = l.find("REG_SZ") {
            let v = l[i + 6..].trim();
            if !v.is_empty() {
                return Some(PathBuf::from(v.replace('/', "\\")));
            }
        }
    }
    None
}

fn push_unique(v: &mut Vec<PathBuf>, p: PathBuf) {
    if !v.contains(&p) {
        v.push(p);
    }
}

#[cfg(windows)]
fn steam_install_roots() -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = vec![];
    if let Some(p) = reg_steam_path() {
        bases.push(p);
    }
    bases.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    bases
}

#[cfg(not(windows))]
fn steam_install_roots() -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = vec![];
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        push_unique(&mut bases, home.join(".steam").join("steam"));
        push_unique(&mut bases, home.join(".local").join("share").join("Steam"));
        push_unique(&mut bases, home.join(".steam").join("root"));
        push_unique(
            &mut bases,
            home.join(".var")
                .join("app")
                .join("com.valvesoftware.Steam")
                .join(".local")
                .join("share")
                .join("Steam"),
        );
    }
    bases
}

#[cfg(windows)]
fn parse_library_path(raw: &str) -> PathBuf {
    PathBuf::from(raw.replace("\\\\", "\\"))
}

#[cfg(not(windows))]
fn parse_library_path(raw: &str) -> PathBuf {
    PathBuf::from(raw)
}

fn steam_roots() -> &'static Vec<PathBuf> {
    static ROOTS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    ROOTS.get_or_init(|| {
        let bases = steam_install_roots();

        let mut libs: Vec<PathBuf> = vec![];
        for b in &bases {
            if !b.join("steamapps").is_dir() {
                continue;
            }
            push_unique(&mut libs, b.clone());
            if let Ok(t) = fs::read_to_string(b.join("steamapps").join("libraryfolders.vdf")) {
                for l in t.lines() {
                    let parts: Vec<&str> = l.split('"').collect();
                    if parts.len() >= 4 && parts[1] == "path" {
                        push_unique(&mut libs, parse_library_path(parts[3]));
                    }
                }
            }
        }
        libs.into_iter()
            .map(|l| l.join("steamapps").join("common"))
            .filter(|c| c.is_dir())
            .collect()
    })
}

fn resolve_common_dir(root: &Path, folder: &str) -> PathBuf {
    let direct = root.join(folder);
    if direct.exists() {
        return direct;
    }
    #[cfg(not(windows))]
    {
        if let Ok(rd) = fs::read_dir(root) {
            for e in rd.flatten() {
                if e.file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(folder)
                {
                    return e.path();
                }
            }
        }
    }
    direct
}

fn inside_root(p: &Path, root: &Path) -> bool {
    match (fs::canonicalize(p), fs::canonicalize(root)) {
        (Ok(a), Ok(b)) => a.starts_with(&b),
        _ => false,
    }
}

fn steam_loc(g: &Game) -> Option<(PathBuf, PathBuf)> {
    let folder = crate::discord::normalize_rel(&g.folder);
    if folder.is_empty() || !rel_is_safe(&folder) {
        return None;
    }
    let roots = steam_roots();
    for r in roots {
        let d = resolve_common_dir(r, &folder);
        if d.exists() {
            return Some((r.clone(), d));
        }
    }
    roots.first().map(|r| (r.clone(), r.join(&folder)))
}

pub fn steam_dir(g: &Game) -> Option<PathBuf> {
    steam_loc(g).map(|x| x.1)
}

fn marker_ok(dir: &Path) -> bool {
    fs::read_to_string(dir.join(MARKER_FILE))
        .map(|s| s.trim_start_matches('\u{feff}').trim() == MARKER_TEXT)
        .unwrap_or(false)
}

pub fn steam_state(g: &Game) -> SteamState {
    let Some((root, dir)) = steam_loc(g) else {
        return SteamState::Invalid;
    };
    if !dir.exists() {
        SteamState::NoDir
    } else if inside_root(&dir, &root) && marker_ok(&dir) {
        SteamState::Ours
    } else {
        SteamState::Foreign
    }
}

pub fn steam_needs_confirm(g: &Game) -> bool {
    let Some(dir) = steam_dir(g) else {
        return true;
    };
    steam_state(g) != SteamState::Ours
        || !g
            .exes
            .iter()
            .any(|r| rel_is_safe(r) && dir.join(r).exists())
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

#[cfg(not(windows))]
pub fn alive_pids() -> Option<Vec<u32>> {
    let mut pids = Vec::new();
    for entry in fs::read_dir("/proc").ok()?.flatten() {
        if let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() {
            if entry.path().join("stat").is_file() {
                pids.push(pid);
            }
        }
    }
    Some(pids)
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
    for root in steam_roots() {
        let Ok(r) = fs::canonicalize(root) else { continue };
        if !p.starts_with(&r) {
            continue;
        }
        for anc in p.ancestors().skip(1) {
            if anc == r || !anc.starts_with(&r) {
                break;
            }
            if marker_ok(anc) {
                return true;
            }
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

pub fn launch_fake(g: &Game) -> (Option<u32>, Vec<String>) {
    let mut log = vec![];

    let me = match env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            log.push(format!("current_exe error: {e}"));
            return (None, log);
        }
    };

    let folder = crate::discord::normalize_rel(&g.folder);
    if !folder.is_empty() && !rel_is_safe(&folder) {
        log.push(format!("Refused unsafe folder: {}", g.folder));
        return (None, log);
    }
    let mut base = if folder.is_empty() {
        games_dir()
    } else {
        games_dir().join(&folder)
    };

    let steam = g.is_steam();
    let mut steam_root: Option<PathBuf> = None;
    if steam {
        let Some((root, dir)) = steam_loc(g) else {
            log.push("Refused: Steam folder not found or unsafe game folder name".to_string());
            return (None, log);
        };
        match steam_state(g) {
            SteamState::Foreign => {
                log.push(format!(
                    "Refused: {} exists and has no valid {MARKER_FILE}",
                    dir.display()
                ));
                return (None, log);
            }
            SteamState::NoDir => {
                if fs::create_dir_all(&dir).is_err() || !inside_root(&dir, &root) {
                    log.push(format!("Failed to create folder {}", dir.display()));
                    return (None, log);
                }
                if fs::write(dir.join(MARKER_FILE), MARKER_TEXT).is_err() {
                    let _ = fs::remove_dir(&dir);
                    log.push(format!("Failed to write {MARKER_FILE}"));
                    return (None, log);
                }
                log.push(format!("Created folder: {}", dir.display()));
            }
            SteamState::Ours => {}
            SteamState::Invalid => return (None, log),
        }
        base = dir.clone();
        steam_root = Some(root);
    }

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
            if steam {
                let _ = fs::create_dir_all(p);
            } else {
                create_dirs_tracked(p, &g.name);
            }
            let outside = match &steam_root {
                Some(r) => !inside_root(p, r),
                None => !inside_games(p),
            };
            if outside {
                log.push(format!("Refused: {rel} resolves outside allowed folder"));
                continue;
            }
        }
        if fs::copy(&me, &target).is_err() && !target.exists() {
            log.push(format!("Failed to copy {rel}"));
            continue;
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
        return match Command::new(&target).arg("--sleep").arg(&g.name).quiet().spawn() {
            Ok(c) => {
                let pid = c.id();
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