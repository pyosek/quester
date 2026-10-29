use crate::{
    config::{exe_dir, games_dir, now_ts},
    discord::Game,
};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
};

const CREATE_NO_WINDOW: u32 = 0x08000000;

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

fn norm_path(s: &str) -> String {
    s.replace('/', "\\")
        .to_lowercase()
        .trim_end_matches('\\')
        .to_string()
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

pub fn cleanup_manifest() -> (usize, usize, Vec<String>) {
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

pub fn alive_pids() -> Option<Vec<u32>> {
    let o = Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
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

pub fn pid_is_ours(pid: u32) -> bool {
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

pub fn kill_pid(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
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

    if !g.folder.is_empty() && !rel_is_safe(&g.folder) {
        log.push(format!("Refused unsafe folder: {}", g.folder));
        return (None, log);
    }
    let base = if g.folder.is_empty() {
        games_dir()
    } else {
        games_dir().join(&g.folder)
    };

    for rel in &g.exes {
        if !rel_is_safe(rel) {
            log.push(format!("Refused unsafe path: {rel}"));
            continue;
        }
        let target = base.join(rel);
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
                log.push(format!("Refused: {rel} resolves outside games folder"));
                continue;
            }
        }
        if fs::copy(&me, &target).is_err() && !target.exists() {
            log.push(format!("Failed to copy {rel}"));
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
        return match Command::new(&target).arg("--sleep").arg(&g.name).spawn() {
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
