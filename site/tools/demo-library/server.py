#!/usr/bin/env python3
"""Run the demo Jellyfin server that the site's screenshots are taken from.

    server.py up        start the container, set it up on first run, scan, set watch state
    server.py down      stop and remove the container (its data stays)
    server.py status    print item counts

The container is `chewvy-demo-jf` on http://127.0.0.1:18098. Its data lives
under $CHEWVY_DEMO_DIR (default ~/.cache/chewvy-demo-jf). Internet metadata
and image providers are switched off, so the server shows only what
build.py wrote: the .nfo files, the posters and the backdrops.
"""
import json
import os
import secrets
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(os.environ.get("CHEWVY_DEMO_DIR", Path.home() / ".cache/chewvy-demo-jf"))
NAME = "chewvy-demo-jf"
BASE = "http://127.0.0.1:18098"
USER = "demo"
SERVER_NAME = "Open Cinema"
CREDENTIALS = ROOT / "credentials"
IMAGE = "jellyfin/jellyfin:latest"
AUTH = 'MediaBrowser Client="demo-setup", Device="demo-setup", DeviceId="chewvy-demo-setup", Version="1"'
LIBRARIES = (("Films", "movies", "/media/movies", "Movie"), ("Shows", "tvshows", "/media/shows", "Series"))
# Titles left part-watched so that Home has a Continue Watching row: (title, fraction).
# The last one is the most recently watched, so it is Home's hero.
IN_PROGRESS = (("Charade", 0.31), ("Tears of Steel", 0.6), ("Llamigos", 0.55), ("Big Buck Bunny", 0.2), ("Sintel", 0.42))


def call(method, path, body=None, token=None):
    headers = {"Content-Type": "application/json", "Authorization": AUTH + (f', Token="{token}"' if token else "")}
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(BASE + path, data=data, method=method, headers=headers)
    # A server that is still starting resets connections or answers 503.
    for attempt in range(30):
        try:
            with urllib.request.urlopen(request, timeout=120) as answer:
                raw = answer.read()
            return json.loads(raw) if raw else None
        except urllib.error.HTTPError as error:
            if error.code != 503 or attempt == 29:
                raise
        except (urllib.error.URLError, ConnectionError, TimeoutError):
            if attempt == 29:
                raise
        time.sleep(2)


def password():
    if not CREDENTIALS.exists():
        CREDENTIALS.touch(mode=0o600)
        CREDENTIALS.write_text(secrets.token_urlsafe(18) + "\n")
    return CREDENTIALS.read_text().strip()


def wait_until_up():
    try:
        return call("GET", "/System/Info/Public")
    except (urllib.error.URLError, ConnectionError, TimeoutError):
        sys.exit(f"{NAME} did not answer on {BASE}")


def start_container():
    running = subprocess.run(["docker", "ps", "-q", "-f", f"name=^{NAME}$"], capture_output=True, text=True).stdout.strip()
    if running:
        return
    for folder in ("config", "cache", "media/movies", "media/shows"):
        (ROOT / folder).mkdir(parents=True, exist_ok=True)
    subprocess.run(["docker", "rm", "-f", NAME], capture_output=True)
    subprocess.run(
        ["docker", "run", "-d", "--name", NAME, "-p", "127.0.0.1:18098:8096",
         "-v", f"{ROOT}/config:/config", "-v", f"{ROOT}/cache:/cache", "-v", f"{ROOT}/media:/media:ro", IMAGE],
        check=True, capture_output=True)


def library_options(path, item_type):
    """Local .nfo and image files only: no internet metadata, no scraped artwork."""
    return {"LibraryOptions": {
        "PathInfos": [{"Path": path}],
        "EnableRealtimeMonitor": False,
        "EnableInternetProviders": False,
        "SaveLocalMetadata": False,
        "MetadataSavers": [],
        "LocalMetadataReaderOrder": ["Nfo"],
        "DisabledLocalMetadataReaders": [],
        "EnableChapterImageExtraction": False,
        "ExtractChapterImagesDuringLibraryScan": False,
        "TypeOptions": [
            {"Type": kind, "MetadataFetchers": [], "MetadataFetcherOrder": [], "ImageFetchers": [], "ImageFetcherOrder": []}
            for kind in (item_type, "Season", "Episode", "Person")
        ],
    }}


def first_run_setup():
    if call("GET", "/System/Info/Public").get("StartupWizardCompleted"):
        return
    call("POST", "/Startup/Configuration", {"UICulture": "en-US", "MetadataCountryCode": "US", "PreferredMetadataLanguage": "en"})
    call("GET", "/Startup/User")
    call("POST", "/Startup/User", {"Name": USER, "Password": password()})
    call("POST", "/Startup/RemoteAccess", {"EnableRemoteAccess": True, "EnableAutomaticPortMapping": False})
    call("POST", "/Startup/Complete")


def sign_in():
    answer = call("POST", "/Users/AuthenticateByName", {"Username": USER, "Pw": password()})
    return answer["AccessToken"], answer["User"]["Id"]


def ensure_server_name(token):
    """The app shows the server's name, which would otherwise be the container's id."""
    settings = call("GET", "/System/Configuration", token=token)
    if settings.get("ServerName") != SERVER_NAME:
        settings["ServerName"] = SERVER_NAME
        call("POST", "/System/Configuration", settings, token=token)


def ensure_libraries(token):
    existing = {folder["Name"] for folder in call("GET", "/Library/VirtualFolders", token=token)}
    for name, kind, path, item_type in LIBRARIES:
        if name not in existing:
            call("POST", f"/Library/VirtualFolders?name={name}&collectionType={kind}&refreshLibrary=false",
                 library_options(path, item_type), token=token)


def scan(token):
    call("POST", "/Library/Refresh", token=token)
    time.sleep(3)
    for _ in range(600):
        tasks = call("GET", "/ScheduledTasks", token=token)
        if not any(task["State"] != "Idle" and task["Key"] == "RefreshLibrary" for task in tasks):
            return
        time.sleep(2)
    sys.exit("the library scan did not finish in 20 minutes")


def items(token, user, kinds):
    answer = call("GET", f"/Items?userId={user}&recursive=true&includeItemTypes={kinds}&fields=Path,MediaSources", token=token)
    return answer["Items"]


def set_watch_state(token, user):
    """Leave the IN_PROGRESS titles part-watched, the last one most recently."""
    found = {item["Name"]: item for item in items(token, user, "Movie,Episode")}
    now = time.time()
    for age, (title, fraction) in enumerate(reversed(IN_PROGRESS)):
        item = found.get(title)
        if item is None:
            print(f"  not in the library yet, so not marked part-watched: {title}")
            continue
        played = time.strftime("%Y-%m-%dT%H:%M:%S.0000000Z", time.gmtime(now - 3600 * age))
        call("POST", f"/UserItems/{item['Id']}/UserData?userId={user}",
             {"PlaybackPositionTicks": int(item["RunTimeTicks"] * fraction), "Played": False, "LastPlayedDate": played},
             token=token)


def status(token, user):
    films, shows, episodes = (len(items(token, user, kind)) for kind in ("Movie", "Series", "Episode"))
    print(f"{BASE}: {films} films, {shows} shows, {episodes} episodes; user '{USER}', password in {CREDENTIALS}")


if __name__ == "__main__":
    action = sys.argv[1] if len(sys.argv) > 1 else "up"
    if action == "down":
        subprocess.run(["docker", "rm", "-f", NAME], check=True, capture_output=True)
    elif action == "status":
        status(*sign_in())
    elif action == "up":
        start_container()
        wait_until_up()
        first_run_setup()
        session = sign_in()
        ensure_server_name(session[0])
        ensure_libraries(session[0])
        scan(session[0])
        set_watch_state(*session)
        status(*session)
    else:
        sys.exit(__doc__)
