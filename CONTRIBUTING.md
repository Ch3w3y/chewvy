# Contributing to Chewvy

Thanks for wanting to help. Chewvy is a small project; a clear bug report is
as useful as a patch.

## Reporting a bug

Open an issue with the bug report form. The things that make a playback or
connection bug fixable are:

- the Chewvy version (Settings > About) and how it was installed;
- the operating system, desktop session (Wayland or X11) and GPU with its
  driver version;
- which kind of server (Plex or Jellyfin) and its version;
- for a file that will not play: container, video and audio codec, resolution
  and bit depth (`ffprobe` or `mediainfo` output is ideal);
- the log from a run started in a terminal with `RUST_LOG=info chewvy`.

Logs are written so that tokens never appear in them. Still, read what you
paste: server names and addresses in a log are yours to share or remove.
**Never post a token, a password or a `plex.direct` address.**

Security problems do not go in the public tracker: see `SECURITY.md`.

## Building

`docs/BUILD.md` has the requirements. In short:

```sh
cargo fetch --locked
cargo build --locked -p plex-app
target/debug/chewvy
```

## Before you open a pull request

Run the gates, which are what the maintainers run:

```sh
scripts/check.sh fmt
scripts/check.sh check
scripts/check.sh test
scripts/check.sh build
scripts/xcheck.sh windows   # if you have the MinGW headers; say so if you do not
```

And keep to the rules the code already follows:

- **Say what you tested and where.** A change to playback or rendering that
  was only compiled should say so. The capability matrix
  (`docs/CAPABILITIES.md`) only gains a row with the measurement behind it.
- **No secrets in logs, errors, URLs or process arguments.** The redaction
  tests enforce this; do not weaken them.
- **Read-only towards servers.** Chewvy reports playback progress and nothing
  else. A new write to a server needs a discussion first.
- **`unsafe` stays in the FFI and GL modules** that already allow it, with a
  one-line `SAFETY:` comment per block.
- **Comments explain why**, briefly. History belongs in the commit message.
- **Keep files and functions small** (roughly 800 and 80 lines) and delete
  code a change makes unused.
- **New dependencies** are pinned in the workspace `Cargo.toml`, must offer a
  licence option outside the GPL family, and need the tables in
  `docs/DEPENDENCIES.md` regenerated (`scripts/gen_dependencies.py`).
- **Trademarks.** "Plex" and "Jellyfin" may appear only to name the service a
  source belongs to. Do not add artwork, colours or wording that makes Chewvy
  look like an official client of either, and do not copy code or assets from
  other clients.

Pull requests from forks do not run the packaging workflows; a maintainer
runs them before a release.

## Licence of contributions

Chewvy is licensed under the GNU General Public License, version 3 or later.
By submitting a contribution you agree that it is licensed under the same
terms and that you have the right to submit it.
