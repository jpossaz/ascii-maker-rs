//! Image -> character-cell conversion.
//!
//! # How the match is scored
//!
//! Each output cell is a glyph drawn in one of 16 foreground colours over one
//! of 16 background colours. The cost of a candidate is the squared RGB
//! distance between the rendered cell and the corresponding patch of the
//! resized image.
//!
//! Rendering a candidate is a linear blend, `B + a*(F - B)` for the glyph's
//! coverage mask `a`, so the cost expands into terms that separate cleanly:
//!
//! ```text
//! cost = sum(I^2)                         per cell, same for every candidate
//!      + N*sum(B^2)
//!      + sum_a2 * sum(D^2)                D = F - B
//!      + 2*sum_a * sum(B*D)
//!      - 2*sum(B * SI)                    SI  = sum(I)      per cell
//!      - 2*sum(D * SAI)                   SAI = sum(a*I)    per cell and glyph
//! ```
//!
//! Rows 2-4 depend only on the glyph and the colour pair, so they are tabulated
//! once up front. The `sum(I^2)` term is identical for every candidate and is
//! dropped. That leaves one 200-pixel pass per glyph to get `SAI`, and a few
//! adds per colour pair - instead of the brute-force 95*16*16*200 inner loop
//! the original did, which is ~200x more work per cell for the same answer.

use crate::edge;
use crate::palette::{ColorMode, palette_f32};
use crate::stamps::{CELL_H, CELL_PX, CELL_W, FIRST_CHAR, NCHARS, Stamps, stamps};

use image::{DynamicImage, imageops::FilterType};

/// Settings for a single matching pass.
#[derive(Debug, Clone)]
pub struct Layer {
    /// Which palette entries may be used.
    pub color: ColorMode,
    /// Allow non-black backgrounds.
    pub background: bool,
    /// Negate the image's colours before matching.
    pub invert: bool,
    /// Flip the image's lightness before matching, leaving hue and saturation
    /// alone - a dark red stays red, it just becomes a light red.
    pub invert_lightness: bool,
    /// Pick glyphs by comparing edge maps rather than raw pixels.
    pub edge: bool,
    /// Multiplier applied to the image's edge map before matching. Above 1.0
    /// pushes faint edges towards solid strokes; below 1.0 favours lighter glyphs.
    /// Only used when `edge` is set.
    pub edge_gain: f32,
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            color: ColorMode::Mono,
            background: false,
            invert: false,
            invert_lightness: false,
            edge: false,
            edge_gain: 1.0,
        }
    }
}

/// Where a composited cell's background comes from when the overlay drew a glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverlayBackground {
    /// The overlay's own - black, unless the overlay enables backgrounds.
    /// The overlay's cells win outright, so its glyphs always stand out.
    #[default]
    Own,
    /// The one the base layer chose, so the overlay draws onto the colour field
    /// below instead of punching holes in it. Where the overlay's foreground
    /// happens to match that background, its glyph disappears.
    Base,
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Output width in characters. `None` keeps the image's natural cell width.
    pub cols: Option<u32>,
    /// The layer everything else is drawn over.
    pub base: Layer,
    /// An optional second pass drawn on top. Wherever it produces a space, the
    /// base layer shows through; everywhere else it wins. Typically an edge
    /// layer over a solid one.
    pub overlay: Option<Layer>,
    /// Which layer supplies the background of a composited cell.
    pub overlay_background: OverlayBackground,
}

/// One output character: a glyph plus its two palette indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    /// Printable ASCII codepoint, 32-126.
    pub ch: u8,
    pub fg: u8,
    pub bg: u8,
}

impl Cell {
    pub fn as_char(self) -> char {
        self.ch as char
    }
}

/// A finished conversion.
#[derive(Debug, Clone)]
pub struct Art {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Cell>,
}

impl Art {
    pub fn row(&self, row: usize) -> &[Cell] {
        &self.cells[row * self.cols..(row + 1) * self.cols]
    }
}

/// Output dimensions, in characters, for an image converted with `opts`.
///
/// Cells are 10x20, matching a terminal's aspect ratio, so scaling the image to
/// `cols * 10` pixels wide and keeping its proportions gives the row count.
pub fn output_size(img: &DynamicImage, opts: &Options) -> (usize, usize) {
    let cols = match opts.cols {
        Some(c) if c > 0 => c as usize,
        _ => (img.width() as usize / CELL_W).max(1),
    };
    let scale = (cols * CELL_W) as f64 / img.width().max(1) as f64;
    let rows = (img.height() as f64 * scale) as usize / CELL_H;
    (cols, rows)
}

/// Convert an image, using every available core on native targets.
pub fn convert(img: &DynamicImage, opts: &Options) -> Art {
    let pipeline = Pipeline::new(img, opts);
    let mut cells = vec![Cell { ch: b' ', fg: 0, bg: 0 }; pipeline.cols * pipeline.rows];

    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        cells
            .par_chunks_mut(pipeline.cols.max(1))
            .enumerate()
            .for_each(|(row, out)| pipeline.solve_row(row, out));
    }
    #[cfg(target_arch = "wasm32")]
    for (row, out) in cells.chunks_mut(pipeline.cols.max(1)).enumerate() {
        pipeline.solve_row(row, out);
    }

    pipeline.finish(cells)
}

/// Convert an image row by row, handing each finished row to `on_row` as it is
/// produced. Always single-threaded, so the callback sees rows in order.
pub fn convert_streaming<F>(img: &DynamicImage, opts: &Options, mut on_row: F) -> Art
where
    F: FnMut(usize, &[Cell]),
{
    let pipeline = Pipeline::new(img, opts);
    let mut cells = Vec::with_capacity(pipeline.cols * pipeline.rows);
    let mut row_buf = vec![Cell { ch: b' ', fg: 0, bg: 0 }; pipeline.cols];

    for row in 0..pipeline.rows {
        pipeline.solve_row(row, &mut row_buf);
        on_row(row, &row_buf);
        cells.extend_from_slice(&row_buf);
    }

    pipeline.finish(cells)
}

/// The base layer, plus the overlay drawn on top of it if there is one.
struct Pipeline<'a> {
    base: Solver<'a>,
    overlay: Option<Solver<'a>>,
    overlay_background: OverlayBackground,
    cols: usize,
    rows: usize,
}

impl<'a> Pipeline<'a> {
    fn new(img: &DynamicImage, opts: &Options) -> Pipeline<'a> {
        let (cols, rows) = output_size(img, opts);
        let width = cols * CELL_W;

        // One resize feeds both layers; each then takes its own inverted copy,
        // so at most two planes are alive at once rather than three.
        let mut plane = resize_plane(img, width, rows * CELL_H);
        let overlay = opts.overlay.as_ref().map(|layer| {
            let mut p = plane.clone();
            apply_inversion(&mut p, layer);
            Solver::new(p, width, rows, layer)
        });
        apply_inversion(&mut plane, &opts.base);
        let base = Solver::new(plane, width, rows, &opts.base);

        Pipeline {
            base,
            overlay,
            overlay_background: opts.overlay_background,
            cols,
            rows,
        }
    }

    fn finish(&self, cells: Vec<Cell>) -> Art {
        Art {
            cols: self.cols,
            rows: self.rows,
            cells,
        }
    }

    /// Solve one row, compositing the two layers if there are two.
    ///
    /// A space in the overlay means it had nothing to say about that cell, so
    /// the base layer shows through. Anywhere else the overlay wins.
    fn solve_row(&self, row: usize, out: &mut [Cell]) {
        for (col, cell) in out.iter_mut().enumerate() {
            *cell = match &self.overlay {
                None => self.base.solve_cell(col, row),
                Some(overlay) => {
                    let top = overlay.solve_cell(col, row);
                    match (top.ch, self.overlay_background) {
                        (b' ', _) => self.base.solve_cell(col, row),
                        (_, OverlayBackground::Own) => top,
                        (_, OverlayBackground::Base) => Cell {
                            bg: self.base.solve_cell(col, row).bg,
                            ..top
                        },
                    }
                }
            };
        }
    }
}

/// Scale an image to exactly `width x height` pixels, as an RGB plane of
/// 0.0-1.0 samples.
fn resize_plane(img: &DynamicImage, width: usize, height: usize) -> Vec<f32> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    img.resize_exact(width as u32, height as u32, FilterType::Lanczos3)
        .to_rgb8()
        .iter()
        .map(|&v| v as f32 / 255.0)
        .collect()
}

/// Apply a layer's inversions to a plane, in place.
fn apply_inversion(plane: &mut [f32], layer: &Layer) {
    if layer.invert {
        for v in plane.iter_mut() {
            *v = 1.0 - *v;
        }
    }
    if layer.invert_lightness {
        for px in plane.chunks_exact_mut(3) {
            invert_lightness(px);
        }
    }
}

/// Flip a pixel's HSL lightness, holding hue and saturation fixed.
///
/// Replacing `L` with `1 - L` leaves HSL saturation - and with it the chroma
/// `max - min` - unchanged, which forces `max' = 1 - min` and `min' = 1 - max`.
/// Holding the hue fixed means each channel keeps its position within that
/// range, so the whole transform collapses to a single offset shared by all
/// three. Nothing can leave 0.0-1.0, so no clamping is needed. Pure red already
/// sits at `L = 0.5` and comes back untouched; white becomes black.
fn invert_lightness(px: &mut [f32]) {
    let max = px[0].max(px[1]).max(px[2]);
    let min = px[0].min(px[1]).min(px[2]);
    let offset = 1.0 - max - min;
    for v in px {
        *v += offset;
    }
}

/// One matching pass over the image: a prepared plane plus the cost tables for
/// the palette this layer is allowed to use.
struct Solver<'a> {
    stamps: &'a Stamps,
    /// Resized image, RGB, 0.0-1.0, row-major, with the layer's inversions applied.
    plane: Vec<f32>,
    /// Edge magnitudes of the plane, present only in edge mode.
    edges: Option<Vec<f32>>,
    width: usize,
    palette: [[f32; 3]; 16],
    /// Candidate (background, foreground) palette pairs.
    pairs: Vec<(u8, u8)>,
    /// `NCHARS * pairs.len()` candidate-only cost terms.
    table: Vec<f32>,
}

impl<'a> Solver<'a> {
    fn new(plane: Vec<f32>, width: usize, rows: usize, layer: &Layer) -> Solver<'a> {
        let stamps = stamps();
        let palette = palette_f32();

        let edges = layer
            .edge
            .then(|| image_edges(&plane, width, rows * CELL_H, layer.edge_gain));
        let pairs = color_pairs(layer.color, layer.background);
        let table = cost_table(stamps, &palette, &pairs);

        Solver {
            stamps,
            plane,
            edges,
            width,
            palette,
            pairs,
            table,
        }
    }

    fn solve_cell(&self, col: usize, row: usize) -> Cell {
        let x0 = col * CELL_W;
        let y0 = row * CELL_H;

        // sum(I) over the cell, and from it the background-only cost term.
        let mut si = [0.0f32; 3];
        for y in 0..CELL_H {
            let base = ((y0 + y) * self.width + x0) * 3;
            for x in 0..CELL_W {
                let p = base + x * 3;
                si[0] += self.plane[p];
                si[1] += self.plane[p + 1];
                si[2] += self.plane[p + 2];
            }
        }
        let mut bg_term = [0.0f32; 16];
        for (k, term) in bg_term.iter_mut().enumerate() {
            let c = self.palette[k];
            *term = -2.0 * (c[0] * si[0] + c[1] * si[1] + c[2] * si[2]);
        }

        // In edge mode the glyph is chosen by shape alone; colours are then
        // fitted to that glyph. Otherwise every glyph competes on colour cost.
        let candidates = match self.edges.as_ref() {
            Some(edges) => {
                let c = self.best_edge_char(edges, x0, y0);
                c..c + 1
            }
            None => 0..NCHARS,
        };

        let mut best = Cell { ch: FIRST_CHAR, fg: 0, bg: 0 };
        let mut best_cost = f32::INFINITY;

        for c in candidates {
            // sum(a*I) over the cell for this glyph...
            let alpha = self.stamps.alpha_of(c);
            let mut sai = [0.0f32; 3];
            for y in 0..CELL_H {
                let base = ((y0 + y) * self.width + x0) * 3;
                let arow = y * CELL_W;
                for x in 0..CELL_W {
                    let a = alpha[arow + x];
                    if a == 0.0 {
                        continue;
                    }
                    let p = base + x * 3;
                    sai[0] += a * self.plane[p];
                    sai[1] += a * self.plane[p + 1];
                    sai[2] += a * self.plane[p + 2];
                }
            }
            // ...projected onto each palette colour, so the pair loop below is
            // three adds rather than three dot products.
            let mut proj = [0.0f32; 16];
            for (k, pr) in proj.iter_mut().enumerate() {
                let col = self.palette[k];
                *pr = col[0] * sai[0] + col[1] * sai[1] + col[2] * sai[2];
            }

            let table = &self.table[c * self.pairs.len()..(c + 1) * self.pairs.len()];
            for (&(bg, fg), &konst) in self.pairs.iter().zip(table) {
                let cost = konst + bg_term[bg as usize]
                    - 2.0 * (proj[fg as usize] - proj[bg as usize]);
                if cost < best_cost {
                    best_cost = cost;
                    best = Cell {
                        ch: FIRST_CHAR + c as u8,
                        fg,
                        bg,
                    };
                }
            }
        }

        // A space shows no foreground, so pin it to the background and let the
        // renderers collapse longer runs of colour.
        if best.ch == b' ' {
            best.fg = best.bg;
        }
        best
    }

    /// Glyph whose edge stamp best matches this cell's edge patch.
    fn best_edge_char(&self, edges: &[f32], x0: usize, y0: usize) -> usize {
        let mut best = 0;
        let mut best_cost = f32::INFINITY;
        for c in 0..NCHARS {
            let stamp = self.stamps.edge_of(c);
            let mut cost = 0.0f32;
            for y in 0..CELL_H {
                let base = (y0 + y) * self.width + x0;
                let srow = y * CELL_W;
                for x in 0..CELL_W {
                    let d = edges[base + x] - stamp[srow + x];
                    cost += d * d;
                }
            }
            if cost < best_cost {
                best_cost = cost;
                best = c;
            }
        }
        best
    }
}

/// Fraction of edge pixels that are allowed to saturate. Normalising against
/// the single strongest pixel lets one hard edge wash out every softer one, so
/// the reference level is a high percentile instead.
const EDGE_REFERENCE_PERCENTILE: f32 = 0.99;

/// Luma-weighted Sobel of the resized image, normalised so that a strong edge
/// reads as a solid glyph stroke.
fn image_edges(plane: &[f32], w: usize, h: usize, gain: f32) -> Vec<f32> {
    let luma: Vec<f32> = plane
        .chunks_exact(3)
        .map(|p| 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2])
        .collect();

    let mut out = vec![0.0f32; w * h];
    edge::sobel(&luma, w, h, &mut out);

    let reference = percentile(&out, EDGE_REFERENCE_PERCENTILE);
    let norm = if reference > 0.0 { gain / reference } else { 0.0 };
    for e in &mut out {
        *e = (*e * norm).min(1.0);
    }
    out
}

/// Approximate percentile of edge magnitudes, via a fixed histogram over the
/// range Sobel can produce. Linear in the pixel count, unlike sorting.
fn percentile(values: &[f32], q: f32) -> f32 {
    const BINS: usize = 1024;

    let mut hist = [0usize; BINS];
    for &v in values {
        let bin = ((v / edge::SOBEL_MAX) * BINS as f32) as usize;
        hist[bin.min(BINS - 1)] += 1;
    }

    let target = (values.len() as f32 * q) as usize;
    let mut seen = 0;
    for (bin, count) in hist.iter().enumerate() {
        seen += count;
        if seen >= target {
            return (bin + 1) as f32 / BINS as f32 * edge::SOBEL_MAX;
        }
    }
    edge::SOBEL_MAX
}

/// Every (background, foreground) pair the mode permits. A pair with equal
/// entries is dropped: it renders as a flat cell, which the space glyph already
/// covers with any foreground.
fn color_pairs(mode: ColorMode, background: bool) -> Vec<(u8, u8)> {
    let allowed = mode.allowed();
    let mut pairs = Vec::with_capacity(allowed.len() * allowed.len());
    for &bg in allowed {
        if !background && bg != 0 {
            continue;
        }
        for &fg in allowed {
            if fg != bg {
                pairs.push((bg, fg));
            }
        }
    }
    pairs
}

/// The part of the cost that depends only on the glyph and the colour pair.
fn cost_table(stamps: &Stamps, palette: &[[f32; 3]; 16], pairs: &[(u8, u8)]) -> Vec<f32> {
    let mut table = Vec::with_capacity(NCHARS * pairs.len());
    for c in 0..NCHARS {
        for &(bg, fg) in pairs {
            let b = palette[bg as usize];
            let f = palette[fg as usize];
            let mut konst = 0.0f32;
            for ch in 0..3 {
                let d = f[ch] - b[ch];
                konst += CELL_PX as f32 * b[ch] * b[ch]
                    + stamps.sum_a2[c] * d * d
                    + 2.0 * stamps.sum_a[c] * b[ch] * d;
            }
            table.push(konst);
        }
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stamps::CELL_PX;
    use image::{Rgb, RgbImage};

    impl Options {
        fn color_with_background() -> Options {
            Options {
                base: Layer {
                    color: ColorMode::Color,
                    background: true,
                    ..Layer::default()
                },
                ..Options::default()
            }
        }
    }

    /// Deterministic noisy-but-structured test image.
    fn test_image(w: u32, h: u32) -> DynamicImage {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 33) as u8
        };
        DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
            let blob = ((x * 7 + y * 3) % 255) as u8;
            Rgb([blob, next(), (x ^ y) as u8])
        }))
    }

    /// Flat regions with hard boundaries. Edge matching finds the boundaries
    /// and leaves the interiors blank, so in a composite both layers get to
    /// appear - which uniform noise would not give us.
    fn shapes_image(w: u32, h: u32) -> DynamicImage {
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
            let r = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            if r < w as f32 / 5.0 {
                Rgb([220, 40, 40])
            } else if x > w * 3 / 4 {
                Rgb([40, 180, 90])
            } else {
                Rgb([20, 20, 30])
            }
        }))
    }

    /// Actual squared error of rendering `cell`, straight from the blend
    /// definition, with no algebra applied.
    fn true_cost(solver: &Solver, col: usize, row: usize, cell: Cell) -> f32 {
        let alpha = solver.stamps.alpha_of((cell.ch - FIRST_CHAR) as usize);
        let bg = solver.palette[cell.bg as usize];
        let fg = solver.palette[cell.fg as usize];
        let (x0, y0) = (col * CELL_W, row * CELL_H);

        let mut cost = 0.0f32;
        for y in 0..CELL_H {
            for x in 0..CELL_W {
                let a = alpha[y * CELL_W + x];
                let p = ((y0 + y) * solver.width + x0 + x) * 3;
                for c in 0..3 {
                    let rendered = bg[c] + a * (fg[c] - bg[c]);
                    let d = solver.plane[p + c] - rendered;
                    cost += d * d;
                }
            }
        }
        cost
    }

    fn brute_force_best(solver: &Solver, col: usize, row: usize) -> f32 {
        let mut best = f32::INFINITY;
        for c in 0..NCHARS {
            for &(bg, fg) in &solver.pairs {
                let cell = Cell {
                    ch: FIRST_CHAR + c as u8,
                    fg,
                    bg,
                };
                best = best.min(true_cost(solver, col, row, cell));
            }
        }
        best
    }

    /// The solver's factored cost has to agree with rendering every candidate
    /// and measuring it - that equivalence is the whole reason it can skip the
    /// 95*16*16*200 inner loop.
    #[test]
    fn analytic_cost_matches_brute_force() {
        let img = test_image(240, 200);
        for base in [
            Layer::default(),
            Layer {
                color: ColorMode::Grayscale,
                ..Layer::default()
            },
            Layer {
                color: ColorMode::Color,
                background: true,
                ..Layer::default()
            },
        ] {
            let opts = Options {
                base: base.clone(),
                ..Options::default()
            };
            let (cols, rows) = output_size(&img, &opts);
            assert!(rows > 0 && cols > 0);
            let solver = Solver::new(
                resize_plane(&img, cols * CELL_W, rows * CELL_H),
                cols * CELL_W,
                rows,
                &base,
            );

            for row in 0..rows {
                for col in 0..cols {
                    let picked = solver.solve_cell(col, row);
                    let picked_cost = true_cost(&solver, col, row, picked);
                    let best = brute_force_best(&solver, col, row);
                    assert!(
                        (picked_cost - best).abs() <= 1e-3 * best.max(1.0),
                        "{:?} at ({col},{row}) costs {picked_cost}, best is {best}",
                        base.color
                    );
                }
            }
        }
    }

    #[test]
    fn modes_restrict_the_palette() {
        let img = test_image(240, 200);

        let mono = convert(&img, &Options::default());
        assert!(mono.cells.iter().all(|c| c.bg == 0));
        assert!(mono.cells.iter().all(|c| c.fg == 0 || c.fg == 15));

        let gray = convert(
            &img,
            &Options {
                base: Layer {
                    color: ColorMode::Grayscale,
                    ..Layer::default()
                },
                ..Options::default()
            },
        );
        assert!(gray.cells.iter().all(|c| matches!(c.fg, 0 | 7 | 8 | 15)));

        let color = convert(&img, &Options::color_with_background());
        assert!(color.cells.iter().any(|c| c.bg != 0));
    }

    #[test]
    fn every_cell_is_printable_ascii() {
        let img = test_image(240, 200);
        let art = convert(&img, &Options::color_with_background());
        assert_eq!(art.cells.len(), art.cols * art.rows);
        assert!(art.cells.iter().all(|c| (0x20..=0x7e).contains(&c.ch)));
    }

    #[test]
    fn edge_mode_picks_glyphs_that_look_like_the_edges() {
        // A hard vertical edge down the middle of a single cell.
        let img = DynamicImage::ImageRgb8(RgbImage::from_fn(CELL_W as u32, CELL_H as u32, |x, _| {
            if x < CELL_W as u32 / 2 {
                Rgb([0, 0, 0])
            } else {
                Rgb([255, 255, 255])
            }
        }));
        let layer = Layer {
            edge: true,
            ..Layer::default()
        };
        let opts = Options {
            cols: Some(1),
            base: layer.clone(),
            ..Options::default()
        };
        let art = convert(&img, &opts);
        assert_eq!(art.cells.len(), 1);

        // Whatever glyph it lands on, its edge stamp must be the closest one.
        let solver = Solver::new(
            resize_plane(&img, CELL_W, CELL_H),
            CELL_W,
            1,
            &layer,
        );
        let edges = solver.edges.as_ref().expect("edge mode builds an edge map");
        let picked = (art.cells[0].ch - FIRST_CHAR) as usize;
        let cost = |c: usize| -> f32 {
            solver
                .stamps
                .edge_of(c)
                .iter()
                .zip(edges)
                .map(|(s, e)| (s - e) * (s - e))
                .sum()
        };
        let best = (0..NCHARS).map(cost).fold(f32::INFINITY, f32::min);
        assert!((cost(picked) - best).abs() < 1e-5);
    }

    #[test]
    fn output_size_preserves_aspect_ratio() {
        // 10x20 cells against a 400x400 image: 20 cells wide, 10 tall.
        let img = test_image(400, 400);
        assert_eq!(
            output_size(
                &img,
                &Options {
                    cols: Some(20),
                    ..Options::default()
                }
            ),
            (20, 10)
        );
        // No width given: one cell per 10 pixels of source.
        assert_eq!(output_size(&img, &Options::default()), (40, 20));
    }

    #[test]
    fn glyph_constants_agree_with_the_masks() {
        let s = stamps();
        for c in 0..NCHARS {
            let alpha = s.alpha_of(c);
            assert_eq!(alpha.len(), CELL_PX);
            let sum: f32 = alpha.iter().sum();
            assert!((sum - s.sum_a[c]).abs() < 1e-3);
        }
        // Glyph 0 is the space and covers nothing.
        assert_eq!(s.sum_a[0], 0.0);
    }

    /// Hue in degrees, HSL saturation, and lightness.
    fn hsl(px: [f32; 3]) -> (f32, f32, f32) {
        let max = px[0].max(px[1]).max(px[2]);
        let min = px[0].min(px[1]).min(px[2]);
        let chroma = max - min;
        let light = (max + min) / 2.0;

        let hue = if chroma == 0.0 {
            0.0
        } else if max == px[0] {
            60.0 * (((px[1] - px[2]) / chroma) % 6.0)
        } else if max == px[1] {
            60.0 * ((px[2] - px[0]) / chroma + 2.0)
        } else {
            60.0 * ((px[0] - px[1]) / chroma + 4.0)
        };

        let sat = if light == 0.0 || light == 1.0 {
            0.0
        } else {
            chroma / (1.0 - (2.0 * light - 1.0).abs())
        };

        ((hue + 360.0) % 360.0, sat, light)
    }

    /// The point of lightness inversion, as opposed to negating the channels:
    /// a dark red has to come back as a light red, not as a cyan.
    #[test]
    fn lightness_inversion_keeps_hue_and_saturation() {
        let colors = [
            [0.8, 0.2, 0.2],
            [0.1, 0.1, 0.4],
            [0.9, 0.9, 0.2],
            [0.3, 0.6, 0.45],
            [0.05, 0.5, 0.95],
            [0.5, 0.5, 0.5],
        ];

        for original in colors {
            let mut px = original;
            invert_lightness(&mut px);

            let (h0, s0, l0) = hsl(original);
            let (h1, s1, l1) = hsl(px);
            assert!((h1 - h0).abs() < 0.05, "hue moved: {original:?} -> {px:?}");
            assert!((s1 - s0).abs() < 1e-4, "saturation moved: {original:?} -> {px:?}");
            assert!((l1 - (1.0 - l0)).abs() < 1e-4, "lightness wrong: {original:?} -> {px:?}");
            assert!(px.iter().all(|v| (0.0..=1.0).contains(v)), "out of gamut: {px:?}");
        }
    }

    #[test]
    fn lightness_inversion_endpoints() {
        let mut white = [1.0, 1.0, 1.0];
        invert_lightness(&mut white);
        assert_eq!(white, [0.0, 0.0, 0.0]);

        // Pure red is already at L = 0.5, so it is its own inverse - where
        // negating the channels would have turned it cyan.
        let mut red = [1.0, 0.0, 0.0];
        invert_lightness(&mut red);
        assert_eq!(red, [1.0, 0.0, 0.0]);
    }

    fn composite_layers() -> (Layer, Layer) {
        let base = Layer {
            color: ColorMode::Color,
            background: true,
            ..Layer::default()
        };
        let overlay = Layer {
            color: ColorMode::Grayscale,
            edge: true,
            edge_gain: 1.5,
            ..Layer::default()
        };
        (base, overlay)
    }

    /// Compositing must be exactly "the overlay wins unless it drew a space",
    /// with each layer solved as if it were on its own.
    #[test]
    fn overlay_shows_the_base_through_its_spaces() {
        let img = shapes_image(400, 300);
        let (base, overlay) = composite_layers();

        let base_only = convert(
            &img,
            &Options {
                base: base.clone(),
                ..Options::default()
            },
        );
        let overlay_only = convert(
            &img,
            &Options {
                base: overlay.clone(),
                ..Options::default()
            },
        );
        let composited = convert(
            &img,
            &Options {
                base,
                overlay: Some(overlay),
                ..Options::default()
            },
        );

        assert_eq!(composited.cells.len(), base_only.cells.len());
        let mut showed_through = 0;
        let mut drew_over = 0;
        for (i, cell) in composited.cells.iter().enumerate() {
            if overlay_only.cells[i].ch == b' ' {
                assert_eq!(*cell, base_only.cells[i], "cell {i} should show the base");
                showed_through += 1;
            } else {
                assert_eq!(*cell, overlay_only.cells[i], "cell {i} should show the overlay");
                drew_over += 1;
            }
        }
        // A composite where one layer never appears would pass the checks above
        // while testing nothing.
        assert!(showed_through > 0 && drew_over > 0);
    }

    #[test]
    fn overlay_can_take_the_background_from_the_layer_below() {
        let img = shapes_image(400, 300);
        let (base, overlay) = composite_layers();

        let base_only = convert(
            &img,
            &Options {
                base: base.clone(),
                ..Options::default()
            },
        );
        let composited = convert(
            &img,
            &Options {
                base,
                overlay: Some(overlay),
                overlay_background: OverlayBackground::Base,
                ..Options::default()
            },
        );

        let mut differed = 0;
        for (i, cell) in composited.cells.iter().enumerate() {
            assert_eq!(cell.bg, base_only.cells[i].bg, "cell {i} kept the wrong background");
            if cell.ch != base_only.cells[i].ch {
                differed += 1;
            }
        }
        assert!(differed > 0, "the overlay drew nothing");
    }

    #[test]
    fn layers_can_be_inverted_independently() {
        let img = shapes_image(400, 300);
        let plain = Layer::default();
        let flipped = Layer {
            invert: true,
            ..Layer::default()
        };

        let a = convert(
            &img,
            &Options {
                base: plain.clone(),
                overlay: Some(Layer { edge: true, ..flipped.clone() }),
                ..Options::default()
            },
        );
        let b = convert(
            &img,
            &Options {
                base: flipped,
                overlay: Some(Layer { edge: true, ..plain }),
                ..Options::default()
            },
        );
        assert_ne!(a.cells, b.cells);
    }
}
