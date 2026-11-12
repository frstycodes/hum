//! Painting helpers and Stash's palette. Coordinates are logical pixels; `k` (the DPI
//! scale) is applied here.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN, D2D1_GRADIENT_STOP, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Brush, ID2D1DeviceContext, ID2D1RenderTarget, D2D1_CAP_STYLE_ROUND, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE,
    D2D1_EXTEND_MODE_CLAMP, D2D1_GAMMA_2_2, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_LINE_JOIN_ROUND, D2D1_ROUNDED_RECT,
    D2D1_STROKE_STYLE_PROPERTIES,
};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteFactory3, IDWriteFactory5, IDWriteFontCollection, IDWriteTextFormat, IDWriteTextLayout, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_METRICS, DWRITE_TRIMMING,
    DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::core::{w, Interface, HSTRING};
use windows_numerics::Vector2;

pub const REGULAR: DWRITE_FONT_WEIGHT = DWRITE_FONT_WEIGHT(400);
pub const MEDIUM: DWRITE_FONT_WEIGHT = DWRITE_FONT_WEIGHT(500);
pub const SEMIBOLD: DWRITE_FONT_WEIGHT = DWRITE_FONT_WEIGHT(600);

static LIGHT: AtomicBool = AtomicBool::new(false);
pub fn set_light(on: bool) {
    LIGHT.store(on, Ordering::Relaxed);
}
pub fn light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

pub fn color(r: u8, g: u8, b: u8, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a }
}
pub fn ink(a: f32) -> D2D1_COLOR_F {
    if light() { color(28, 25, 32, a) } else { color(255, 255, 255, a) }
}
pub fn text() -> D2D1_COLOR_F {
    ink(if light() { 0.9 } else { 0.92 })
}
pub fn dim() -> D2D1_COLOR_F {
    ink(if light() { 0.6 } else { 0.55 })
}
pub fn faint() -> D2D1_COLOR_F {
    ink(if light() { 0.42 } else { 0.32 })
}
pub fn link() -> D2D1_COLOR_F {
    if light() { color(196, 86, 16, 1.0) } else { rgb(ORANGE, 1.0) }
}
pub fn warn_text() -> D2D1_COLOR_F {
    if light() { color(184, 92, 20, 1.0) } else { color(245, 169, 122, 1.0) }
}
pub fn danger_text() -> D2D1_COLOR_F {
    if light() { color(196, 43, 48, 1.0) } else { color(245, 120, 124, 1.0) }
}
/// The level scale, one hue (the brand orange): `lo` at silence, `hi` at full.
pub fn lo(a: f32) -> D2D1_COLOR_F {
    if light() { color(247, 178, 122, a) } else { color(255, 196, 150, a) }
}
pub fn hi(a: f32) -> D2D1_COLOR_F {
    if light() { color(184, 70, 8, a) } else { color(226, 98, 22, a) }
}
pub fn green() -> D2D1_COLOR_F {
    color(48, 209, 88, 1.0)
}
pub const ORANGE: (u8, u8, u8) = (242, 125, 46);
pub fn rgb(c: (u8, u8, u8), a: f32) -> D2D1_COLOR_F {
    color(c.0, c.1, c.2, a)
}

// ---------- DM Sans, built into the exe ----------

static DM_SANS: &[u8] = include_bytes!("../assets/DMSans.ttf");

thread_local! {
    static FONTS: std::cell::OnceCell<Option<(IDWriteFontCollection, String)>> = const { std::cell::OnceCell::new() };
}

fn fonts(dw: &IDWriteFactory) -> Option<(IDWriteFontCollection, String)> {
    FONTS.with(|cell| {
        cell.get_or_init(|| unsafe {
            let f5: IDWriteFactory5 = dw.cast().ok()?;
            let loader = f5.CreateInMemoryFontFileLoader().ok()?;
            f5.RegisterFontFileLoader(&loader).ok()?;
            let file = loader.CreateInMemoryFontFileReference(dw, DM_SANS.as_ptr() as _, DM_SANS.len() as u32, None).ok()?;
            let builder = f5.CreateFontSetBuilder().ok()?;
            builder.AddFontFile(&file).ok()?;
            let set = builder.CreateFontSet().ok()?;
            let f3: IDWriteFactory3 = dw.cast().ok()?;
            let coll = f3.CreateFontCollectionFromFontSet(&set).ok()?.cast::<IDWriteFontCollection>().ok()?;
            let names = coll.GetFontFamily(0).ok()?.GetFamilyNames().ok()?;
            let mut buf = [0u16; 64];
            names.GetString(0, &mut buf).ok()?;
            let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
            Some((coll, String::from_utf16_lossy(&buf[..n])))
        })
        .clone()
    })
}

fn format(dw: &IDWriteFactory, size: f32, weight: DWRITE_FONT_WEIGHT) -> windows::core::Result<IDWriteTextFormat> {
    let f = fonts(dw);
    let family = f.as_ref().map_or("Segoe UI".to_string(), |(_, n)| n.clone());
    unsafe {
        dw.CreateTextFormat(&HSTRING::from(family), f.as_ref().map(|(c, _)| c), weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size.max(1.0), w!("en-us"))
    }
}

fn layout(dw: &IDWriteFactory, s: &str, size: f32, weight: DWRITE_FONT_WEIGHT, max_w: f32) -> Option<(IDWriteTextLayout, f32, f32)> {
    unsafe {
        let format = format(dw, size, weight).ok()?;
        let _ = format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
        if let Ok(sign) = dw.CreateEllipsisTrimmingSign(&format) {
            let _ = format.SetTrimming(&DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 }, &sign);
        }
        let wide: Vec<u16> = s.encode_utf16().collect();
        let l = dw.CreateTextLayout(&wide, &format, max_w, size * 2.0).ok()?;
        let mut m = DWRITE_TEXT_METRICS::default();
        l.GetMetrics(&mut m).ok()?;
        Some((l, m.width.min(max_w), m.height))
    }
}

pub struct P<'a> {
    pub dc: &'a ID2D1DeviceContext,
    pub dw: &'a IDWriteFactory,
    pub k: f32,
}

impl P<'_> {
    fn brush(&self, c: D2D1_COLOR_F) -> Option<ID2D1Brush> {
        unsafe { self.dc.CreateSolidColorBrush(&c, None).ok().and_then(|b| b.cast().ok()) }
    }

    fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
        let k = self.k;
        D2D_RECT_F { left: x * k, top: y * k, right: (x + w) * k, bottom: (y + h) * k }
    }

    pub fn capsule(&self, x: f32, y: f32, w: f32, h: f32, c: D2D1_COLOR_F) {
        if let Some(b) = self.brush(c) {
            self.capsule_with(x, y, w, h, &b);
        }
    }

    fn capsule_with(&self, x: f32, y: f32, w: f32, h: f32, b: &ID2D1Brush) {
        let r = h / 2.0 * self.k;
        let rr = D2D1_ROUNDED_RECT { rect: self.rect(x, y, w.max(h), h), radiusX: r, radiusY: r };
        unsafe { self.dc.FillRoundedRectangle(&rr, b) };
    }

    pub fn fill(&self, x: f32, y: f32, w: f32, h: f32, c: D2D1_COLOR_F) {
        if let Some(b) = self.brush(c) {
            unsafe { self.dc.FillRectangle(&self.rect(x, y, w, h), &b) };
        }
    }

    pub fn dot(&self, cx: f32, cy: f32, r: f32, c: D2D1_COLOR_F) {
        if let Some(b) = self.brush(c) {
            let e = D2D1_ELLIPSE { point: Vector2 { X: cx * self.k, Y: cy * self.k }, radiusX: r * self.k, radiusY: r * self.k };
            unsafe { self.dc.FillEllipse(&e, &b) };
        }
    }

    /// A linear gradient from (x0, y0) to (x1, y1) through `stops` (position, colour).
    fn gradient(&self, x0: f32, y0: f32, x1: f32, y1: f32, stops: &[(f32, D2D1_COLOR_F)]) -> Option<ID2D1Brush> {
        let k = self.k;
        let stops: Vec<D2D1_GRADIENT_STOP> = stops.iter().map(|&(position, color)| D2D1_GRADIENT_STOP { position, color }).collect();
        unsafe {
            let rt: &ID2D1RenderTarget = self.dc;
            let coll = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP).ok()?;
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2 { X: x0 * k, Y: y0 * k }, endPoint: Vector2 { X: x1 * k, Y: y1 * k } };
            self.dc.CreateLinearGradientBrush(&props, None, &coll).ok()?.cast().ok()
        }
    }

    /// The level meter, Stash's progress bar: a filled pill, a 4px gap, then the rest of
    /// the track. The fill runs light to deep across the whole track, so its tip has the
    /// same shade as the level history at that height.
    pub fn meter(&self, x: f32, y: f32, w: f32, h: f32, v: f32) {
        let fw = w * v.clamp(0.0, 1.0);
        if fw < h {
            self.capsule(x, y, w, h, ink(0.1));
            return;
        }
        if let Some(b) = self.gradient(x, 0.0, x + w, 0.0, &[(0.0, lo(1.0)), (1.0, hi(1.0))]) {
            self.capsule_with(x, y, fw, h, &b);
        }
        let rest = w - fw - 4.0;
        if rest >= h {
            self.capsule(x + fw + 4.0, y, rest, h, ink(0.1));
        }
    }

    /// The level history: a line shaded by height (deep at the top, light at the
    /// bottom), a fill under it fading out downward, the oldest end fading in.
    pub fn history(&self, x: f32, y: f32, w: f32, h: f32, values: &[f32]) {
        use std::mem::ManuallyDrop;
        use windows::Win32::Graphics::Direct2D::{D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_LAYER_OPTIONS1_NONE, D2D1_LAYER_PARAMETERS1};
        use windows_numerics::Matrix3x2;
        if values.len() < 2 {
            return;
        }
        let k = self.k;
        let n = values.len() - 1;
        let pt = |i: usize, v: f32| Vector2 { X: (x + w * i as f32 / n as f32) * k, Y: (y + h - 1.5 - v.clamp(0.0, 1.0) * (h - 3.0)) * k };
        unsafe {
            let Ok(factory) = self.dc.GetFactory() else { return };
            let Some(fade) = self.gradient(x, 0.0, x + w * 0.3, 0.0, &[(0.0, color(0, 0, 0, 0.0)), (1.0, color(0, 0, 0, 1.0))]) else { return };
            let mut layer = D2D1_LAYER_PARAMETERS1 {
                contentBounds: D2D_RECT_F { left: -1e6, top: -1e6, right: 1e6, bottom: 1e6 },
                geometricMask: ManuallyDrop::new(None),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: Matrix3x2::identity(),
                opacity: 1.0,
                opacityBrush: ManuallyDrop::new(Some(fade)),
                layerOptions: D2D1_LAYER_OPTIONS1_NONE,
            };
            self.dc.PushLayer(&layer, None);
            if let (Ok(area), Some(fill)) = (factory.CreatePathGeometry(), self.gradient(0.0, y, 0.0, y + h, &[(0.0, hi(0.32)), (0.55, lo(0.12)), (1.0, lo(0.0))])) {
                if let Ok(sink) = area.Open() {
                    sink.BeginFigure(Vector2 { X: x * k, Y: (y + h) * k }, D2D1_FIGURE_BEGIN_FILLED);
                    for (i, v) in values.iter().enumerate() {
                        sink.AddLine(pt(i, *v));
                    }
                    sink.AddLine(Vector2 { X: (x + w) * k, Y: (y + h) * k });
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    let _ = sink.Close();
                    self.dc.FillGeometry(&area, &fill, None);
                }
            }
            if let (Ok(line), Some(stroke)) = (factory.CreatePathGeometry(), self.gradient(0.0, y, 0.0, y + h, &[(0.0, hi(1.0)), (1.0, lo(1.0))])) {
                if let Ok(sink) = line.Open() {
                    sink.BeginFigure(pt(0, values[0]), D2D1_FIGURE_BEGIN_HOLLOW);
                    for (i, v) in values.iter().enumerate().skip(1) {
                        sink.AddLine(pt(i, *v));
                    }
                    sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    let _ = sink.Close();
                    let style = factory
                        .CreateStrokeStyle(&D2D1_STROKE_STYLE_PROPERTIES { startCap: D2D1_CAP_STYLE_ROUND, endCap: D2D1_CAP_STYLE_ROUND, lineJoin: D2D1_LINE_JOIN_ROUND, ..Default::default() }, None)
                        .ok();
                    self.dc.DrawGeometry(&line, &stroke, 1.6 * k, style.as_ref());
                }
            }
            self.dc.PopLayer();
            ManuallyDrop::drop(&mut layer.opacityBrush);
        }
    }

    /// A QR code drawn straight on the surface in the text ink: rounded modules, the
    /// three finder patterns as rounded rings around a rounded square. `size` across,
    /// its top-left at (x, y), each module snapped to whole device pixels.
    pub fn qr(&self, x: f32, y: f32, size: f32, width: usize, dark: &[bool]) {
        let k = self.k;
        let m = ((size * k) / width as f32).floor().max(1.0);
        let side = m * width as f32;
        let ox = (x * k + (size * k - side) / 2.0).round();
        let oy = (y * k + (size * k - side) / 2.0).round();
        let Some(b) = self.brush(text()) else { return };
        let n = width;
        let finder = |r: usize, c: usize| (r < 7 && c < 7) || (r < 7 && c >= n - 7) || (r >= n - 7 && c < 7);
        let rr = |l: f32, t: f32, s: f32, r: f32| D2D1_ROUNDED_RECT { rect: D2D_RECT_F { left: l, top: t, right: l + s, bottom: t + s }, radiusX: r, radiusY: r };
        unsafe {
            for (i, on) in dark.iter().enumerate() {
                let (r, c) = (i / width, i % width);
                if *on && !finder(r, c) {
                    let (l, t) = (ox + c as f32 * m, oy + r as f32 * m);
                    self.dc.FillRoundedRectangle(&rr(l + 0.35 * k, t + 0.35 * k, m - 0.7 * k, m * 0.3), &b);
                }
            }
            for (r, c) in [(0, 0), (0, n - 7), (n - 7, 0)] {
                let (l, t) = (ox + c as f32 * m, oy + r as f32 * m);
                self.dc.DrawRoundedRectangle(&rr(l + m / 2.0, t + m / 2.0, 6.0 * m, m * 2.0), &b, m, None);
                self.dc.FillRoundedRectangle(&rr(l + 2.0 * m, t + 2.0 * m, 3.0 * m, m * 0.9), &b);
            }
        }
    }

    pub fn measure(&self, s: &str, size: f32, weight: DWRITE_FONT_WEIGHT) -> f32 {
        layout(self.dw, s, size * self.k, weight, 100_000.0).map_or(0.0, |(_, w, _)| w / self.k)
    }

    /// Text on one line, vertically centered on `cy`, trimmed at `max_w`. Returns its width.
    pub fn text_mid(&self, s: &str, x: f32, cy: f32, size: f32, weight: DWRITE_FONT_WEIGHT, c: D2D1_COLOR_F, max_w: f32) -> f32 {
        let k = self.k;
        let Some((l, w, h)) = layout(self.dw, s, size * k, weight, max_w.max(1.0) * k) else { return 0.0 };
        if let Some(b) = self.brush(c) {
            unsafe { self.dc.DrawTextLayout(Vector2 { X: x * k, Y: cy * k - h / 2.0 }, &l, &b, D2D1_DRAW_TEXT_OPTIONS_NONE) };
        }
        w / k
    }

    pub fn text_center(&self, s: &str, cx: f32, cy: f32, size: f32, weight: DWRITE_FONT_WEIGHT, c: D2D1_COLOR_F, max_w: f32) {
        let w = self.measure(s, size, weight).min(max_w);
        self.text_mid(s, cx - w / 2.0, cy, size, weight, c, max_w);
    }

    pub fn text_right(&self, s: &str, right: f32, cy: f32, size: f32, weight: DWRITE_FONT_WEIGHT, c: D2D1_COLOR_F) {
        let w = self.measure(s, size, weight);
        self.text_mid(s, right - w, cy, size, weight, c, w + 1.0);
    }

    /// Hum's logo, `size` across, centered on (cx, cy). `lift` (0..1 per bar) stretches
    /// each bar toward full height, so it can dance with the level.
    pub fn logo(&self, cx: f32, cy: f32, size: f32, lift: [f32; 4]) {
        use crate::logo::{LOGO_BARS, LOGO_BAR_R, LOGO_BAR_W};
        let k = self.k;
        let s = size * k / 1024.0;
        let (ox, oy) = ((cx - size / 2.0) * k, (cy - size / 2.0) * k);
        for ((bx, h, c), l) in LOGO_BARS.into_iter().zip(lift) {
            let Some(b) = self.brush(rgb(c, 1.0)) else { continue };
            let h = h + (940.0 - h) * l.clamp(0.0, 1.0);
            let rect = D2D_RECT_F { left: ox + (bx - LOGO_BAR_W / 2.0) * s, top: oy + (512.0 - h / 2.0) * s, right: ox + (bx + LOGO_BAR_W / 2.0) * s, bottom: oy + (512.0 + h / 2.0) * s };
            let rr = D2D1_ROUNDED_RECT { rect, radiusX: LOGO_BAR_R * s, radiusY: LOGO_BAR_R * s };
            unsafe { self.dc.FillRoundedRectangle(&rr, &b) };
        }
    }

    /// Stash's primary button: an orange capsule with a white label (dimmed when off).
    pub fn button(&self, x: f32, y: f32, w: f32, h: f32, label: &str, hot: bool, enabled: bool) {
        let fill = match (enabled, hot) {
            (false, _) => rgb(ORANGE, 0.4),
            (true, true) => color(250, 150, 80, 1.0),
            (true, false) => rgb(ORANGE, 1.0),
        };
        self.capsule(x, y, w, h, fill);
        self.text_center(label, x + w / 2.0, y + h / 2.0, 13.0, SEMIBOLD, color(255, 255, 255, if enabled { 1.0 } else { 0.7 }), w - 16.0);
    }

    /// Stash's switch: a capsule track (the accent when on) and a wider-than-tall white
    /// capsule knob with a soft shadow, 2px in from the track's ends.
    pub fn switch(&self, x: f32, y: f32, w: f32, h: f32, on: bool, hot: bool) {
        let track = match (on, hot) {
            (true, true) => color(250, 150, 80, 1.0),
            (true, false) => rgb(ORANGE, 1.0),
            (false, true) => ink(0.22),
            (false, false) => ink(0.16),
        };
        self.capsule(x, y, w, h, track);
        let (kw, kh) = (w * 0.6, h - 4.0);
        let kx = if on { x + w - kw - 2.0 } else { x + 2.0 };
        self.capsule(kx, y + 2.5, kw, kh, color(0, 0, 0, 0.18)); // shadow
        self.capsule(kx, y + 2.0, kw, kh, color(255, 255, 255, 1.0));
    }

    /// A Windows caption glyph from the system icon font, like every other window's.
    pub fn caption_glyph(&self, ch: char, cx: f32, cy: f32, size: f32, c: D2D1_COLOR_F) {
        unsafe {
            let mut coll = None;
            let mut family = "Segoe MDL2 Assets";
            if self.dw.GetSystemFontCollection(&mut coll, false).is_ok() {
                if let Some(c) = &coll {
                    let (mut i, mut found) = (0u32, Default::default());
                    if c.FindFamilyName(w!("Segoe Fluent Icons"), &mut i, &mut found).is_ok() && found.as_bool() {
                        family = "Segoe Fluent Icons";
                    }
                }
            }
            let k = self.k;
            let Ok(format) = self.dw.CreateTextFormat(&HSTRING::from(family), None, REGULAR, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size * k, w!("en-us")) else { return };
            let _ = format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            let wide: Vec<u16> = ch.to_string().encode_utf16().collect();
            let Ok(l) = self.dw.CreateTextLayout(&wide, &format, size * 3.0 * k, size * 2.0 * k) else { return };
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = l.GetMetrics(&mut m);
            if let Some(b) = self.brush(c) {
                self.dc.DrawTextLayout(Vector2 { X: (cx - size * 1.5) * k, Y: cy * k - m.height / 2.0 }, &l, &b, D2D1_DRAW_TEXT_OPTIONS_NONE);
            }
        }
    }
}
