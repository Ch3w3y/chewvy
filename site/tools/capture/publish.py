#!/usr/bin/env python3
"""Turn the capture run's PPM files into the images the site and the docs use.

    publish.py SHOTS_DIR [--site-only]

For each SHOTS_DIR/<name>.ppm it writes site/assets/shots/<name>.webp at the
captured size, <name>-1600.webp and <name>-960.webp, and docs/media/<name>.jpg
at 1600 px wide. A theme-<key> capture becomes one 960 px image for the site. home also gets a JPEG for link previews, and five screens
become docs/media/chewvy-tour.webp, the animation at the top of the README.
Needs Pillow.
"""
import sys
from pathlib import Path

from PIL import Image

REPO = Path(__file__).resolve().parents[3]
SITE = REPO / "site/assets/shots"
DOCS = REPO / "docs/media"


def main(shots, docs=True):
    SITE.mkdir(parents=True, exist_ok=True)
    DOCS.mkdir(parents=True, exist_ok=True)
    sources = sorted(Path(shots).glob("*.ppm"))
    if not sources:
        sys.exit(f"no .ppm captures in {shots}")
    for source in sources:
        name = source.stem
        with Image.open(source) as full:
            full = full.convert("RGB")
            if name.startswith("theme-"):
                # One of twelve near-identical grids: the site shows them small.
                full.resize((960, round(full.height * 960 / full.width)), Image.LANCZOS).save(
                    SITE / f"{name}-960.webp", "WEBP", quality=78, method=6)
                print(f"{name}: 960 px")
                continue
            full.save(SITE / f"{name}.webp", "WEBP", quality=82, method=6)
            for width in (1600, 960):
                small = full.resize((width, round(full.height * width / full.width)), Image.LANCZOS)
                small.save(SITE / f"{name}-{width}.webp", "WEBP", quality=80, method=6)
                if width == 1600 and docs:
                    small.save(DOCS / f"{name}.jpg", "JPEG", quality=86, optimize=True, progressive=True)
                if width == 1600 and name == "home":
                    small.save(SITE / "home-1600.jpg", "JPEG", quality=84, optimize=True, progressive=True)
        print(f"{name}: {full.width}x{full.height}")
    if docs:
        tour(Path(shots))


def tour(shots):
    """The animated tour at the top of the README: five screens, 2.6 s each."""
    frames = []
    for name in ("home", "detail", "library", "search", "player"):
        source = shots / f"{name}.ppm"
        if not source.exists():
            return
        with Image.open(source) as full:
            frames.append(full.convert("RGB").resize((1280, round(full.height * 1280 / full.width)), Image.LANCZOS))
    frames[0].save(DOCS / "chewvy-tour.webp", "WEBP", save_all=True, append_images=frames[1:],
                   duration=2600, loop=0, quality=78, method=6)
    print("chewvy-tour.webp")


if __name__ == "__main__":
    arguments = [argument for argument in sys.argv[1:] if argument != "--site-only"]
    if len(arguments) != 1:
        sys.exit(__doc__)
    main(arguments[0], docs="--site-only" not in sys.argv)
