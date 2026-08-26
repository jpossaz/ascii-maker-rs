//! Glyph coverage masks and the per-glyph constants the solver needs.
//!
//! The original C tool shipped every glyph pre-rendered at all 256
//! (background, foreground) palette pairs: 14.6 MB of stamps. Each of those
//! pixels is exactly `bg + alpha * (fg - bg)`, so all that is really needed is
//! one 10x20 coverage mask per glyph - 19 KB for the whole set. `tools/gen_alpha.py`
//! derives `alpha.bin` from the legacy blob and checks the blend model holds.

use std::sync::OnceLock;

use crate::edge;

pub const CELL_W: usize = 10;
pub const CELL_H: usize = 20;
pub const CELL_PX: usize = CELL_W * CELL_H;
/// Printable ASCII, space (0x20) through '~' (0x7e).
pub const NCHARS: usize = 95;
/// Codepoint of glyph 0.
pub const FIRST_CHAR: u8 = 32;

const ALPHA_RAW: &[u8] = include_bytes!("alpha.bin");

pub struct Stamps {
    /// Coverage per glyph pixel, 0.0-1.0, `NCHARS * CELL_PX`.
    pub alpha: Vec<f32>,
    /// Sum of alpha over each glyph.
    pub sum_a: [f32; NCHARS],
    /// Sum of alpha squared over each glyph.
    pub sum_a2: [f32; NCHARS],
    /// Sobel magnitude of each coverage mask, normalised to 0.0-1.0.
    pub edges: Vec<f32>,
}

pub fn stamps() -> &'static Stamps {
    static STAMPS: OnceLock<Stamps> = OnceLock::new();
    STAMPS.get_or_init(build)
}

fn build() -> Stamps {
    assert_eq!(
        ALPHA_RAW.len(),
        NCHARS * CELL_PX,
        "alpha.bin has the wrong size"
    );

    let alpha: Vec<f32> = ALPHA_RAW.iter().map(|&v| v as f32 / 255.0).collect();

    let mut sum_a = [0.0f32; NCHARS];
    let mut sum_a2 = [0.0f32; NCHARS];
    for c in 0..NCHARS {
        let cell = &alpha[c * CELL_PX..(c + 1) * CELL_PX];
        sum_a[c] = cell.iter().sum();
        sum_a2[c] = cell.iter().map(|a| a * a).sum();
    }

    // Edge stamps: Sobel each mask on its own, then normalise the whole set by
    // its strongest response so glyph and image edge maps share a scale.
    let mut edges = vec![0.0f32; NCHARS * CELL_PX];
    for c in 0..NCHARS {
        edge::sobel(
            &alpha[c * CELL_PX..(c + 1) * CELL_PX],
            CELL_W,
            CELL_H,
            &mut edges[c * CELL_PX..(c + 1) * CELL_PX],
        );
    }
    let peak = edges.iter().copied().fold(0.0f32, f32::max);
    if peak > 0.0 {
        for e in &mut edges {
            *e /= peak;
        }
    }

    Stamps {
        alpha,
        sum_a,
        sum_a2,
        edges,
    }
}

impl Stamps {
    pub fn alpha_of(&self, char_id: usize) -> &[f32] {
        &self.alpha[char_id * CELL_PX..(char_id + 1) * CELL_PX]
    }

    pub fn edge_of(&self, char_id: usize) -> &[f32] {
        &self.edges[char_id * CELL_PX..(char_id + 1) * CELL_PX]
    }
}
