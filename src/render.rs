//! Turning cells into something you can look at.


use crate::convert::{Art, Cell};
use crate::palette::hex;

/// Just the characters, no colour.
pub fn to_plain(art: &Art) -> String {
    let mut out = String::with_capacity(art.cells.len() + art.rows);
    for row in 0..art.rows {
        for cell in art.row(row) {
            out.push(cell.as_char());
        }
        out.push('\n');
    }
    out
}

/// ANSI escapes for a terminal. Colours are emitted only where they change and
/// reset at the end of each line, which keeps the output a fraction of the size
/// of a naive per-character encoding.
///
/// With `defaults` set, background 0 and foreground 7 are left to the terminal's
/// own colours instead of being written out.
pub fn to_ansi(art: &Art, defaults: bool) -> String {
    let mut out = String::with_capacity(art.cells.len() * 3 + art.rows * 8);

    for row in 0..art.rows {
        let mut cur: Option<(Option<u8>, Option<u8>)> = None;

        for cell in art.row(row) {
            let want = (
                (!(defaults && cell.bg == 0)).then_some(cell.bg),
                (!(defaults && cell.fg == 7)).then_some(cell.fg),
            );

            if cur != Some(want) {
                let (bg, fg) = want;
                let prev = cur.unwrap_or((None, None));
                if prev.0 != bg {
                    match bg {
                        Some(i) => push_sgr(&mut out, if i < 8 { 40 + i } else { 92 + i }),
                        None => push_sgr(&mut out, 49),
                    }
                }
                if prev.1 != fg {
                    match fg {
                        Some(i) => push_sgr(&mut out, if i < 8 { 30 + i } else { 82 + i }),
                        None => push_sgr(&mut out, 39),
                    }
                }
                cur = Some(want);
            }

            out.push(cell.as_char());
        }

        if cur.is_some_and(|(bg, fg)| bg.is_some() || fg.is_some()) {
            out.push_str("\x1b[0m");
        }
        out.push('\n');
    }

    out
}

fn push_sgr(out: &mut String, code: u8) {
    out.push_str("\x1b[");
    out.push_str(&code.to_string());
    out.push('m');
}

/// Coloured markup for the browser. Runs of identical colours share a span;
/// the caller is expected to wrap this in a `white-space: pre` monospace block.
pub fn to_html(art: &Art) -> String {
    let mut out = String::with_capacity(art.cells.len() * 2);

    for row in 0..art.rows {
        let mut run: Option<(u8, u8)> = None;
        for cell in art.row(row) {
            if run != Some((cell.bg, cell.fg)) {
                if run.is_some() {
                    out.push_str("</span>");
                }
                out.push_str(&format!(
                    "<span style=\"color:{};background:{}\">",
                    hex(cell.fg),
                    hex(cell.bg)
                ));
                run = Some((cell.bg, cell.fg));
            }
            push_escaped(&mut out, cell.as_char());
        }
        if run.is_some() {
            out.push_str("</span>");
        }
        out.push('\n');
    }

    out
}

fn push_escaped(out: &mut String, ch: char) {
    match ch {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        c => out.push(c),
    }
}

/// Flat `char, fg, bg` triples, for callers that want to do their own drawing.
pub fn to_triples(cells: &[Cell]) -> Vec<u8> {
    let mut out = Vec::with_capacity(cells.len() * 3);
    for c in cells {
        out.extend_from_slice(&[c.ch, c.fg, c.bg]);
    }
    out
}
