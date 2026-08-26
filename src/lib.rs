//! Image to ASCII art, shared by the CLI and the web frontend.
//!
//! [`convert::convert`] does the work; [`render`] turns the result into plain
//! text, ANSI, or HTML. The web build additionally exposes the `wasm` module's
//! bindings.

pub mod convert;
pub mod edge;
pub mod palette;
pub mod render;
pub mod stamps;

pub use convert::{Art, Cell, Layer, Options, OverlayBackground, convert, convert_streaming};
pub use palette::ColorMode;

#[cfg(target_arch = "wasm32")]
mod wasm;
