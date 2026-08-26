//! wasm-bindgen surface for the web frontend.

use wasm_bindgen::prelude::*;

use crate::convert::{self, Layer, Options, OverlayBackground};
use crate::palette;
use crate::render;

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// The result of a conversion, kept on the wasm side so the page can ask for
/// whichever representation it wants without converting twice.
#[wasm_bindgen]
pub struct AsciiArt {
    inner: convert::Art,
}

#[wasm_bindgen]
impl AsciiArt {
    #[wasm_bindgen(getter)]
    pub fn cols(&self) -> usize {
        self.inner.cols
    }

    #[wasm_bindgen(getter)]
    pub fn rows(&self) -> usize {
        self.inner.rows
    }

    /// Characters only.
    pub fn text(&self) -> String {
        render::to_plain(&self.inner)
    }

    /// Coloured `<span>` markup, for a `white-space: pre` container.
    pub fn html(&self) -> String {
        render::to_html(&self.inner)
    }

    /// ANSI escapes, for pasting into a terminal.
    pub fn ansi(&self, defaults: bool) -> String {
        render::to_ansi(&self.inner, defaults)
    }

    /// Flat `char, fg, bg` triples.
    pub fn triples(&self) -> Vec<u8> {
        render::to_triples(&self.inner.cells)
    }
}

/// Convert an encoded image (PNG, JPEG, GIF, WebP, ...).
///
/// `options` is a plain object:
///
/// ```js
/// {
///   cols: 100,
///   base:    { color: 'color', background: true, invert: false,
///              invertLightness: false, edge: false, edgeGain: 1.0 },
///   overlay: { color: 'gray', invert: false, invertLightness: false,
///              edgeGain: 1.5 },            // omit or null for a single layer
///   overlayBackground: 'own',              // or 'base'
/// }
/// ```
///
/// The overlay always matches on edges and never paints its own background;
/// those are what make it an overlay.
///
/// `on_row`, if given, is called as `(rowIndex, triples, totalRows)` with each
/// row of `char, fg, bg` triples as it is solved, so the page can draw
/// progressively. Passing it forces the single-threaded path.
#[wasm_bindgen]
pub fn convert_image(
    image: &[u8],
    options: &JsValue,
    on_row: Option<js_sys::Function>,
) -> Result<AsciiArt, JsValue> {
    if image.is_empty() {
        return Err(JsValue::from_str("no image data"));
    }

    let opts = read_options(options)?;

    let img = image::load_from_memory(image)
        .map_err(|e| JsValue::from_str(&format!("could not decode image: {e}")))?;

    let art = match on_row {
        Some(cb) => {
            let total = convert::output_size(&img, &opts).1 as f64;
            convert::convert_streaming(&img, &opts, |row, cells| {
                let triples = render::to_triples(cells);
                let _ = cb.call3(
                    &JsValue::NULL,
                    &JsValue::from_f64(row as f64),
                    &js_sys::Uint8Array::from(&triples[..]).into(),
                    &JsValue::from_f64(total),
                );
            })
        }
        None => convert::convert(&img, &opts),
    };

    Ok(AsciiArt { inner: art })
}

fn read_options(options: &JsValue) -> Result<Options, JsValue> {
    let base = read_layer(&field(options, "base"))?;

    let overlay = match field(options, "overlay") {
        v if v.is_undefined() || v.is_null() => None,
        v => Some(Layer {
            edge: true,
            background: false,
            ..read_layer(&v)?
        }),
    };

    let overlay_background = match field(options, "overlayBackground").as_string().as_deref() {
        None | Some("own") => OverlayBackground::Own,
        Some("base") => OverlayBackground::Base,
        Some(other) => {
            return Err(JsValue::from_str(&format!(
                "unknown overlay background `{other}`"
            )));
        }
    };

    Ok(Options {
        cols: field(options, "cols").as_f64().map(|c| c as u32),
        base,
        overlay,
        overlay_background,
    })
}

fn read_layer(layer: &JsValue) -> Result<Layer, JsValue> {
    let defaults = Layer::default();

    let color = match field(layer, "color").as_string() {
        Some(s) => s.parse().map_err(|e: String| JsValue::from_str(&e))?,
        None => defaults.color,
    };

    Ok(Layer {
        color,
        background: field(layer, "background").as_bool().unwrap_or(defaults.background),
        invert: field(layer, "invert").as_bool().unwrap_or(defaults.invert),
        invert_lightness: field(layer, "invertLightness")
            .as_bool()
            .unwrap_or(defaults.invert_lightness),
        edge: field(layer, "edge").as_bool().unwrap_or(defaults.edge),
        edge_gain: field(layer, "edgeGain")
            .as_f64()
            .map_or(defaults.edge_gain, |g| g as f32),
    })
}

/// Read a property, treating a missing object or property alike: absent values
/// fall back to the Rust-side defaults rather than being an error.
fn field(obj: &JsValue, key: &str) -> JsValue {
    js_sys::Reflect::get(obj, &JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED)
}

/// The 16 palette colours as `#rrggbb`, in ANSI order.
#[wasm_bindgen]
pub fn palette_hex() -> Vec<String> {
    (0..16u8).map(palette::hex).collect()
}
