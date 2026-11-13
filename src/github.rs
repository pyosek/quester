use crate::{
    config::{Msg, Notifier, GITHUB_DB_URL, UA},
    discord::{normalize_rel, Game},
};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT_ENCODING};
use std::{thread, time::Duration};

fn fetch(n: Notifier) {
    let mut h = HeaderMap::new();
    h.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    let client = match reqwest::blocking::Client::builder()
        .user_agent(UA)
        .default_headers(h)
        .timeout(Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            n.send(Msg::Log(format!("GitHub DB client error: {e}")));
            return;
        }
    };
    let text = match client.get(GITHUB_DB_URL).send().and_then(|r| r.text()) {
        Ok(t) => t,
        Err(e) => {
            n.send(Msg::Log(format!("GitHub DB network error: {e}")));
            return;
        }
    };
    match serde_json::from_str::<Vec<Game>>(&text) {
        Ok(mut games) => {
            games.retain(|g| !g.exes.is_empty());
            for g in games.iter_mut() {
                for e in g.exes.iter_mut() {
                    *e = normalize_rel(e);
                }
                if g.name.trim().is_empty() {
                    g.name = format!("{}/{}", g.folder, g.exes[0]);
                }
                g.lname = g.name.to_lowercase();
            }
            games.sort_by(|a, b| a.lname.cmp(&b.lname));
            n.send(Msg::GithubGames(games));
        }
        Err(e) => n.send(Msg::Log(format!("GitHub DB JSON error: {e}"))),
    }
}

pub fn spawn_refresh(n: &Notifier) {
    let n = n.clone();
    thread::spawn(move || fetch(n));
}