use std::{env, fs, path::PathBuf, process::Command};

fn build_sleeper() {
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let manifest = root.join("sleeper").join("Cargo.toml");
    let target_dir = out.join("sleeper-target");
    let target = env::var("TARGET").expect("TARGET");
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());

    let status = Command::new(cargo)
        .args(["build", "--release", "--manifest-path"])
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&target_dir)
        .args(["--target", &target])
        .env_remove("CARGO_MAKEFLAGS")
        .env_remove("MAKEFLAGS")
        .env_remove("MFLAGS")
        .status()
        .expect("failed to run cargo for sleeper");
    assert!(status.success(), "sleeper build failed");

    let exe = if target.contains("windows") {
        "sleeper.exe"
    } else {
        "sleeper"
    };
    let built = target_dir.join(&target).join("release").join(exe);
    fs::copy(&built, out.join("sleeper.bin")).expect("copy sleeper.bin");

    println!("cargo:rerun-if-changed=sleeper/src/main.rs");
    println!("cargo:rerun-if-changed=sleeper/Cargo.toml");
    println!("cargo:rerun-if-changed=sleeper/build.rs");
}

fn main() {
    build_sleeper();

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Quester");
        res.set("FileDescription", "Quester");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon not embedded: {e}");
        }
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
}