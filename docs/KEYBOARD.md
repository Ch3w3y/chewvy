# Keyboard shortcuts

Every key Chewvy answers, by screen. The keys come from
`crates/plex-app/src/ui/app.rs` (browsing) and
`crates/plex-app/src/ui/player/mod.rs` (the player).

## Every screen

| Key | Action |
|---|---|
| F11 | Enter or leave full screen |

## Browsing

| Key | Action |
|---|---|
| Esc, Alt+Left | Go back |
| Backspace | Go back, unless a text field has the focus |
| `/`, Ctrl+K | Go to the search field |
| Arrow keys | Move between posters in a grid and in search results |
| Enter | Open the selected poster. In the search field, run the search. |

## Player

| Key | Action |
|---|---|
| Space, K | Play or pause |
| Left, J | Seek back 10 seconds |
| Right, L | Seek forward 10 seconds |
| Up, Down | Volume up or down by 5 |
| M | Mute |
| F | Enter or leave full screen |
| C | Next subtitle track |
| A | Next audio track |
| `[`, `]` | Slower, faster. Not available while you watch together. |
| S | Skip the intro or the credits, when the title has a marker |
| N | Next episode |
| I | Show or hide playback information |
| Esc | Leave full screen. Out of full screen, leave the player. |

## The HDR window

The HDR window is mpv's own window, so its keys are mpv's. Chewvy adds these:

| Key | Action |
|---|---|
| N, Enter, `>` | Next episode |

**Settings > HDR & Dolby Vision > Start full screen** opens the HDR window
full screen. F toggles full screen there.
