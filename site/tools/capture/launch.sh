#!/usr/bin/env bash
# usage: launch.sh PROFILE_DIR DRIVE_DIR [BINARY]
#
# Starts Chewvy in a headless gamescope session at 2560x1600 physical pixels
# (1280x800 points at 2x), with a profile of its own. It cannot reach the
# desktop's keyring, config or sound: HOME and the XDG directories point into
# PROFILE_DIR, the D-Bus session address names a socket that does not exist,
# and XDG_RUNTIME_DIR is private, so PipeWire and PulseAudio are not found.
set -eu
profile=${1:?profile dir}; drive=${2:?drive dir}
binary=${3:-$(cd "$(dirname "$0")/../../.." && pwd)/target/debug/chewvy}
mkdir -p "$profile"/{home,config,cache,data,run} "$drive"
chmod 700 "$profile/run"
exec env -i PATH="$PATH" LANG=C.UTF-8 \
	HOME="$profile/home" XDG_CONFIG_HOME="$profile/config" XDG_CACHE_HOME="$profile/cache" \
	XDG_DATA_HOME="$profile/data" XDG_RUNTIME_DIR="$profile/run" \
	DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent" \
	WINIT_X11_SCALE_FACTOR=2 PLEX_APP_DRIVE="$drive" \
	gamescope --backend headless -W 2560 -H 1600 -w 2560 -h 1600 -r 60 -- "$binary"
