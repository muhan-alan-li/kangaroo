#!/usr/bin/env python3
"""Draw the Kangaroo marks from one set of shapes.

Two outputs, both PNG, both from the same silhouette:

  assets/kangaroo-menubar.png   36x36, black on transparent, used as a macOS
                                template image, so the menu bar tints it.
  assets/kangaroo-app.png       1024x1024 dock icon, warm gradient behind a
                                dark kangaroo, on the macOS rounded square.

Run it again after a shape change:

    python3 assets/make-icons.py
"""

import math
import os

from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
UNIT = 1024          # design canvas, y grows down
SS = 4               # supersample factor


def ellipse(cx, cy, rx, ry, angle=0.0, steps=180):
    cos_a, sin_a = math.cos(math.radians(angle)), math.sin(math.radians(angle))
    points = []
    for i in range(steps):
        t = 2 * math.pi * i / steps
        x, y = rx * math.cos(t), ry * math.sin(t)
        points.append((cx + x * cos_a - y * sin_a, cy + x * sin_a + y * cos_a))
    return points


def spine(points, widths, steps=40):
    """Outline of a tapered stroke along a polyline."""
    left, right = [], []
    for i in range(len(points) - 1):
        (x0, y0), (x1, y1) = points[i], points[i + 1]
        length = math.hypot(x1 - x0, y1 - y0)
        nx, ny = -(y1 - y0) / length, (x1 - x0) / length
        for j in range(steps):
            t = j / steps
            x, y = x0 + (x1 - x0) * t, y0 + (y1 - y0) * t
            w = (widths[i] + (widths[i + 1] - widths[i]) * t) / 2
            left.append((x + nx * w, y + ny * w))
            right.append((x - nx * w, y - ny * w))
    return left + right[::-1]


def capsule(points, widths):
    """A tapered stroke with round joins, as separate solid parts."""
    parts = []
    for i in range(len(points) - 1):
        (x0, y0), (x1, y1) = points[i], points[i + 1]
        length = math.hypot(x1 - x0, y1 - y0)
        nx, ny = -(y1 - y0) / length, (x1 - x0) / length
        w0, w1 = widths[i] / 2, widths[i + 1] / 2
        parts.append(
            [
                (x0 + nx * w0, y0 + ny * w0),
                (x1 + nx * w1, y1 + ny * w1),
                (x1 - nx * w1, y1 - ny * w1),
                (x0 - nx * w0, y0 - ny * w0),
            ]
        )
    for (x, y), width in zip(points, widths):
        parts.append(ellipse(x, y, width / 2, width / 2))
    return parts


def bezier(p0, p1, p2, p3, steps=24):
    out = []
    for i in range(steps + 1):
        t = i / steps
        u = 1 - t
        out.append(
            (
                u**3 * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t**3 * p3[0],
                u**3 * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t**3 * p3[1],
            )
        )
    return out


def curve(points, widths):
    """Tapered stroke along a cubic bezier. `widths` is spread over the spine."""
    path = bezier(*points)
    spread = []
    for i in range(len(path)):
        t = i / (len(path) - 1) * (len(widths) - 1)
        low = min(int(t), len(widths) - 2)
        spread.append(widths[low] + (widths[low + 1] - widths[low]) * (t - low))
    return spine(path, spread)


def catmull(points, samples=14, closed=True):
    """Smooth polyline through `points` (uniform Catmull-Rom)."""
    n = len(points)
    out = []
    span = range(n) if closed else range(n - 1)
    for i in span:
        p0 = points[(i - 1) % n] if closed else points[max(i - 1, 0)]
        p1 = points[i % n]
        p2 = points[(i + 1) % n]
        p3 = points[(i + 2) % n] if closed else points[min(i + 2, n - 1)]
        for j in range(samples):
            t = j / samples
            t2, t3 = t * t, t * t * t
            out.append(
                tuple(
                    0.5
                    * (
                        2 * p1[k]
                        + (-p0[k] + p2[k]) * t
                        + (2 * p0[k] - 5 * p1[k] + 4 * p2[k] - p3[k]) * t2
                        + (-p0[k] + 3 * p1[k] - 3 * p2[k] + p3[k]) * t3
                    )
                    for k in range(2)
                )
            )
    return out


# A kangaroo in profile, facing right: one closed outline, then the ears on top.
# Landmarks run clockwise from the nose across the skull, down the neck and back,
# out along the tail, round the foot, up the belly and the near arm, and back up
# the chest and throat.
OUTLINE = [
    # nose tip, muzzle, forehead and skull, right to left across the top
    (914, 280), (888, 240), (858, 200), (822, 176), (790, 168),
    # back of the head and the nape
    (752, 202), (724, 250),
    # back line, withers to loin to rump
    (700, 274), (640, 320), (520, 412), (420, 486), (330, 552), (272, 622),
    # tail, out along the top and back along the bottom
    (232, 676), (160, 772), (96, 858), (56, 918),
    (114, 940), (200, 900), (266, 842), (316, 776),
    # back of the hock, heel, sole and toes
    (332, 830), (332, 878), (342, 906), (360, 928),
    (470, 942), (560, 932), (600, 912),
    # top of the foot, ankle, shin, knee
    (580, 890), (470, 880), (410, 868), (384, 828), (372, 770), (392, 712),
    # thigh front, belly, and up the chest
    (440, 672), (520, 646), (596, 608), (654, 556),
    # chest, throat, jaw and muzzle back to the nose
    (676, 506), (694, 446), (708, 384), (742, 326), (786, 330), (838, 308),
    (872, 296),
]

# Ear blades: base, bow and tip, with the width at each of those three points.
# The near blade is taller; the far one leans further back, which keeps a gap
# between them at menu-bar size.
EARS = [
    ([(752, 198), (712, 142), (688, 96)], [78, 58, 26], 15),
    ([(794, 182), (776, 116), (768, 66)], [88, 70, 34], 18),
]

# The arms hang off the chest as separate parts, so the outline above stays
# simple. The far arm is set back and drawn first.
ARMS = [
    ([(640, 556), (648, 612), (628, 648)], [44, 32, 26], (626, 652), 16),
    ([(672, 566), (682, 628), (662, 668)], [52, 36, 30], (658, 672), 19),
]

# Negative space: nothing yet. The profile carries its own gaps.
CUTS = []


def shapes(compact=False):
    """Solid parts, and the cuts that go with them.

    `compact` is the menu-bar mark: no far arm, ears further apart, a little
    heavier, so the glyph still reads at 18 points.
    """
    solid = [catmull(OUTLINE)]
    for axis, widths, tip in EARS:
        if compact:
            axis = [(x - 10, y) for x, y in axis]
            widths = [w + 6 for w in widths]
        solid.extend(capsule(axis, widths))
        solid.append(ellipse(axis[-1][0], axis[-1][1], tip, tip))
    for axis, widths, paw, radius in ARMS:
        solid.extend(capsule(axis, widths))
        solid.append(ellipse(paw[0], paw[1], radius, radius))
    return solid, CUTS


def mask(size, compact=False):
    """Return an L mask of the silhouette at `size` pixels."""
    big = size * SS
    scale = big / UNIT
    image = Image.new("L", (big, big), 0)
    draw = ImageDraw.Draw(image)
    solid, cuts = shapes(compact)

    def scaled(points):
        return [(x * scale, y * scale) for x, y in points]

    for shape in solid:
        draw.polygon(scaled(shape), fill=255)
    for shape in cuts:
        draw.polygon(scaled(shape), fill=0)
    return image.resize((size, size), Image.LANCZOS)


def trim(mask_image, box, size):
    """Crop to `box` in unit space, then fit `size` with a small margin."""
    scale = mask_image.width / UNIT
    left, top, right, bottom = (int(v * scale) for v in box)
    cropped = mask_image.crop((left, top, right, bottom))
    span = max(cropped.size)
    canvas = Image.new("L", (span, span), 0)
    canvas.paste(cropped, ((span - cropped.width) // 2, (span - cropped.height) // 2))
    return canvas.resize((size, size), Image.LANCZOS)


MENUBAR_BOX = (40, 44, 960, 960)      # the glyph alone, tight
APP_BOX = (24, 24, 976, 976)          # the glyph, padded for the dock tile


def write_menubar():
    # The glyph sits in a 40 pixel tile with a 2 pixel margin all round. The
    # menu bar is 22 points tall and the item does not centre the image, so the
    # margin keeps the tail and the feet off the edge.
    size = 40
    inset = 2
    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    glyph = trim(mask(512, compact=True), MENUBAR_BOX, size - inset * 2)
    black = Image.new("RGBA", (size - inset * 2, size - inset * 2), (0, 0, 0, 255))
    image.paste(black, (inset, inset), glyph)
    path = os.path.join(HERE, "kangaroo-menubar.png")
    image.save(path)
    print("wrote", path, image.size)


def rounded_square(size, radius, top, bottom):
    """macOS app tile: vertical gradient clipped to a rounded square."""
    gradient = Image.new("RGB", (1, size))
    for y in range(size):
        t = y / (size - 1)
        gradient.putpixel(
            (0, y),
            tuple(int(top[i] + (bottom[i] - top[i]) * t) for i in range(3)),
        )
    tile = gradient.resize((size, size))
    mask_image = Image.new("L", (size * SS, size * SS), 0)
    draw = ImageDraw.Draw(mask_image)
    draw.rounded_rectangle(
        (0, 0, size * SS - 1, size * SS - 1), radius=radius * SS, fill=255
    )
    tile.putalpha(mask_image.resize((size, size), Image.LANCZOS))
    return tile


def write_app():
    # The dock draws the image into a fixed slot, so the tile size here sets the
    # size on screen. 856 of 1024 matches the other icons in this dock pixel for
    # pixel; a full-bleed tile reads as oversized beside them.
    canvas_size = 1024
    tile_size = 856
    tile = rounded_square(tile_size, 191, (247, 199, 108), (214, 126, 51))
    glyph_size = int(tile_size * 0.88)
    glyph = trim(mask(tile_size), APP_BOX, glyph_size)
    ink = Image.new("RGBA", (glyph_size, glyph_size), (42, 26, 15, 255))
    tile.paste(ink, ((tile_size - glyph_size) // 2, (tile_size - glyph_size) // 2), glyph)
    canvas = Image.new("RGBA", (canvas_size, canvas_size), (0, 0, 0, 0))
    canvas.alpha_composite(tile, ((canvas_size - tile_size) // 2,) * 2)
    path = os.path.join(HERE, "kangaroo-app.png")
    canvas.save(path)
    print("wrote", path, canvas.size)


if __name__ == "__main__":
    write_menubar()
    write_app()
