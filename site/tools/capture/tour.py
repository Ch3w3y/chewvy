#!/usr/bin/env python3
"""Drive Chewvy through its screens and save a capture of each.

    tour.py OUT_DIR [--binary PATH] [--only NAME[,NAME...]]

Chewvy runs in a headless gamescope session (launch.sh) against the demo
Jellyfin server (demo-library/server.py), with a profile of its own under
$CHEWVY_DEMO_DIR. Input goes through the app's PLEX_APP_DRIVE driver, so
every position below is a place a person would click.

Positions are in points on the 1280x800 window. When the interface moves a
control, change its entry in POINTS and run the tour again.
"""
import argparse
import json
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = Path(os.environ.get("CHEWVY_DEMO_DIR", Path.home() / ".cache/chewvy-demo-jf"))
SERVER = "http://127.0.0.1:18098"
USER = "demo"
SCALE = 2  # physical pixels per point in the capture session

POINTS = {
    # First start and the Jellyfin sign-in card.
    "first.jellyfin": (640, 500),
    "signin.address": (640, 364),
    "signin.check": (640, 420),
    "signin.password_route": (640, 417),
    "signin.username": (600, 374),
    "signin.password": (600, 449),
    "signin.submit": (640, 513),
    # The rail.
    "rail.search": (124, 135),
    "rail.home": (77, 188),
    "rail.films": (48, 262),
    "rail.shows": (53, 296),
    "rail.settings": (85, 764),
    # Pages.
    "home.first_dot": (1124, 456),
    "results.first": (383, 330),
    "detail.play": (345, 305),
    "page.middle": (760, 500),
    "player.middle": (640, 400),
}
# Settings > Background, after BACKGROUND_SCROLL: the tiles sit in rows of four.
BACKGROUND_SCROLL = 900  # points of wheel travel from the top of Settings
TILE_COLUMNS = (392, 556, 720, 884)
TILE_ROWS = (246, 368, 490, 612)
TILES = ("automatic", "aurora", "fantasy", "scifi", "epic", "horror", "western", "adventure",
         "christmas", "halloween", "winter", "spring", "summer", "autumn")
LIBRARY_ART_SWITCH = (976, 757)
# The theme behind every other capture. Automatic would change them with the date.
BASE_THEME = "aurora"
# The themes that get a capture of their own, shown behind the Films grid. The
# site no longer shows them; add keys here to capture them again.
THEME_SHOTS = ()


class App:
    """One running Chewvy, driven through the files in its drive directory."""

    def __init__(self, name, binary, out):
        self.profile = ROOT / "profiles" / name
        self.drive = ROOT / "drive" / name
        self.out = out
        shutil.rmtree(self.drive, ignore_errors=True)
        self.drive.mkdir(parents=True)
        log = open(ROOT / f"capture-{name}.log", "w")
        self.process = subprocess.Popen(
            [str(HERE / "launch.sh"), str(self.profile), str(self.drive), str(binary)],
            stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        self.shot("_started", settle=240, keep=False)

    def send(self, *commands):
        """Hand the driver a batch of commands and wait until it has taken them."""
        pending = self.drive / "cmd"
        staged = self.drive / "cmd.new"
        staged.write_text("\n".join(commands) + "\n")
        staged.replace(pending)
        deadline = time.time() + 90
        while pending.exists():
            if self.process.poll() is not None:
                sys.exit(f"Chewvy stopped; see {ROOT}/capture-*.log")
            if time.time() > deadline:
                sys.exit("the driver did not take its commands in 90 s; is the window drawing?")
            time.sleep(0.1)

    def shot(self, name, settle=60, keep=True):
        """Wait `settle` frames, then save the window as OUT_DIR/<name>.ppm."""
        target = self.drive / f"{name}.ppm"
        target.unlink(missing_ok=True)
        self.send(f"wait {settle}", f"shot {name}")
        deadline = time.time() + 120
        while not target.exists() or target.stat().st_size < 1000:
            if self.process.poll() is not None:
                sys.exit(f"Chewvy stopped; see {ROOT}/capture-*.log")
            if time.time() > deadline:
                sys.exit(f"no capture called {name} after 120 s")
            time.sleep(0.2)
        time.sleep(0.5)
        if keep:
            shutil.copyfile(target, self.out / f"{name}.ppm")
            print(f"captured {name}", flush=True)

    def click(self, where, settle=90):
        x, y = (value * SCALE for value in (POINTS[where] if isinstance(where, str) else where))
        self.send(f"move {x} {y}", "wait 4", f"click {x} {y}", f"wait {settle}")

    def hover(self, where):
        x, y = (value * SCALE for value in (POINTS[where] if isinstance(where, str) else where))
        self.send(f"move {x} {y}", "wait 6")

    def key(self, name, settle=30):
        self.send(f"key {name}", f"wait {settle}")

    def type(self, text):
        self.send(f"text {text}", "wait 6")

    def search(self, words):
        """Type `words` into the rail's search field and run the search."""
        self.click("rail.search", settle=10)
        self.send("key ctrl+A", "wait 3", "key Backspace", "wait 3")
        self.type(words)
        self.key("Enter", settle=200)

    def stop(self):
        try:
            os.killpg(self.process.pid, signal.SIGTERM)
        except ProcessLookupError:
            return
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(self.process.pid, signal.SIGKILL)


def signed_in(profile):
    """True when the profile's source list names a Jellyfin server."""
    sources = profile / "config/chewvy/sources.json"
    try:
        return bool(json.loads(sources.read_text()).get("jellyfin"))
    except (OSError, ValueError):
        return False


def sign_in(app):
    """Add the demo server from the first-start screen, with a username and password."""
    password = (ROOT / "credentials").read_text().strip()
    app.click("first.jellyfin", settle=40)
    app.click("signin.address", settle=6)
    app.type(SERVER)
    app.click("signin.check", settle=120)
    app.click("signin.password_route", settle=40)
    app.click("signin.username", settle=6)
    app.type(USER)
    app.click("signin.password", settle=6)
    app.type(password)
    app.click("signin.submit", settle=300)
    # A capture is the driver's only acknowledgement: it lands after every queued command has run.
    app.shot("_after-sign-in", settle=10, keep=False)
    if not signed_in(app.profile):
        sys.exit(f"sign-in did not finish; look at {app.drive}/_after-sign-in.ppm and fix POINTS")


def to_top(app):
    """Scroll the page under the pointer to its top. A page can open at the last page's offset."""
    app.hover("page.middle")
    app.send("scroll 0 6000", "wait 20")


def first_run(app):
    app.shot("first-run", settle=30)


def home(app):
    app.click("rail.home", settle=200)
    to_top(app)
    # The hero moves on to its next title every few seconds. Its first dot brings the first back.
    app.click("home.first_dot", settle=45)
    app.hover("page.middle")
    app.shot("home", settle=6)


def library(app):
    app.click("rail.films", settle=300)
    to_top(app)
    app.hover((1270, 500))
    app.shot("library", settle=60)


def detail(app):
    app.search("Sintel")
    app.click("results.first", settle=260)
    to_top(app)
    app.hover((1270, 700))
    app.shot("detail", settle=40)


def show(app):
    app.search("Caminandes")
    app.click("results.first", settle=260)
    # Far enough down that the episode list shows under the title.
    to_top(app)
    app.send("scroll 0 -330", "wait 20")
    app.hover((1270, 700))
    app.shot("show", settle=200)


def search(app):
    app.search("the")
    to_top(app)
    app.hover((1270, 700))
    app.shot("search", settle=60)


def player(app):
    app.search("Big Buck Bunny")
    app.click("results.first", settle=220)
    app.click("detail.play", settle=420)
    app.key("Space", settle=20)
    app.hover("player.middle")
    app.shot("player", settle=12)
    app.key("I", settle=30)
    app.hover("player.middle")
    app.shot("player-info", settle=12)
    app.key("I", settle=10)
    app.key("Escape", settle=120)


def settings(app):
    app.click("rail.settings", settle=120)
    to_top(app)
    app.hover((1270, 700))
    app.shot("settings", settle=40)


def background_section(app):
    """Open Settings and scroll until the Background section fills the window."""
    app.click("rail.home", settle=30)
    app.click("rail.settings", settle=60)
    to_top(app)
    app.send(f"scroll 0 -{BACKGROUND_SCROLL}", "wait 40")


def choose_theme(app, key):
    index = TILES.index(key)
    background_section(app)
    app.click((TILE_COLUMNS[index % 4], TILE_ROWS[index // 4]), settle=150)


def backgrounds(app):
    """The picker, then the Films grid over each theme's bundled photograph."""
    background_section(app)
    app.hover((1150, 420))
    app.shot("settings-background", settle=300)
    # With library art off, a theme shows its bundled photograph, which is the same on every run.
    app.click(LIBRARY_ART_SWITCH, settle=30)
    for key in THEME_SHOTS:
        choose_theme(app, key)
        app.click("rail.films", settle=240)
        to_top(app)
        app.hover((1270, 500))
        app.shot(f"theme-{key}", settle=40)
    background_section(app)
    app.click(LIBRARY_ART_SWITCH, settle=30)
    choose_theme(app, BASE_THEME)


SCREENS = {"home": home, "library": library, "detail": detail, "show": show,
           "search": search, "player": player, "settings": settings, "backgrounds": backgrounds}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("out", type=Path)
    parser.add_argument("--binary", type=Path, default=HERE.parents[2] / "target/debug/chewvy")
    parser.add_argument("--only", default="")
    arguments = parser.parse_args()
    wanted = [name for name in arguments.only.split(",") if name] or ["first-run", *SCREENS]
    unknown = [name for name in wanted if name != "first-run" and name not in SCREENS]
    if unknown:
        sys.exit(f"unknown screen: {', '.join(unknown)}; choose from first-run, {', '.join(SCREENS)}")
    arguments.out.mkdir(parents=True, exist_ok=True)

    if "first-run" in wanted:
        shutil.rmtree(ROOT / "profiles/first-run", ignore_errors=True)
        app = App("first-run", arguments.binary, arguments.out)
        try:
            first_run(app)
        finally:
            app.stop()

    screens = [name for name in wanted if name in SCREENS]
    if screens:
        app = App("main", arguments.binary, arguments.out)
        try:
            if not signed_in(app.profile):
                sign_in(app)
            choose_theme(app, BASE_THEME)
            for name in screens:
                SCREENS[name](app)
        finally:
            app.stop()


if __name__ == "__main__":
    main()
