#!/usr/bin/env python3
"""Make the site's page background from the app's bundled Aurora photograph.

Reads crates/plex-app/assets/backdrop/SOURCES.md and the JPEG it names for
Aurora, and writes site/assets/themes/aurora.webp: 1600x900, softened, the
picture behind the glass on every section.

Run it again whenever the app's bundled photographs change. Needs Pillow.
"""
import re
from pathlib import Path

from PIL import Image, ImageFilter

REPO = Path(__file__).resolve().parents[2]
SOURCE = REPO / "crates/plex-app/assets/backdrop"
OUT = REPO / "site/assets/themes"
def rows():
    for line in (SOURCE / "SOURCES.md").read_text().splitlines():
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if len(cells) < 6 or not re.match(r"[\w-]+\.jpg", cells[0]):
            continue
        licence = re.match(r"(.*?)\s*(?:\((https?://[^)]+)\))?$", cells[4])
        yield {"file": cells[0].split()[0], "theme": cells[1], "title": cells[2], "author": cells[3],
               "licence": licence.group(1), "licence_url": licence.group(2) or "", "source": cells[5]}


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    row = next(row for row in rows() if row["theme"] == "Aurora")
    with Image.open(SOURCE / row["file"]) as photo:
        photo.convert("RGB").resize((1600, 900), Image.LANCZOS).filter(ImageFilter.GaussianBlur(3)).save(
            OUT / "aurora.webp", "WEBP", quality=62, method=6)
    print(f"aurora.webp -> {OUT}")


if __name__ == "__main__":
    main()
