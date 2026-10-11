"""Builds a game crate and copies it to an SD card as <sd-root>/ogo/games/<id>/, or packs it into
an .ogo file to upload on the device's web page.

Writes game.bin (the firmware image), game.toml (with its sha256) and, if the crate has one,
icon.rgb565. Run it from the project folder after `. .\\env.ps1`, so cargo finds the toolchain.

Usage:  python tools/install_game.py games/snake E:\\
        python tools/install_game.py games/snake --ogo snake.ogo

The id and version come from [package] in the crate's Cargo.toml, the display name and
sdk_version from [package.metadata.ogo]. `cargo xtask pack` replaces this script in M9.
"""

import argparse
import hashlib
import io
import re
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent
# First byte of every ESP32 app image.
IMAGE_MAGIC = 0xE9


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("crate", type=Path, help="game crate folder, e.g. games/snake")
    parser.add_argument("sd_root", type=Path, nargs="?", help="root of the SD card, e.g. E:\\")
    parser.add_argument("--ogo", type=Path, help="write an .ogo package to this file instead")
    args = parser.parse_args()

    if (args.sd_root is None) == (args.ogo is None):
        parser.error("give either an SD card root or --ogo")
    if args.sd_root is not None and not args.sd_root.is_dir():
        print(f"{args.sd_root} is not a folder", file=sys.stderr)
        return 1
    manifest = tomllib.loads((args.crate / "Cargo.toml").read_text(encoding="utf-8"))
    package = manifest["package"]
    ogo = package.get("metadata", {}).get("ogo", {})
    game_id = package["name"]
    if not re.fullmatch(r"[a-z0-9-]+", game_id):
        print(f'package name "{game_id}" is not a valid game id (use a-z, 0-9 and -)', file=sys.stderr)
        return 1

    with tempfile.TemporaryDirectory() as tmp:
        image = Path(tmp) / "game.bin"
        print(f"Building {game_id}...")
        subprocess.run(
            ["cargo", "espflash", "save-image", "--chip", "esp32", "--flash-size", "16mb",
             "--release", "--package", game_id, str(image)],
            cwd=PROJECT_ROOT,
            check=True,
        )
        data = image.read_bytes()
    if not data or data[0] != IMAGE_MAGIC:
        print("cargo espflash didn't produce an ESP32 app image", file=sys.stderr)
        return 1

    authors = package.get("authors", [])
    toml = (
        f'id          = "{game_id}"\n'
        f'name        = "{ogo.get("name", game_id)}"\n'
        f'version     = "{package["version"]}"\n'
        f'author      = "{authors[0] if authors else ""}"\n'
        f'sdk_version = {ogo.get("sdk_version", 1)}\n'
        f'sha256      = "{hashlib.sha256(data).hexdigest()}"\n'
    )
    files = {"game.toml": toml.encode(), "game.bin": data}
    icon = args.crate / "icon.rgb565"
    if icon.is_file():
        files["icon.rgb565"] = icon.read_bytes()

    if args.ogo is not None:
        # Plain ustar with flat file names: what the launcher's package reader expects.
        with tarfile.open(args.ogo, "w", format=tarfile.USTAR_FORMAT) as tar:
            for name, content in files.items():
                info = tarfile.TarInfo(name)
                info.size = len(content)
                info.mtime = int(time.time())
                tar.addfile(info, io.BytesIO(content))
        print(f"Packed {game_id} ({len(data) // 1024} KB) into {args.ogo}")
        return 0

    target = args.sd_root / "ogo" / "games" / game_id
    target.mkdir(parents=True, exist_ok=True)
    for name, content in files.items():
        (target / name).write_bytes(content)
    print(f"Installed {game_id} ({len(data) // 1024} KB) to {target}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
