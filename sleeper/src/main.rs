#![cfg_attr(windows, windows_subsystem = "windows")]

use std::{env, thread, time::Duration};

fn main() {
    let a: Vec<String> = env::args().collect();
    let ttl: u64 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(120);
    let hidden = a.get(2).map(|s| s == "1").unwrap_or(false);
    let tiling = a.get(3).map(|s| s == "1").unwrap_or(false);
    let title = if a.len() > 4 {
        a[4..].join(" ")
    } else {
        "Game".to_string()
    };
    run(title, ttl, hidden, tiling);
}

#[cfg(windows)]
fn run(title: String, ttl: u64, hidden: bool, _tiling: bool) {
    use std::ffi::c_void;

    #[repr(C)]
    struct WndClassW {
        style: u32,
        wnd_proc: unsafe extern "system" fn(isize, u32, usize, isize) -> isize,
        cls_extra: i32,
        wnd_extra: i32,
        instance: isize,
        icon: isize,
        cursor: isize,
        background: isize,
        menu_name: *const u16,
        class_name: *const u16,
    }
    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[repr(C)]
    struct MsgW {
        hwnd: isize,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt: Point,
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassW(c: *const WndClassW) -> u16;
        fn CreateWindowExW(
            ex: u32,
            class: *const u16,
            name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: isize,
            menu: isize,
            instance: isize,
            param: *const c_void,
        ) -> isize;
        fn ShowWindow(h: isize, cmd: i32) -> i32;
        fn DefWindowProcW(h: isize, m: u32, w: usize, l: isize) -> isize;
        fn GetMessageW(msg: *mut MsgW, h: isize, min: u32, max: u32) -> i32;
        fn TranslateMessage(msg: *const MsgW) -> i32;
        fn DispatchMessageW(msg: *const MsgW) -> isize;
        fn PostQuitMessage(code: i32);
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> isize;
        fn GetCurrentProcess() -> isize;
        fn SetProcessWorkingSetSize(h: isize, min: usize, max: usize) -> i32;
    }
    #[link(name = "shell32")]
    extern "system" {
        fn SetCurrentProcessExplicitAppUserModelID(app_id: *const u16) -> i32;
    }

    unsafe extern "system" fn wnd_proc(h: isize, m: u32, w: usize, l: isize) -> isize {
        if m == 0x0002 {
            PostQuitMessage(0);
            return 0;
        }
        DefWindowProcW(h, m, w, l)
    }

    unsafe {
        let id: Vec<u16> = "Quester.App"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        SetCurrentProcessExplicitAppUserModelID(id.as_ptr());
    }

    thread::spawn(move || {
        thread::sleep(Duration::from_secs(ttl));
        std::process::exit(0);
    });

    thread::spawn(|| loop {
        thread::sleep(Duration::from_secs(5));
        unsafe {
            SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
        }
    });

    unsafe {
        let inst = GetModuleHandleW(std::ptr::null());
        let class: Vec<u16> = "QuesterLite\0".encode_utf16().collect();
        let name: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let wc = WndClassW {
            style: 0,
            wnd_proc,
            cls_extra: 0,
            wnd_extra: 0,
            instance: inst,
            icon: 0,
            cursor: 0,
            background: 0,
            menu_name: std::ptr::null(),
            class_name: class.as_ptr(),
        };
        if RegisterClassW(&wc) == 0 {
            return;
        }
        let use_default = 0x8000_0000u32 as i32;
        let (ex, pos) = if hidden {
            (0x0000_0080u32 | 0x0800_0000u32, -32000)
        } else {
            (0, use_default)
        };
        let hwnd = CreateWindowExW(
            ex,
            class.as_ptr(),
            name.as_ptr(),
            0x00CF_0000,
            pos,
            pos,
            320,
            120,
            0,
            0,
            inst,
            std::ptr::null(),
        );
        if hwnd == 0 {
            return;
        }
        ShowWindow(hwnd, if hidden { 4 } else { 7 });
        let mut msg: MsgW = std::mem::zeroed();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(not(windows))]
fn run(_title: String, ttl: u64, _hidden: bool, tiling: bool) {
    extern "C" {
        fn getppid() -> i32;
    }
    if tiling {
        let parent = unsafe { getppid() };
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(1));
            if unsafe { getppid() } != parent {
                std::process::exit(0);
            }
        });
    }
    thread::sleep(Duration::from_secs(ttl));
}