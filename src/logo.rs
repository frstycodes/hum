// Hum's logo, on a 1024 canvas: Stash's four rounded bars and sunset colours, stood
// upright as a level meter. Shared by the window (Direct2D) and build.rs (the .ico).

/// Each bar: centre x, height, colour (gold, orange, rose, plum).
pub const LOGO_BARS: [(f32, f32, (u8, u8, u8)); 4] = [
    (176.0, 500.0, (249, 199, 79)),
    (400.0, 880.0, (242, 125, 46)),
    (624.0, 680.0, (209, 56, 75)),
    (848.0, 400.0, (106, 44, 112)),
];
/// Bar thickness and corner radius: Stash's bars' proportions.
pub const LOGO_BAR_W: f32 = 154.0;
pub const LOGO_BAR_R: f32 = 43.1;
