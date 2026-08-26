//! The 16-colour terminal palette the stamps were rendered against.

/// Palette entries, in ANSI order: 0-7 are the normal colours, 8-15 the bright
/// ones. These values were recovered from the original stamp blob, so they are
/// the exact colours the loss function was calibrated for.
pub const PALETTE: [[u8; 3]; 16] = [
    [24, 24, 24],    // 0  black
    [177, 60, 61],   // 1  red
    [120, 177, 60],  // 2  green
    [177, 148, 60],  // 3  yellow
    [60, 72, 177],   // 4  blue
    [150, 60, 177],  // 5  magenta
    [66, 168, 156],  // 6  cyan
    [207, 207, 207], // 7  white
    [79, 79, 79],    // 8  bright black
    [255, 86, 88],   // 9  bright red
    [173, 255, 86],  // 10 bright green
    [255, 213, 86],  // 11 bright yellow
    [86, 104, 255],  // 12 bright blue
    [216, 86, 255],  // 13 bright magenta
    [100, 255, 239], // 14 bright cyan
    [255, 255, 255], // 15 bright white
];

/// Palette normalised to 0.0-1.0, which is the space the solver works in.
pub fn palette_f32() -> [[f32; 3]; 16] {
    let mut out = [[0.0f32; 3]; 16];
    for (dst, src) in out.iter_mut().zip(PALETTE.iter()) {
        for c in 0..3 {
            dst[c] = src[c] as f32 / 255.0;
        }
    }
    out
}

/// `#rrggbb` for palette entry `i`.
pub fn hex(i: u8) -> String {
    let [r, g, b] = PALETTE[i as usize & 15];
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Which palette entries a conversion is allowed to pick from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    /// Black and bright white only - the classic monochrome look.
    #[default]
    Mono,
    /// The four neutral entries: black, white, and the two greys.
    Grayscale,
    /// The full 16-colour palette.
    Color,
}

impl ColorMode {
    pub fn allowed(self) -> &'static [u8] {
        match self {
            ColorMode::Mono => &[0, 15],
            ColorMode::Grayscale => &[0, 7, 8, 15],
            ColorMode::Color => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        }
    }
}

impl std::str::FromStr for ColorMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "mono" | "none" | "bw" => Ok(ColorMode::Mono),
            "gray" | "grey" | "grayscale" | "greyscale" => Ok(ColorMode::Grayscale),
            "color" | "colour" | "full" => Ok(ColorMode::Color),
            other => Err(format!("unknown color mode `{other}`")),
        }
    }
}
