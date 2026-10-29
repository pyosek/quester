#![windows_subsystem = "windows"]

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