//! Hum's window, built the way Stash's is: a Win32 window with Mica edge to edge and
//! the caption folded in, every part a composition sprite painted with Direct2D.
//! Nothing is boxed. While waiting: a big QR in the text ink, the link, a breathing dot.
//! Once a phone streams, the QR shrinks (another phone can still scan it) and the level
//! meter and history slide in; the compositor runs those moves, and only the live parts
//! repaint, 30 times a second, and only while a phone is connected.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;

use windows::Foundation::TimeSpan;
use windows::UI::Color;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{
    AnimationIterationBehavior, CompositionStretch, ContainerVisual, ICompositionAnimationBase, ImplicitAnimationCollection, SpriteVisual, Visual,
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::{D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE};
use windows::Win32::Graphics::Dwm::{DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT};
use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, NOTIFY_ICON_MESSAGE};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{w, Interface, Result, HSTRING, PCWSTR};
use windows_numerics::{Vector2, Vector3};

use crate::gfx::Gfx;
use crate::paint::{self, P, MEDIUM, REGULAR, SEMIBOLD};
use crate::server::{self, Install, INFO, PEAK};

// Layout, in DIPs (the prototype's numbers: design/hum-prototype.html).
const W: f32 = 380.0;
const H: f32 = 616.0;
const CAP_W: f32 = 46.0;
const CAP_H: f32 = 32.0;
const SIDE: f32 = 24.0;
const QR: f32 = 212.0;
const QR_Y: f32 = 82.0;
const QR_LIVE_SCALE: f32 = 0.62;
const SCAN_H: f32 = 64.0;
const STATUS_H: f32 = 24.0;
const LIVE_W: f32 = W - SIDE * 2.0;
const LIVE_H: f32 = 82.0; // meter 8, gap 12, history 62
const FOOT_Y: f32 = 542.0;
const FOOT_H: f32 = 52.0;
const BTN_W: f32 = 132.0;
const BTN_H: f32 = 32.0;
const BTN_X: f32 = W - 20.0 - BTN_W;
const BTN_Y: f32 = FOOT_Y + (FOOT_H - BTN_H) / 2.0;
// the Allow in Firewall button, under the scan text (only while waiting)
const FW_W: f32 = 150.0;
const FW_GAP: f32 = 4.0;
// Stash's switch, at desktop size: a ~40×22 capsule track, a wider-than-tall knob
const SWITCH_W: f32 = 40.0;
const SWITCH_H: f32 = 22.0;
const HISTORY: usize = 120; // 4 s at 30 fps
const WM_NCMOUSELEAVE: u32 = 0x02A2;
const WM_MOUSELEAVE: u32 = 0x02A3;
/// The tray icon's callback, and "show yourself" from a second launch.
const WM_TRAY: u32 = 0x8000 + 2;
pub const WM_SHOW_HUM: u32 = 0x8000 + 3;
const MENU_OPEN: usize = 1;
const MENU_QUIT: usize = 2;
/// Sent to every window when Explorer (and the taskbar) restarts.
static TASKBAR_CREATED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Where things sit while waiting, and while a phone streams.
struct Spots {
    scan_y: f32,
    status_y: f32,
    live_y: f32,
}
const WAITING: Spots = Spots { scan_y: 316.0, status_y: 418.0, live_y: 450.0 };
const STREAMING: Spots = Spots { scan_y: 226.0, status_y: 330.0, live_y: 362.0 };

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    None,
    Min,
    Close,
    Install,
    Firewall,
    Listen,
}

struct Ui {
    hwnd: HWND,
    gfx: Gfx,
    k: f32,
    mica: bool,
    _target: DesktopWindowTarget,
    backdrop: SpriteVisual,
    head: SpriteVisual,
    mark: SpriteVisual,
    qr: SpriteVisual,
    scan: SpriteVisual,
    status: ContainerVisual,
    dot: SpriteVisual,
    status_text: SpriteVisual,
    live: SpriteVisual,
    foot: SpriteVisual,
    caption: SpriteVisual,
    hover: Hit,
    press: Option<u32>,
    pressed: bool,
    tracking: bool,
    level: f32,
    history: VecDeque<f32>,
    ticking: bool,
    breathing: Option<bool>,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|u| u.try_borrow_mut().ok().and_then(|mut u| u.as_mut().map(f)))
}

fn system_light() -> bool {
    windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|k| k.get_u32("AppsUseLightTheme"))
        .is_ok_and(|v| v != 0)
}

fn streaming() -> bool {
    INFO.lock().unwrap().phones > 0
}

/// The VB-Cable button's label and whether it can be pressed, or None when there's
/// nothing to do. A missing cable gets Install; an installed one whose playback
/// device didn't come up gets Repair.
fn install_button() -> Option<(&'static str, bool)> {
    let i = INFO.lock().unwrap();
    if i.cable || i.device.is_empty() {
        return None;
    }
    match i.install {
        Install::Idle if i.broken => Some(("Repair VB-Cable", true)),
        Install::Idle => Some(("Install VB-Cable", true)),
        Install::Downloading => Some(("Downloading…", false)),
        Install::Installing => Some(("Installing…", false)),
        Install::Repairing => Some(("Repairing…", false)),
        Install::Failed(_) => Some(("Try again", true)),
        Install::Restart => None,
    }
}

/// The Allow in Firewall button's label and whether it can be pressed, or None when
/// phones can get in (or one is already streaming).
fn firewall_button() -> Option<(&'static str, bool)> {
    let i = INFO.lock().unwrap();
    if i.phones > 0 {
        return None;
    }
    match i.firewall {
        server::Firewall::Open => None,
        server::Firewall::Blocked => Some(("Allow in Firewall", true)),
        server::Firewall::Allowing => Some(("Allowing…", false)),
        server::Firewall::Failed(_) => Some(("Try again", true)),
    }
}

fn ms(n: i64) -> TimeSpan {
    TimeSpan { Duration: n * 10_000 }
}

// ---------- the tray icon ----------

fn wide<const N: usize>(s: &str) -> [u16; N] {
    let mut out = [0u16; N];
    for (o, c) in out.iter_mut().zip(s.encode_utf16().take(N - 1)) {
        *o = c;
    }
    out
}

fn tray_tip() -> String {
    match INFO.lock().unwrap().phones {
        0 => "Hum: waiting for your phone".into(),
        1 => "Hum: phone connected".into(),
        n => format!("Hum: {n} phones connected"),
    }
}

/// Adds the tray icon (`NIM_ADD`), updates its tooltip (`NIM_MODIFY`) or removes it.
fn tray(hwnd: HWND, op: NOTIFY_ICON_MESSAGE) {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let cx = GetSystemMetrics(SM_CXSMICON);
        let icon = LoadImageW(Some(instance.into()), PCWSTR(1 as _), IMAGE_ICON, cx, cx, LR_SHARED).ok().map(|h| HICON(h.0)).unwrap_or_default();
        let data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: WM_TRAY,
            hIcon: icon,
            szTip: wide(&tray_tip()),
            ..Default::default()
        };
        let _ = Shell_NotifyIconW(op, &data);
    }
}

/// The first time Hum hides to the tray, say where it went (once ever).
fn tray_hint(hwnd: HWND) {
    let key = windows_registry::CURRENT_USER.create(r"Software\Hum");
    if key.as_ref().is_ok_and(|k| k.get_u32("TrayHintShown").is_ok()) {
        return;
    }
    if let Ok(k) = key {
        let _ = k.set_u32("TrayHintShown", 1);
    }
    unsafe {
        let data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_INFO,
            szInfoTitle: wide("Hum is still running"),
            szInfo: wide("Your phone keeps working as a mic. Open Hum or quit it from here."),
            ..Default::default()
        };
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}

fn show(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, if IsIconic(hwnd).as_bool() { SW_RESTORE } else { SW_SHOW });
        let _ = SetForegroundWindow(hwnd);
    }
    // the live parts don't repaint while hidden
    with(|u| {
        let _ = u.paint_live();
        let _ = u.paint_mark();
    });
}

fn tray_menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING, MENU_OPEN, w!("Open Hum"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, MENU_QUIT, w!("Quit Hum"));
        let _ = SetMenuDefaultItem(menu, MENU_OPEN as u32, 0);
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // the menu closes on a click elsewhere only if our window is in front
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, None, hwnd, None);
        let _ = DestroyMenu(menu);
        match cmd.0 as usize {
            MENU_OPEN => show(hwnd),
            MENU_QUIT => {
                let _ = DestroyWindow(hwnd);
            }
            _ => {}
        }
    }
}

pub fn create() -> Result<HWND> {
    paint::set_light(system_light());
    let gfx = Gfx::new()?;
    let hwnd = unsafe {
        let instance = GetModuleHandleW(None)?;
        let icon = |cx: i32| LoadImageW(Some(instance.into()), PCWSTR(1 as _), IMAGE_ICON, cx, cx, LR_SHARED).ok().map(|h| HICON(h.0)).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hIcon: icon(GetSystemMetrics(SM_CXICON)),
            hIconSm: icon(GetSystemMetrics(SM_CXSMICON)),
            lpszClassName: w!("Hum"),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let k = windows::Win32::UI::HiDpi::GetDpiForSystem() as f32 / 96.0;
        CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_APPWINDOW,
            w!("Hum"),
            w!("Hum"),
            WINDOW_STYLE(WS_OVERLAPPEDWINDOW.0 & !WS_SYSMENU.0 & !WS_MAXIMIZEBOX.0),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            (W * k) as i32,
            (H * k) as i32,
            None,
            None,
            Some(instance.into()),
            None,
        )?
    };
    // Dark frame, Mica over the whole window (Windows 11), the caption folded into it.
    let mica = unsafe {
        let dark: u32 = (!paint::light()) as u32;
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &dark as *const _ as _, 4);
        let backdrop: u32 = 2; // DWMSBT_MAINWINDOW
        let ok = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(38), &backdrop as *const _ as _, 4).is_ok();
        let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 });
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER);
        ok
    };
    let c = &gfx.compositor;
    let target = gfx.target(hwnd)?;
    let root: ContainerVisual = c.CreateContainerVisual()?;
    target.SetRoot(&root)?;
    let s = || c.CreateSpriteVisual();
    let (backdrop, head, mark, qr, scan, dot, status_text, live, foot, caption) = (s()?, s()?, s()?, s()?, s()?, s()?, s()?, s()?, s()?, s()?);
    let status = c.CreateContainerVisual()?;
    status.Children()?.InsertAtTop(&dot)?;
    status.Children()?.InsertAtTop(&status_text)?;
    let all: [&Visual; 9] = [&backdrop.cast()?, &head.cast()?, &mark.cast()?, &qr.cast()?, &scan.cast()?, &status.cast()?, &live.cast()?, &foot.cast()?, &caption.cast()?];
    for v in all {
        root.Children()?.InsertAtTop(v)?;
    }
    let k = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.0;
    let ui = Ui {
        hwnd,
        gfx,
        k,
        mica,
        _target: target,
        backdrop,
        head,
        mark,
        qr,
        scan,
        status,
        dot,
        status_text,
        live,
        foot,
        caption,
        hover: Hit::None,
        press: None,
        pressed: false,
        tracking: false,
        level: 0.0,
        history: VecDeque::from(vec![0.0; HISTORY]),
        ticking: false,
        breathing: None,
    };
    UI.with(|u| *u.borrow_mut() = Some(ui));
    with(|u| {
        u.paint_all();
        // what's there at first paint doesn't animate; later state changes do
        let _ = u.animate_moves();
    });
    server::NOTIFY.store(hwnd.0 as isize, Ordering::Relaxed);
    unsafe {
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
    tray(hwnd, NIM_ADD);
    Ok(hwnd)
}

impl Ui {
    /// Paint `v` at w×h DIPs (offset left to `place`).
    fn paint(&self, v: &SpriteVisual, w: f32, h: f32, draw: impl FnOnce(&P)) -> Result<()> {
        let k = self.k;
        let dw = self.gfx.dwrite.clone();
        let surface = self.gfx.surface(w * k, h * k, |dc| unsafe {
            dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            draw(&P { dc, dw: &dw, k });
        })?;
        let brush = self.gfx.compositor.CreateSurfaceBrushWithSurface(&surface)?;
        brush.SetStretch(CompositionStretch::Fill)?;
        v.SetBrush(&brush)?;
        v.SetSize(Vector2 { X: (w * k).ceil(), Y: (h * k).ceil() })
    }

    fn at(&self, v: &impl Interface, x: f32, y: f32) -> Result<()> {
        let k = self.k;
        v.cast::<Visual>()?.SetOffset(Vector3 { X: (x * k).round(), Y: (y * k).round(), Z: 0.0 })
    }

    /// Move everything to where it sits in the current state.
    fn place(&mut self) -> Result<()> {
        let live = streaming();
        let s = if live { &STREAMING } else { &WAITING };
        let k = self.k;
        self.at(&self.head, 0.0, 0.0)?;
        self.at(&self.mark, 14.0, 14.0)?;
        self.at(&self.qr, (W - QR) / 2.0, QR_Y)?;
        self.qr.SetCenterPoint(Vector3 { X: QR / 2.0 * k, Y: 0.0, Z: 0.0 })?;
        let q = if live { QR_LIVE_SCALE } else { 1.0 };
        self.qr.SetScale(Vector3 { X: q, Y: q, Z: 1.0 })?;
        self.at(&self.scan, 0.0, s.scan_y)?;
        self.at(&self.status, SIDE, s.status_y)?;
        self.at(&self.dot, -4.0, 0.0)?;
        self.at(&self.status_text, 16.0, 0.0)?;
        self.at(&self.live, SIDE, s.live_y)?;
        self.live.SetOpacity(if live { 1.0 } else { 0.0 })?;
        self.at(&self.foot, 0.0, FOOT_Y)?;
        self.at(&self.caption, W - CAP_W * 2.0, 0.0)?;
        // the waiting dot breathes; a connected one holds still
        if self.breathing != Some(!live) {
            self.breathing = Some(!live);
            let opacity = HSTRING::from("Opacity");
            if live {
                self.dot.StopAnimation(&opacity)?;
                self.dot.SetOpacity(1.0)?;
            } else {
                let c = &self.gfx.compositor;
                let a = c.CreateScalarKeyFrameAnimation()?;
                let ease = c.CreateCubicBezierEasingFunction(Vector2 { X: 0.45, Y: 0.0 }, Vector2 { X: 0.55, Y: 1.0 })?;
                a.InsertKeyFrame(0.0, 1.0)?;
                a.InsertKeyFrameWithEasingFunction(0.5, 0.35, &ease)?;
                a.InsertKeyFrameWithEasingFunction(1.0, 1.0, &ease)?;
                a.SetDuration(ms(2400))?;
                a.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
                self.dot.StartAnimation(&opacity, &a)?;
            }
        }
        Ok(())
    }

    /// From now on, moves between states glide (ease-out, 420 ms) and the live section
    /// fades.
    fn animate_moves(&self) -> Result<()> {
        let c = &self.gfx.compositor;
        let ease = c.CreateCubicBezierEasingFunction(Vector2 { X: 0.23, Y: 1.0 }, Vector2 { X: 0.32, Y: 1.0 })?;
        let collection = |props: &[(&str, i64)]| -> Result<ImplicitAnimationCollection> {
            let set = c.CreateImplicitAnimationCollection()?;
            for (prop, dur) in props {
                let name = HSTRING::from(*prop);
                let anim: ICompositionAnimationBase = if *prop == "Opacity" {
                    let a = c.CreateScalarKeyFrameAnimation()?;
                    a.InsertExpressionKeyFrameWithEasingFunction(1.0, &HSTRING::from("this.FinalValue"), &ease)?;
                    a.SetDuration(ms(*dur))?;
                    a.SetTarget(&name)?;
                    a.cast()?
                } else {
                    let a = c.CreateVector3KeyFrameAnimation()?;
                    a.InsertExpressionKeyFrameWithEasingFunction(1.0, &HSTRING::from("this.FinalValue"), &ease)?;
                    a.SetDuration(ms(*dur))?;
                    a.SetTarget(&name)?;
                    a.cast()?
                };
                set.Insert(&name, &anim)?;
            }
            Ok(set)
        };
        self.qr.SetImplicitAnimations(&collection(&[("Scale", 420)])?)?;
        self.scan.SetImplicitAnimations(&collection(&[("Offset", 420)])?)?;
        self.status.SetImplicitAnimations(&collection(&[("Offset", 420)])?)?;
        self.live.SetImplicitAnimations(&collection(&[("Offset", 420), ("Opacity", 300)])?)
    }

    fn paint_all(&mut self) {
        let _ = self.paint_backdrop();
        let _ = self.paint_head();
        let _ = self.paint_mark();
        let _ = self.paint_qr();
        let _ = self.paint_scan();
        let _ = self.paint_status();
        let _ = self.paint_live();
        let _ = self.paint_foot();
        let _ = self.paint_caption();
        let _ = self.place();
    }

    fn paint_backdrop(&self) -> Result<()> {
        let color = match (self.mica, paint::light()) {
            (true, _) => Color { A: 0, R: 0, G: 0, B: 0 },
            (false, true) => Color { A: 255, R: 243, G: 242, B: 245 },
            (false, false) => Color { A: 255, R: 30, G: 28, B: 33 },
        };
        self.backdrop.SetBrush(&self.gfx.compositor.CreateColorBrushWithColor(color)?)?;
        self.backdrop.SetSize(Vector2 { X: W * self.k, Y: H * self.k })
    }

    fn paint_head(&self) -> Result<()> {
        self.paint(&self.head, 120.0, 52.0, |p| {
            p.text_mid("Hum", 45.0, 28.0, 15.0, SEMIBOLD, paint::text(), 70.0);
        })
    }

    /// The logo, whose bars ride the level while a phone streams.
    fn paint_mark(&self) -> Result<()> {
        let n = self.history.len();
        let lift: [f32; 4] = std::array::from_fn(|i| self.history.get(n.saturating_sub(1 + i * 4)).copied().unwrap_or(0.0) * 0.9);
        self.paint(&self.mark, 28.0, 28.0, |p| p.logo(14.0, 14.0, 22.0, lift))
    }

    fn paint_qr(&self) -> Result<()> {
        let url = INFO.lock().unwrap().url.clone();
        let qr = qrcode::QrCode::with_error_correction_level(url.as_bytes(), qrcode::EcLevel::M).ok().filter(|_| !url.is_empty());
        self.paint(&self.qr, QR, QR, |p| {
            if let Some(qr) = &qr {
                let dark: Vec<bool> = qr.to_colors().into_iter().map(|c| c == qrcode::Color::Dark).collect();
                p.qr(0.0, 0.0, QR, qr.width(), &dark);
            }
        })
    }

    /// The link and how to use it; while Windows Firewall blocks Hum, that instead,
    /// with the button that fixes it.
    fn paint_scan(&self) -> Result<()> {
        let (url, firewall) = {
            let i = INFO.lock().unwrap();
            (i.url.clone(), i.firewall.clone())
        };
        let lead = if streaming() { "Scan to add another phone" } else { "Scan with your phone's camera" };
        let button = firewall_button();
        let (hot, pressed) = (self.hover == Hit::Firewall, self.pressed);
        self.paint(&self.scan, W, SCAN_H + FW_GAP + BTN_H, |p| {
            let cx = W / 2.0;
            p.text_center(lead, cx, 10.0, 14.0, MEDIUM, paint::text(), W - 40.0);
            p.text_center(if url.is_empty() { "Starting…" } else { &url }, cx, 31.0, 13.5, MEDIUM, paint::link(), W - 40.0);
            let (note, ink) = match &firewall {
                // a phone got through, so this isn't the moment for a firewall warning
                _ if button.is_none() && !matches!(firewall, server::Firewall::Failed(_)) => ("Same Wi-Fi. Accept the certificate warning once.".to_string(), paint::faint()),
                server::Firewall::Open => ("Same Wi-Fi. Accept the certificate warning once.".to_string(), paint::faint()),
                server::Firewall::Blocked => ("Windows Firewall is blocking your phone.".into(), paint::warn_text()),
                server::Firewall::Allowing => ("Allow the Windows prompt.".into(), paint::dim()),
                server::Firewall::Failed(e) => (e.clone(), paint::danger_text()),
            };
            p.text_center(&note, cx, 51.0, 12.0, REGULAR, ink, W - 40.0);
            if let Some((label, enabled)) = button {
                p.button(cx - FW_W / 2.0, SCAN_H + FW_GAP, FW_W, BTN_H, label, hot && enabled && !pressed, enabled);
            }
        })
    }

    fn paint_status(&self) -> Result<()> {
        let (phones, rate) = {
            let i = INFO.lock().unwrap();
            (i.phones, i.rate)
        };
        self.paint(&self.dot, 16.0, STATUS_H, |p| {
            if phones > 0 {
                p.dot(7.5, 12.0, 7.5, paint::color(48, 209, 88, 0.2));
                p.dot(7.5, 12.0, 3.5, paint::green());
            } else {
                p.dot(7.5, 12.0, 3.5, paint::faint());
            }
        })?;
        let label = match phones {
            0 => "Waiting for your phone".to_string(),
            1 => "Phone connected".to_string(),
            n => format!("{n} phones connected"),
        };
        let w = LIVE_W - 16.0;
        let (listen, hot) = (server::listening(), self.hover == Hit::Listen);
        self.paint(&self.status_text, w, STATUS_H, |p| {
            let lw = p.text_mid(&label, 0.0, 12.0, 14.0, MEDIUM, paint::text(), w - 120.0);
            if phones > 0 && rate > 0 {
                p.text_mid(&format!("· {} kHz", rate as f32 / 1000.0), lw + 6.0, 12.0, 12.5, MEDIUM, paint::dim(), 70.0);
            }
            // Listen: hear yourself on the speakers (only while a phone streams)
            if phones > 0 {
                let sx = w - SWITCH_W;
                p.text_right("Listen", sx - 8.0, 12.0, 13.0, MEDIUM, if hot || listen { paint::text() } else { paint::dim() });
                p.switch(sx, 12.0 - SWITCH_H / 2.0, SWITCH_W, SWITCH_H, listen, hot);
            }
        })
    }

    /// Listen's label and switch, in window DIPs (only there while a phone streams).
    fn listen_rect() -> (f32, f32, f32, f32) {
        let right = SIDE + LIVE_W;
        (right - SWITCH_W - 56.0, STREAMING.status_y, SWITCH_W + 56.0, STATUS_H)
    }

    /// The live section: the gapped level meter over a few seconds of history.
    fn paint_live(&self) -> Result<()> {
        let (level, history) = (self.level, self.history.iter().copied().collect::<Vec<f32>>());
        self.paint(&self.live, LIVE_W, LIVE_H, |p| {
            p.meter(0.0, 0.0, LIVE_W, 8.0, level);
            p.history(0.0, 20.0, LIVE_W, 62.0, &history);
        })
    }

    /// Where the sound goes, and the way to get VB-Cable when it's missing.
    fn paint_foot(&self) -> Result<()> {
        let (device, cable, broken, install, error) = {
            let i = INFO.lock().unwrap();
            (i.device.clone(), i.cable, i.broken, i.install.clone(), i.error.clone())
        };
        let button = install_button();
        let listen = server::listening();
        let (hot, pressed) = (self.hover == Hit::Install, self.pressed);
        self.paint(&self.foot, W, FOOT_H, |p| {
            let x = SIDE;
            let tw = if button.is_some() { BTN_X - 12.0 - x } else { W - 20.0 - x };
            p.text_mid("Output", x, 8.0, 12.0, MEDIUM, paint::faint(), tw);
            p.text_mid(if device.is_empty() { "…" } else { &device }, x, 27.0, 13.0, MEDIUM, paint::text(), tw);
            let (note, ink) = match (&error, cable, &install) {
                (Some(e), _, _) => (e.clone(), paint::danger_text()),
                // Listen puts you on the speakers, where screen shares can hear you
                _ if listen && button.is_none() => ("Listening on your speakers: screen shares can hear it.".into(), paint::warn_text()),
                _ if listen => ("Screen shares can hear Listen.".into(), paint::warn_text()),
                (None, true, _) => ("In your app, choose “CABLE Output” as the microphone.".into(), paint::dim()),
                (None, false, Install::Idle) if broken => ("VB-Cable needs a repair.".into(), paint::warn_text()),
                (None, false, Install::Idle) => ("Hum needs VB-Cable to be a mic.".into(), paint::warn_text()),
                (None, false, Install::Downloading) => ("Getting the free driver…".into(), paint::dim()),
                (None, false, Install::Installing | Install::Repairing) => ("Allow the Windows prompt.".into(), paint::dim()),
                (None, false, Install::Failed(e)) => (e.clone(), paint::danger_text()),
                (None, false, Install::Restart) => ("Still missing. Restart your PC to finish.".into(), paint::warn_text()),
            };
            p.text_mid(&note, x, 46.0, 12.0, REGULAR, ink, tw);
            if let Some((label, enabled)) = button {
                p.button(BTN_X, BTN_Y - FOOT_Y, BTN_W, BTN_H, label, hot && enabled && !pressed, enabled);
            }
        })
    }

    fn paint_caption(&self) -> Result<()> {
        let active = unsafe { GetForegroundWindow() == self.hwnd };
        let (hover, press) = (self.hover, self.press);
        self.paint(&self.caption, CAP_W * 2.0, CAP_H, |p| {
            let ink = if active { paint::ink(0.9) } else { paint::ink(0.45) };
            for (i, (hit, code, glyph)) in [(Hit::Min, HTMINBUTTON, '\u{E921}'), (Hit::Close, HTCLOSE, '\u{E8BB}')].into_iter().enumerate() {
                let x = i as f32 * CAP_W;
                let on = hover == hit;
                let down = press == Some(code) && on;
                if hit == Hit::Close && on {
                    p.fill(x, 0.0, CAP_W, CAP_H, if down { paint::color(148, 18, 14, 1.0) } else { paint::color(196, 43, 28, 1.0) });
                } else if on {
                    p.fill(x, 0.0, CAP_W, CAP_H, paint::ink(if down { 0.06 } else { 0.09 }));
                }
                let c = if hit == Hit::Close && on { paint::color(255, 255, 255, 1.0) } else { ink };
                p.caption_glyph(glyph, x + CAP_W / 2.0, CAP_H / 2.0, 10.0, c);
            }
        })
    }

    fn set_hover(&mut self, h: Hit) {
        if h != self.hover {
            let old = self.hover;
            self.hover = h;
            if matches!(old, Hit::Min | Hit::Close) || matches!(h, Hit::Min | Hit::Close) {
                let _ = self.paint_caption();
            }
            if old == Hit::Install || h == Hit::Install {
                let _ = self.paint_foot();
            }
            if old == Hit::Firewall || h == Hit::Firewall {
                let _ = self.paint_scan();
            }
            if old == Hit::Listen || h == Hit::Listen {
                let _ = self.paint_status();
            }
        }
    }

    fn hit(&self, x: f32, y: f32) -> Hit {
        if y < CAP_H && x >= W - CAP_W * 2.0 {
            return if x < W - CAP_W { Hit::Min } else { Hit::Close };
        }
        if install_button().is_some() && x >= BTN_X && x < BTN_X + BTN_W && y >= BTN_Y && y < BTN_Y + BTN_H {
            return Hit::Install;
        }
        let (fx, fy) = (W / 2.0 - FW_W / 2.0, WAITING.scan_y + SCAN_H + FW_GAP);
        if firewall_button().is_some() && x >= fx && x < fx + FW_W && y >= fy && y < fy + BTN_H {
            return Hit::Firewall;
        }
        let (lx, ly, lw, lh) = Self::listen_rect();
        if streaming() && x >= lx && x < lx + lw && y >= ly && y < ly + lh {
            return Hit::Listen;
        }
        Hit::None
    }

    fn nc_hit(&self, sx: i32, sy: i32) -> u32 {
        let mut pt = POINT { x: sx, y: sy };
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut pt);
        }
        let (x, y) = (pt.x as f32 / self.k, pt.y as f32 / self.k);
        match self.hit(x, y) {
            Hit::Min => HTMINBUTTON,
            Hit::Close => HTCLOSE,
            Hit::Install | Hit::Firewall | Hit::Listen => HTCLIENT,
            Hit::None if y < 52.0 => HTCAPTION,
            Hit::None => HTCLIENT,
        }
    }

    /// Something changed on the server side.
    fn on_changed(&mut self) {
        let _ = self.paint_qr();
        let _ = self.paint_scan();
        let _ = self.paint_status();
        let _ = self.paint_foot();
        let _ = self.place();
        tray(self.hwnd, NIM_MODIFY);
        if streaming() && !self.ticking {
            self.ticking = true;
            unsafe {
                SetTimer(Some(self.hwnd), 1, 33, None);
            }
        }
    }

    fn tick(&mut self) {
        let peak = f32::from_bits(PEAK.swap(0, Ordering::Relaxed));
        // a meter that jumps up and eases down; perceptual-ish (square root)
        self.level = (self.level * 0.82).max(peak.sqrt());
        self.history.pop_front();
        self.history.push_back(self.level);
        if !streaming() && self.history.iter().all(|v| *v < 0.002) {
            // the live section has faded out by now: stop and reset it
            self.ticking = false;
            self.level = 0.0;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), 1);
            }
        }
        // hidden in the tray: keep the history going, skip the painting
        if unsafe { IsWindowVisible(self.hwnd).as_bool() && !IsIconic(self.hwnd).as_bool() } {
            let _ = self.paint_live();
            let _ = self.paint_mark();
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let x = (lparam.0 & 0xffff) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
    match msg {
        WM_NCCALCSIZE if wparam.0 != 0 => return LRESULT(0),
        WM_NCHITTEST => {
            if let Some(h) = with(|u| u.nc_hit(x, y)) {
                return LRESULT(h as isize);
            }
        }
        WM_ACTIVATE => unsafe {
            let _ = DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 });
            with(|u| u.paint_caption());
            // the firewall may have been changed in Windows Security meanwhile
            if (wparam.0 & 0xffff) != 0 {
                crate::firewall::recheck();
            }
        },
        WM_NCACTIVATE => {
            with(|u| u.paint_caption());
        }
        server::WM_CHANGED => {
            with(|u| u.on_changed());
            return LRESULT(0);
        }
        WM_TIMER => {
            with(|u| u.tick());
            return LRESULT(0);
        }
        WM_SETTINGCHANGE => {
            let light = system_light();
            if light != paint::light() {
                paint::set_light(light);
                unsafe {
                    let dark: u32 = (!light) as u32;
                    let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &dark as *const _ as _, 4);
                }
                with(|u| u.paint_all());
            }
        }
        WM_DPICHANGED => unsafe {
            let r = &*(lparam.0 as *const RECT);
            with(|u| u.k = GetDpiForWindow(hwnd) as f32 / 96.0);
            let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            with(|u| u.paint_all());
            return LRESULT(0);
        },
        // ---- the install button, in the client area ----
        WM_MOUSEMOVE => {
            with(|u| {
                if !u.tracking {
                    u.tracking = true;
                    let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    unsafe {
                        let _ = TrackMouseEvent(&mut t);
                    }
                }
                let h = u.hit(x as f32 / u.k, y as f32 / u.k);
                u.set_hover(h);
            });
            return LRESULT(0);
        }
        WM_MOUSELEAVE => {
            with(|u| {
                u.tracking = false;
                u.set_hover(Hit::None);
            });
            return LRESULT(0);
        }
        WM_SETCURSOR if (lparam.0 & 0xffff) as u32 == HTCLIENT => {
            let hand = with(|u| match u.hover {
                Hit::Install => install_button().is_some_and(|(_, on)| on),
                Hit::Firewall => firewall_button().is_some_and(|(_, on)| on),
                Hit::Listen => true,
                _ => false,
            })
            .unwrap_or(false);
            unsafe {
                if let Ok(c) = LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW }) {
                    SetCursor(Some(c));
                }
            }
            return LRESULT(1);
        }
        WM_LBUTTONDOWN => {
            with(|u| {
                if matches!(u.hover, Hit::Install | Hit::Firewall | Hit::Listen) {
                    u.pressed = true;
                    unsafe { SetCapture(hwnd) };
                    let _ = u.paint_foot();
                    let _ = u.paint_scan();
                }
            });
            return LRESULT(0);
        }
        WM_LBUTTONUP => {
            let click = with(|u| {
                let was = std::mem::take(&mut u.pressed);
                unsafe {
                    let _ = ReleaseCapture();
                }
                let _ = u.paint_foot();
                let _ = u.paint_scan();
                let h = u.hit(x as f32 / u.k, y as f32 / u.k);
                (was && h == u.hover).then_some(h)
            })
            .flatten();
            match click {
                Some(Hit::Install) if install_button().is_some_and(|(_, on)| on) => {
                    if INFO.lock().unwrap().broken {
                        crate::install::repair();
                    } else {
                        crate::install::start();
                    }
                }
                Some(Hit::Firewall) if firewall_button().is_some_and(|(_, on)| on) => crate::firewall::allow(),
                Some(Hit::Listen) => {
                    server::set_listen(!server::listening());
                    with(|u| {
                        let _ = u.paint_status();
                        let _ = u.paint_foot();
                    });
                }
                _ => {}
            }
            return LRESULT(0);
        }
        // ---- the caption buttons live in the non-client area (as Stash's do) ----
        WM_NCMOUSEMOVE => {
            with(|u| {
                let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE | TME_NONCLIENT, hwndTrack: hwnd, dwHoverTime: 0 };
                unsafe {
                    let _ = TrackMouseEvent(&mut t);
                }
                let h = match wparam.0 as u32 {
                    HTMINBUTTON => Hit::Min,
                    HTCLOSE => Hit::Close,
                    _ => Hit::None,
                };
                u.set_hover(h);
            });
            if matches!(wparam.0 as u32, HTMINBUTTON | HTCLOSE) {
                return LRESULT(0);
            }
        }
        WM_NCMOUSELEAVE => {
            with(|u| {
                u.press = None;
                if matches!(u.hover, Hit::Min | Hit::Close) {
                    u.set_hover(Hit::None);
                }
            });
            return LRESULT(0);
        }
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if matches!(wparam.0 as u32, HTMINBUTTON | HTCLOSE) => {
            with(|u| {
                u.press = Some(wparam.0 as u32);
                let _ = u.paint_caption();
            });
            return LRESULT(0);
        }
        WM_NCLBUTTONUP if matches!(wparam.0 as u32, HTMINBUTTON | HTCLOSE) => {
            let pressed = with(|u| {
                let p = u.press.take();
                let _ = u.paint_caption();
                p
            })
            .flatten();
            if pressed == Some(wparam.0 as u32) {
                let cmd = if wparam.0 as u32 == HTMINBUTTON { SC_MINIMIZE } else { SC_CLOSE };
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(cmd as usize), LPARAM(0));
                }
            }
            return LRESULT(0);
        }
        // closing hides to the tray; Quit in the tray menu really quits
        WM_CLOSE => unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
            tray_hint(hwnd);
            return LRESULT(0);
        },
        WM_TRAY => {
            match (lparam.0 & 0xffff) as u32 {
                WM_LBUTTONUP => show(hwnd),
                WM_RBUTTONUP => tray_menu(hwnd),
                _ => {}
            }
            return LRESULT(0);
        }
        WM_SHOW_HUM => {
            show(hwnd);
            return LRESULT(0);
        }
        WM_DESTROY => unsafe {
            tray(hwnd, NIM_DELETE);
            PostQuitMessage(0);
            return LRESULT(0);
        },
        m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => tray(hwnd, NIM_ADD),
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
