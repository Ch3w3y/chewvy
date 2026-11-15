# Questions and answers

## Is Chewvy free?

Yes. Chewvy is free software under the GNU General Public License, version 3
or later. There is no paid tier and no account with the project.

## Is Chewvy made by Plex or Jellyfin?

No. Chewvy is an independent project, not affiliated with Plex, Inc. or the
Jellyfin project. The names say which services it connects to.

## Do I need both Plex and Jellyfin?

No. Either is enough. The first start asks which you have. The switcher at
the top of the sidebar adds the other later, and any number of Jellyfin
servers. Chewvy handles one Plex account at a time, and does not switch
between Plex Home users.

## Which platforms does it run on?

Linux x86_64 is where Chewvy is developed and used. Windows and macOS
packages exist and are less tested. [The README](../README.md#what-works-where)
has the table. There is no Android, iOS or TV app. A
[porting plan](MOBILE_PORT.md) exists and no work on it has started.

## Will it play every file I have?

Chewvy does not claim that. mpv and FFmpeg do the decoding, and
[the capability matrix](CAPABILITIES.md) lists what has been measured. When
mpv cannot play the original file, the item reopens at the same position as
a server remux, then as a 1080p transcode.

## Does it use hardware decoding?

Yes, where the driver offers it. On the Linux development machine H.264,
HEVC, AV1 and VP9 decode through VA-API. Older codecs decode in software.
Hardware decoding on Intel and NVIDIA graphics, on Windows and on macOS has
not been measured. **Settings > Playback > Hardware decoding** turns it off.

## Does HDR work?

In the main window, HDR10 and HLG are tone-mapped to SDR. A separate HDR
window plays HDR10, HLG, HDR10+ and Dolby Vision profiles 5 and 8 with real
HDR output. The HDR window needs a compositor with colour management, such as
KDE Plasma 6.4 or newer on Wayland. Light output at the display has not been
measured, and HDR output on Windows and macOS is untested.

## Can it send Dolby Atmos or DTS:X to my receiver?

No. Audio is always decoded to PCM. Bitstream passthrough is not supported.

## Does it play music, photos or live TV?

No. Chewvy plays films and television.

## How does Watch together work?

For a Jellyfin item, Chewvy starts or joins a SyncPlay group on your server.
For a Plex film or episode, it uses Plex's Watch Together service, and people
in Plex's web app can join the same room. In both, a play, pause or seek by
anyone moves everyone. [Plex Watch Together](PLEX_WATCH_TOGETHER.md) and
[Jellyfin integration](JELLYFIN.md) have the measured timing.

## Where are my sign-in tokens kept?

On Linux, in the system keyring through the Secret Service: KDE Wallet or
GNOME Keyring. If no secret service is running, Chewvy explains that the
alternative is a file that only your user can read and that is not
encrypted, and uses the file only after you agree. On Windows and macOS the
token file lives in the per-user application-data folder. The platform
keychains are not used yet.

Chewvy never stores a password. Plex sign-in uses a code at plex.tv/link.
Jellyfin sign-in uses Quick Connect, or a username and password sent once to
your own server.

## What does Chewvy connect to?

Your own Plex and Jellyfin servers, Plex's account service when you link a
Plex account, and GitHub for the update check and download. The update check
sends only the app's name and version. **Settings > Updates** turns it off.

## Does it change anything on my server?

Only if you ask it to. With **Settings > Playback > Progress sync** turned on
(it is off by default) it reports playback progress and played state. Chewvy never edits metadata, deletes media or
changes server settings, and a test pins the exact set of requests that write
anything.

## What are the backgrounds?

The interface is neutral smoked glass over a photograph. **Settings >
Background** picks the photograph: Aurora, six genres, Christmas, Halloween
and the four seasons. Automatic follows the calendar. With **Use art from my
libraries** on, a theme shows backdrops of films and shows from your own
servers. The bundled photographs are public domain or Creative Commons, and
**Settings > About** credits each one.

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
