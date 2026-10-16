#!/usr/bin/env python3
"""Regenerate the assets that are derived from `logo.png`.

Run from the repository root:

    python3 tools/gen_logo.py

It writes:

* ``src/logo.rs``        - the truecolour pixel grid the TUI draws at runtime
* ``assets/hyprdeck.png`` - the 256x256 application icon

Requires Pillow (``python -m pip install pillow``).
"""

from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "logo.png"

# The square artwork is padded with a dark background; only this region holds
# the actual mark.
CONTENT_BOX = (292, 272, 964, 968)

# Resolution of the embedded grid. Kept small on purpose: it is downsampled at
# runtime, so a bigger grid only bloats the binary.
GRID = 32

# Pixels darker than this (summed RGB) are treated as transparent so the mark
# sits cleanly on any terminal background, light or dark.
LUMA_CUTOFF = 90


def pixel_grid():
    image = Image.open(SOURCE).convert("RGB").crop(CONTENT_BOX).resize((GRID, GRID), Image.LANCZOS)
    px = image.load()

    rows = []
    for y in range(GRID):
        values = []
        for x in range(GRID):
            r, g, b = px[x, y]
            values.append(0 if r + g + b < LUMA_CUTOFF else (r << 16) | (g << 8) | b)
        rows.append(values)
    return rows


def write_logo_module(rows):
    data = ",\n    ".join(
        "[" + ", ".join(f"0x{value:06x}" for value in row) + "]" for row in rows
    )
    template = (ROOT / "tools" / "logo.rs.in").read_text()
    output = template.replace("__SRC__", str(GRID)).replace("__DATA__", data)
    (ROOT / "src" / "logo.rs").write_text(output)
    print(f"wrote src/logo.rs ({len(output)} bytes)")


def write_icon():
    logo = Image.open(SOURCE).convert("RGB")
    left, top, right, bottom = CONTENT_BOX
    margin = int(max(right - left, bottom - top) * 0.14)

    mark = logo.crop(
        (
            max(left - margin, 0),
            max(top - margin, 0),
            min(right + margin, logo.width),
            min(bottom + margin, logo.height),
        )
    )

    side = max(mark.size)
    scale = 256 / side
    resized = mark.resize(
        (max(1, round(mark.width * scale)), max(1, round(mark.height * scale))),
        Image.LANCZOS,
    )

    canvas = Image.new("RGB", (256, 256), (12, 12, 19))
    canvas.paste(resized, ((256 - resized.width) // 2, (256 - resized.height) // 2))
    canvas.save(ROOT / "assets" / "hyprdeck.png", optimize=True)
    print("wrote assets/hyprdeck.png (256x256)")


def main():
    rows = pixel_grid()
    write_logo_module(rows)
    write_icon()


if __name__ == "__main__":
    main()
