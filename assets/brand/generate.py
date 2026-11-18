"""Generates den's brand assets from the same pixel grid as the TUI sprite.

    python3 assets/brand/generate.py      (needs Pillow; macOS for den.icns)
"""
import os, shutil, subprocess, tempfile
from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
ORANGE = (0xD9, 0x77, 0x57)
EYE = (0x1F, 0x1B, 0x18)
INK = (0x1F, 0x1B, 0x18)
CREAM = (0xF5, 0xF1, 0xEA)

# Clawd on a grid of square "half cells": 16 wide, 10 tall.
def mark_cells():
    cells = []
    for x in range(1, 15):
        for y in range(0, 8):
            cells.append((x, y, ORANGE))
    for x in (0, 15):
        cells.append((x, 2, ORANGE))
    for x in (2, 4, 11, 13):
        cells.append((x, 9, ORANGE))
    eyes = {(4, 2), (4, 3), (11, 2), (11, 3)}
    return [(x, y, EYE if (x, y) in eyes else c) for x, y, c in cells]

def svg_mark(unit=10, pad=0):
    w, h = 16 * unit + 2 * pad, 10 * unit + 2 * pad
    rects = "".join(
        f'<rect x="{pad + x*unit}" y="{pad + y*unit}" width="{unit}" height="{unit}" fill="#{c[0]:02X}{c[1]:02X}{c[2]:02X}"/>'
        for x, y, c in mark_cells()
    )
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}" shape-rendering="crispEdges">{rects}</svg>\n'

def draw_mark(img, ox, oy, unit):
    d = ImageDraw.Draw(img)
    for x, y, c in mark_cells():
        d.rectangle([ox + x*unit, oy + y*unit, ox + (x+1)*unit - 1, oy + (y+1)*unit - 1], fill=c)

def app_icon(size):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    inset = round(size * 0.1)
    d.rounded_rectangle([inset, inset, size - inset - 1, size - inset - 1], radius=round(size * 0.18), fill=INK)
    unit = max(1, (size - 2*inset) * 62 // 100 // 16)
    mw, mh = 16 * unit, 10 * unit
    draw_mark(img, (size - mw) // 2, (size - mh) // 2 + unit // 2, unit)
    return img

def main():
    open(os.path.join(HERE, "den-mark.svg"), "w").write(svg_mark())
    # Wordmark: mark plus "den" set in pixel letters (5x7 grid each).
    letters = {
        "d": ["....#", "....#", ".####", "#...#", "#...#", "#...#", ".####"],
        "e": [".....", ".....", ".###.", "#...#", "#####", "#....", ".####"],
        "n": [".....", ".....", "####.", "#...#", "#...#", "#...#", "#...#"],
    }
    unit = 10
    parts = [svg_mark(unit).split(">", 1)[1].rsplit("</svg>", 1)[0]]
    x0 = 16 * unit + 3 * unit
    for i, ch in enumerate("den"):
        for row, line in enumerate(letters[ch]):
            for col, px in enumerate(line):
                if px == "#":
                    parts.append(f'<rect x="{x0 + (i*6 + col)*unit}" y="{(row + 3)*unit - 30}" width="{unit}" height="{unit}" fill="currentColor"/>')
    w = x0 + 3 * 6 * unit
    open(os.path.join(HERE, "den-wordmark.svg"), "w").write(
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {10*unit}" width="{w}" height="{10*unit}" shape-rendering="crispEdges" color="#{INK[0]:02X}{INK[1]:02X}{INK[2]:02X}">{"".join(parts)}</svg>\n'
    )
    for s in (64, 256, 1024):
        app_icon(s).save(os.path.join(HERE, f"den-icon-{s}.png"))
    m = Image.new("RGBA", (16 * 32, 10 * 32), (0, 0, 0, 0))
    draw_mark(m, 0, 0, 32)
    m.save(os.path.join(HERE, "den-mark.png"))
    if shutil.which("iconutil"):
        tmp = tempfile.mkdtemp()
        iset = os.path.join(tmp, "den.iconset")
        os.mkdir(iset)
        for s in (16, 32, 128, 256, 512):
            app_icon(s).save(os.path.join(iset, f"icon_{s}x{s}.png"))
            app_icon(s * 2).save(os.path.join(iset, f"icon_{s}x{s}@2x.png"))
        subprocess.run(["iconutil", "-c", "icns", iset, "-o", os.path.join(HERE, "den.icns")], check=True)
        shutil.rmtree(tmp)

if __name__ == "__main__":
    main()
