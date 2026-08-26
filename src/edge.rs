//! Sobel edge detection, used by the edge-matching conversion mode.

/// Largest magnitude a Sobel kernel pair can produce on 0.0-1.0 input:
/// `sqrt(4^2 + 4^2)`.
pub const SOBEL_MAX: f32 = 5.656_854_2;

/// Sobel gradient magnitude of a single-channel `w * h` plane.
///
/// Samples outside the plane are clamped to the edge pixel, so a glyph mask
/// keeps the strong border response that makes it recognisable in isolation.
pub fn sobel(src: &[f32], w: usize, h: usize, dst: &mut [f32]) {
    debug_assert_eq!(src.len(), w * h);
    debug_assert_eq!(dst.len(), w * h);

    let at = |x: isize, y: isize| -> f32 {
        let x = x.clamp(0, w as isize - 1) as usize;
        let y = y.clamp(0, h as isize - 1) as usize;
        src[y * w + x]
    };

    for y in 0..h as isize {
        for x in 0..w as isize {
            let (tl, t, tr) = (at(x - 1, y - 1), at(x, y - 1), at(x + 1, y - 1));
            let (l, r) = (at(x - 1, y), at(x + 1, y));
            let (bl, b, br) = (at(x - 1, y + 1), at(x, y + 1), at(x + 1, y + 1));

            let gx = (tr + 2.0 * r + br) - (tl + 2.0 * l + bl);
            let gy = (bl + 2.0 * b + br) - (tl + 2.0 * t + tr);

            dst[(y * w as isize + x) as usize] = (gx * gx + gy * gy).sqrt();
        }
    }
}
