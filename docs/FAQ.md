# Questions and answers

## Is Chewvy free?

Yes. Chewvy is free software under the GNU General Public License, version 3
or later. There is no paid tier and no account with the project.

## Is Chewvy made by Plex or Jellyfin?

No. Chewvy is an independent project, not affiliated with Plex, Inc. or the
Jellyfin project. The names say which services it connects to.

## Do I need both Plex and Jellyfin?

No. Either is enough. The first start asks which you have, and the switcher
at the top of the sidebar adds the other, or as many Jellyfin servers as you
like.

## Which platforms does it run on?

Linux, Windows 10 and 11, and macOS 15 or newer on Apple Silicon. There is
no phone or TV app yet. [Capabilities](CAPABILITIES.md) has the details for
each platform.

## Will it play every file I have?

Almost certainly. mpv and FFmpeg decode nearly every format in use. If a
file will not play directly, Chewvy has the server remux or transcode it and
carries on from the same position. [Capabilities](CAPABILITIES.md) lists each
format.

## Does it use hardware decoding?

Yes. On Linux, H.264, HEVC, AV1 and VP9 decode in hardware up to 4K through
VA-API or NVDEC. **Settings > Playback > Hardware decoding** turns it off.

## Does HDR work?

Yes. The HDR window plays HDR10, HDR10+, HLG and Dolby Vision profiles 5 and
8 with real HDR output; the main window tone-maps them for SDR screens. For
HDR output, turn HDR on for your display in your system settings, on a
desktop with HDR colour management such as KDE Plasma 6.4 or newer.

## Can it send Dolby Atmos or DTS:X to my receiver?

Not yet. Chewvy decodes all audio itself, including Dolby TrueHD and DTS-HD,
and outputs PCM.

## Does it play music, photos or live TV?

No. Chewvy plays films and television.

## How does Watch together work?

For a Jellyfin item, Chewvy starts or joins a SyncPlay group on your server.
For a Plex film or episode, it uses Plex's Watch Together service, and people
in Plex's web app can join the same room. A play, pause or seek by anyone
moves everyone. [Plex Watch Together](PLEX_WATCH_TOGETHER.md) and
[Jellyfin integration](JELLYFIN.md) have the details.

## Where are my sign-in tokens kept?

In your system keyring on Linux (KDE Wallet or GNOME Keyring). Without one,
Chewvy offers a file only your user can read. On Windows and macOS the token
file lives in your application-data folder. Tokens never appear in a URL or a
log.

Chewvy never stores a password. Plex sign-in uses a code at plex.tv/link;
Jellyfin uses Quick Connect, or a username and password sent once to your own
server.

## What does Chewvy connect to?

Your own Plex and Jellyfin servers, Plex's account service when you link a
Plex account, and GitHub for the update check and download. The update check
sends only the app's name and version. **Settings > Updates** turns it off.

## Does it change anything on my server?

Only if you ask it to. With **Settings > Playback > Progress sync** turned
on, it reports playback progress and played state. Chewvy never edits
metadata, deletes media or changes server settings.

## What are the backgrounds?

The interface is smoked glass over a photograph. **Settings > Background**
picks one, or leaves it on Automatic to follow the season.

## Why do Windows and macOS warn me about it?

The packages are not code-signed, and the macOS app is not notarised.
[Install Chewvy](INSTALL.md) says how to open each one and how to check a
download against `SHA256SUMS`.

## How do I update it?

Chewvy tells you when a release is out and can install it. See
[Update](INSTALL.md#update).

## Which player does it use?

libmpv. mpv draws each video frame into the app's own OpenGL window, and the
interface is painted over it with egui. There is no web view.

## How do I report a bug or a security problem?

Bugs go to the issue tracker. See [Contributing](../CONTRIBUTING.md). A
security problem goes through [SECURITY.md](../SECURITY.md), not the public
tracker.
