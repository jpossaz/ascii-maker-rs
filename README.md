# ascii-maker-rs

Image to ASCII art. A Rust rewrite of [ascii-maker](../ascii-maker), with two
frontends over one converter: a command line tool and a WebAssembly build that
runs entirely in the browser.

```
                                                
           _gwgggg_                             
        _gWM^    ^$Wg_     $WWWWWWWWWWWWWWWWW   
       ,WK          $Wy    ]W ^Mw_         ]W   
       M$            $W    ]W    ^Mw_      ]W   
       W|            ]W    ]W       ^Wg_   ]W   
       ^Wg          _WF    ]W          ^Wg_]W   
        ^WWg__   _gWW^     ]WgggggggggggggWWW   
           ^$MWWMM^                             
                                                
```

## What makes the output different

Most ASCII art tools map each block of the image to a character by brightness
alone. This one compares *shape*: every 10x20 block is scored against every
printable glyph, and - when colour is on - against every foreground/background
pair of the 16-colour terminal palette. The glyph that renders closest to the
original block wins. That is what makes it usable at small sizes, where a
brightness ramp turns into mush.

## CLI

```
cargo build --release
./target/release/asciimaker -i image.png -w 100 --color --background
```

| Option | Meaning |
| --- | --- |
| `-i, --input FILE` | Input image: PNG, JPEG, GIF, WebP, BMP, TIFF, ... |
| `-o, --output FILE` | Write to a file instead of standard output |
| `-w, --width COLS` | Output width in characters (default: one cell per 10 source pixels) |
| `-c, --color` | Use the full 16-colour palette |
| `-g, --grayscale` | Use only black, white, and the two greys |
| `-b, --background` | Also colour the background of each cell |
| `-d, --defaults` | Leave black backgrounds and white text to the terminal's own colours |
| `--invert` | Negate the image before converting |
| `--invert-lightness` | Flip the image's lightness, keeping hue and saturation |
| `-e, --edge` | Match edge maps instead of raw pixels |
| `--edge-gain GAIN` | Scale the image's edge map; above 1.0 draws faint edges as solid strokes |
| `-f, --format FMT` | `auto`, `ansi`, `text`, or `html` |

Everything above configures the base layer. `--overlay` adds a second one, with
its own copies of the same settings: `--overlay-color`, `--overlay-invert`,
`--overlay-invert-lightness`, `--overlay-edge-gain`, and
`--overlay-background`.

`auto` writes ANSI when standard output is a terminal and plain text
otherwise, so `asciimaker -i logo.png` looks right on screen and
`asciimaker -i logo.png > logo.txt` stays clean.

## Web

```
wasm-pack build --target web --out-dir test/out
python3 -m http.server -d test
```

Then open <http://localhost:8000>. The page exposes the same options, streams
rows as they are solved, and can copy the result out as text, ANSI, or HTML.
Pushes to `main` deploy `test/` to GitHub Pages.

## Inverting

`--invert` negates the channels, so a dark red comes back cyan. `--invert-lightness`
flips the HSL lightness instead and leaves hue and saturation where they were,
so a dark red comes back a light red. They are independent, per layer, and can
be combined.

## Edge mode

Normally a glyph is picked by how closely it reproduces the block's pixels, so
solid areas come out solid. In edge mode both sides are run through a Sobel
filter first and the glyphs are matched on the *edges* instead, which draws
outlines and leaves flat regions blank. Colours, if enabled, are still fitted to
the block afterwards.

The image's edge map is normalised against its 99th percentile, so a single
hard edge cannot wash out the softer ones. `--edge-gain` moves that reference:
raise it to turn faint edges into solid strokes.

## Layers

`--overlay` runs the conversion twice and composites the results. The overlay
always matches on edges and never paints a background; the base layer below it
does whatever you tell it to. Each layer carries its own palette and inversion
settings, so the pairing the whole thing was built for is one command:

```
asciimaker -i image.png -w 120 --color --background \
           --overlay --overlay-color gray --overlay-edge-gain 1.5
```

The composite rule is deliberately blunt: **where the overlay produced a space,
the base layer shows through; everywhere else the overlay wins.** A space is the
overlay saying it found no edge in that cell.

By default the overlay's cells win outright, backgrounds included, so its
glyphs always stand out against black. `--overlay-background base` keeps the
background the layer below chose instead, so the edges are drawn onto the colour
field rather than punched through it - at the cost of the occasional glyph
vanishing where its foreground matches what is behind it.

## Stamps

The original C tool shipped every glyph pre-rendered at all 256 palette pairs -
14.6 MB of image data. Every one of those pixels turns out to be exactly
`bg + alpha * (fg - bg)` for a per-glyph coverage mask, so this version stores
only the masks: `src/alpha.bin`, 19 KB.

That same identity is what keeps the colour search affordable. Expanding the
squared error over the blend separates the terms that depend on the glyph and
the colour pair from the ones that depend on the image, so the 95x16x16x200
inner loop collapses to one 200-pixel pass per glyph plus a few adds per colour
pair - about 200x less work per cell for the identical answer. `cargo test`
checks that equivalence against brute force on every cell of a test image.

`tools/gen_alpha.py` regenerates `alpha.bin` from the legacy blob
(`src/stamps.bin.br`, kept for that purpose) and verifies the blend model.
