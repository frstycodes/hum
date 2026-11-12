//! Installs VB-Audio Virtual Cable from inside the app: download VB-Audio's driver
//! pack, unzip it with Windows' own tar, run its (signed) setup elevated, then
//! switch the output to the cable as soon as Windows lists it.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED};
use windows::Win32::System::Com::Urlmon::URLDownloadToFileW;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL};
use windows::core::{w, HSTRING, PCWSTR};

use crate::server::{self, Install};

/// Newest pack first; VB-Audio keeps older packs up.
const PACKS: [&str; 2] = [
    "https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack46.zip",
    "https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip",
];
static BUSY: AtomicBool = AtomicBool::new(false);

pub fn start() {
    work(run);
}

/// VB-Cable is installed but its playback device is missing (Windows can leave it
/// "phantom" after a restart): restart its driver, which brings the device back.
pub fn repair() {
    work(|| {
        server::update(|i| i.install = Install::Repairing);
        let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let ps = root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        elevate(
            &ps,
            "-NoProfile -NonInteractive -WindowStyle Hidden -Command \"Get-PnpDevice -Class MEDIA | Where-Object FriendlyName -like '*VB-Audio Virtual Cable*' | ForEach-Object { pnputil /restart-device $_.InstanceId }\"",
            true,
        )
    });
}

/// Runs `job` on its own thread, then waits for the cable to show up; whatever the
/// job left behind becomes the install state.
fn work(job: fn() -> Result<(), String>) {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let result = job();
        server::update(|i| {
            i.install = match &result {
                Ok(_) => Install::Restart, // replaced as soon as the cable shows up
                Err(e) => Install::Failed(e.clone()),
            }
        });
        if result.is_ok() {
            // a fresh or restarted driver can take a moment to appear
            for _ in 0..10 {
                if server::rescan() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1000));
            }
        }
        BUSY.store(false, Ordering::SeqCst);
    });
}

fn run() -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    server::update(|i| i.install = Install::Downloading);
    let dir = std::env::temp_dir().join("hum-vbcable");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let zip = dir.join("vbcable.zip");
    let downloaded = PACKS.iter().any(|url| unsafe { URLDownloadToFileW(None, &HSTRING::from(*url), &HSTRING::from(zip.as_os_str()), 0, None).is_ok() });
    if !downloaded {
        return Err("Couldn't download VB-Cable. Check your connection and try again.".into());
    }
    unzip(&zip, &dir)?;
    let setup = setup_exe(&dir).ok_or("The VB-Cable download didn't contain its setup.")?;
    server::update(|i| i.install = Install::Installing);
    elevate(&setup, "-i -h", false) // install, without its window
}

fn unzip(zip: &Path, dir: &Path) -> Result<(), String> {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let ok = std::process::Command::new(root.join(r"System32\tar.exe"))
        .arg("-xf")
        .arg(zip)
        .arg("-C")
        .arg(dir)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok { Ok(()) } else { Err("Couldn't unpack the VB-Cable download.".into()) }
}

fn setup_exe(dir: &Path) -> Option<PathBuf> {
    let arm = std::env::var("PROCESSOR_ARCHITECTURE").is_ok_and(|a| a.eq_ignore_ascii_case("ARM64"));
    let names: &[&str] = if arm { &["VBCABLE_Setup_ARM64.exe", "VBCABLE_Setup_x64.exe"] } else { &["VBCABLE_Setup_x64.exe", "VBCABLE_Setup.exe"] };
    names.iter().map(|n| dir.join(n)).find(|p| p.exists())
}

/// Runs `exe` as administrator (Windows asks first) and waits for it.
pub fn elevate(exe: &Path, params: &str, hidden: bool) -> Result<(), String> {
    let file = HSTRING::from(exe.as_os_str());
    let dir = HSTRING::from(exe.parent().unwrap_or(exe).as_os_str());
    let params = HSTRING::from(params);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        lpDirectory: PCWSTR(dir.as_ptr()),
        nShow: if hidden { SW_HIDE.0 } else { SW_SHOWNORMAL.0 },
        ..Default::default()
    };
    unsafe {
        if let Err(e) = ShellExecuteExW(&mut info) {
            return Err(if e.code() == ERROR_CANCELLED.to_hresult() { "Cancelled at the Windows prompt.".into() } else { format!("Couldn't ask Windows: {}", e.message()) });
        }
        if !info.hProcess.is_invalid() {
            WaitForSingleObject(info.hProcess, INFINITE);
            let _ = CloseHandle(info.hProcess);
        }
    }
    Ok(())
}
