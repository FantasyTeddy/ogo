"""Writes sample games into <sd-root>/ogo/games/ to test the launcher's game list.

The games aren't runnable: game.bin is a placeholder. Two entries are deliberately broken
to show how the launcher reports problems.

Usage:  python tools/make_sample_games.py E:\\
        python tools/make_sample_games.py E:\\ --force   (overwrite existing sample folders)
"""

import argparse
import hashlib
import math
import sys
from pathlib import Path

ICON_SIZE = 64

# (id, name, author, version, icon pattern or None, base color)
GAMES = [
    ("snake", "Snake", "Matthias", "1.2.0", "rings", (40, 200, 80)),
    ("breakout", "Breakout", "Matthias", "0.9.1", "bricks", (230, 90, 40)),
    ("blocks", "Falling Blocks", "ogo examples", "2.0.0", "checker", (80, 120, 240)),
    ("pong", "Pong", "ogo examples", "1.0.0", "stripes", (240, 240, 240)),
    ("rocks", "Space Rocks", "Someone with a rather long name", "0.3.0", "dots", (160, 160, 180)),
    ("mines", "Mines", "", "1.1.0", "grid", (200, 200, 60)),
    ("2048", "2048", "ogo examples", "1.0.0", "diamond", (240, 180, 60)),
    ("sokoban", "Sokoban", "ogo examples", "1.4.2", "bricks", (160, 110, 60)),
    ("flappy", "Flappy Bit", "ogo examples", "0.1.0", "gradient", (90, 200, 240)),
    ("lander", "Lunar Lander", "ogo examples", "1.0.0", "rings", (200, 200, 200)),
    ("solitaire", "Solitaire", "ogo examples", "3.1.0", "diamond", (40, 140, 60)),
    ("long-name", "The Incredibly Long Named Adventure of Rusty the Crab", "Ferris", "0.0.1", "gradient", (230, 70, 30)),
    ("no-icon", "No Icon Game", "ogo examples", "1.0.0", None, (0, 0, 0)),
]


def rgb565_le(r: int, g: int, b: int) -> bytes:
    value = ((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)
    return value.to_bytes(2, "little")


def icon(pattern: str, base: tuple) -> bytes:
    dark = tuple(c // 4 for c in base)
    out = bytearray()
    for y in range(ICON_SIZE):
        for x in range(ICON_SIZE):
            cx, cy = x - 31.5, y - 31.5
            if pattern == "rings":
                on = int(math.hypot(cx, cy)) // 6 % 2 == 0
            elif pattern == "bricks":
                on = (x + (8 if y // 8 % 2 else 0)) % 16 != 0 and y % 8 != 0
            elif pattern == "checker":
                on = (x // 8 + y // 8) % 2 == 0
            elif pattern == "stripes":
                on = (x + y) // 6 % 2 == 0
            elif pattern == "dots":
                on = math.hypot(x % 12 - 5.5, y % 12 - 5.5) < 4
            elif pattern == "grid":
                on = x % 10 not in (0, 1) and y % 10 not in (0, 1)
            elif pattern == "diamond":
                on = abs(cx) + abs(cy) < 28
            else:  # gradient
                t = y / (ICON_SIZE - 1)
                out += rgb565_le(*(int(d + (c - d) * t) for c, d in zip(base, dark)))
                continue
            out += rgb565_le(*(base if on else dark))
    return bytes(out)


def write_game(folder: Path, game_id: str, name: str, author: str, version: str,
               sdk_version: int = 1, pattern=None, base=(0, 0, 0)) -> None:
    folder.mkdir(parents=True, exist_ok=True)
    game_bin = f"placeholder for {game_id}, not a real firmware image\n".encode() * 64
    (folder / "game.bin").write_bytes(game_bin)
    sha256 = hashlib.sha256(game_bin).hexdigest()
    (folder / "game.toml").write_text(
        f'id          = "{game_id}"\n'
        f'name        = "{name}"\n'
        f'version     = "{version}"\n'
        f'author      = "{author}"\n'
        f"sdk_version = {sdk_version}\n"
        f'sha256      = "{sha256}"\n',
        encoding="utf-8",
    )
    if pattern:
        (folder / "icon.rgb565").write_bytes(icon(pattern, base))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("sd_root", type=Path, help="root of the SD card, e.g. E:\\")
    parser.add_argument("--force", action="store_true", help="overwrite existing sample game folders")
    args = parser.parse_args()

    if not args.sd_root.is_dir():
        print(f"{args.sd_root} is not a folder", file=sys.stderr)
        return 1
    games_dir = args.sd_root / "ogo" / "games"

    samples = [(g[0], lambda f, g=g: write_game(f, *g[:4], pattern=g[4], base=g[5])) for g in GAMES]
    # Broken on purpose: needs a newer SDK, and a game.toml that doesn't parse.
    samples.append(("future-game", lambda f: write_game(f, "future-game", "From The Future", "ogo examples", "9.0.0",
                                                         sdk_version=2, pattern="rings", base=(180, 60, 220))))
    samples.append(("bad-toml", lambda f: (f.mkdir(parents=True, exist_ok=True),
                                           (f / "game.toml").write_text('id = "bad-toml"\nname = "Missing quote\n'))))

    written = skipped = 0
    for game_id, write in samples:
        folder = games_dir / game_id
        if folder.exists() and not args.force:
            print(f"skip   {folder} (exists, use --force to overwrite)")
            skipped += 1
            continue
        write(folder)
        print(f"write  {folder}")
        written += 1
    print(f"{written} written, {skipped} skipped")
    return 0


if __name__ == "__main__":
    sys.exit(main())
