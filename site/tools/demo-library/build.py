#!/usr/bin/env python3
"""Build the demo media library that the site's screenshots are taken from.

    build.py fetch     download every film in library.json and check its hash
    build.py art       posters and backdrops: Commons files, else frames of the film
    build.py nfo       write the .nfo files Jellyfin reads its metadata from
    build.py credits   regenerate site/CREDITS.md from what was downloaded
    build.py all       the four steps in that order

The library lives under $CHEWVY_DEMO_DIR (default ~/.cache/chewvy-demo-jf).
Every step skips work that is already done, so a stopped run can be started
again. Needs python3 with Pillow, curl, unzip and ffmpeg.
"""
import fcntl
import hashlib
import io
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from xml.sax.saxutils import escape

from PIL import Image, ImageDraw, ImageFont, ImageStat

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
ROOT = Path(os.environ.get("CHEWVY_DEMO_DIR", Path.home() / ".cache/chewvy-demo-jf"))
MEDIA = ROOT / "media"
PROVENANCE = ROOT / "provenance"
USER_AGENT = "demo-library-builder/1.0"
FONT = REPO / "crates/plex-app/assets/fonts/PlusJakartaSans-ExtraBold.ttf"
COMMONS_API = "https://commons.wikimedia.org/w/api.php"


def library():
    return json.loads((HERE / "library.json").read_text())


def provenance_file(key):
    return PROVENANCE / (re.sub(r"[^A-Za-z0-9._-]", "_", key) + ".json")


def load_provenance():
    """Every record, by key. One file per key, so that two steps can run at once."""
    return {entry["key"]: entry for entry in
            (json.loads(path.read_text()) for path in sorted(PROVENANCE.glob("*.json")))}


def record(key, entry):
    PROVENANCE.mkdir(parents=True, exist_ok=True)
    tmp = provenance_file(key).with_suffix(".tmp")
    tmp.write_text(json.dumps({"key": key, **entry}, indent=1, ensure_ascii=False, sort_keys=True))
    tmp.replace(provenance_file(key))


def strip_html(text):
    return re.sub(r"\s+", " ", re.sub(r"<[^>]+>", "", str(text or ""))).strip()


def commons_info(title):
    query = urllib.parse.urlencode({
        "action": "query", "titles": title, "prop": "imageinfo", "format": "json",
        "formatversion": "2", "iiprop": "url|size|mime|sha1|extmetadata"})
    request = urllib.request.Request(f"{COMMONS_API}?{query}", headers={"User-Agent": USER_AGENT})
    for attempt in range(8):
        try:
            time.sleep(1.5)
            with urllib.request.urlopen(request, timeout=60) as answer:
                page = json.load(answer)["query"]["pages"][0]
            break
        except urllib.error.HTTPError as error:
            if error.code != 429:
                raise
            time.sleep(15 * (attempt + 1))
    else:
        sys.exit(f"Commons kept answering 429 for {title}")
    if "imageinfo" not in page:
        sys.exit(f"Commons has no file called {title}")
    info = page["imageinfo"][0]
    meta = {k: strip_html(v.get("value")) for k, v in info.get("extmetadata", {}).items()}
    licence = meta.get("LicenseShortName", "")
    if not re.match(r"(?i)^(public domain|cc0|cc by(-sa)? [0-9.]+|pd)", licence):
        sys.exit(f"{title}: licence '{licence}' is not one this library accepts")
    return {
        "page": "https://commons.wikimedia.org/wiki/" + urllib.parse.quote(title.replace(" ", "_")),
        "url": info["url"], "sha1": info["sha1"], "bytes": info["size"],
        "licence": licence, "licence_url": meta.get("LicenseUrl", ""),
        "author": meta.get("Artist", "")[:200], "credit": meta.get("Credit", "")[:200],
    }


def curl(url, dest):
    part = dest.with_name(dest.name + ".part")
    subprocess.run(
        # No --retry-delay: curl then waits as long as a 429 answer's Retry-After asks.
        ["curl", "-fL", "-C", "-", "--retry", "40", "--retry-max-time", "28800", "--retry-all-errors",
         "-A", USER_AGENT, "-sS", "-o", str(part), url], check=True)
    part.replace(dest)


def file_hash(path, algorithm):
    digest = hashlib.new(algorithm)
    with open(path, "rb") as handle:
        while chunk := handle.read(1 << 22):
            digest.update(chunk)
    return digest.hexdigest()


def fetch_commons(title, dest, key):
    """Download a Commons file to `dest` (suffix taken from the source) and return its path."""
    known = load_provenance().get(key)
    if known and (ROOT / known["file"]).exists() and known.get("title") == title:
        return ROOT / known["file"]
    info = commons_info(title)
    dest = dest.with_suffix(Path(urllib.parse.urlparse(info["url"]).path).suffix.lower())
    dest.parent.mkdir(parents=True, exist_ok=True)
    print(f"  downloading {title} ({info['bytes'] / 1e6:.0f} MB)", flush=True)
    curl(info["url"], dest)
    if file_hash(dest, "sha1") != info["sha1"]:
        dest.unlink()
        sys.exit(f"{title}: SHA-1 does not match the one Commons lists; file removed")
    record(key, {**info, "title": title, "file": str(dest.relative_to(ROOT)), "verified": "sha1"})
    return dest


def fetch_direct(video, dest, key, licence, attribution):
    """Download a publisher's file (a zip holding one member) and return the member's path."""
    known = load_provenance().get(key)
    if known and (ROOT / known["file"]).exists() and known.get("url") == video["url"]:
        return ROOT / known["file"]
    member = video["member"]
    dest = dest.with_suffix(Path(member).suffix.lower())
    dest.parent.mkdir(parents=True, exist_ok=True)
    archive = ROOT / "downloads" / Path(urllib.parse.urlparse(video["url"]).path).name
    archive.parent.mkdir(parents=True, exist_ok=True)
    print(f"  downloading {video['url']}", flush=True)
    curl(video["url"], archive)
    listing = subprocess.run(["unzip", "-Z1", str(archive)], check=True, capture_output=True, text=True)
    inner = next((n for n in listing.stdout.splitlines() if n.endswith(member)), None)
    if inner is None:
        sys.exit(f"{archive.name} does not hold {member}")
    with open(dest, "wb") as out:
        subprocess.run(["unzip", "-p", str(archive), inner], check=True, stdout=out)
    verified = "zip CRC only; the publisher lists no checksum"
    if "md5_url" in video:
        with urllib.request.urlopen(urllib.request.Request(video["md5_url"], headers={"User-Agent": USER_AGENT}), timeout=60) as answer:
            listed = answer.read().decode().split()[0].lower()
        if file_hash(dest, "md5") != listed:
            dest.unlink()
            sys.exit(f"{member}: MD5 does not match {video['md5_url']}; file removed")
        verified = "md5 from " + video["md5_url"]
    archive.unlink()
    record(key, {"url": video["url"], "page": video["url"].rsplit("/", 1)[0] + "/", "licence": licence,
                 "author": attribution, "file": str(dest.relative_to(ROOT)), "verified": verified})
    return dest


def movie_dir(movie):
    name = re.sub(r'[<>:"/\\|?*]', "", f"{movie['title']} ({movie['year']})")
    return MEDIA / "movies" / name, name


def episode_stem(show, episode):
    title = re.sub(r'[<>:"/\\|?*]', "", episode["title"])
    return f"{show['title']} S{episode['season']:02d}E{episode['episode']:02d} {title}"


def each_video(lib):
    """Yield (key, title, entry, folder, file stem) for every film and episode."""
    for movie in lib["movies"]:
        folder, name = movie_dir(movie)
        yield f"video:{movie['id']}", movie["title"], movie, folder, name
    for show in lib["shows"]:
        for episode in show["episodes"]:
            folder = MEDIA / "shows" / show["title"] / f"Season {episode['season']:02d}"
            merged = {"licence": show.get("licence"), "attribution": show.get("attribution"), **episode}
            yield (f"video:{show['id']}:s{episode['season']}e{episode['episode']}",
                   f"{show['title']}: {episode['title']}", merged, folder, episode_stem(show, episode))


def video_path(key):
    """The downloaded file for `key`, or None (with a note) when 'fetch' has not got to it yet."""
    known = load_provenance().get(key)
    path = ROOT / known["file"] if known else None
    if path is None or not path.exists():
        print(f"  skipped: {key} is not downloaded yet", flush=True)
        return None
    return path


def step_fetch(lib):
    # One downloader at a time: two would write the same partial file.
    ROOT.mkdir(parents=True, exist_ok=True)
    lock = open(ROOT / "fetch.lock", "w")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        sys.exit(f"another 'build.py fetch' is running on {ROOT}; wait for it to finish")
    # Episodes first and the three largest files last, so the library is usable sooner.
    order = lambda item: (bool(item[2].get("large")), "episode" not in item[2])
    for key, title, entry, folder, stem in sorted(each_video(lib), key=order):
        print(title, flush=True)
        video = entry["video"]
        if "commons" in video:
            fetch_commons(video["commons"], folder / stem, key)
        else:
            fetch_direct(video, folder / stem, key, entry["licence"], entry.get("attribution", ""))


def duration(path):
    out = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", str(path)],
                         check=True, capture_output=True, text=True)
    return float(out.stdout.strip())


def frame(path, at, width=None):
    """One frame of `path` at `at` seconds, tone-mapped to SDR when the source is HDR."""
    probe = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
                            "stream=color_transfer", "-of", "csv=p=0", str(path)], capture_output=True, text=True)
    filters = []
    if "smpte2084" in probe.stdout or "arib-std-b67" in probe.stdout:
        filters.append("zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=hable,zscale=t=bt709:m=bt709:r=tv,format=yuv420p")
    if width:
        filters.append(f"scale={width}:-2")
    command = ["ffmpeg", "-v", "error", "-ss", f"{at:.2f}", "-i", str(path), "-frames:v", "1"]
    if filters:
        command += ["-vf", ",".join(filters)]
    command += ["-f", "image2pipe", "-vcodec", "png", "-"]
    out = subprocess.run(command, check=True, capture_output=True)
    return Image.open(io.BytesIO(out.stdout)).convert("RGB")


def interest(image):
    """Higher for a frame with contrast and colour that is neither murky nor blown out."""
    grey = ImageStat.Stat(image.convert("L"))
    mean, spread = grey.mean[0], grey.stddev[0]
    saturation = ImageStat.Stat(image.convert("HSV")).mean[1]
    exposure = 1.0 - abs(mean - 105) / 150
    return spread * max(exposure, 0.05) * (1.0 + saturation / 255)


def pick_time(path, wanted):
    """The moment to take a still from: the manifest's choice, else the best of 24 samples."""
    if wanted is not None:
        return float(wanted)
    length = duration(path)
    samples = [length * (0.08 + 0.74 * i / 23) for i in range(24)]
    return max(samples, key=lambda at: interest(frame(path, at, 480)))


def cover(image, width, height):
    """Scale and centre-crop `image` to exactly width x height."""
    scale = max(width / image.width, height / image.height)
    image = image.resize((round(image.width * scale + 0.5), round(image.height * scale + 0.5)), Image.LANCZOS)
    left, top = (image.width - width) // 2, (image.height - height) // 2
    return image.crop((left, top, left + width, top + height))


def save_jpeg(image, dest):
    tmp = dest.with_suffix(".tmp.jpg")
    image.save(tmp, "JPEG", quality=90, optimize=True)
    tmp.replace(dest)


def title_poster(still, title, year):
    """A 2:3 poster made from a frame of the film, for titles with no free poster."""
    width, height = 1000, 1500
    poster = cover(still, width, height)
    shade = Image.new("L", (1, height), 0)
    for y in range(height):
        shade.putpixel((0, y), int(235 * max(0.0, (y - height * 0.45) / (height * 0.55)) ** 1.4))
    poster = Image.composite(Image.new("RGB", (width, height), (8, 8, 10)), poster, shade.resize((width, height)))
    draw = ImageDraw.Draw(poster)
    size = 118
    while size > 40:
        font = ImageFont.truetype(str(FONT), size)
        words, lines = title.split(), [""]
        for word in words:
            trial = (lines[-1] + " " + word).strip()
            if draw.textlength(trial, font=font) <= width - 140 or not lines[-1]:
                lines[-1] = trial
            else:
                lines.append(word)
        if len(lines) <= 3 and all(draw.textlength(line, font=font) <= width - 140 for line in lines):
            break
        size -= 6
    y = height - 150 - len(lines) * int(size * 1.04)
    for line in lines:
        draw.text((70, y), line, font=font, fill=(245, 245, 247))
        y += int(size * 1.04)
    draw.text((72, y + 16), str(year), font=ImageFont.truetype(str(FONT), 40), fill=(190, 190, 196))
    return poster


def art_for(key_id, title, year, entry, folder, source, names):
    """Write the poster and backdrop for one film or show into `folder`."""
    poster, backdrop = folder / names[0], folder / names[1]
    picks = entry.get("backdrop", {})
    if not backdrop.exists():
        at = pick_time(source, picks.get("at"))
        save_jpeg(cover(frame(source, at), 3840, 2160), backdrop)
        record(f"backdrop:{key_id}", {"file": str(backdrop.relative_to(ROOT)), "frame_of": title, "at_seconds": round(at, 2)})
    if poster.exists():
        return
    want = entry.get("poster", {})
    if "commons" in want:
        raw = fetch_commons(want["commons"], ROOT / "downloads" / f"poster-{key_id}", f"poster-source:{key_id}")
        with Image.open(raw) as image:
            save_jpeg(cover(image.convert("RGB"), 1000, 1500), poster)
        record(f"poster:{key_id}", {"file": str(poster.relative_to(ROOT)), "from": f"poster-source:{key_id}"})
    else:
        at = pick_time(source, want.get("at", load_provenance()[f"backdrop:{key_id}"]["at_seconds"]))
        save_jpeg(title_poster(frame(source, at), title, year), poster)
        record(f"poster:{key_id}", {"file": str(poster.relative_to(ROOT)), "frame_of": title, "at_seconds": round(at, 2),
                                    "typeset": "title set in Plus Jakarta Sans by this script"})


def step_art(lib):
    for movie in lib["movies"]:
        print(movie["title"], flush=True)
        folder, _ = movie_dir(movie)
        source = video_path(f"video:{movie['id']}")
        if source:
            art_for(movie["id"], movie["title"], movie["year"], movie, folder, source, ("poster.jpg", "backdrop.jpg"))
    for show in lib["shows"]:
        print(show["title"], flush=True)
        folder = MEDIA / "shows" / show["title"]
        last = show["episodes"][-1]
        source = video_path(f"video:{show['id']}:s{last['season']}e{last['episode']}")
        if source:
            art_for(show["id"], show["title"], show["year"], show, folder, source, ("poster.jpg", "backdrop.jpg"))
        for episode in show["episodes"]:
            key = f"{show['id']}:s{episode['season']}e{episode['episode']}"
            source = video_path(f"video:{key}")
            if source is None:
                continue
            thumb = source.with_name(source.stem + "-thumb.jpg")
            if not thumb.exists():
                at = pick_time(source, episode.get("thumb", {}).get("at"))
                save_jpeg(cover(frame(source, at), 1920, 1080), thumb)
                record(f"thumb:{key}", {"file": str(thumb.relative_to(ROOT)), "frame_of": f"{show['title']}: {episode['title']}", "at_seconds": round(at, 2)})


def nfo(root, entry, extra=""):
    lines = ['<?xml version="1.0" encoding="utf-8" standalone="yes"?>', f"<{root}>",
             f"  <title>{escape(entry['title'])}</title>"]
    if "original_title" in entry:
        lines.append(f"  <originaltitle>{escape(entry['original_title'])}</originaltitle>")
    lines += [f"  <year>{entry['year']}</year>", f"  <plot>{escape(entry['plot'])}</plot>"]
    lines += [f"  <genre>{escape(g)}</genre>" for g in entry.get("genres", [])]
    lines += [f"  <director>{escape(d)}</director>" for d in entry.get("directors", [])]
    lines += [f"  <actor><name>{escape(a)}</name><type>Actor</type></actor>" for a in entry.get("cast", [])]
    lines += [extra, "  <lockdata>true</lockdata>", f"</{root}>", ""]
    return "\n".join(line for line in lines if line)


def step_nfo(lib):
    for movie in lib["movies"]:
        folder, _ = movie_dir(movie)
        if folder.is_dir():
            (folder / "movie.nfo").write_text(nfo("movie", movie))
    for show in lib["shows"]:
        if not (MEDIA / "shows" / show["title"]).is_dir():
            continue
        (MEDIA / "shows" / show["title"] / "tvshow.nfo").write_text(nfo("tvshow", show))
        for episode in show["episodes"]:
            source = video_path(f"video:{show['id']}:s{episode['season']}e{episode['episode']}")
            if source is None:
                continue
            extra = f"  <season>{episode['season']}</season>\n  <episode>{episode['episode']}</episode>"
            source.with_suffix(".nfo").write_text(nfo("episodedetails", episode, extra))


def step_credits(lib):
    subprocess.run([sys.executable, str(HERE / "credits.py")], check=True)


STEPS = {"fetch": step_fetch, "art": step_art, "nfo": step_nfo, "credits": step_credits}

if __name__ == "__main__":
    wanted = sys.argv[1:] or ["all"]
    if wanted == ["all"]:
        wanted = list(STEPS)
    if any(step not in STEPS for step in wanted):
        sys.exit(__doc__)
    for step in wanted:
        print(f"== {step}", flush=True)
        STEPS[step](library())
