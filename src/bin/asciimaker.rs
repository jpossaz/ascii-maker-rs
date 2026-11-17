//! Command line frontend.

use std::io::{BufWriter, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use ascii_maker::{ColorMode, Layer, Options, OverlayBackground, convert, render};
use clap::{Parser, ValueEnum};

#[derive(Parser)]
#[command(
    name = "asciimaker",
    about = "Convert an image to ASCII art",
    long_about = "Convert an image to ASCII art.\n\n\
        Each 10x20 block of the image is matched against every printable ASCII \
        glyph - and, with --color, against every foreground/background pair of \
        the 16-colour terminal palette - keeping whichever renders closest to \
        the original.",
    version
)]
struct Args {
    /// Input image (PNG, JPEG, GIF, WebP, BMP, TIFF, ...)
    #[arg(short, long, value_name = "FILE")]
    input: PathBuf,

    /// Write to FILE instead of standard output
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Width of the output in characters [default: the image's own cell width]
    #[arg(short, long, value_name = "COLS")]
    width: Option<u32>,

    /// Use the full 16-colour palette
    #[arg(short, long, conflicts_with = "grayscale")]
    color: bool,

    /// Use only the palette's black, white, and two greys
    #[arg(short, long)]
    grayscale: bool,

    /// Also colour the background of each cell
    #[arg(short, long)]
    background: bool,

    /// Treat black and white as the terminal's own colours, and leave them unset
    #[arg(short, long)]
    defaults: bool,

    /// Invert the image before converting
    #[arg(long)]
    invert: bool,

    /// Flip the image's lightness before converting, keeping hue and
    /// saturation - dark red becomes light red rather than cyan
    #[arg(long)]
    invert_lightness: bool,

    /// Match glyphs against edge maps instead of raw pixels, which favours
    /// outlines over fill
    #[arg(short, long)]
    edge: bool,

    /// Scale the image's edge map before matching; above 1.0 makes faint edges
    /// draw as solid strokes
    #[arg(long, value_name = "GAIN", default_value_t = 1.0, requires = "edge")]
    edge_gain: f32,

    /// Draw an edge-matched layer on top of the result. Wherever that layer
    /// produces a space, the layer below shows through
    #[arg(long)]
    overlay: bool,

    /// Palette for the overlay layer
    #[arg(long, value_name = "MODE", value_enum, default_value_t = Palette::Mono, requires = "overlay")]
    overlay_color: Palette,

    /// Invert the image for the overlay layer only
    #[arg(long, requires = "overlay")]
    overlay_invert: bool,

    /// Flip the lightness for the overlay layer only
    #[arg(long, requires = "overlay")]
    overlay_invert_lightness: bool,

    /// Edge gain for the overlay layer
    #[arg(long, value_name = "GAIN", default_value_t = 1.0, requires = "overlay")]
    overlay_edge_gain: f32,

    /// Where a composited cell's background comes from when the overlay drew a
    /// glyph: its own (black), or the one the layer below chose
    #[arg(long, value_name = "SOURCE", value_enum, default_value_t = Background::Own, requires = "overlay")]
    overlay_background: Background,

    /// Output format
    #[arg(short, long, value_enum, default_value_t = Format::Auto)]
    format: Format,

    /// Match and draw with this TTF/OTF font instead of the built-in glyphs.
    /// Any font works; each glyph is fitted into the terminal cell
    #[cfg(feature = "raster")]
    #[arg(long, value_name = "FILE")]
    font: Option<PathBuf>,

    /// Colour theme for PNG output: how the 16 palette slots, the default
    /// background and the default foreground look on screen
    #[cfg(feature = "raster")]
    #[arg(long, value_name = "NAME", default_value = "tool",
          value_parser = clap::builder::PossibleValuesParser::new(ascii_maker::raster::Theme::names()))]
    theme: String,

    /// Height in pixels of one character cell in PNG output; cells are half
    /// as wide. The built-in glyphs are drawn at 20
    #[cfg(feature = "raster")]
    #[arg(long, value_name = "PX", default_value_t = 20)]
    cell_height: usize,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Palette {
    /// Black and white only
    Mono,
    /// Black, white, and the two greys
    Gray,
    /// All 16 palette colours
    Color,
}

impl From<Palette> for ColorMode {
    fn from(p: Palette) -> ColorMode {
        match p {
            Palette::Mono => ColorMode::Mono,
            Palette::Gray => ColorMode::Grayscale,
            Palette::Color => ColorMode::Color,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Background {
    /// The overlay's own background
    Own,
    /// The background the layer below chose
    Base,
}

impl From<Background> for OverlayBackground {
    fn from(b: Background) -> OverlayBackground {
        match b {
            Background::Own => OverlayBackground::Own,
            Background::Base => OverlayBackground::Base,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Format {
    /// ANSI when writing to a terminal, plain text otherwise
    Auto,
    /// ANSI escape sequences
    Ansi,
    /// Characters only, no colour
    Text,
    /// A standalone HTML page
    Html,
    /// A PNG image of the art as a terminal would draw it (needs --output)
    #[cfg(feature = "raster")]
    Png,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("asciimaker: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> anyhow::Result<()> {
    let img = image::open(&args.input)
        .map_err(|e| anyhow::anyhow!("{}: {e}", args.input.display()))?;

    let opts = Options {
        cols: args.width,
        base: Layer {
            color: if args.color {
                ColorMode::Color
            } else if args.grayscale {
                ColorMode::Grayscale
            } else {
                ColorMode::Mono
            },
            background: args.background,
            invert: args.invert,
            invert_lightness: args.invert_lightness,
            edge: args.edge,
            edge_gain: args.edge_gain,
        },
        // Only the base layer paints backgrounds; the overlay is there to draw
        // over whatever the base produced.
        overlay: args.overlay.then(|| Layer {
            color: args.overlay_color.into(),
            background: false,
            invert: args.overlay_invert,
            invert_lightness: args.overlay_invert_lightness,
            edge: true,
            edge_gain: args.overlay_edge_gain,
        }),
        overlay_background: args.overlay_background.into(),
        glyphs: None,
    };

    #[cfg(feature = "raster")]
    let font_glyphs = match &args.font {
        Some(path) => {
            let data =
                std::fs::read(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
            let at = |w, h| {
                ascii_maker::raster::Glyphs::from_font(&data, w, h)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
            };
            Some((at(10, 20)?, at(args.cell_height / 2, args.cell_height)?))
        }
        None => None,
    };
    #[cfg(feature = "raster")]
    let opts = Options {
        glyphs: font_glyphs
            .as_ref()
            .map(|(m, _)| std::sync::Arc::new(m.stamps())),
        ..opts
    };

    let art = convert(&img, &opts);
    if art.rows == 0 {
        anyhow::bail!("image is too short to fill a single 20-pixel row at this width");
    }

    let format = match args.format {
        Format::Auto if args.output.is_some() => Format::Text,
        Format::Auto if std::io::stdout().is_terminal() => Format::Ansi,
        Format::Auto => Format::Text,
        explicit => explicit,
    };

    let body = match format {
        Format::Ansi => render::to_ansi(&art, args.defaults),
        Format::Text => render::to_plain(&art),
        Format::Html => html_page(&render::to_html(&art)),
        #[cfg(feature = "raster")]
        Format::Png => {
            use ascii_maker::raster::{Glyphs, Theme, to_image};
            let path = args
                .output
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("--format png needs --output"))?;
            if args.cell_height < 2 {
                anyhow::bail!("--cell-height must be at least 2");
            }
            let glyphs = match &font_glyphs {
                Some((_, draw)) => draw.clone(),
                None => Glyphs::builtin(args.cell_height / 2, args.cell_height),
            };
            let theme = Theme::by_name(&args.theme).expect("clap checked the name");
            to_image(&art, &glyphs, &theme).save(path)?;
            return Ok(());
        }
        Format::Auto => unreachable!("resolved above"),
    };

    match &args.output {
        Some(path) => {
            let file = std::fs::File::create(path)
                .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
            BufWriter::new(file).write_all(body.as_bytes())?;
        }
        None => {
            let stdout = std::io::stdout();
            let mut out = BufWriter::new(stdout.lock());
            out.write_all(body.as_bytes())?;
            out.flush()?;
        }
    }

    Ok(())
}

fn html_page(body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n\
         <html><head><meta charset=\"utf-8\"><title>ascii-maker</title>\n\
         <style>body{{background:#181818;margin:0}}\
         pre{{font:1em/1.2 monospace;white-space:pre;margin:0;padding:1em}}</style>\n\
         </head><body><pre>{body}</pre></body></html>\n"
    )
}
