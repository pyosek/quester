#![cfg_attr(windows, windows_subsystem = "windows")]

mod badge;
mod config;
mod discord;
mod github;
mod gui;
mod process;

use std::env;

fn main() {
    config::install_crash_log();
    config::set_app_id();

    let args: Vec<String> = env::args().collect();

    if args.get(1).map(String::as_str) == Some("--sleep-lite") {
        let ttl = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(120);
        let title = if args.len() > 3 {
            args[3..].join(" ")
        } else {
            "Game".to_string()
        };
        badge::run_lite_window(title, ttl);
        return;
    }

    if args.get(1).map(String::as_str) == Some("--sleep") {
        let title = if args.len() > 2 {
            args[2..].join(" ")
        } else {
            "Game".to_string()
        };
        gui::run_fake_window(title);
        return;
    }

    gui::run_launcher();
}