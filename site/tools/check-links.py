#!/usr/bin/env python3
"""Check every link and image reference in the site's pages.

    check-links.py [--external]

Local references must name a file under site/, and a #fragment must name an id
on the page it points at. With --external, every http(s) link is also
requested and must answer below 400. Exits 1 when anything is broken.
"""
import re
import sys
import urllib.error
import urllib.request
from html.parser import HTMLParser
from pathlib import Path

SITE = Path(__file__).resolve().parents[1]
PAGES = ("index.html", "credits.html")


class Page(HTMLParser):
    def __init__(self):
        super().__init__()
        self.ids, self.links = set(), []

    def handle_starttag(self, tag, attributes):
        for name, value in attributes:
            if value is None:
                continue
            if name == "id":
                self.ids.add(value)
            elif name in ("href", "src"):
                self.links.append(value)
            elif name == "srcset":
                self.links += [part.split()[0] for part in value.split(",") if part.strip()]


def parse(path):
    page = Page()
    page.feed(path.read_text())
    return page


def main():
    external = "--external" in sys.argv
    pages = {name: parse(SITE / name) for name in PAGES if (SITE / name).exists()}
    missing = [name for name in PAGES if name not in pages]
    broken = [f"{name}: the page is missing" for name in missing]
    seen = set()
    checked = 0
    for name, page in pages.items():
        links = page.links + re.findall(r"url\(['\"]?([^'\")]+)", (SITE / "styles.css").read_text())
        for link in links:
            if link.startswith(("data:", "mailto:")):
                continue
            checked += 1
            if link.startswith(("http://", "https://")):
                if external and link not in seen:
                    seen.add(link)
                    request = urllib.request.Request(link, method="GET", headers={"User-Agent": "link-check/1.0"})
                    try:
                        with urllib.request.urlopen(request, timeout=30) as answer:
                            if answer.status >= 400:
                                broken.append(f"{name}: {link} answered {answer.status}")
                    except (urllib.error.URLError, TimeoutError) as error:
                        broken.append(f"{name}: {link} failed: {error}")
                continue
            target, _, fragment = link.partition("#")
            target_name = target or name
            if not (SITE / target_name).exists():
                broken.append(f"{name}: {link} names no file")
            elif fragment and target_name in pages and fragment not in pages[target_name].ids:
                broken.append(f"{name}: {link} names no id")
    # The gallery builds its image names in main.js from each tab's data-shot.
    for shot in re.findall(r'data-shot="([^"]+)"', (SITE / "index.html").read_text()):
        for suffix in ("", "-960", "-1600"):
            checked += 1
            if not (SITE / f"assets/shots/{shot}{suffix}.webp").exists():
                broken.append(f"index.html: the {shot} tab needs assets/shots/{shot}{suffix}.webp")
    for line in broken:
        print(line)
    print(f"{checked} references checked, {len(broken)} broken" + ("" if external else " (external links not requested)"))
    sys.exit(1 if broken else 0)


if __name__ == "__main__":
    main()
