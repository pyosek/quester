# Quester

> Formerly known as **Yarok Launcher**.

A small utility that fakes game launches so Discord detects you as "playing" a game. Lets you complete Discord quests without installing the games.

## How it works
Quester downloads Discord's list of detectable games. When you press **Run**, it copies itself under the game's executable name into a `games` folder next to the app and starts it as a small window. Discord sees that process and shows the game as running. No real game files are downloaded.

On Windows, Steam games can optionally be placed in the matching Steam library folder (`steamapps/common/<game>`) after confirmation.
On Linux, the same works for native, distro, and Flatpak Steam libraries discovered under `~/.steam`, `~/.local/share/Steam`, and `~/.var/app/com.valvesoftware.Steam`. Quester writes a `quester.txt` marker into folders it creates and refuses to overwrite folders or files it did not create.

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
