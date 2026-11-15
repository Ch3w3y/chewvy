#!/usr/bin/env bash
# Regenerate every screenshot on the website and in docs/media from scratch.
#
#   site/tools/capture.sh [--binary PATH] [--only NAME[,NAME...]] [--keep-profile] [--skip-fetch]
#
# What it does, in order:
#   1. demo-library/build.py   downloads the freely licensed films (about 36 GB the
#                              first time; later runs skip what is there), makes
#                              their posters and backdrops, writes site/CREDITS.md
#   2. demo-library/server.py  starts the demo Jellyfin container on 127.0.0.1:18098
#   3. capture/tour.py         runs Chewvy in a headless 2560x1600 session with a
#                              profile of its own, signs in, and captures each screen
#   4. capture/publish.py      writes site/assets/shots/ and docs/media/
#   5. photos.py               remakes the site's theme photographs from the app's
#
# Nothing here touches ~/.config/chewvy, the desktop keyring or any other Jellyfin
# container. State lives under $CHEWVY_DEMO_DIR (default ~/.cache/chewvy-demo-jf).
#
# --binary PATH    the chewvy to capture (default target/debug/chewvy)
# --only NAMES     capture only these screens: first-run, home, library, detail,
#                  show, search, player, settings, backgrounds
# --keep-profile   reuse the signed-in profile of the last run instead of signing in again
# --skip-fetch     download nothing: capture the library as it is now
#
# Needs: docker, gamescope, ffmpeg, curl, unzip, python3 with Pillow.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
root=${CHEWVY_DEMO_DIR:-$HOME/.cache/chewvy-demo-jf}
binary=$repo/target/debug/chewvy
only=
keep_profile=0
fetch=fetch

while [ $# -gt 0 ]; do
	case "$1" in
		--binary) binary=$2; shift ;;
		--only) only=$2; shift ;;
		--keep-profile) keep_profile=1 ;;
		--skip-fetch) fetch= ;;
		-h | --help) sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
		*) echo "unknown option: $1" >&2; exit 2 ;;
	esac
	shift
done

for tool in docker gamescope ffmpeg ffprobe curl unzip python3; do
	command -v "$tool" >/dev/null || { echo "capture.sh needs $tool" >&2; exit 1; }
done
python3 -c 'import PIL' 2>/dev/null || { echo "capture.sh needs Pillow for python3" >&2; exit 1; }
[ -x "$binary" ] || { echo "no chewvy at $binary; build it with: cargo build --locked -p plex-app" >&2; exit 1; }

python3 "$here/demo-library/build.py" $fetch art nfo
python3 "$here/demo-library/server.py" up
python3 "$here/demo-library/build.py" credits

shots=$root/shots
rm -rf "$shots"
[ "$keep_profile" = 1 ] || rm -rf "$root/profiles"
python3 "$here/capture/tour.py" "$shots" --binary "$binary" ${only:+--only "$only"}
python3 "$here/capture/publish.py" "$shots"
python3 "$here/photos.py"
echo "captures are in $repo/site/assets/shots and $repo/docs/media"
