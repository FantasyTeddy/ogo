"""Draws the ogo boot image: the mascot (a crab whose shell is a handheld console) above the
"ogo" wordmark, anti-aliased, on black.

Writes ogo-launcher/assets/logo.rgb565 (RGB565 little-endian, row by row, for the boot screen)
and docs/logo.png (a 2x preview for the README). Only needs the Python standard library.

Usage:  python tools/make_logo.py
"""

import math
import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WIDTH, HEIGHT = 200, 184
SUPERSAMPLE = 4

# Palette
ORANGE_TOP = (255, 138, 61)
ORANGE_BOTTOM = (222, 72, 24)
LEG = (186, 66, 20)
SCREEN = (28, 38, 48)
GLOW = (80, 220, 200)
DARK = (40, 34, 34)
BUTTON = (246, 233, 214)
WHITE = (255, 255, 255)
PUPIL = (20, 20, 24)


# --- Shapes: each is a function (x, y) -> bool, painted in order (later on top). ---

def circle(cx, cy, r):
    return lambda x, y: (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def ring(cx, cy, outer, stroke):
    return lambda x, y: outer - stroke <= math.hypot(x - cx, y - cy) <= outer


def capsule(ax, ay, bx, by, r):
    """A line from (ax, ay) to (bx, by) with round ends."""
    dx, dy = bx - ax, by - ay
    length2 = dx * dx + dy * dy

    def inside(x, y):
        t = max(0.0, min(1.0, ((x - ax) * dx + (y - ay) * dy) / length2))
        return (x - ax - t * dx) ** 2 + (y - ay - t * dy) ** 2 <= r * r

    return inside


def rounded_box(cx, cy, w, h, r):
    def inside(x, y):
        qx = max(abs(x - cx) - (w / 2 - r), 0.0)
        qy = max(abs(y - cy) - (h / 2 - r), 0.0)
        return abs(x - cx) <= w / 2 and abs(y - cy) <= h / 2 and qx * qx + qy * qy <= r * r

    return inside


def claw(cx, cy, r, opening_deg, facing_deg):
    """A circle with a wedge cut out, like a pincer, opening towards `facing_deg` (0 = right, 90 = down)."""
    body = circle(cx, cy, r)
    half = math.radians(opening_deg) / 2
    facing = math.radians(facing_deg)

    def inside(x, y):
        if not body(x, y):
            return False
        angle = math.atan2(y - cy, x - cx) - facing
        angle = (angle + math.pi) % (2 * math.pi) - math.pi
        return abs(angle) > half

    return inside


def union(*shapes):
    return lambda x, y: any(shape(x, y) for shape in shapes)


def minus(shape, cut):
    return lambda x, y: shape(x, y) and not cut(x, y)


def gradient(top, bottom, y0, y1):
    def color(x, y):
        t = max(0.0, min(1.0, (y - y0) / (y1 - y0)))
        return tuple(a + (b - a) * t for a, b in zip(top, bottom))

    return color


def solid(rgb):
    return lambda x, y: rgb


# --- Scene ---

def mascot(ox, oy):
    """Layers for the crab-console, with its shell centered at (ox, oy)."""
    orange = gradient(ORANGE_TOP, ORANGE_BOTTOM, oy - 40, oy + 40)
    layers = []

    # Legs: three on each side, behind the shell.
    for i in range(3):
        y = oy + 6 + i * 11
        for side in (-1, 1):
            layers.append((union(capsule(ox + side * 26, y, ox + side * 46, y + 10, 3.2),
                                 capsule(ox + side * 46, y + 10, ox + side * 52, y + 22, 3.0)), solid(LEG)))

    # Arms and claws, raised like waving.
    for side in (-1, 1):
        layers.append((capsule(ox + side * 30, oy - 4, ox + side * 52, oy - 22, 5.0), orange))
        layers.append((claw(ox + side * 58, oy - 32, 15, 70, -90 - side * 25), orange))

    # Eye stalks and eyes, like antennae.
    for side in (-1, 1):
        layers.append((capsule(ox + side * 12, oy - 38, ox + side * 18, oy - 54, 3.0), orange))
        layers.append((circle(ox + side * 18, oy - 58, 8), solid(WHITE)))
        layers.append((circle(ox + side * 18 + 2, oy - 57, 3.6), solid(PUPIL)))

    # The shell is the console: body, screen with a smile, D-pad and A/B buttons.
    layers.append((rounded_box(ox, oy, 64, 82, 12), orange))
    layers.append((rounded_box(ox, oy - 18, 48, 32, 5), solid(SCREEN)))
    smile = minus(ring(ox, oy - 26, 12, 3.2), lambda x, y: y < oy - 22)
    layers.append((smile, solid(GLOW)))
    layers.append((union(circle(ox - 8, oy - 27, 2.4), circle(ox + 8, oy - 27, 2.4)), solid(GLOW)))
    dpad = union(rounded_box(ox - 15, oy + 18, 16, 5.5, 1.5), rounded_box(ox - 15, oy + 18, 5.5, 16, 1.5))
    layers.append((dpad, solid(DARK)))
    layers.append((union(circle(ox + 12, oy + 21, 4), circle(ox + 21, oy + 15, 4)), solid(BUTTON)))
    return layers


def wordmark(ox, oy, scale):
    """"ogo" in geometric lowercase; (ox, oy) is the center of the letter rings' row."""
    radius, stroke = 26 * scale, 11 * scale
    letters = []
    for dx in (-64, 64):
        letters.append(ring(ox + dx * scale, oy, radius, stroke))
    letters.append(ring(ox, oy, radius, stroke))
    stem_right = ox + radius
    hook_y = oy + 32 * scale
    hook_r = 22 * scale
    letters.append(lambda x, y: stem_right - stroke <= x <= stem_right and oy <= y <= hook_y)
    letters.append(lambda x, y: y >= hook_y and ring(stem_right - hook_r, hook_y, hook_r, stroke)(x, y))
    color = gradient(ORANGE_TOP, ORANGE_BOTTOM, oy - radius, oy + 54 * scale)
    return [(union(*letters), color)]


LAYERS = mascot(100, 74) + wordmark(100, 152, 0.42)


def render():
    pixels = []
    step = 1 / SUPERSAMPLE
    for py in range(HEIGHT):
        row = []
        for px in range(WIDTH):
            total = [0.0, 0.0, 0.0]
            for sy in range(SUPERSAMPLE):
                for sx in range(SUPERSAMPLE):
                    x, y = px + (sx + 0.5) * step, py + (sy + 0.5) * step
                    for shape, color in reversed(LAYERS):
                        if shape(x, y):
                            total = [t + c for t, c in zip(total, color(x, y))]
                            break
            row.append(tuple(round(t / SUPERSAMPLE ** 2) for t in total))
        pixels.append(row)
    return pixels


def write_rgb565(pixels, path: Path) -> None:
    data = bytearray()
    for row in pixels:
        for r, g, b in row:
            data += (((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)).to_bytes(2, "little")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def write_png(pixels, path: Path, scale: int) -> None:
    height, width = len(pixels) * scale, len(pixels[0]) * scale
    raw = bytearray()
    for row in pixels:
        line = b"\x00" + b"".join(bytes(p) * scale for p in row)
        raw += line * scale

    def chunk(kind: bytes, body: bytes) -> bytes:
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(png)


def main() -> None:
    pixels = render()
    write_rgb565(pixels, ROOT / "ogo-launcher" / "assets" / "logo.rgb565")
    write_png(pixels, ROOT / "docs" / "logo.png", scale=2)
    print(f"Wrote {WIDTH}x{HEIGHT} logo")


if __name__ == "__main__":
    main()
