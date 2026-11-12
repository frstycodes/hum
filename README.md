# Hum

Use your phone as your PC's microphone, over Wi-Fi.

Hum is a small Windows app (Rust, about 2.6 MB) that shows a QR code. Scan it with your phone, tap Start, and your phone's mic comes out of a virtual microphone on the PC that Discord, OBS, Zoom or any other app can pick. There's a phone page that works in any browser, and an Android app that keeps going with the screen locked.

## Download

From the [latest release](https://github.com/frstycodes/hum/releases/latest):

- **Hum-Setup.exe**: installs Hum for your user (no admin rights needed).
- **hum.exe**: the same app, portable; run it from any folder.
- **Hum-Android.apk**: the Android app. Hum also hands it to your phone itself, so you don't need to download it here.

## How to use it

1. Run Hum on your PC. It needs [VB-Audio Virtual Cable](https://vb-audio.com/Cable/), a free virtual audio driver; if it's missing, Hum offers to install it.
2. The first time, Windows asks whether Hum may use your network. Allow it, or your phone can't reach the PC (Hum also offers an **Allow in Firewall** button if it notices it's blocked).
3. Scan the QR code with your phone's camera (same Wi-Fi as the PC) and accept the certificate warning once.
4. Tap **Start**. In Discord, OBS or any other app, choose **CABLE Output** as the microphone.

Closing Hum's window keeps it running in the tray; quit it from the tray icon's menu.

### Android app

On Android, the phone page offers **Use the Hum app**: it opens the app already paired with your PC (or offers it for download). Unlike a browser tab, the app keeps streaming with the screen locked, with a notification while it's on. Some phones (Samsung, Xiaomi and others) put background apps to sleep: if it cuts out while locked, set Hum's battery usage to **Unrestricted**.

On iPhone, use the phone page; **Turn screen off** keeps the mic running with a black screen.

## What it does, and doesn't

- Everything stays on your network. The phone talks straight to your PC over HTTPS and a WebSocket; nothing goes through the internet.
- The PC makes its own certificate on first run. The Android app pins it (it trusts only the PC it paired with), so there's no warning in the app.
- Hum never plays your voice through your speakers on its own, so screen shares and desktop-audio capture don't pick it up. **Listen** plays it there on purpose, to check how you sound (off at every launch).

## Building

**Hum (Windows):** Rust stable with the MSVC toolchain.

```bash
cargo build --release
```

If the project sits in a deep folder, the linker can trip over long paths; set `CARGO_TARGET_DIR` to a short folder.

**Android app:** plain Java, built straight with the Android SDK's tools (no Gradle). Needs JDK 17, Android platform 35 and build-tools 35.0.0, by default in `%LOCALAPPDATA%\AndroidBuild` (or pass `-Sdk` and `-Jdk`).

```bash
powershell -File android/build.ps1
```

The first build makes a signing key, `android/hum-release.keystore`. Keep it: updates only install over an existing app when they're signed with the same key.

**Everything plus the installer:** needs NSIS 3 as well (`makensis` on PATH, in Program Files, or in electron-builder's cache). Builds the Android app first, so Hum carries it.

```bash
powershell -File installer/build.ps1
```

## Credits

- [VB-Audio Virtual Cable](https://vb-audio.com/Cable/) by VB-Audio Software (donationware), which Hum downloads from VB-Audio when you ask it to; it isn't part of this repository.
- [DM Sans](https://github.com/googlefonts/dm-fonts), SIL Open Font License (`assets/OFL.txt`).

## License

[PolyForm Noncommercial 1.0.0](LICENSE.md): free to use, change and share, but not for commercial use.
