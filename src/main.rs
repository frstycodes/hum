//! Hum: a window with a QR code; the phone that scans it streams its microphone
//! here, and it plays into VB-Audio's virtual cable so apps can use it as a mic.
//!
//! Usage: hum [output-device-substring]   (default "CABLE Input")

#![windows_subsystem = "windows"]

mod firewall;
mod gfx;
mod install;
mod logo;
mod paint;
mod server;
mod window;

use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, MessageBoxW, TranslateMessage, MB_ICONERROR, MSG};
use windows::core::{w, HSTRING};

fn main() {
    // Already running (perhaps hidden in the tray): bring that one up instead.
    unsafe {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};
        if let Ok(h) = FindWindowW(w!("Hum"), windows::core::PCWSTR::null()) {
            if !h.is_invalid() {
                let _ = PostMessageW(Some(h), window::WM_SHOW_HUM, WPARAM(0), LPARAM(0));
                return;
            }
        }
    }
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    if let Err(e) = window::create() {
        unsafe { MessageBoxW(None, &HSTRING::from(format!("Hum couldn't open its window: {e}")), w!("Hum"), MB_ICONERROR) };
        return;
    }
    server::start();
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
