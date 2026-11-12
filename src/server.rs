//! The HTTPS + WebSocket server the phone talks to, and the audio output it plays into
//! (VB-Audio's "CABLE Input", so apps can pick "CABLE Output" as their microphone).

use std::collections::VecDeque;
use std::error::Error;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tungstenite::{protocol::Role, WebSocket};

const PAGE: &str = include_str!("page.html");
const FONT: &[u8] = include_bytes!("../assets/DMSans.ttf");
/// The Android app, if it was built before Hum (see build.rs); empty otherwise.
const APK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/hum.apk"));
/// SHA-256 of this PC's certificate (hex): what the Android app pins.
static FINGERPRINT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
pub const PORT: u16 = 8443;

type Res<T> = Result<T, Box<dyn Error>>;

/// Whether Windows Firewall lets phones reach Hum.
#[derive(Clone, PartialEq)]
pub enum Firewall {
    Open,
    /// Inbound Block rules cover this exe: phones can't connect.
    Blocked,
    /// Swapping them for an Allow rule (Windows is asking).
    Allowing,
    Failed(String),
}

/// Where installing VB-Cable from inside the app has got to.
#[derive(Clone, PartialEq)]
pub enum Install {
    Idle,
    Downloading,
    Installing,
    /// Restarting VB-Cable's driver to bring its playback device back.
    Repairing,
    Failed(String),
    /// Installed, but Windows doesn't list the cable yet.
    Restart,
}

/// What the window shows.
pub struct Info {
    pub url: String,
    pub device: String,
    pub cable: bool,
    /// VB-Cable's driver is there (its CABLE Output records) but its playback device
    /// isn't: Windows can leave it "phantom" after a restart. A repair, not an install.
    pub broken: bool,
    pub phones: u32,
    pub rate: u32,
    pub error: Option<String>,
    pub install: Install,
    pub firewall: Firewall,
    /// While Listen is on: the device you hear yourself on.
    pub listen: Option<String>,
}
pub static INFO: Mutex<Info> = Mutex::new(Info {
    url: String::new(),
    device: String::new(),
    cable: false,
    broken: false,
    phones: 0,
    rate: 0,
    error: None,
    install: Install::Idle,
    firewall: Firewall::Open,
    listen: None,
});

/// What the audio thread is asked to do.
enum Msg {
    /// Look for the cable again; answer whether it's in use.
    Rescan(mpsc::Sender<bool>),
    /// Start or stop playing a copy to the default speakers.
    Listen(bool),
}
static AUDIO: Mutex<Option<mpsc::Sender<Msg>>> = Mutex::new(None);

/// Re-open the audio output (after installing VB-Cable). True if the cable is in use.
pub fn rescan() -> bool {
    let (tx, rx) = mpsc::channel();
    let sent = AUDIO.lock().unwrap().as_ref().is_some_and(|s| s.send(Msg::Rescan(tx)).is_ok());
    sent && rx.recv().unwrap_or(false)
}

/// Listen: hear yourself on the default speakers. Off at every launch, because anything
/// on the speakers reaches screen shares and desktop-audio capture.
static LISTEN: AtomicBool = AtomicBool::new(false);
pub fn set_listen(on: bool) {
    LISTEN.store(on, Ordering::Relaxed);
    if let Some(s) = AUDIO.lock().unwrap().as_ref() {
        let _ = s.send(Msg::Listen(on));
    }
}
pub fn listening() -> bool {
    LISTEN.load(Ordering::Relaxed)
}
/// The loudest sample since the window last looked (f32 bits).
pub static PEAK: AtomicU32 = AtomicU32::new(0);
/// The window to poke when `INFO` changes.
pub static NOTIFY: AtomicIsize = AtomicIsize::new(0);
pub const WM_CHANGED: u32 = 0x8000 + 1; // WM_APP + 1

fn changed() {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    let h = NOTIFY.load(Ordering::Relaxed);
    if h != 0 {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(Some(HWND(h as _)), WM_CHANGED, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn update(f: impl FnOnce(&mut Info)) {
    f(&mut INFO.lock().unwrap());
    changed();
}

/// One output's queue of mono samples at the phone's rate; its callback resamples.
#[derive(Default)]
struct Lane {
    q: VecDeque<f32>,
    pos: f32,
    primed: bool,
}
const CABLE: usize = 0;
const MONITOR: usize = 1;

/// The phone's audio, fed to two lanes: VB-Cable, and (while Listen is on) the speakers.
struct Buf {
    rate: f32,
    lanes: [Lane; 2],
}
type Shared = Arc<Mutex<Buf>>;

/// Runs the audio outputs and the server, each on its own thread.
pub fn start() {
    let buf: Shared = Arc::new(Mutex::new(Buf { rate: 48000.0, lanes: Default::default() }));
    let (tx, rx) = mpsc::channel::<Msg>();
    *AUDIO.lock().unwrap() = Some(tx);
    let audio_buf = buf.clone();
    // Streams can't leave the thread that made them, so this thread keeps both and
    // re-opens them: when asked, when a device goes away under one, and when the cable
    // shows up (it can appear late after sign-in, or after a repair or install).
    thread::spawn(move || {
        let mut reply: Option<mpsc::Sender<bool>> = None;
        let lost = Arc::new(AtomicBool::new(false));
        let monitor_lost = Arc::new(AtomicBool::new(false));
        let mut monitor: Option<cpal::Stream> = None;
        let open_monitor = |on: bool, monitor: &mut Option<cpal::Stream>| {
            *monitor = None;
            monitor_lost.store(false, Ordering::Relaxed);
            let opened = if on { open_listen(audio_buf.clone(), monitor_lost.clone()) } else { Ok(None) };
            match opened {
                Ok(m) => {
                    let name = m.as_ref().map(|(_, n)| n.clone());
                    *monitor = m.map(|(s, _)| s);
                    update(|i| i.listen = name);
                }
                Err(e) => {
                    LISTEN.store(false, Ordering::Relaxed);
                    update(|i| {
                        i.listen = None;
                        i.error = Some(format!("Couldn't listen: {e}"));
                    });
                }
            }
        };
        loop {
            lost.store(false, Ordering::Relaxed);
            let (stream, failed) = match open_output(audio_buf.clone(), lost.clone()) {
                Ok(s) => (s, false),
                Err(e) => {
                    update(|i| i.error = Some(format!("Audio output: {e}")));
                    (None, true)
                }
            };
            let cable = stream.is_some();
            if let Some(r) = reply.take() {
                let _ = r.send(cable);
            }
            reply = loop {
                match rx.recv_timeout(Duration::from_secs(if cable { 2 } else { 3 })) {
                    Ok(Msg::Rescan(r)) => break Some(r),
                    Ok(Msg::Listen(on)) => open_monitor(on, &mut monitor),
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if monitor_lost.load(Ordering::Relaxed) {
                            open_monitor(listening(), &mut monitor);
                        }
                        if lost.load(Ordering::Relaxed) || failed || (!cable && find_cable(&cpal::default_host()).is_some()) {
                            break None;
                        }
                    }
                }
            };
            drop(stream);
        }
    });
    thread::spawn(move || {
        if let Err(e) = run(buf) {
            update(|i| i.error = Some(e.to_string()));
        }
    });
}

fn run(buf: Shared) -> Res<()> {
    let ip = lan_ip();
    let tls = tls_config(&ip)?;
    let listener = TcpListener::bind(("0.0.0.0", PORT)).map_err(|e| format!("port {PORT} is busy ({e})"))?;
    update(|i| i.url = format!("https://{ip}:{PORT}"));
    crate::firewall::recheck();
    for tcp in listener.incoming().flatten() {
        let (tls, buf) = (tls.clone(), buf.clone());
        thread::spawn(move || {
            let _ = handle(tcp, tls, buf);
        });
    }
    Ok(())
}

/// VB-Cable's playback device. Matched by its driver's name, since Windows sometimes
/// renames the endpoint ("CABLE Input" can come back as "Speakers"); the plain one is
/// preferred over "CABLE In 16ch". A device named on the command line wins.
fn find_cable(host: &cpal::Host) -> Option<cpal::Device> {
    let named: Vec<(String, cpal::Device)> = host.output_devices().ok()?.filter_map(|d| Some((d.name().ok()?.to_lowercase(), d))).collect();
    if let Some(want) = std::env::args().nth(1).map(|a| a.to_lowercase()) {
        return named.into_iter().find(|(n, _)| n.contains(&want)).map(|(_, d)| d);
    }
    let rank = |n: &str| match () {
        _ if n.contains("cable input") => Some(0),
        _ if n.contains("vb-audio virtual cable") && !n.contains("16") => Some(1),
        _ if n.contains("vb-audio virtual cable") => Some(2),
        _ => None,
    };
    named.into_iter().filter_map(|(n, d)| rank(&n).map(|r| (r, d))).min_by_key(|(r, _)| *r).map(|(_, d)| d)
}

/// Whether VB-Cable's driver is installed: its recording side, CABLE Output, is there.
fn cable_driver_present(host: &cpal::Host) -> bool {
    host.input_devices().is_ok_and(|mut ds| ds.any(|d| d.name().is_ok_and(|n| n.to_lowercase().contains("vb-audio virtual cable"))))
}

/// What the window says plays the sound when there's no cable to play into.
pub const NO_OUTPUT: &str = "Not playing anywhere";

/// Opens VB-Cable's playback device. Without one there's no output at all: a mic must
/// never come out of the speakers, where screen shares and desktop-audio capture
/// would pick it up. The audio thread keeps looking and opens the cable when it shows.
fn open_output(buf: Shared, lost: Arc<AtomicBool>) -> Res<Option<cpal::Stream>> {
    let host = cpal::default_host();
    let Some(dev) = find_cable(&host) else {
        let broken = cable_driver_present(&host);
        let same = {
            let i = INFO.lock().unwrap();
            i.device == NO_OUTPUT && !i.cable && i.broken == broken && i.error.is_none()
        };
        if !same {
            update(|i| {
                i.device = NO_OUTPUT.into();
                i.cable = false;
                i.broken = broken;
                i.error = None;
            });
        }
        return Ok(None);
    };
    let name = dev.name()?;
    update(|i| {
        i.device = name;
        i.cable = true;
        i.broken = false;
        i.error = None;
        i.install = Install::Idle;
    });
    Ok(Some(play(&dev, buf, CABLE, lost)?))
}

/// Listen's output: the default speakers, playing the monitor lane. Returns the stream
/// and the device's name.
fn open_listen(buf: Shared, lost: Arc<AtomicBool>) -> Res<Option<(cpal::Stream, String)>> {
    let dev = cpal::default_host().default_output_device().ok_or("no speakers")?;
    let name = dev.name()?;
    buf.lock().unwrap().lanes[MONITOR] = Lane::default();
    Ok(Some((play(&dev, buf, MONITOR, lost)?, name)))
}

fn play(dev: &cpal::Device, buf: Shared, lane: usize, lost: Arc<AtomicBool>) -> Res<cpal::Stream> {
    let cfg = dev.default_output_config()?;
    let stream = match cfg.sample_format() {
        SampleFormat::F32 => build::<f32>(dev, &cfg.into(), buf, lane, lost)?,
        SampleFormat::I16 => build::<i16>(dev, &cfg.into(), buf, lane, lost)?,
        SampleFormat::I32 => build::<i32>(dev, &cfg.into(), buf, lane, lost)?,
        f => return Err(format!("unsupported sample format {f}").into()),
    };
    stream.play()?;
    Ok(stream)
}

fn build<T: SizedSample + FromSample<f32>>(dev: &cpal::Device, cfg: &cpal::StreamConfig, buf: Shared, lane: usize, lost: Arc<AtomicBool>) -> Res<cpal::Stream> {
    let ch = cfg.channels as usize;
    let out_rate = cfg.sample_rate.0 as f32;
    let stream = dev.build_output_stream(
        cfg,
        move |data: &mut [T], _| {
            let mut b = buf.lock().unwrap();
            let rate = b.rate;
            let step = rate / out_rate;
            let l = &mut b.lanes[lane];
            for frame in data.chunks_mut(ch) {
                let mut v = 0.0;
                if !l.primed && l.q.len() as f32 >= rate * 0.04 {
                    l.primed = true; // 40 ms of cushion before playing
                }
                if l.primed {
                    if l.q.len() < 2 {
                        l.primed = false;
                    } else {
                        v = l.q[0] + (l.q[1] - l.q[0]) * l.pos;
                        l.pos += step;
                        while l.pos >= 1.0 && !l.q.is_empty() {
                            l.q.pop_front();
                            l.pos -= 1.0;
                        }
                    }
                }
                let s = T::from_sample(v);
                frame.iter_mut().for_each(|x| *x = s);
            }
        },
        // the device went away or was reset: the audio thread re-opens the output
        move |_| lost.store(true, Ordering::Relaxed),
        None,
    )?;
    Ok(stream)
}

fn handle(tcp: TcpStream, tls: Arc<rustls::ServerConfig>, buf: Shared) -> Res<()> {
    tcp.set_nodelay(true)?;
    let mut s = rustls::StreamOwned::new(rustls::ServerConnection::new(tls)?, tcp);

    let mut req = Vec::new();
    let mut chunk = [0u8; 2048];
    while !req.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = s.read(&mut chunk)?;
        if n == 0 || req.len() > 16384 {
            return Ok(());
        }
        req.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&req);
    let path = head.split_whitespace().nth(1).unwrap_or("/");
    let key = head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case("sec-websocket-key").then(|| v.trim().to_string())
    });

    if let (true, Some(key)) = (path == "/ws", key) {
        let accept = tungstenite::handshake::derive_accept_key(key.as_bytes());
        s.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").as_bytes())?;
        s.flush()?;
        let mut ws = WebSocket::from_raw_socket(s, Role::Server, None);
        buf.lock().unwrap().lanes = Default::default();
        update(|i| i.phones += 1);
        stream(&mut ws, &buf);
        update(|i| i.phones = i.phones.saturating_sub(1));
    } else {
        // the page learns this PC's certificate fingerprint (for the app's pairing link)
        // and whether there's an app to offer
        let page;
        let (ctype, extra, body): (&str, &str, &[u8]) = match path {
            "/font.ttf" => ("font/ttf", "", FONT),
            "/hum.apk" if !APK.is_empty() => ("application/vnd.android.package-archive", "Content-Disposition: attachment; filename=\"Hum.apk\"\r\n", APK),
            _ => {
                page = PAGE.replace("__HUM_FP__", FINGERPRINT.get().map_or("", |s| s.as_str())).replace("__HUM_APK__", if APK.is_empty() { "" } else { "1" });
                ("text/html; charset=utf-8", "", page.as_bytes())
            }
        };
        let cache = if path == "/" || path.starts_with("/?") { "no-cache" } else { "max-age=3600" };
        let mut res = format!("HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\n{extra}Content-Length: {}\r\nCache-Control: {cache}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
        res.extend_from_slice(body);
        s.write_all(&res)?;
        s.conn.send_close_notify();
        s.flush()?;
    }
    Ok(())
}

fn stream<S: Read + Write>(ws: &mut WebSocket<S>, buf: &Shared) {
    while let Ok(msg) = ws.read() {
        if msg.is_text() {
            let text = msg.to_text().unwrap_or("");
            // the Android app sends its last crash here, to land beside hum.exe
            if let Some(report) = text.strip_prefix("crash\n") {
                if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)) {
                    let _ = std::fs::write(dir.join("hum-android-crash.txt"), report.chars().take(64 * 1024).collect::<String>());
                }
                continue;
            }
            if let Ok(r) = text.trim().parse::<f32>() {
                buf.lock().unwrap().rate = r;
                update(|i| i.rate = r as u32);
            }
        } else if msg.is_binary() {
            let data = msg.into_data();
            let samples: Vec<f32> = data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect();
            let peak = samples.iter().fold(0f32, |p, v| p.max(v.abs()));
            let listen = listening();
            let mut b = buf.lock().unwrap();
            // Never fall more than 250 ms behind: drop back to 60 ms.
            let (max, keep) = ((b.rate * 0.25) as usize, (b.rate * 0.06) as usize);
            for (i, lane) in b.lanes.iter_mut().enumerate() {
                if i == MONITOR && !listen {
                    continue;
                }
                lane.q.extend(&samples);
                if lane.q.len() > max {
                    let drop = lane.q.len() - keep;
                    lane.q.drain(..drop);
                }
            }
            drop(b);
            let _ = PEAK.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| Some(f32::from_bits(old).max(peak).to_bits()));
        } else if msg.is_close() {
            break;
        }
    }
}

/// Self-signed cert, made once and kept next to the exe so the phone only has to
/// accept it once.
fn tls_config(ip: &str) -> Res<Arc<rustls::ServerConfig>> {
    let dir = std::env::current_exe()?.parent().map(PathBuf::from).unwrap_or_default();
    let (cp, kp) = (dir.join("hum-cert.der"), dir.join("hum-key.der"));
    if !cp.exists() || !kp.exists() {
        let ck = rcgen::generate_simple_self_signed(vec![ip.to_string(), "localhost".into()])?;
        std::fs::write(&cp, ck.cert.der())?;
        std::fs::write(&kp, ck.key_pair.serialize_der())?;
    }
    let der = std::fs::read(&cp)?;
    let digest = ring::digest::digest(&ring::digest::SHA256, &der);
    let _ = FINGERPRINT.set(digest.as_ref().iter().map(|b| format!("{b:02x}")).collect());
    let cert = CertificateDer::from(der);
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(std::fs::read(&kp)?));
    let cfg = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)?;
    Ok(Arc::new(cfg))
}

/// The LAN address (connecting a UDP socket sends nothing, it just picks a route).
fn lan_ip() -> String {
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("8.8.8.8:80").and(s.local_addr()))
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".into())
}
