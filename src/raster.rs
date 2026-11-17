//! Drawing a conversion as an image: glyph sets from TTF/OTF fonts, terminal
//! colour themes, and the cell-by-cell compositor.
//!
//! Built only with the `raster` feature.

use image::{GrayImage, Luma, RgbImage, imageops::FilterType};

use crate::convert::Art;
use crate::palette::PALETTE;
use crate::stamps::{CELL_H, CELL_W, FIRST_CHAR, NCHARS, Stamps, builtin_alpha};

/// Coverage masks for every printable glyph at one cell size.
#[derive(Debug, Clone)]
pub struct Glyphs {
    pub cell_w: usize,
    pub cell_h: usize,
    /// `NCHARS * cell_w * cell_h`, 0.0-1.0.
    pub alpha: Vec<f32>,
}

impl Glyphs {
    /// The built-in masks, resampled to `cell_w x cell_h`.
    pub fn builtin(cell_w: usize, cell_h: usize) -> Glyphs {
        let alpha: Vec<f32> = builtin_alpha().iter().map(|&v| v as f32 / 255.0).collect();
        Glyphs {
            cell_w: CELL_W,
            cell_h: CELL_H,
            alpha,
        }
        .resized(cell_w, cell_h)
    }

    /// Rasterise a font into `cell_w x cell_h` cells.
    ///
    /// Every glyph is placed on one shared baseline and scaled by one shared
    /// factor, so the font keeps its own proportions between glyphs. That factor
    /// fits the font's line height into the cell; a font whose widest advance
    /// would then overflow the cell is squeezed horizontally, by up to 1.6x,
    /// before the whole thing shrinks instead. Proportional and display fonts
    /// work too: each glyph sits centred on its own advance. Characters the
    /// font lacks come out blank.
    pub fn from_font(data: &[u8], cell_w: usize, cell_h: usize) -> Result<Glyphs, String> {
        let font = fontdue::Font::from_bytes(data, fontdue::FontSettings::default())
            .map_err(|e| e.to_string())?;

        // Work at a large size and area-average down, so thin strokes keep
        // their weight as partial coverage instead of vanishing.
        const PX: f32 = 256.0;
        let lm = font
            .horizontal_line_metrics(PX)
            .ok_or("font has no horizontal line metrics")?;
        let line_h = (lm.ascent - lm.descent).max(1.0);
        let chars: Vec<char> = (0..NCHARS as u8)
            .map(|i| (FIRST_CHAR + i) as char)
            .collect();
        let max_adv = chars
            .iter()
            .filter(|&&c| font.lookup_glyph_index(c) != 0)
            .map(|&c| font.metrics(c, PX).advance_width)
            .fold(1.0f32, f32::max);

        // Scale from the big raster to the cell, per axis.
        let mut sy = cell_h as f32 / line_h;
        let mut sx = cell_w as f32 / max_adv;
        if sx < sy / 1.6 {
            sy = sx * 1.6;
        }
        sx = sx.min(sy);

        // Draw each glyph into a canvas the size of the cell in raster units.
        let canvas_w = (cell_w as f32 / sx).ceil() as u32;
        let canvas_h = (cell_h as f32 / sy).ceil() as u32;
        // Centre the line vertically when the cell ended up taller than it.
        let baseline = (canvas_h as f32 - line_h) / 2.0 + lm.ascent;

        let mut alpha = Vec::with_capacity(NCHARS * cell_w * cell_h);
        for &c in &chars {
            let mut canvas = GrayImage::new(canvas_w, canvas_h);
            if c != ' ' && font.lookup_glyph_index(c) != 0 {
                let (m, bitmap) = font.rasterize(c, PX);
                let left = (canvas_w as f32 - m.advance_width) / 2.0 + m.xmin as f32;
                let top = baseline - (m.ymin as f32 + m.height as f32);
                for y in 0..m.height {
                    for x in 0..m.width {
                        let cx = left.round() as i64 + x as i64;
                        let cy = top.round() as i64 + y as i64;
                        if cx >= 0 && cy >= 0 && (cx as u32) < canvas_w && (cy as u32) < canvas_h {
                            canvas.put_pixel(cx as u32, cy as u32, Luma([bitmap[y * m.width + x]]));
                        }
                    }
                }
            }
            let small = image::imageops::resize(
                &canvas,
                cell_w as u32,
                cell_h as u32,
                FilterType::Triangle,
            );
            alpha.extend(small.iter().map(|&v| v as f32 / 255.0));
        }
        Ok(Glyphs {
            cell_w,
            cell_h,
            alpha,
        })
    }

    /// The same glyphs at another cell size.
    pub fn resized(&self, cell_w: usize, cell_h: usize) -> Glyphs {
        if (cell_w, cell_h) == (self.cell_w, self.cell_h) {
            return self.clone();
        }
        // Shrinking averages; enlarging keeps the pixel edges a terminal
        // font would have, so upscaled built-in glyphs stay crisp.
        let filter = if cell_w < self.cell_w {
            FilterType::Triangle
        } else {
            FilterType::Nearest
        };
        let px = self.cell_w * self.cell_h;
        let mut alpha = Vec::with_capacity(NCHARS * cell_w * cell_h);
        for c in 0..NCHARS {
            let mask = GrayImage::from_fn(self.cell_w as u32, self.cell_h as u32, |x, y| {
                Luma([
                    (self.alpha[c * px + y as usize * self.cell_w + x as usize] * 255.0).round()
                        as u8,
                ])
            });
            let out = image::imageops::resize(&mask, cell_w as u32, cell_h as u32, filter);
            alpha.extend(out.iter().map(|&v| v as f32 / 255.0));
        }
        Glyphs {
            cell_w,
            cell_h,
            alpha,
        }
    }

    /// A glyph set the converter can match against, at its 10x20 cell size.
    pub fn stamps(&self) -> Stamps {
        Stamps::from_alpha(self.resized(CELL_W, CELL_H).alpha)
    }

    fn mask(&self, ch: u8) -> &[f32] {
        let px = self.cell_w * self.cell_h;
        let i = (ch.saturating_sub(FIRST_CHAR) as usize).min(NCHARS - 1);
        &self.alpha[i * px..(i + 1) * px]
    }
}

/// How the 16 palette slots look on screen.
///
/// Like a real terminal, a theme has its own default background and
/// foreground: cells whose background is slot 0 take the theme's background,
/// and glyphs drawn in slot 7 take its foreground.
#[derive(Debug, Clone)]
pub struct Theme {
    pub palette: [[u8; 3]; 16],
    pub background: [u8; 3],
    pub foreground: [u8; 3],
    /// A light-background theme; pair it with `--invert-lightness` (or
    /// `--invert`) so dark parts of the image come out as dark ink.
    pub light: bool,
}

const fn rgb(v: u32) -> [u8; 3] {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

const fn pal(v: [u32; 16]) -> [[u8; 3]; 16] {
    let mut out = [[0u8; 3]; 16];
    let mut i = 0;
    while i < 16 {
        out[i] = rgb(v[i]);
        i += 1;
    }
    out
}

const SOLARIZED: [u32; 16] = [
    0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5, 0x002b36,
    0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
];

/// Named themes: (name, palette, background, foreground, light). Single-hue
/// ones ("phosphor", "paper") are built by [`Theme::single_hue`].
const THEMES: &[(&str, [u32; 16], u32, u32, bool)] = &[
    (
        "xterm",
        [
            0x000000, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5,
            0x7f7f7f, 0xff0000, 0x00ff00, 0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
        ],
        0x000000,
        0xe5e5e5,
        false,
    ),
    (
        "vga",
        [
            0x000000, 0xaa0000, 0x00aa00, 0xaa5500, 0x0000aa, 0xaa00aa, 0x00aaaa, 0xaaaaaa,
            0x555555, 0xff5555, 0x55ff55, 0xffff55, 0x5555ff, 0xff55ff, 0x55ffff, 0xffffff,
        ],
        0x000000,
        0xaaaaaa,
        false,
    ),
    ("solarized-dark", SOLARIZED, 0x002b36, 0x839496, false),
    ("solarized-light", SOLARIZED, 0xfdf6e3, 0x657b83, true),
    (
        "gruvbox-dark",
        [
            0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984,
            0x928374, 0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2,
        ],
        0x282828,
        0xebdbb2,
        false,
    ),
    (
        "dracula",
        [
            0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2,
            0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
        ],
        0x282a36,
        0xf8f8f2,
        false,
    ),
    (
        "nord",
        [
            0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0,
            0x4c566a, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
        ],
        0x2e3440,
        0xd8dee9,
        false,
    ),
    (
        "monokai",
        [
            0x272822, 0xf92672, 0xa6e22e, 0xf4bf75, 0x66d9ef, 0xae81ff, 0xa1efe4, 0xf8f8f2,
            0x75715e, 0xf92672, 0xa6e22e, 0xf4bf75, 0x66d9ef, 0xae81ff, 0xa1efe4, 0xf9f8f5,
        ],
        0x272822,
        0xf8f8f2,
        false,
    ),
    (
        "tokyo-night",
        [
            0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6,
            0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
        ],
        0x1a1b26,
        0xc0caf5,
        false,
    ),
    (
        "one-light",
        [
            0x383a42, 0xe45649, 0x50a14f, 0xc18401, 0x0184bc, 0xa626a4, 0x0997b3, 0xfafafa,
            0x4f525e, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xffffff,
        ],
        0xfafafa,
        0x383a42,
        true,
    ),
];

/// Single-hue displays: (name, background, ink, light).
const SINGLE_HUE: &[(&str, u32, u32, bool)] = &[
    ("green-phosphor", 0x0a120a, 0x33ff66, false),
    ("amber-phosphor", 0x140c00, 0xffb000, false),
    ("white-phosphor", 0x101418, 0xe8f0ff, false),
    ("blue-screen", 0x0000a8, 0xffffff, false),
    ("paper", 0xf4f1e8, 0x1a1a1a, true),
    ("blueprint", 0x1d3f73, 0xe6eefc, false),
];

impl Theme {
    /// The palette the converter itself scores against.
    pub fn tool() -> Theme {
        Theme {
            palette: PALETTE,
            background: PALETTE[0],
            foreground: PALETTE[7],
            light: false,
        }
    }

    /// A display that only has one ink: every slot becomes that ink at the
    /// slot's own brightness, blended over the background.
    pub fn single_hue(background: [u8; 3], ink: [u8; 3], light: bool) -> Theme {
        let mut palette = [[0u8; 3]; 16];
        for (dst, src) in palette.iter_mut().zip(PALETTE.iter()) {
            let lum =
                (0.299 * src[0] as f32 + 0.587 * src[1] as f32 + 0.114 * src[2] as f32) / 255.0;
            for c in 0..3 {
                dst[c] = (background[c] as f32 + lum * (ink[c] as f32 - background[c] as f32))
                    .round() as u8;
            }
        }
        Theme {
            palette,
            background,
            foreground: palette[7],
            light,
        }
    }

    pub fn names() -> Vec<&'static str> {
        let mut out = vec!["tool"];
        out.extend(THEMES.iter().map(|t| t.0));
        out.extend(SINGLE_HUE.iter().map(|t| t.0));
        out
    }

    pub fn by_name(name: &str) -> Option<Theme> {
        if name == "tool" {
            return Some(Theme::tool());
        }
        if let Some(&(_, p, bg, fg, light)) = THEMES.iter().find(|t| t.0 == name) {
            return Some(Theme {
                palette: pal(p),
                background: rgb(bg),
                foreground: rgb(fg),
                light,
            });
        }
        SINGLE_HUE
            .iter()
            .find(|t| t.0 == name)
            .map(|&(_, bg, ink, light)| Theme::single_hue(rgb(bg), rgb(ink), light))
    }

    fn fg(&self, i: u8) -> [f32; 3] {
        let c = if i == 7 {
            self.foreground
        } else {
            self.palette[i as usize & 15]
        };
        c.map(|v| v as f32)
    }

    fn bg(&self, i: u8) -> [f32; 3] {
        let c = if i == 0 {
            self.background
        } else {
            self.palette[i as usize & 15]
        };
        c.map(|v| v as f32)
    }
}

/// Draw a conversion: one `glyphs.cell_w x glyphs.cell_h` cell per character,
/// each pixel `bg + coverage * (fg - bg)`.
pub fn to_image(art: &Art, glyphs: &Glyphs, theme: &Theme) -> RgbImage {
    let (cw, ch) = (glyphs.cell_w, glyphs.cell_h);
    let mut img = RgbImage::new((art.cols * cw) as u32, (art.rows * ch) as u32);
    for row in 0..art.rows {
        for (col, cell) in art.row(row).iter().enumerate() {
            let (f, b) = (theme.fg(cell.fg), theme.bg(cell.bg));
            let mask = glyphs.mask(cell.ch);
            for y in 0..ch {
                for x in 0..cw {
                    let a = mask[y * cw + x];
                    let px = std::array::from_fn(|c| {
                        (b[c] + a * (f[c] - b[c])).round().clamp(0.0, 255.0) as u8
                    });
                    img.put_pixel((col * cw + x) as u32, (row * ch + y) as u32, image::Rgb(px));
                }
            }
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::{Cell, Options, convert};
    use crate::stamps::{CELL_PX, stamps};
    use image::{DynamicImage, RgbImage};

    #[test]
    fn builtin_glyphs_round_trip_to_the_solver_stamps() {
        let ours = Glyphs::builtin(CELL_W, CELL_H).stamps();
        assert_eq!(ours.alpha, stamps().alpha);
    }

    #[test]
    fn drawing_is_the_blend_the_solver_scored() {
        let art = Art {
            cols: 2,
            rows: 1,
            cells: vec![
                Cell {
                    ch: b'@',
                    fg: 9,
                    bg: 4,
                },
                Cell {
                    ch: b'#',
                    fg: 15,
                    bg: 0,
                },
            ],
        };
        let img = to_image(&art, &Glyphs::builtin(CELL_W, CELL_H), &Theme::tool());
        let s = stamps();
        for (i, cell) in art.cells.iter().enumerate() {
            let a = s.alpha_of((cell.ch - FIRST_CHAR) as usize);
            for p in 0..CELL_PX {
                let (x, y) = (i * CELL_W + p % CELL_W, p / CELL_W);
                let got = img.get_pixel(x as u32, y as u32).0;
                for c in 0..3 {
                    let (f, b) = (
                        PALETTE[cell.fg as usize][c] as f32,
                        PALETTE[cell.bg as usize][c] as f32,
                    );
                    assert!((got[c] as f32 - (b + a[p] * (f - b))).abs() <= 0.5);
                }
            }
        }
    }

    #[test]
    fn custom_glyphs_drive_the_match() {
        // A glyph set where only '#' has ink: a dark/light image can then only
        // ever come out as spaces and '#'.
        let mut alpha = vec![0.0f32; NCHARS * CELL_PX];
        let hash = (b'#' - FIRST_CHAR) as usize;
        alpha[hash * CELL_PX..(hash + 1) * CELL_PX].fill(1.0);
        let img = DynamicImage::ImageRgb8(RgbImage::from_fn(80, 80, |x, _| {
            if x < 40 {
                image::Rgb([0, 0, 0])
            } else {
                image::Rgb([255, 255, 255])
            }
        }));
        let opts = Options {
            glyphs: Some(std::sync::Arc::new(Stamps::from_alpha(alpha))),
            ..Options::default()
        };
        let art = convert(&img, &opts);
        assert!(art.cells.iter().all(|c| c.ch == b' ' || c.ch == b'#'));
        assert!(art.cells.iter().any(|c| c.ch == b'#'));
    }

    #[test]
    fn every_theme_resolves() {
        for name in Theme::names() {
            assert!(Theme::by_name(name).is_some(), "{name}");
        }
    }
}
