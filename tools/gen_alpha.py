#!/usr/bin/env python3
"""Derive the compact alpha-mask + palette representation from the legacy stamp blob.

The legacy `src/stamps.bin.br` holds every glyph pre-rendered at every
(background, foreground) pair of the 16-colour terminal palette:
    stamps[char][bg][fg][y][x] -> (r, g, b)      95 * 16 * 16 * 20 * 10 * 3 bytes

Every one of those pixels is exactly `bg + alpha * (fg - bg)` for a per-glyph
coverage mask `alpha`, so the whole 14.6 MB table is redundant: the 16 palette
entries plus a 95 * 20 * 10 alpha mask reproduce it to within a rounding error.

This script verifies that claim and writes the mask to `src/alpha.bin`.
Run it only if the stamp blob is ever regenerated.
"""
import sys
import brotli

CELL_W, CELL_H, NCHARS = 10, 20, 95
COL, ROW = 3, 3 * CELL_W
FG, BG, CHR = ROW * CELL_H, ROW * CELL_H * 16, ROW * CELL_H * 16 * 16


def main():
    blob = brotli.decompress(open("src/stamps.bin.br", "rb").read())
    expected = NCHARS * 16 * 16 * CELL_H * CELL_W * 3
    assert len(blob) == expected, f"stamp blob is {len(blob)} bytes, expected {expected}"

    def px(c, b, f, y, x):
        i = c * CHR + b * BG + f * FG + y * ROW + x * COL
        return blob[i], blob[i + 1], blob[i + 2]

    # Char 0 is the space: fully uncovered, so it reads back the raw bg colour.
    palette = [px(0, b, 0, 0, 0) for b in range(16)]
    black, white = palette[0], palette[15]

    # Recover alpha from the black-on-white rendering of each glyph.
    mask = bytearray(NCHARS * CELL_H * CELL_W)
    for c in range(NCHARS):
        for y in range(CELL_H):
            for x in range(CELL_W):
                p = px(c, 0, 15, y, x)
                a = [(p[k] - black[k]) / (white[k] - black[k]) for k in range(3)]
                assert max(a) - min(a) < 1e-9, f"channels disagree at {c},{y},{x}: {a}"
                mask[c * CELL_H * CELL_W + y * CELL_W + x] = round(a[0] * 255)

    # Check the blend reproduces every stamp pixel.
    worst = 0
    for c in range(NCHARS):
        for b in range(16):
            for f in range(16):
                for y in range(CELL_H):
                    for x in range(CELL_W):
                        a = mask[c * CELL_H * CELL_W + y * CELL_W + x] / 255
                        act = px(c, b, f, y, x)
                        for k in range(3):
                            pred = palette[b][k] + a * (palette[f][k] - palette[b][k])
                            worst = max(worst, abs(pred - act[k]))
    print(f"worst blend reconstruction error: {worst:.3f} / 255")
    if worst > 1.5:
        sys.exit("blend model does not hold; refusing to write alpha.bin")

    with open("src/alpha.bin", "wb") as fh:
        fh.write(mask)
    print(f"wrote src/alpha.bin ({len(mask)} bytes)")
    print("palette =", palette)


if __name__ == "__main__":
    main()
