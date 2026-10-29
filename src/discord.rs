use crate::config::{Msg, Notifier, DISCORD_DB_URL, UA};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{thread, time::Duration};

#[derive(Clone, Serialize, Deserialize)]
pub struct Game {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub folder: String,
    #[serde(alias = "exe")]
    pub exes: Vec<String>,
    #[serde(skip)]
    pub lname: String,
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

fn build_games(apps: &[Value]) -> Vec<Game> {
    let mut out: Vec<Game> = apps
        .iter()
        .filter_map(|a| {
            let name = a["name"].as_str()?.to_string();
            let exes = exes_for_app(a);
            if exes.is_empty() {
                return None;
            }
            Some(Game {
                lname: name.to_lowercase(),
                name,
                folder: String::new(),
                exes,
            })
        })
        .collect();
    out.sort_by(|a, b| a.lname.cmp(&b.lname));
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

fn fetch(n: Notifier) {
    let client = match reqwest::blocking::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            n.send(Msg::Log(format!("Discord DB client error: {e}")));
            return;
        }
    };
    match client.get(DISCORD_DB_URL).send() {
        Ok(r) => {
            if !r.status().is_success() {
                n.send(Msg::Log(format!("Discord DB HTTP error: {}", r.status())));
                return;
            }
            match safe_json_parse(r) {
                Ok(val) => match val.as_array() {
                    Some(apps) => n.send(Msg::DiscordGames(build_games(apps))),
                    None => n.send(Msg::Log("Discord DB: invalid JSON".to_string())),
                },
                Err(e) => n.send(Msg::Log(format!("Discord DB error: {e}"))),
            }
        }
        Err(e) => n.send(Msg::Log(format!("No network (Discord): {e}"))),
    }
}

pub fn spawn_refresh(n: &Notifier) {
    let n = n.clone();
    thread::spawn(move || fetch(n));
}
