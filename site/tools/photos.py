#!/usr/bin/env python3
"""Make the site's theme photographs from the ones the app bundles.

Reads crates/plex-app/assets/backdrop/SOURCES.md and the JPEGs beside it, and
writes into site/assets/themes/:

    <file>.webp        1600x900, softened: the page background behind the glass
    <file>-thumb.webp  480x270, sharp: the picture on a theme's button
    themes.json        each theme's name, photographs and their credits

Run it again whenever the app's bundled photographs change. Needs Pillow.
"""
import json
import re
from pathlib import Path

from PIL import Image, ImageFilter

REPO = Path(__file__).resolve().parents[2]
SOURCE = REPO / "crates/plex-app/assets/backdrop"
OUT = REPO / "site/assets/themes"
# The order and grouping of the app's theme list (theme_art/theme.rs).
GROUPS = {
    "Aurora": ("aurora", "Aurora", "default"),
    "Fantasy": ("fantasy", "Fantasy", "genre"), "SciFi": ("scifi", "Science fiction", "genre"),
    "Epic": ("epic", "Epic", "genre"), "Horror": ("horror", "Horror", "genre"),
    "Western": ("western", "Western", "genre"), "Adventure": ("adventure", "Adventure", "genre"),
    "Christmas": ("christmas", "Christmas", "festival"), "Halloween": ("halloween", "Halloween", "festival"),
    "Winter": ("winter", "Winter", "season"), "Spring": ("spring", "Spring", "season"),
    "Summer": ("summer", "Summer", "season"), "Autumn": ("autumn", "Autumn", "season"),
}


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
    themes = {}
    for row in rows():
        key, name, group = GROUPS[row["theme"]]
        stem = Path(row["file"]).stem
        with Image.open(SOURCE / row["file"]) as photo:
            photo = photo.convert("RGB")
            photo.resize((1600, 900), Image.LANCZOS).filter(ImageFilter.GaussianBlur(3)).save(
                OUT / f"{stem}.webp", "WEBP", quality=62, method=6)
            photo.resize((480, 270), Image.LANCZOS).save(OUT / f"{stem}-thumb.webp", "WEBP", quality=74, method=6)
        theme = themes.setdefault(key, {"key": key, "name": name, "group": group, "photos": []})
        theme["photos"].append({"image": stem, "title": row["title"], "author": row["author"],
                                "licence": row["licence"], "licence_url": row["licence_url"], "source": row["source"]})
    ordered = [themes[key] for key, _, _ in GROUPS.values() if key in themes]
    (OUT / "themes.json").write_text(json.dumps(ordered, indent=1, ensure_ascii=False) + "\n")
    print(f"{len(ordered)} themes, {sum(len(t['photos']) for t in ordered)} photographs -> {OUT}")


if __name__ == "__main__":
    main()
