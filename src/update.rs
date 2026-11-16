use crate::config::{Msg, Notifier, RELEASES_API, RELEASES_PAGE, UA, VERSION};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_ENCODING};
use serde_json::Value;
use std::{thread, time::Duration};

fn parse_ver(v: &str) -> Vec<u64> {
    v.trim()
        .trim_start_matches(['v', 'V'])
        .split('.')
        .map(|p| {
            p.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .collect()
}

fn is_newer(remote: &str, local: &str) -> bool {
    let mut a = parse_ver(remote);
    let mut b = parse_ver(local);
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

fn check() -> Result<Option<(String, String)>, String> {
    let mut h = HeaderMap::new();
    h.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    h.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    let client = reqwest::blocking::Client::builder()
        .user_agent(UA)
        .default_headers(h)
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("client: {e}"))?;
    let resp = client
        .get(RELEASES_API)
        .send()
        .map_err(|e| format!("network: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let text = resp.text().map_err(|e| format!("read body: {e}"))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("json: {e}"))?;
    let tag = v["tag_name"].as_str().ok_or("no tag_name")?.to_string();
    let url = v["html_url"].as_str().unwrap_or(RELEASES_PAGE).to_string();
    Ok(if is_newer(&tag, VERSION) {
        Some((tag, url))
    } else {
        None
    })
}

pub fn spawn_check(n: &Notifier) {
    let n = n.clone();
    thread::spawn(move || n.send(Msg::UpdateChecked(check())));
}