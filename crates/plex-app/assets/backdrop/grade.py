"""Regenerate the bundled background photographs, their credits and the app's
table of them, from the licensed originals.

Usage: python3 -I grade.py <originals-dir>

<originals-dir> holds <theme>/<rank>/original.jpg as staged from Wikimedia
Commons (the file each PHOTOS entry names by `source`, checked by SHA-256).
Writes, beside this script, one <id>.jpg per photo and SOURCES.md, and the
Rust table crates/plex-app/src/ui/backdrop/photos.rs.

Each photo is cropped to 16:9 around its `crop` focus, resized to WIDTH
(never enlarged), and darkened by `exposure` with no change of hue or
saturation: the app's glass is neutral, so the photograph carries no grade of
its own. The JPEG quality is the highest that keeps the file under MAX_BYTES.
The app derives the blurred browsing copy from this file when it loads it.
"""
import hashlib
import io
import json
import sys
from pathlib import Path

from PIL import Image, ImageEnhance, ImageFilter

Image.MAX_IMAGE_PIXELS = 200_000_000

WIDTH = 2560
MAX_BYTES = 480_000
QUALITIES = range(92, 63, -2)
# Fine foliage and water do not fit MAX_BYTES at any quality worth keeping, so
# those photos are softened by the smallest of these radii that lets them fit.
SOFTEN = (0.0, 0.5, 0.7, 0.9, 1.1)
HERE = Path(__file__).resolve().parent
RUST = HERE.parent.parent / "src" / "ui" / "backdrop" / "photos.rs"

CC_BY = {
    "2.0": "https://creativecommons.org/licenses/by/2.0/",
    "3.0": "https://creativecommons.org/licenses/by/3.0/",
    "4.0": "https://creativecommons.org/licenses/by/4.0/",
}
PD = ("Public domain", "")
CC0 = ("CC0 1.0", "https://creativecommons.org/publicdomain/zero/1.0/")


def by(version):
    return (f"CC BY {version}", CC_BY[version])


# theme: the ThemeId the photo belongs to, as the Rust path after `ThemeId::`.
# crop: where the 16:9 crop sits in the original (0..1 across the room left
# over in each axis). focus: which part the app keeps when a window is not
# 16:9. exposure: brightness factor, 1 is unchanged.
PHOTOS = [
    dict(id="aurora", theme="Aurora", source="aurora/1", 
         sha256="26ce9fd8344c370f5de6babe7f745af61d7e2e4cc814d4930720e8ba4bae0b35",
         title="Aurora", author="Russo Francesco", licence=by("3.0"),
         page="https://commons.wikimedia.org/wiki/File:Aurora_(140959963).jpeg",
         crop=(0.5, 0.3), focus=(0.5, 0.5), exposure=0.95),
    dict(id="fantasy-1", theme="Genre(Genre::Fantasy)", source="fantasy/1",
         sha256="a1cbcaaabbc3a3b1f3ad5048b47a1b5a3a0e3df3fe0ab2486f07a4253e9ca2c4",
         title="Morning fog in the Yellowstone River Valley",
         author="Yellowstone National Park", licence=PD,
         page="https://commons.wikimedia.org/wiki/File:Morning_fog_in_the_Yellowstone_River_Valley_(14563692904).jpg",
         crop=(0.5, 0.55), focus=(0.5, 0.5), exposure=0.88),
    dict(id="fantasy-2", theme="Genre(Genre::Fantasy)", source="fantasy/2",
         sha256="a25bafaee95c416a5979aea3f42fb4d47215bde690edf3ce6c3a90d99982a400",
         title="Eilean Donan castle", author="Eusebius", licence=by("3.0"),
         page="https://commons.wikimedia.org/wiki/File:Eilean_Donan_castle_-_140mm.jpg",
         crop=(0.5, 0.45), focus=(0.45, 0.5), exposure=0.92),
    dict(id="scifi-1", theme="Genre(Genre::SciFi)", source="scifi/1",
         sha256="cf12eb6b9479382f8d9c49b3ebb5ae4363c00dbc803dd1ca8410ab8f1f70ae23",
         title="Cosmic Cliffs in the Carina Nebula (JWST)",
         author="NASA, ESA, CSA, STScI", licence=PD,
         page="https://commons.wikimedia.org/wiki/File:NASA%E2%80%99s_Webb_Reveals_Cosmic_Cliffs,_Glittering_Landscape_of_Star_Birth.jpg",
         crop=(0.5, 0.5), focus=(0.5, 0.5), exposure=0.95),
    dict(id="scifi-2", theme="Genre(Genre::SciFi)", source="scifi/2",
         sha256="e5b0d3ecd93710380c58ac8652f08717b2e72f0fe3317d73e3d5d3ab7a0085a9",
         title="Tarantula Nebula (JWST)",
         author="NASA, ESA, CSA, STScI, Webb ERO Production Team", licence=PD,
         page="https://commons.wikimedia.org/wiki/File:Tarantula_Nebula_by_JWST.jpg",
         crop=(0.5, 0.5), focus=(0.5, 0.5), exposure=0.95),
    dict(id="epic-1", theme="Genre(Genre::Epic)", source="epic/1",
         sha256="a5815cf3f9c313781b016db232ecbaab7f6483c6a1fa55d638310ccb5f44c2e0",
         title="Dunes at sunset, Great Sand Dunes National Park",
         author="Debra Miller, National Park Service", licence=PD,
         page="https://commons.wikimedia.org/wiki/File:Dunes_at_sunset_along_the_southern_edge_of_the_dune_field_in_Great_Sand_Dunes_National_Park._NPS-Debra_Miller_(18659276586).jpg",
         crop=(0.5, 0.5), focus=(0.5, 0.5), exposure=0.92),
    dict(id="epic-2", theme="Genre(Genre::Epic)", source="epic/2",
         sha256="02e12d0a539f205d22c2eeef0d740ddaf48fdf96f8b6598f6bc0ea18fb91c483",
         title="Dune and Cleveland Peak at sunset",
         author="Great Sand Dunes National Park and Preserve", licence=PD,
         page="https://commons.wikimedia.org/wiki/File:Dune_and_Cleveland_Peak_at_Sunset_-_Flickr_-_Great_Sand_Dunes_National_Park_and_Preserve.jpg",
         crop=(0.5, 0.45), focus=(0.5, 0.5), exposure=0.92),
    dict(id="horror-1", theme="Genre(Genre::Horror)", source="horror/1",
         sha256="d0492af8d4a45be0fc8c8e1006a839ca94cac90e674d8122b11ae665862cdf8a",
         title="Moon over the dead forest, Kamchatka", author="kuhnmi",
         licence=by("2.0"),
         page="https://commons.wikimedia.org/wiki/File:Kamchatka_Moon_over_the_dead_forest_(19873266190).jpg",
         crop=(0.5, 0.5), focus=(0.5, 0.5), exposure=0.95),
    dict(id="horror-2", theme="Genre(Genre::Horror)", source="horror/2",
         sha256="8c7f66738cd59ae3020973b2a62a5cf75ad6772e048332a01623baaf72a07d29",
         title="Morning fog shrouds trees, Okanogan-Wenatchee National Forest",
         author="U.S. Forest Service, Pacific Northwest Region", licence=PD,
         page="https://commons.wikimedia.org/wiki/File:Okanogan-Wenatchee_National_Forest,_morning_fog_shrouds_trees_(37171636495).jpg",
         crop=(0.5, 0.5), focus=(0.5, 0.5), exposure=0.85),
    dict(id="western-1", theme="Genre(Genre::Western)", source="western/1",
         sha256="0f1572278cba7cb4ffa80ecb790f6d41f95435044a3146b7efe5552b95861c74",
         title="Monument Valley sunset", author="Romain Guy", licence=CC0,
         page="https://commons.wikimedia.org/wiki/File:Monument_Valley_Sunset_(44943115621).jpg",
         crop=(0.5, 0.5), focus=(0.5, 0.5), exposure=0.92),
    dict(id="western-2", theme="Genre(Genre::Western)", source="western/2",
         sha256="7ece2a20e92e0a2df990cc76d14e8ca274917d7199a23fd7e6efd6e0755c7fc8",
         title="Monument Valley", author="Marco Bellucci", licence=by("2.0"),
         page="https://commons.wikimedia.org/wiki/File:Monument_Valley_(5067709777).jpg",
         crop=(0.5, 0.55), focus=(0.5, 0.5), exposure=0.92),
    dict(id="adventure-1", theme="Genre(Genre::Adventure)", source="adventure/1",
         sha256="5270f6ca58c5fe30b3dee29b1ba07300dc487790a34c88339dccb0b76d87d26f",
         title="Zagedan Lakes, Caucasus Mountains", author="Vyacheslav Argenberg",
         licence=by("4.0"),
         page="https://commons.wikimedia.org/wiki/File:Zagedan_Lakes,_Mountain_cirque,_Caucasus_Mountains.jpg",
         crop=(0.5, 0.55), focus=(0.5, 0.5), exposure=0.9),
    dict(id="adventure-2", theme="Genre(Genre::Adventure)", source="adventure/2",
         sha256="8a81591b92bb1f00b89f96858f2e100ca6a167e7ab0963b1d403e94e96ed5e8c",
         title="Breiðamerkurjökull glacier, Iceland", author="Jakub Hałun",
         licence=by("4.0"),
         page="https://commons.wikimedia.org/wiki/File:Brei%C3%B0amerkurj%C3%B6kull_glacier,_Iceland,_20240719_1427_2618.jpg",
         crop=(0.5, 0.6), focus=(0.5, 0.5), exposure=0.85),
    dict(id="christmas-1", theme="Festival(Festival::Christmas)", source="christmas/1",
         sha256="fa0eb30c873c981e14facab4238c2323489393d54bcc227413bbf6a061aced6b",
         title="Glowing Christmas tree lights in the winter night", author="epSos.de",
         licence=by("2.0"),
         page="https://commons.wikimedia.org/wiki/File:Glowing_Christmas_Tree_Lights_in_the_Winter_Night.jpg",
         crop=(0.5, 0.5), focus=(0.5, 0.5), exposure=0.95),
    dict(id="christmas-2", theme="Festival(Festival::Christmas)", source="christmas/2",
         sha256="4d3a54f45e2c7db0a8420e46d41a844c0eed0989e28592f7d2901a9400505625",
         title="Marketplace after Christmas", author="J. Triepke", licence=by("2.0"),
         page="https://commons.wikimedia.org/wiki/File:Marketplace_after_christmas_(25746763234).jpg",
         crop=(0.5, 0.5), focus=(0.6, 0.5), exposure=0.92),
    dict(id="halloween-1", theme="Festival(Festival::Halloween)", source="halloween/1",
         sha256="82aa58cd8fa18a977ac8b521fabdb3b0e3d684c9622236bb925b7936012956cf",
         title="Autumn in the mountains at night with moonlight", author="Dreamy Pixel",
         licence=by("4.0"),
         page="https://commons.wikimedia.org/wiki/File:Autumn_in_the_mountains_at_night_with_moonlight.jpg",
         crop=(0.5, 0.5), focus=(0.5, 0.5), exposure=0.92),
    dict(id="halloween-2", theme="Festival(Festival::Halloween)", source="halloween/2",
         sha256="b55b044dc2562cdefb7d5de9c7ac5361ca3c8817d2af5aed84d90c61e2282240",
         title="Sunset east of Yaki Point with full moon rising",
         author="Grand Canyon National Park", licence=PD,
         page="https://commons.wikimedia.org/wiki/File:Sunset_East_of_Yaki_Point_with_Full_Moon_Rising_-_Thursday,_May_4,_2023_(52888420997).jpg",
         crop=(0.5, 0.5), focus=(0.5, 0.5), exposure=0.92),
    dict(id="winter-1", theme="Season(Season::Winter)", source="winter/1",
         sha256="139ba1cee099aa92f8e61a210a09103c789977a1d00d8b619cbd78ad5548c652",
         title="Electric Peak at sunrise", author="Jacob W. Frank", licence=PD,
         page="https://commons.wikimedia.org/wiki/File:Electric_Peak_at_sunrise_(77985b2c-ab66-4079-b503-fe42f5391b04).jpg",
         crop=(0.5, 0.4), focus=(0.5, 0.4), exposure=0.85),
    dict(id="winter-2", theme="Season(Season::Winter)", source="winter/2",
         sha256="132b5f793703216671f70c11e2dea92af4d14a4a0e8687f3e93ddf18f13c7e7f",
         title="Gore Range sunrise", author="Zach Dischner", licence=by("2.0"),
         page="https://commons.wikimedia.org/wiki/File:Gore_Range_Sunrise_(6822005770).jpg",
         crop=(0.5, 0.5), focus=(0.5, 0.5), exposure=0.92),
    dict(id="spring-1", theme="Season(Season::Spring)", source="spring/1",
         sha256="aba2ff57ba3f379b4a2b0e6357d7cc01163aece86abfa16be042b70a6c3d06eb",
         title="Cherry blossoms over Matsushima Bay, Miyagi", author="Makoto_Lab",
         licence=by("4.0"),
         page="https://commons.wikimedia.org/wiki/File:Miyagi-Landscape_of_cherry_blossoms_and_Matsushima_Bay-xl.jpg",
         crop=(0.5, 0.5), focus=(0.5, 0.5), exposure=0.88),
    dict(id="spring-2", theme="Season(Season::Spring)", source="spring/2",
         sha256="6dd34cd94b805c9945adacc3a4bf85f1a78923d24c6c22e41134c5ba53469ffd",
         title="Pretty blossoms all in a row", author="Savannah River Site",
         licence=PD,
         page="https://commons.wikimedia.org/wiki/File:Pretty_Blossoms_All_in_a_Row_(50326184663).jpg",
         crop=(0.5, 0.5), focus=(0.5, 0.5), exposure=0.88),
    dict(id="summer-1", theme="Season(Season::Summer)", source="summer/1",
         sha256="413a98841ca331af49f5afbb6dac59df6dab93e2b1b470d80edfdea5873fc475",
         title="Summer meadow, Coconino National Forest", author="U.S. Forest Service",
         licence=PD,
         page="https://commons.wikimedia.org/wiki/File:Landscape_(20170823-FS-Coconino-DLS-012).jpg",
         crop=(0.5, 0.45), focus=(0.5, 0.5), exposure=0.9),
    dict(id="summer-2", theme="Season(Season::Summer)", source="summer/2",
         sha256="b0410dc1f926a9a3fa9e7a4de5f3566a0426b5298bc7b87f6313f2625d7e8816",
         title="Summer wildflowers at Yaquina Head", author="BLM Oregon & Washington",
         licence=PD,
         page="https://commons.wikimedia.org/wiki/File:Summer_wildflowers_at_Yaquina_Head_(48302262812).jpg",
         crop=(0.5, 0.5), focus=(0.5, 0.5), exposure=0.9),
    dict(id="autumn-1", theme="Season(Season::Autumn)", source="autumn/1",
         sha256="a6cf3150dfc679ecb2ee4cff619cca1c21d34bb38fd5362a4c1c0d6d32c58b4e",
         title="Chivruay Valley in autumn colours, Murmansk Oblast",
         author="Artem Abdukakharov", licence=by("4.0"),
         page="https://commons.wikimedia.org/wiki/File:Chivruay_Valley_in_fall_colors_near_Saydozero_Lake_Murmansk_Oblast_Russia.jpg",
         crop=(0.5, 0.5), focus=(0.5, 0.5), exposure=0.92),
    dict(id="autumn-2", theme="Season(Season::Autumn)", source="autumn/2",
         sha256="b7c61e680e14c686416197ab5dd01d927c131f2bf175d5d118d8414a57aee8b0",
         title="Boggy tundra in autumn colours near Lovozero",
         author="Artem Abdukakharov", licence=by("4.0"),
         page="https://commons.wikimedia.org/wiki/File:Boggy_tundra_in_fall_colors_near_Lovozero_lake_Murmansk_Russia.jpg",
         crop=(0.5, 0.5), focus=(0.5, 0.5), exposure=0.92),
]


def crop_16_9(im, fx, fy):
    w, h = im.size
    if w * 9 > h * 16:
        cw, ch = h * 16 // 9, h
    else:
        cw, ch = w, w * 9 // 16
    x = round((w - cw) * fx)
    y = round((h - ch) * fy)
    return im.crop((x, y, x + cw, y + ch))


def encode(im):
    for radius in SOFTEN:
        source = im.filter(ImageFilter.GaussianBlur(radius)) if radius else im
        for quality in QUALITIES:
            out = io.BytesIO()
            source.save(out, "JPEG", quality=quality, optimize=True,
                        progressive=True, subsampling="4:2:0")
            if out.tell() <= MAX_BYTES:
                return out.getvalue(), quality, radius
    sys.exit("no quality and softening keeps the file under MAX_BYTES")


def grade(src, photo):
    data = src.read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    if not digest.startswith(photo["sha256"]):
        sys.exit(f"{src}: sha256 {digest} is not the staged original")
    im = Image.open(io.BytesIO(data))
    im.draft("RGB", (WIDTH * 2, WIDTH * 2))
    im = crop_16_9(im.convert("RGB"), *photo["crop"])
    width = min(WIDTH, im.width)
    im = im.resize((width, width * 9 // 16), Image.LANCZOS)
    im = ImageEnhance.Brightness(im).enhance(photo["exposure"])
    return encode(im) + (im.size,)


def licence_line(photo):
    name, url = photo["licence"]
    return f"{name} ({url})" if url else name


def rust_str(text):
    return json.dumps(text, ensure_ascii=False)


def write_rust(rows):
    out = [
        "//! The bundled background photographs. Generated by",
        "//! `assets/backdrop/grade.py` from the same list that grades the files and",
        "//! writes `assets/backdrop/SOURCES.md`; edit the list there, not this file.",
        "",
        "use crate::theme_art::{Festival, Genre, Season, ThemeId};",
        "",
        "use super::Photo;",
        "",
        "/// Every bundled photograph, in the theme picker's order.",
        f"pub const PHOTOS: [Photo; {len(rows)}] = [",
    ]
    for photo, size in rows:
        name, url = photo["licence"]
        out += [
            "    Photo {",
            f"        id: {rust_str(photo['id'])},",
            f"        theme: ThemeId::{photo['theme']},",
            f"        bytes: include_bytes!(\"../../../assets/backdrop/{photo['id']}.jpg\"),",
            f"        size: [{size[0]}, {size[1]}],",
            f"        focus: [{photo['focus'][0]:.2f}, {photo['focus'][1]:.2f}],",
            f"        title: {rust_str(photo['title'])},",
            f"        author: {rust_str(photo['author'])},",
            f"        licence: {rust_str(name)},",
            f"        licence_url: {rust_str(url)},",
            f"        page: {rust_str(photo['page'])},",
            "    },",
        ]
    out += ["];", ""]
    families = [f for f in ("Festival", "Genre", "Season")
                if any(p["theme"].startswith(f + "(") for p, _ in rows)]
    out[4] = "use crate::theme_art::{" + ", ".join(families + ["ThemeId"]) + "};"
    RUST.write_text("\n".join(out))


def write_sources(rows):
    lines = [
        "# Bundled background photographs",
        "",
        "Every file here is a freely licensed photograph from Wikimedia Commons,",
        "graded by `grade.py` from the original named below. Changes made to each:",
        f"cropped to 16:9, resized to at most {WIDTH} px wide, darkened by the",
        "exposure factor shown (no change of hue or saturation), softened by the",
        "Gaussian radius shown where fine detail would not otherwise fit, and saved",
        f"as a progressive JPEG of at most {MAX_BYTES // 1000} KB. The app blurs a",
        "downscaled copy for the browsing screens when it loads one. Every credit is",
        "shown in Settings > About.",
        "",
        "| File | Theme | Title | Author | Licence | Source | Exposure | Softened (px) | JPEG quality |",
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    for photo, (size, quality, radius) in rows:
        lines.append(
            f"| {photo['id']}.jpg ({size[0]}x{size[1]}) | {photo['theme'].split('::')[-1].rstrip(')')} "
            f"| {photo['title']} | {photo['author']} | {licence_line(photo)} "
            f"| {photo['page']} | {photo['exposure']} | {radius or "no"} | {quality} |"
        )
    (HERE / "SOURCES.md").write_text("\n".join(lines) + "\n")


def main():
    originals = Path(sys.argv[1])
    rust_rows, source_rows = [], []
    for photo in PHOTOS:
        jpeg, quality, radius, size = grade(originals / photo["source"] / "original.jpg", photo)
        (HERE / f"{photo['id']}.jpg").write_bytes(jpeg)
        print(f"{photo['id']:12} {size[0]}x{size[1]} q{quality} soft {radius} {len(jpeg) // 1000} KB")
        rust_rows.append((photo, size))
        source_rows.append((photo, (size, quality, radius)))
    write_rust(rust_rows)
    write_sources(source_rows)


main()
