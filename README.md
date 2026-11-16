# Quester

<p>
  <a href="https://github.com/pyosek/quester/releases"><img src="https://img.shields.io/github/downloads/pyosek/quester/total?style=flat&color=21262d&labelColor=30363d&label=downloads" alt="downloads"></a>
  <a href="https://discord.gg/5dGwUu8HaX"><img src="https://img.shields.io/badge/Discord-Join%20Server-5865F2?style=flat&logo=discord&logoColor=white" alt="Discord"></a>
</p>

A utility that fakes game launches so Discord detects you as "playing" a game. Lets you complete Discord quests and farm the Games Played / Hours Played badges without installing the games

## How it works
Quester downloads Discord's list of detectable games. When you press **Run**, it copies itself under the game's executable name into a `games` folder next to the app and starts it as a small window. Discord sees that process and shows the game as running. No real game files are downloaded.
## Features

- **Fake quests** — Quester tricks Discord into thinking you're playing a game, so you can complete "Play a game on Desktop" quests in seconds.
- **Farm games** — Get the "Games Played" badge by cycling through dozens of different titles.
- **Farm hours** — Accumulate the total hours played on your Discord profile automatically. Just set the timer and let Quester do the work.
## Linux
Requirements: a Rust toolchain and the usual desktop/OpenGL libraries (X11 or Wayland). Build and run with:

```sh
cargo build --release
./target/release/quester
```

The binary is self-contained; it does not need the repository assets at runtime. On Debian/Ubuntu the build dependencies are `libxkbcommon-dev libwayland-dev libx11-dev libxi-dev libgl1-mesa-dev`.

## Disclaimer
This tool may violate Discord's Terms of Service and could put your account at risk. Use at your own risk. Not affiliated with or endorsed by Discord. Download only from this repository.

## Screenshot
<img width="352" height="552" alt="image" src="https://github.com/user-attachments/assets/937b661c-4069-4584-8b12-65d61b8bd2e7" />

## 
<a href="https://star-history.com/#pyosek/quester&Date">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=pyosek/quester&type=Date&theme=dark" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=pyosek/quester&type=Date" />
    <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=pyosek/quester&type=Date" />
  </picture>
</a>
