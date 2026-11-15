# The Chewvy website

A static page that shows what Chewvy is: `index.html`, `styles.css` and
`main.js`, with no build step and no dependencies. [DEPLOY.md](DEPLOY.md) says
how to publish it.

## Look at it

```sh
python3 -m http.server 8765 --bind 127.0.0.1 --directory site
```

Then open <http://127.0.0.1:8765/>.

## What is here

| Path | What it is |
|---|---|
| `index.html`, `styles.css`, `main.js` | The page |
| `credits.html`, `CREDITS.md` | Every photograph, film and image with its author, licence and source. Generated. |
| `assets/themes/` | The background photographs and `themes.json`. Generated from the app's bundled photographs. |
| `assets/shots/` | Captures of the running app. Generated. |
| `assets/fonts/`, `assets/brand/` | Plus Jakarta Sans as WOFF2 with its licence, and the Chewvy mark and icon |
| `tools/capture.sh` | Regenerates every capture, here and in `docs/media/` |
| `tools/demo-library/` | The demo library of freely licensed films, and the Jellyfin server that holds it |
| `tools/capture/` | The headless session, the tour through the app's screens, and the image export |
| `tools/photos.py` | Remakes `assets/themes/` from `crates/plex-app/assets/backdrop/` |
| `tools/check-links.py` | Checks every link and image reference |
| `deploy/pages.yml` | The proposed GitHub Pages workflow |

## Change the words

Every statement on the page comes from the project's documents, mostly
[`docs/CAPABILITIES.md`](../docs/CAPABILITIES.md). When a capability changes
there, change it here too. Do not add a claim that those documents cannot
back, and keep "not measured" where they say it.

The Backdrops section carries an "In the next release" label while
`<html data-release="pending">`. Set it to `shipped` when the release with
themes is published.

## Regenerate the pictures

```sh
site/tools/capture.sh
```

The first run downloads about 36 GB of films into `~/.cache/chewvy-demo-jf`.
The script starts a Jellyfin container named `chewvy-demo-jf` on
`127.0.0.1:18098`, runs Chewvy in a headless gamescope session with a profile
of its own, and writes the images. It does not touch `~/.config/chewvy` or the
desktop keyring. `site/tools/capture.sh --help` lists its options.

The tour clicks at fixed positions. When the interface moves a control, a
capture shows the wrong screen. Change that control's entry in `POINTS` at the
top of `tools/capture/tour.py` and run the script again.

When the app's bundled photographs change, run `python3 site/tools/photos.py`.
`capture.sh` runs it as its last step.
