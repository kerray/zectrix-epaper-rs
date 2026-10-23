#!/usr/bin/env python3
"""Reference renderer for NOTE4 screen bundles and patches (docs/INTERACTIVE.md).

    pip install pillow
    tools/screen-bundle.py bundle lights.n4s --title "Lights" --selected 1 \
        "Kitchen: off" "Living room: scene 1" "Hall: off"
    tools/screen-bundle.py patch reply.n4p --title "Lights" --index 0 "Kitchen: on"

Layout (the same as the Rust test fixture in crates/zectrix-screen/tests/common):
an optional title is a non-selectable header (flags bit 0, row 0, empty rectangle, no
strip), then one full-width row of --row-height px per item. A row's highlight strip is
the row inverted. --selected counts items from 0 (the header is not an item).

`bundle` writes application/x-note4-screen, `patch` writes application/x-note4-patch for
item --index with both planes (unselected and highlighted), so the row keeps its
highlight if it is selected. Check the output with
`cargo run -p zectrix-screen --example n4s-check -- FILE [out.pbm]`.

Integers are big-endian. 1 bpp: 1 = white, MSB = leftmost pixel, top line first
(Pillow's mode "1" tobytes() for widths that are multiples of 8).
"""

import argparse
import struct
import sys

from PIL import Image, ImageChops, ImageDraw, ImageFont

W, H = 400, 300
MAX_ROWS = 32


def font(path, size):
    if path:
        return ImageFont.truetype(path, size)
    try:
        return ImageFont.truetype("DejaVuSans.ttf", size)
    except OSError:
        return ImageFont.load_default()


def render(title, items, row_h, fnt):
    """Base frame (mode "1", every row unselected) and the rectangle of each item."""
    img = Image.new("1", (W, H), 1)
    d = ImageDraw.Draw(img)
    top = 0
    if title is not None:
        d.text((8, row_h // 2), title, font=fnt, fill=0, anchor="lm")
        d.rectangle((0, row_h - 2, W - 1, row_h - 1), fill=0)
        top = row_h
    rects = []
    for i, text in enumerate(items):
        y = top + i * row_h
        if y + row_h > H:
            sys.exit(f"item {i} ends at y={y + row_h}, below the panel ({H} px)")
        d.text((8, y + row_h // 2), text, font=fnt, fill=0, anchor="lm")
        rects.append((0, y, W, row_h))
    return img, rects


def plane(img, rect, invert=False):
    x, y, w, h = rect
    assert x % 8 == 0 and w % 8 == 0
    part = img.crop((x, y, x + w, y + h))
    if invert:
        part = ImageChops.invert(part.convert("L")).convert("1")
    return part.tobytes()


def rect_bytes(rect):
    return struct.pack(">4H", *rect)


def bundle(args, fnt):
    img, rects = render(args.title, args.items, args.row_height, fnt)
    header = args.title is not None
    rows = ([((0, 0, 0, 0), b"")] if header else []) + [(r, plane(img, r, True)) for r in rects]
    if len(rows) > MAX_ROWS:
        sys.exit(f"{len(rows)} rows, at most {MAX_ROWS}")
    selected = (args.selected + header) if rects else 0
    if rects and not 0 <= args.selected < len(rects):
        sys.exit(f"--selected {args.selected}: there are {len(rects)} items")
    out = b"N4S1" + bytes([1, int(header), len(rows), selected]) + img.tobytes()
    for r, strip in rows:
        out += rect_bytes(r) + strip
    return out


def patch(args, fnt):
    # Render the whole list with the new text for one item, then cut that item's row.
    if len(args.items) != 1:
        sys.exit("patch takes exactly one item text")
    items = [""] * args.index + args.items
    img, rects = render(args.title, items, args.row_height, fnt)
    r = rects[args.index]
    # Only the row itself is cut, so the title and other rows are not redrawn.
    return rect_bytes(r) + plane(img, r) + plane(img, r, True)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("kind", choices=["bundle", "patch"])
    p.add_argument("out")
    p.add_argument("items", nargs="*")
    p.add_argument("--title")
    p.add_argument("--selected", type=int, default=0, help="selected item (bundle)")
    p.add_argument("--index", type=int, default=0, help="item to replace (patch)")
    p.add_argument("--row-height", type=int, default=40)
    p.add_argument("--font", help="TrueType font file (default DejaVuSans)")
    p.add_argument("--size", type=int, default=22)
    args = p.parse_args()
    fnt = font(args.font, args.size)
    data = bundle(args, fnt) if args.kind == "bundle" else patch(args, fnt)
    with open(args.out, "wb") as f:
        f.write(data)
    print(f"{args.out}: {len(data)} B")


if __name__ == "__main__":
    main()
