//! Renders the logo into hum.ico (several sizes) and embeds it as the exe's icon.

include!("src/logo.rs");

fn sdf(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (hw - r);
    let qy = (py - cy).abs() - (hh - r);
    let (ox, oy) = (qx.max(0.0), qy.max(0.0));
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
}

/// Straight-alpha BGRA, bottom-up rows (as an .ico's DIB wants).
fn render(size: u32) -> Vec<u8> {
    const SS: u32 = 4;
    let s = 1024.0 / size as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let (mut cov, mut col) = (0.0f32, (0u8, 0u8, 0u8));
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = (x as f32 + (sx as f32 + 0.5) / SS as f32) * s;
                    let py = (y as f32 + (sy as f32 + 0.5) / SS as f32) * s;
                    for (cx, h, c) in LOGO_BARS {
                        if sdf(px, py, cx, 512.0, LOGO_BAR_W / 2.0, h / 2.0, LOGO_BAR_R) <= 0.0 {
                            cov += 1.0;
                            col = c;
                            break;
                        }
                    }
                }
            }
            let a = cov / (SS * SS) as f32;
            let i = (((size - 1 - y) * size + x) * 4) as usize;
            out[i..i + 4].copy_from_slice(&[col.2, col.1, col.0, (a * 255.0).round() as u8]);
        }
    }
    out
}

fn main() {
    println!("cargo:rerun-if-changed=src/logo.rs");
    println!("cargo:rerun-if-changed=build.rs");
    let sizes = [16u32, 20, 24, 32, 40, 48, 64, 256];
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&n| {
            let mut dib = Vec::new();
            // BITMAPINFOHEADER: height counts the (empty) AND mask too
            for v in [40u32, n, n * 2] {
                dib.extend_from_slice(&v.to_le_bytes());
            }
            dib.extend_from_slice(&1u16.to_le_bytes());
            dib.extend_from_slice(&32u16.to_le_bytes());
            dib.extend_from_slice(&[0u8; 24]);
            dib.extend_from_slice(&render(n));
            let mask_row = n.div_ceil(32) * 4;
            dib.extend(std::iter::repeat_n(0u8, (mask_row * n) as usize));
            dib
        })
        .collect();
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (n, img) in sizes.iter().zip(&images) {
        let b = if *n >= 256 { 0 } else { *n as u8 };
        ico.extend_from_slice(&[b, b, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(img.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in &images {
        ico.extend_from_slice(img);
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico_path = out.join("hum.ico");
    std::fs::write(&ico_path, ico).unwrap();
    let rc = out.join("hum.rc");
    std::fs::write(&rc, format!("1 ICON \"{}\"\n", ico_path.display().to_string().replace('\\', "\\\\"))).unwrap();
    embed_resource::compile(&rc, embed_resource::NONE).manifest_optional().unwrap();

    // The Android app (android\build.ps1 makes it) rides inside the exe, so Hum can hand
    // it to phones itself. Without it the build still works; the page just doesn't offer it.
    println!("cargo:rerun-if-changed=android/out/hum.apk");
    let apk = std::fs::read("android/out/hum.apk").unwrap_or_default();
    std::fs::write(out.join("hum.apk"), apk).unwrap();
}
