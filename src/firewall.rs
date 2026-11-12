//! Windows Firewall and Hum. The first time Hum listens, Windows asks whether it may
//! take connections; dismissing that (or moving the exe, which counts as a new app)
//! leaves inbound Block rules, and then no phone can reach the page. Hum spots those
//! rules and offers to replace them with one narrow Allow rule.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::server;

const RULES: &str = r"SYSTEM\CurrentControlSet\Services\SharedAccess\Parameters\FirewallPolicy\FirewallRules";

static BUSY: AtomicBool = AtomicBool::new(false);

fn exe() -> String {
    std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()
}

/// True when an active inbound Block rule covers this exe (Block beats any Allow).
pub fn blocked() -> bool {
    let me = exe().to_lowercase();
    let Ok(key) = windows_registry::LOCAL_MACHINE.open(RULES) else { return false };
    let Ok(values) = key.values() else { return false };
    values.filter_map(|(_, v)| String::try_from(v).ok()).any(|rule| {
        let field = |name: &str| rule.split('|').find_map(|f| f.strip_prefix(name).and_then(|f| f.strip_prefix('=')));
        field("Action") == Some("Block")
            && field("Active").is_some_and(|a| a.eq_ignore_ascii_case("TRUE"))
            && field("Dir") == Some("In")
            && field("Protocol").is_none_or(|p| p == "6")
            && field("App").is_some_and(|a| a.to_lowercase() == me)
    })
}

/// Swap the Block rules for one Allow rule (Windows asks first): TCP on Hum's port
/// only, and only from the local network, so a phone on your Wi-Fi gets in and the
/// rest of a public network doesn't.
pub fn allow() {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    server::update(|i| i.firewall = server::Firewall::Allowing);
    std::thread::spawn(|| {
        let path = exe().replace('\'', "''");
        let script = format!(
            "Get-NetFirewallApplicationFilter -Program '{path}' | Get-NetFirewallRule | Where-Object {{ $_.Action -eq 'Block' -and $_.Direction -eq 'Inbound' }} | Remove-NetFirewallRule; \
             Get-NetFirewallRule -DisplayName 'Hum' -ErrorAction SilentlyContinue | Remove-NetFirewallRule; \
             New-NetFirewallRule -DisplayName 'Hum' -Description 'Lets your phone reach Hum on your local network.' -Direction Inbound -Action Allow -Program '{path}' -Protocol TCP -LocalPort {port} -RemoteAddress LocalSubnet -Profile Any | Out-Null",
            port = server::PORT
        );
        let root = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_else(|| r"C:\Windows".into());
        let ps = root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        let result = crate::install::elevate(&ps, &format!("-NoProfile -NonInteractive -WindowStyle Hidden -Command \"{script}\""), true);
        let still = blocked();
        server::update(|i| {
            i.firewall = match (&result, still) {
                (_, false) => server::Firewall::Open,
                (Err(e), true) => server::Firewall::Failed(e.clone()),
                (Ok(_), true) => server::Firewall::Failed("Windows still blocks Hum.".into()),
            }
        });
        BUSY.store(false, Ordering::SeqCst);
    });
}

/// Look again (the user may have changed it in Windows Security meanwhile).
pub fn recheck() {
    if BUSY.load(Ordering::SeqCst) {
        return;
    }
    let now = if blocked() { server::Firewall::Blocked } else { server::Firewall::Open };
    let changed = {
        let i = server::INFO.lock().unwrap();
        !matches!((&i.firewall, &now), (server::Firewall::Open, server::Firewall::Open) | (server::Firewall::Blocked, server::Firewall::Blocked) | (server::Firewall::Failed(_), server::Firewall::Blocked))
    };
    if changed {
        server::update(|i| i.firewall = now);
    }
}
