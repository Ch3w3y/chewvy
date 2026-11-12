# Dependencies and licences

Every third-party crate is pinned in the workspace `Cargo.toml` and frozen in
`Cargo.lock`. This file records the licence each locked crate version declares,
which native libraries the app needs, and how the licences combine. It is
checked by `crates/plex-core/tests/dependency_licensing.rs`: a crate that is
locked but missing here, or listed here but no longer locked, fails `cargo test`.

## Regenerating the tables

```sh
source scripts/env.sh
python3 scripts/gen_dependencies.py --write docs/DEPENDENCIES.md   # rewrites only the marked regions
python3 scripts/gen_dependencies.py --summary
```

What each column is based on:

* **Version / source**: read from the committed `Cargo.lock`.
* **License**: `Cargo.lock` has no licence field, so each cell is the SPDX
  expression that exact crate version declares in its own `Cargo.toml`, read
  from the local crate cache. Workspace members inherit the root manifest's
  licence.
* **Compiled vs locked only**: `cargo tree --locked --edges normal,build,dev`
  for the default feature set on the host target. The second table holds
  optional and target-specific edges that this build does not compile (for
  example `bindgen`, which is only used to regenerate the libmpv bindings, and
  `windows-sys`, which only the Windows target compiles). They are recorded
  rather than dropped.
* Licences are as declared by each crate, not independently audited against
  its source files.

## Workspace crates

`plex-player -> plex-core`; `jellyfin-core -> plex-core`;
`plex-cli -> plex-core, plex-player`;
`plex-app -> plex-core, jellyfin-core, plex-player`. `plex-core` depends on no
display or FFI crate (enforced by `crates/plex-core/tests/workspace_contract.rs`).

## Table A: crates compiled by the default build

<!-- deps:begin:built -->
| crate | locked version | license (Cargo.lock) | declared by / graph role | source |
|---|---|---|---|---|
| ab_glyph | 0.2.32 | license: Apache-2.0 | transitive | crates.io |
| ab_glyph_rasterizer | 0.1.10 | license: Apache-2.0 | transitive | crates.io |
| accesskit | 0.24.1 | license: MIT OR Apache-2.0 | transitive | crates.io |
| adler2 | 2.0.1 | license: 0BSD OR MIT OR Apache-2.0 | transitive | crates.io |
| ahash | 0.8.12 | license: MIT OR Apache-2.0 | transitive | crates.io |
| aho-corasick | 1.1.5 | license: Unlicense OR MIT | transitive | crates.io |
| anstream | 1.0.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| anstyle | 1.0.14 | license: MIT OR Apache-2.0 | transitive | crates.io |
| anstyle-parse | 1.0.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| anstyle-query | 1.1.5 | license: MIT OR Apache-2.0 | transitive | crates.io |
| arboard | 3.6.1 | license: MIT OR Apache-2.0 | transitive | crates.io |
| arrayref | 0.3.9 | license: BSD-2-Clause | transitive | crates.io |
| arrayvec | 0.7.8 | license: MIT OR Apache-2.0 | transitive | crates.io |
| as-raw-xcb-connection | 1.0.1 | license: MIT OR Apache-2.0 | transitive | crates.io |
| ascii | 1.1.0 | license: Apache-2.0 OR MIT | transitive | crates.io |
| async-broadcast | 0.7.2 | license: MIT OR Apache-2.0 | transitive | crates.io |
| async-channel | 2.5.0 | license: Apache-2.0 OR MIT | transitive | crates.io |
| async-executor | 1.14.0 | license: Apache-2.0 OR MIT | transitive | crates.io |
| async-io | 2.6.0 | license: Apache-2.0 OR MIT | transitive | crates.io |
| async-lock | 3.4.2 | license: Apache-2.0 OR MIT | transitive | crates.io |
| async-process | 2.5.0 | license: Apache-2.0 OR MIT | transitive | crates.io |
| async-recursion | 1.1.1 | license: MIT OR Apache-2.0 | transitive | crates.io |
| async-signal | 0.2.14 | license: Apache-2.0 OR MIT | transitive | crates.io |
| async-task | 4.7.1 | license: Apache-2.0 OR MIT | transitive | crates.io |
| async-trait | 0.1.92 | license: MIT OR Apache-2.0 | transitive | crates.io |
| atomic-waker | 1.1.2 | license: Apache-2.0 OR MIT | transitive | crates.io |
| autocfg | 1.5.1 | license: Apache-2.0 OR MIT | transitive | crates.io |
| base64 | 0.22.1 | license: MIT OR Apache-2.0 | plex-core (dependencies) | crates.io |
| bitflags | 2.13.2 | license: MIT OR Apache-2.0 | transitive | crates.io |
| blocking | 1.7.0 | license: Apache-2.0 OR MIT | transitive | crates.io |
| bytemuck | 1.25.2 | license: Zlib OR Apache-2.0 OR MIT | transitive | crates.io |
| bytemuck_derive | 1.12.0 | license: Zlib OR Apache-2.0 OR MIT | transitive | crates.io |
| byteorder-lite | 0.1.0 | license: Unlicense OR MIT | transitive | crates.io |
| bytes | 1.12.1 | license: MIT | transitive | crates.io |
| calloop | 0.13.0 | license: MIT | transitive | crates.io |
| calloop | 0.14.4 | license: MIT | transitive | crates.io |
| calloop-wayland-source | 0.3.0 | license: MIT | transitive | crates.io |
| calloop-wayland-source | 0.4.1 | license: MIT | transitive | crates.io |
| cc | 1.4.5 | license: MIT OR Apache-2.0 | transitive | crates.io |
| cfg-if | 1.0.4 | license: MIT OR Apache-2.0 | transitive | crates.io |
| cfg_aliases | 0.2.2 | license: MIT | transitive | crates.io |
| chunked_transfer | 1.5.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| clap | 4.6.6 | license: MIT OR Apache-2.0 | plex-cli (dependencies) | crates.io |
| clap_builder | 4.6.6 | license: MIT OR Apache-2.0 | transitive | crates.io |
| clap_derive | 4.6.4 | license: MIT OR Apache-2.0 | transitive | crates.io |
| clap_lex | 1.1.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| color | 0.3.3 | license: Apache-2.0 OR MIT | transitive | crates.io |
| colorchoice | 1.0.5 | license: MIT OR Apache-2.0 | transitive | crates.io |
| concurrent-queue | 2.5.0 | license: Apache-2.0 OR MIT | transitive | crates.io |
| crc32fast | 1.5.1 | license: MIT OR Apache-2.0 | transitive | crates.io |
| crossbeam-utils | 0.8.22 | license: MIT OR Apache-2.0 | transitive | crates.io |
| cursor-icon | 1.2.0 | license: MIT OR Apache-2.0 OR Zlib | transitive | crates.io |
| data-url | 0.3.2 | license: MIT OR Apache-2.0 | transitive | crates.io |
| dirs-sys | 0.5.0 | license: MIT OR Apache-2.0 | plex-core (dependencies) | crates.io |
| displaydoc | 0.2.7 | license: MIT OR Apache-2.0 | transitive | crates.io |
| dlib | 0.5.3 | license: MIT | transitive | crates.io |
| downcast-rs | 1.2.1 | license: MIT/Apache-2.0 | transitive | crates.io |
| dpi | 0.1.2 | license: Apache-2.0 AND MIT | transitive | crates.io |
| ecolor | 0.36.1 | license: MIT OR Apache-2.0 | transitive | crates.io |
| egui | 0.36.1 | license: MIT OR Apache-2.0 | plex-app (dependencies) | crates.io |
| egui-winit | 0.36.1 | license: MIT OR Apache-2.0 | plex-app (dependencies) | crates.io |
| egui_glow | 0.36.1 | license: MIT OR Apache-2.0 | plex-app (dependencies) | crates.io |
| either | 1.18.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| emath | 0.36.1 | license: MIT OR Apache-2.0 | transitive | crates.io |
| endi | 1.1.1 | license: MIT | transitive | crates.io |
| enumflags2 | 0.7.12 | license: MIT OR Apache-2.0 | transitive | crates.io |
| enumflags2_derive | 0.7.12 | license: MIT OR Apache-2.0 | transitive | crates.io |
| env_filter | 2.0.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| env_logger | 0.11.11 | license: MIT OR Apache-2.0 | plex-app (dev-dependencies), plex-core (dev-dependencies) | crates.io |
| epaint | 0.36.1 | license: MIT OR Apache-2.0 | plex-app (dependencies) | crates.io |
| epaint_default_fonts | 0.36.1 | license: (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 | transitive | crates.io |
| equivalent | 1.0.2 | license: Apache-2.0 OR MIT | transitive | crates.io |
| errno | 0.3.14 | license: MIT OR Apache-2.0 | transitive | crates.io |
| euclid | 0.22.14 | license: MIT OR Apache-2.0 | transitive | crates.io |
| event-listener | 5.4.2 | license: Apache-2.0 OR MIT | transitive | crates.io |
| event-listener-strategy | 0.5.4 | license: Apache-2.0 OR MIT | transitive | crates.io |
| fastrand | 2.5.0 | license: Apache-2.0 OR MIT | transitive | crates.io |
| fdeflate | 0.3.7 | license: MIT OR Apache-2.0 | transitive | crates.io |
| fearless_simd | 0.4.1 | license: Apache-2.0 OR MIT | transitive | crates.io |
| find-msvc-tools | 0.1.12 | license: MIT OR Apache-2.0 | transitive | crates.io |
| flate2 | 1.1.10 | license: MIT OR Apache-2.0 | transitive | crates.io |
| float-cmp | 0.9.0 | license: MIT | transitive | crates.io |
| foldhash | 0.2.0 | license: Zlib | transitive | crates.io |
| font-types | 0.12.4 | license: MIT OR Apache-2.0 | transitive | crates.io |
| form_urlencoded | 1.2.2 | license: MIT OR Apache-2.0 | transitive | crates.io |
| futures-core | 0.3.34 | license: MIT OR Apache-2.0 | transitive | crates.io |
| futures-io | 0.3.34 | license: MIT OR Apache-2.0 | transitive | crates.io |
| futures-lite | 2.6.1 | license: Apache-2.0 OR MIT | transitive | crates.io |
| gethostname | 1.1.0 | license: Apache-2.0 | transitive | crates.io |
| getrandom | 0.2.17 | license: MIT OR Apache-2.0 | plex-core (dependencies) | crates.io |
| getrandom | 0.3.4 | license: MIT OR Apache-2.0 | plex-core (dependencies) | crates.io |
| getrandom | 0.4.3 | license: MIT OR Apache-2.0 | plex-core (dependencies) | crates.io |
| gl_generator | 0.14.0 | license: Apache-2.0 | transitive | crates.io |
| glow | 0.17.0 | license: MIT OR Apache-2.0 OR Zlib | plex-app (dependencies) | crates.io |
| glutin | 0.32.3 | license: Apache-2.0 | plex-app (dependencies) | crates.io |
| glutin-winit | 0.5.0 | license: MIT | plex-app (dependencies) | crates.io |
| glutin_egl_sys | 0.7.1 | license: Apache-2.0 | transitive | crates.io |
| glutin_glx_sys | 0.6.1 | license: Apache-2.0 | transitive | crates.io |
| guillotiere | 0.7.0 | license: MIT/Apache-2.0 | transitive | crates.io |
| harfrust | 0.12.0 | license: MIT | transitive | crates.io |
| hashbrown | 0.17.1 | license: MIT OR Apache-2.0 | transitive | crates.io |
| heck | 0.5.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| hex | 0.4.3 | license: MIT OR Apache-2.0 | transitive | crates.io |
| http | 1.5.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| httparse | 1.10.1 | license: MIT OR Apache-2.0 | transitive | crates.io |
| httpdate | 1.0.3 | license: MIT OR Apache-2.0 | transitive | crates.io |
| icu_collections | 2.3.0 | license: Unicode-3.0 | transitive | crates.io |
| icu_locale_core | 2.3.0 | license: Unicode-3.0 | transitive | crates.io |
| icu_normalizer | 2.3.0 | license: Unicode-3.0 | transitive | crates.io |
| icu_normalizer_data | 2.3.0 | license: Unicode-3.0 | transitive | crates.io |
| icu_properties | 2.3.0 | license: Unicode-3.0 | transitive | crates.io |
| icu_properties_data | 2.3.0 | license: Unicode-3.0 | transitive | crates.io |
| icu_provider | 2.3.1 | license: Unicode-3.0 | transitive | crates.io |
| idna | 1.1.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| idna_adapter | 1.2.2 | license: Apache-2.0 OR MIT | transitive | crates.io |
| image | 0.25.10 | license: MIT OR Apache-2.0 | plex-app (dependencies) | crates.io |
| imagesize | 0.14.0 | license: MIT | transitive | crates.io |
| indexmap | 2.14.2 | license: Apache-2.0 OR MIT | transitive | crates.io |
| is_terminal_polyfill | 1.70.2 | license: MIT OR Apache-2.0 | transitive | crates.io |
| itertools | 0.15.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| itoa | 1.0.18 | license: MIT OR Apache-2.0 | transitive | crates.io |
| jellyfin-core | 0.1.3 | GPL-3.0-or-later | workspace member target | workspace member (path) |
| jiff | 0.2.31 | license: Unlicense OR MIT | transitive | crates.io |
| khronos_api | 3.1.0 | license: Apache-2.0 | transitive | crates.io |
| kurbo | 0.13.1 | license: Apache-2.0 OR MIT | transitive | crates.io |
| libc | 0.2.189 | license: MIT OR Apache-2.0 | transitive | crates.io |
| libloading | 0.8.9 | license: ISC | transitive | crates.io |
| linebender_resource_handle | 0.1.1 | license: Apache-2.0 OR MIT | transitive | crates.io |
| linux-raw-sys | 0.12.1 | license: Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | transitive | crates.io |
| linux-raw-sys | 0.4.15 | license: Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | transitive | crates.io |
| litemap | 0.8.3 | license: Unicode-3.0 | transitive | crates.io |
| lock_api | 0.4.14 | license: MIT OR Apache-2.0 | transitive | crates.io |
| log | 0.4.34 | license: MIT OR Apache-2.0 | plex-app (dependencies), plex-core (dependencies) | crates.io |
| memchr | 2.8.3 | license: Unlicense OR MIT | transitive | crates.io |
| memmap2 | 0.9.11 | license: MIT OR Apache-2.0 | transitive | crates.io |
| miniz_oxide | 0.8.9 | license: MIT OR Zlib OR Apache-2.0 | transitive | crates.io |
| miniz_oxide | 0.9.1 | license: MIT OR Zlib OR Apache-2.0 | transitive | crates.io |
| moxcms | 0.8.1 | license: BSD-3-Clause OR Apache-2.0 | transitive | crates.io |
| nohash-hasher | 0.2.0 | license: Apache-2.0 OR MIT | transitive | crates.io |
| num-traits | 0.2.19 | license: MIT OR Apache-2.0 | transitive | crates.io |
| once_cell | 1.21.4 | license: MIT OR Apache-2.0 | transitive | crates.io |
| openssl-probe | 0.2.1 | license: MIT OR Apache-2.0 | transitive | crates.io |
| option-ext | 0.2.0 | license: MPL-2.0 | transitive | crates.io |
| ordered-stream | 0.2.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| owned_ttf_parser | 0.25.1 | license: Apache-2.0 | transitive | crates.io |
| parking | 2.2.1 | license: Apache-2.0 OR MIT | transitive | crates.io |
| parking_lot | 0.12.5 | license: MIT OR Apache-2.0 | transitive | crates.io |
| parking_lot_core | 0.9.12 | license: MIT OR Apache-2.0 | transitive | crates.io |
| peniko | 0.6.1 | license: Apache-2.0 OR MIT | transitive | crates.io |
| percent-encoding | 2.3.2 | license: MIT OR Apache-2.0 | transitive | crates.io |
| pico-args | 0.5.0 | license: MIT | transitive | crates.io |
| pin-project-lite | 0.2.17 | license: Apache-2.0 OR MIT | transitive | crates.io |
| piper | 0.2.5 | license: MIT OR Apache-2.0 | transitive | crates.io |
| pkg-config | 0.3.34 | license: MIT OR Apache-2.0 | plex-cli (build-dependencies), plex-player (build-dependencies) | crates.io |
| plex-app | 0.1.3 | GPL-3.0-or-later | workspace member target | workspace member (path) |
| plex-cli | 0.1.3 | GPL-3.0-or-later | workspace member target | workspace member (path) |
| plex-core | 0.1.3 | GPL-3.0-or-later | workspace member target | workspace member (path) |
| plex-player | 0.1.3 | GPL-3.0-or-later | workspace member target | workspace member (path) |
| png | 0.18.1 | license: MIT OR Apache-2.0 | transitive | crates.io |
| polling | 3.11.0 | license: Apache-2.0 OR MIT | transitive | crates.io |
| polycool | 0.4.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| potential_utf | 0.1.6 | license: Unicode-3.0 | transitive | crates.io |
| proc-macro-crate | 3.5.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| proc-macro2 | 1.0.107 | license: MIT OR Apache-2.0 | transitive | crates.io |
| profiling | 1.0.18 | license: MIT OR Apache-2.0 | transitive | crates.io |
| pxfm | 0.1.30 | license: BSD-3-Clause OR Apache-2.0 | transitive | crates.io |
| quick-xml | 0.41.0 | license: MIT | transitive | crates.io |
| quote | 1.0.47 | license: MIT OR Apache-2.0 | transitive | crates.io |
| raw-window-handle | 0.6.2 | license: MIT OR Apache-2.0 OR Zlib | plex-player (dependencies) | crates.io |
| read-fonts | 0.41.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| regex | 1.13.1 | license: MIT OR Apache-2.0 | transitive | crates.io |
| regex-automata | 0.4.18 | license: MIT OR Apache-2.0 | transitive | crates.io |
| regex-syntax | 0.8.11 | license: MIT OR Apache-2.0 | transitive | crates.io |
| resvg | 0.47.0 | license: Apache-2.0 OR MIT | plex-app (dependencies) | crates.io |
| rgb | 0.8.53 | license: MIT | transitive | crates.io |
| ring | 0.17.14 | license: Apache-2.0 AND ISC | plex-app (dependencies) | crates.io |
| roxmltree | 0.21.1 | license: MIT OR Apache-2.0 | transitive | crates.io |
| rustix | 0.38.44 | license: Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | transitive | crates.io |
| rustix | 1.1.4 | license: Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | transitive | crates.io |
| rustls | 0.23.44 | license: Apache-2.0 OR ISC OR MIT | plex-core (dependencies) | crates.io |
| rustls-native-certs | 0.8.4 | license: Apache-2.0 OR ISC OR MIT | plex-core (dependencies) | crates.io |
| rustls-pki-types | 1.15.1 | license: MIT OR Apache-2.0 | transitive | crates.io |
| rustls-webpki | 0.103.15 | license: ISC | transitive | crates.io |
| scoped-tls | 1.0.1 | license: MIT/Apache-2.0 | transitive | crates.io |
| scopeguard | 1.2.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| sctk-adwaita | 0.10.1 | license: MIT | transitive | crates.io |
| self_cell | 1.3.0 | license: Apache-2.0 OR GPL-2.0-only | transitive | crates.io |
| serde | 1.0.229 | license: MIT OR Apache-2.0 | jellyfin-core (dependencies), plex-app (dependencies), plex-core (dependencies) | crates.io |
| serde_core | 1.0.229 | license: MIT OR Apache-2.0 | transitive | crates.io |
| serde_derive | 1.0.229 | license: MIT OR Apache-2.0 | transitive | crates.io |
| serde_json | 1.0.151 | license: MIT OR Apache-2.0 | jellyfin-core (dependencies), plex-app (dependencies), plex-cli (dependencies), plex-core (dependencies) | crates.io |
| serde_repr | 0.1.21 | license: MIT OR Apache-2.0 | transitive | crates.io |
| shlex | 2.0.1 | license: MIT OR Apache-2.0 | transitive | crates.io |
| signal-hook-registry | 1.4.8 | license: MIT OR Apache-2.0 | transitive | crates.io |
| simd-adler32 | 0.3.10 | license: MIT | transitive | crates.io |
| simplecss | 0.2.2 | license: Apache-2.0 OR MIT | transitive | crates.io |
| siphasher | 1.0.3 | license: MIT/Apache-2.0 | transitive | crates.io |
| skrifa | 0.44.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| slab | 0.4.12 | license: MIT | transitive | crates.io |
| smallvec | 1.16.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| smithay-client-toolkit | 0.19.2 | license: MIT | transitive | crates.io |
| smithay-client-toolkit | 0.20.0 | license: MIT | transitive | crates.io |
| smithay-clipboard | 0.7.3 | license: MIT | transitive | crates.io |
| smol_str | 0.2.2 | license: MIT OR Apache-2.0 | transitive | crates.io |
| stable_deref_trait | 1.2.1 | license: MIT OR Apache-2.0 | transitive | crates.io |
| strict-num | 0.1.1 | license: MIT | transitive | crates.io |
| strsim | 0.11.1 | license: MIT | transitive | crates.io |
| subtle | 2.6.1 | license: BSD-3-Clause | transitive | crates.io |
| svgtypes | 0.16.1 | license: Apache-2.0 OR MIT | transitive | crates.io |
| syn | 2.0.119 | license: MIT OR Apache-2.0 | transitive | crates.io |
| syn | 3.0.5 | license: MIT OR Apache-2.0 | transitive | crates.io |
| synstructure | 0.13.2 | license: MIT | transitive | crates.io |
| thiserror | 1.0.69 | license: MIT OR Apache-2.0 | jellyfin-core (dependencies), plex-core (dependencies), plex-player (dependencies) | crates.io |
| thiserror | 2.0.20 | license: MIT OR Apache-2.0 | jellyfin-core (dependencies), plex-core (dependencies), plex-player (dependencies) | crates.io |
| thiserror-impl | 1.0.69 | license: MIT OR Apache-2.0 | transitive | crates.io |
| thiserror-impl | 2.0.20 | license: MIT OR Apache-2.0 | transitive | crates.io |
| tiny-skia | 0.11.4 | license: BSD-3-Clause | transitive | crates.io |
| tiny-skia | 0.12.0 | license: BSD-3-Clause | transitive | crates.io |
| tiny-skia-path | 0.11.4 | license: BSD-3-Clause | transitive | crates.io |
| tiny-skia-path | 0.12.0 | license: BSD-3-Clause | transitive | crates.io |
| tiny_http | 0.12.0 | license: MIT OR Apache-2.0 | jellyfin-core (dev-dependencies), plex-core (dev-dependencies) | crates.io |
| tinystr | 0.8.4 | license: Unicode-3.0 | transitive | crates.io |
| toml_datetime | 1.1.1+spec-1.1.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| toml_edit | 0.25.13+spec-1.1.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| toml_parser | 1.1.3+spec-1.1.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| tracing | 0.1.44 | license: MIT | transitive | crates.io |
| tracing-attributes | 0.1.31 | license: MIT | transitive | crates.io |
| tracing-core | 0.1.36 | license: MIT | transitive | crates.io |
| ttf-parser | 0.25.1 | license: MIT OR Apache-2.0 | transitive | crates.io |
| unicode-general-category | 1.1.0 | license: Apache-2.0 | transitive | crates.io |
| unicode-ident | 1.0.24 | license: (MIT OR Apache-2.0) AND Unicode-3.0 | transitive | crates.io |
| unicode-segmentation | 1.13.3 | license: MIT OR Apache-2.0 | transitive | crates.io |
| untrusted | 0.9.0 | license: ISC | transitive | crates.io |
| ureq | 3.3.0 | license: MIT OR Apache-2.0 | jellyfin-core (dependencies), plex-core (dependencies) | crates.io |
| ureq-proto | 0.6.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| url | 2.5.8 | license: MIT OR Apache-2.0 | jellyfin-core (dependencies), plex-core (dependencies) | crates.io |
| usvg | 0.47.0 | license: Apache-2.0 OR MIT | transitive | crates.io |
| utf8-zero | 0.8.1 | license: MIT OR Apache-2.0 | transitive | crates.io |
| utf8_iter | 1.0.4 | license: Apache-2.0 OR MIT | transitive | crates.io |
| utf8parse | 0.2.2 | license: Apache-2.0 OR MIT | transitive | crates.io |
| uuid | 1.26.0 | license: Apache-2.0 OR MIT | transitive | crates.io |
| vello_common | 0.1.0 | license: Apache-2.0 OR MIT | transitive | crates.io |
| vello_cpu | 0.1.0 | license: Apache-2.0 OR MIT | transitive | crates.io |
| version_check | 0.9.5 | license: MIT/Apache-2.0 | transitive | crates.io |
| wayland-backend | 0.3.17 | license: MIT | transitive | crates.io |
| wayland-client | 0.31.15 | license: MIT | transitive | crates.io |
| wayland-csd-frame | 0.3.0 | license: MIT | transitive | crates.io |
| wayland-cursor | 0.31.14 | license: MIT | transitive | crates.io |
| wayland-protocols | 0.32.13 | license: MIT | transitive | crates.io |
| wayland-protocols-experimental | 20250721.0.1 | license: MIT | transitive | crates.io |
| wayland-protocols-misc | 0.3.12 | license: MIT | transitive | crates.io |
| wayland-protocols-plasma | 0.3.12 | license: MIT | transitive | crates.io |
| wayland-protocols-wlr | 0.3.12 | license: MIT | transitive | crates.io |
| wayland-scanner | 0.31.11 | license: MIT | transitive | crates.io |
| wayland-sys | 0.31.11 | license: MIT | transitive | crates.io |
| web-time | 1.1.0 | license: MIT OR Apache-2.0 | transitive | crates.io |
| webbrowser | 1.2.4 | license: MIT OR Apache-2.0 | transitive | crates.io |
| webpki-roots | 1.0.8 | license: CDLA-Permissive-2.0 | transitive | crates.io |
| winit | 0.30.13 | license: Apache-2.0 | plex-app (dependencies) | crates.io |
| winnow | 1.0.4 | license: MIT | transitive | crates.io |
| writeable | 0.6.4 | license: Unicode-3.0 | transitive | crates.io |
| x11-dl | 2.21.0 | license: MIT | transitive | crates.io |
| x11rb | 0.13.2 | license: MIT OR Apache-2.0 | transitive | crates.io |
| x11rb-protocol | 0.13.2 | license: MIT OR Apache-2.0 | transitive | crates.io |
| xcursor | 0.3.11 | license: MIT | transitive | crates.io |
| xdg | 3.0.0 | license: Apache-2.0 OR MIT | plex-app (dependencies), plex-core (dependencies) | crates.io |
| xkbcommon-dl | 0.4.2 | license: MIT | transitive | crates.io |
| xkeysym | 0.2.1 | license: MIT OR Apache-2.0 OR Zlib | transitive | crates.io |
| xml-rs | 0.8.29 | license: MIT | transitive | crates.io |
| xmlwriter | 0.1.0 | license: MIT | transitive | crates.io |
| yoke | 0.8.3 | license: Unicode-3.0 | transitive | crates.io |
| yoke-derive | 0.8.2 | license: Unicode-3.0 | transitive | crates.io |
| zbus | 5.19.0 | license: MIT | plex-core (dependencies) | crates.io |
| zbus_macros | 5.19.0 | license: MIT | transitive | crates.io |
| zbus_names | 4.3.4 | license: MIT | transitive | crates.io |
| zcheapstr | 1.1.0 | license: MIT | transitive | crates.io |
| zerocopy | 0.8.56 | license: BSD-2-Clause OR Apache-2.0 OR MIT | transitive | crates.io |
| zerofrom | 0.1.8 | license: Unicode-3.0 | transitive | crates.io |
| zerofrom-derive | 0.1.7 | license: Unicode-3.0 | transitive | crates.io |
| zeroize | 1.9.0 | license: Apache-2.0 OR MIT | transitive | crates.io |
| zerotrie | 0.2.5 | license: Unicode-3.0 | transitive | crates.io |
| zerovec | 0.11.8 | license: Unicode-3.0 | transitive | crates.io |
| zerovec-derive | 0.11.6 | license: Unicode-3.0 | transitive | crates.io |
| zmij | 1.0.23 | license: MIT | transitive | crates.io |
| zune-core | 0.5.3 | license: MIT OR Apache-2.0 OR Zlib | transitive | crates.io |
| zune-jpeg | 0.5.15 | license: MIT OR Apache-2.0 OR Zlib | transitive | crates.io |
| zvariant | 5.15.0 | license: MIT | transitive | crates.io |
| zvariant_derive | 5.15.0 | license: MIT | transitive | crates.io |
| zvariant_utils | 4.2.0 | license: MIT | transitive | crates.io |
<!-- deps:end:built -->

## Table B: locked in Cargo.lock but not compiled by the default build

<!-- deps:begin:locked-unused -->
| crate | locked version | license (Cargo.lock) | graph role | source |
|---|---|---|---|---|
| android-activity | 0.6.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| android-properties | 0.2.2 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| anstyle-wincon | 3.0.11 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| bindgen | 0.72.1 | license: BSD-3-Clause | declared by a member, optional/off by default: not compiled by the default build | crates.io |
| bitflags | 1.3.2 | license: MIT/Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| block2 | 0.5.1 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| bumpalo | 3.20.3 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| cexpr | 0.6.0 | license: Apache-2.0/MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| cgl | 0.3.2 | license: MIT / Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| clang-sys | 1.8.1 | license: Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| clipboard-win | 5.4.3 | license: BSL-1.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| combine | 4.6.9 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| core-foundation | 0.10.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| core-foundation | 0.9.4 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| core-foundation-sys | 0.8.7 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| core-graphics | 0.23.2 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| core-graphics-types | 0.1.3 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| crunchy | 0.2.4 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| defmt | 1.1.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| defmt-macros | 1.1.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| defmt-parser | 1.0.0 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| dispatch | 0.2.0 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| dispatch2 | 0.3.1 | license: Zlib OR Apache-2.0 OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| error-code | 3.4.0 | license: BSL-1.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| fax | 0.2.7 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| foreign-types | 0.5.0 | license: MIT/Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| foreign-types-macros | 0.2.4 | license: MIT/Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| foreign-types-shared | 0.3.1 | license: MIT/Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| futures-task | 0.3.34 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| futures-util | 0.3.34 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| glifo | 0.2.0 | license: Apache-2.0 OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| glob | 0.3.3 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| glutin_wgl_sys | 0.6.1 | license: Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| half | 2.7.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| hermit-abi | 0.5.3 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| itertools | 0.13.0 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| jiff-static | 0.2.31 | license: Unlicense OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| jni | 0.22.4 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| jni-macros | 0.22.4 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| jni-sys | 0.3.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| jni-sys | 0.4.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| jni-sys-macros | 0.4.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| jobserver | 0.1.35 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| js-sys | 0.3.103 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| libredox | 0.1.25 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| memoffset | 0.9.1 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| minimal-lexical | 0.2.1 | license: MIT/Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| ndk | 0.9.0 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| ndk-context | 0.1.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| ndk-sys | 0.6.0+11769913 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| nom | 7.1.3 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| num_enum | 0.7.6 | license: BSD-3-Clause OR MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| num_enum_derive | 0.7.6 | license: BSD-3-Clause OR MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc-sys | 0.3.5 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2 | 0.5.3 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2 | 0.6.5 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-app-kit | 0.2.2 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-app-kit | 0.3.2 | license: Zlib OR Apache-2.0 OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-cloud-kit | 0.2.2 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-contacts | 0.2.2 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-core-data | 0.2.2 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-core-foundation | 0.3.2 | license: Zlib OR Apache-2.0 OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-core-graphics | 0.3.2 | license: Zlib OR Apache-2.0 OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-core-image | 0.2.2 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-core-location | 0.2.2 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-encode | 4.1.0 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-foundation | 0.2.2 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-foundation | 0.3.2 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-io-surface | 0.3.2 | license: Zlib OR Apache-2.0 OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-link-presentation | 0.2.2 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-metal | 0.2.2 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-quartz-core | 0.2.2 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-symbols | 0.2.2 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-ui-kit | 0.2.2 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-ui-kit | 0.3.2 | license: Zlib OR Apache-2.0 OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-uniform-type-identifiers | 0.2.2 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| objc2-user-notifications | 0.2.2 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| once_cell_polyfill | 1.70.2 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| orbclient | 0.3.55 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| pin-project | 1.1.13 | license: Apache-2.0 OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| pin-project-internal | 1.1.13 | license: Apache-2.0 OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| plain | 0.2.3 | license: MIT/Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| portable-atomic | 1.15.0 | license: Apache-2.0 OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| portable-atomic-util | 0.2.8 | license: Apache-2.0 OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| prettyplease | 0.2.37 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| quick-error | 2.0.1 | license: MIT/Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| r-efi | 5.3.0 | license: MIT OR Apache-2.0 OR LGPL-2.1-or-later | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| r-efi | 6.0.0 | license: MIT OR Apache-2.0 OR LGPL-2.1-or-later | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| redox_syscall | 0.4.1 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| redox_syscall | 0.5.18 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| redox_syscall | 0.9.5 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| redox_users | 0.5.3 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| rustc-hash | 2.1.3 | license: Apache-2.0 OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| rustc_version | 0.4.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| rustversion | 1.0.23 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| same-file | 1.0.6 | license: Unlicense/MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| schannel | 0.1.30 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| security-framework | 3.7.0 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| security-framework-sys | 2.17.0 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| semver | 1.0.28 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| shlex | 1.3.0 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| simd_cesu8 | 1.2.0 | license: Apache-2.0 OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| simdutf8 | 0.1.5 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| slotmap | 1.1.1 | license: Zlib | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| tempfile | 3.27.0 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| tiff | 0.11.3 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| uds_windows | 1.2.1 | license: MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| walkdir | 2.5.0 | license: Unlicense/MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| wasi | 0.11.1+wasi-snapshot-preview1 | license: Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| wasip2 | 1.0.4+wasi-0.2.12 | license: Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| wasm-bindgen | 0.2.126 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| wasm-bindgen-futures | 0.4.76 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| wasm-bindgen-macro | 0.2.126 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| wasm-bindgen-macro-support | 0.2.126 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| wasm-bindgen-shared | 0.2.126 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| web-sys | 0.3.103 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| weezl | 0.1.12 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| winapi-util | 0.1.11 | license: Unlicense OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows-link | 0.2.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows-sys | 0.52.0 | license: MIT OR Apache-2.0 | declared by a member, optional/off by default: not compiled by the default build | crates.io |
| windows-sys | 0.59.0 | license: MIT OR Apache-2.0 | declared by a member, optional/off by default: not compiled by the default build | crates.io |
| windows-sys | 0.60.2 | license: MIT OR Apache-2.0 | declared by a member, optional/off by default: not compiled by the default build | crates.io |
| windows-sys | 0.61.2 | license: MIT OR Apache-2.0 | declared by a member, optional/off by default: not compiled by the default build | crates.io |
| windows-targets | 0.52.6 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows-targets | 0.53.5 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows_aarch64_gnullvm | 0.52.6 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows_aarch64_gnullvm | 0.53.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows_aarch64_msvc | 0.52.6 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows_aarch64_msvc | 0.53.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows_i686_gnu | 0.52.6 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows_i686_gnu | 0.53.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows_i686_gnullvm | 0.52.6 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows_i686_gnullvm | 0.53.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows_i686_msvc | 0.52.6 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows_i686_msvc | 0.53.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows_x86_64_gnu | 0.52.6 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows_x86_64_gnu | 0.53.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows_x86_64_gnullvm | 0.52.6 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows_x86_64_gnullvm | 0.53.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows_x86_64_msvc | 0.52.6 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| windows_x86_64_msvc | 0.53.1 | license: MIT OR Apache-2.0 | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| wit-bindgen | 0.57.1 | license: Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| zerocopy-derive | 0.8.56 | license: BSD-2-Clause OR Apache-2.0 OR MIT | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
| zlib-rs | 0.6.7 | license: Zlib | locked via another crate's optional or dev edge: not compiled by the default build | crates.io |
<!-- deps:end:locked-unused -->

## Native (non-crate) components

Found at build time with pkg-config (`crates/plex-player/build.rs`,
`crates/plex-cli/build.rs`), or through `MPV_DIR` for libmpv.

| component | role | upstream licence | in the packages |
|---|---|---|---|
| libmpv (`mpv.pc`, client API 2.5 or newer) | playback engine and renderer | GPL-2.0-or-later as built by distributions (LGPL-2.1-or-later only when built without GPL files) | bundled |
| FFmpeg (`libavcodec`, `libavformat`, ...) | demuxing and decoding, through libmpv | LGPL-2.1-or-later; GPL-2.0-or-later or GPL-3.0-or-later depending on the build's configure flags | bundled |
| libplacebo | video rendering, through libmpv | LGPL-2.1-or-later | bundled |
| libass | subtitle rendering, through libmpv | ISC | bundled |
| EGL (`egl.pc`) | GL context on Linux | from the graphics driver | never bundled |
| GL (`gl.pc`) | OpenGL entry points | from the graphics driver | never bundled |
| wayland-client (`wayland-client.pc`, 1.20 or newer) | Wayland display connection | MIT | host library |
| libxkbcommon (`xkbcommon.pc`, 1.0 or newer) | keyboard layouts | MIT | bundled in the AppImage |

`THIRD-PARTY-NOTICES.md` at the repository root lists what each package
actually bundles.

## License notes

* **Chewvy is licensed `GPL-3.0-or-later`** (`Cargo.toml`
  `[workspace.package]`, full text in `LICENSE`). The reasons, checked on
  2026-11-11:
  * mpv "as a whole is licensed under the GNU General Public License GPL
    version 2 or later ... by default", and is LGPL only "if built without
    using any GPL only files"
    (<https://github.com/mpv-player/mpv/blob/master/Copyright>). The libmpv
    builds the packages bundle are GPL builds.
  * The FFmpeg inside the Windows libmpv is configured with `--enable-gpl
    --enable-version3`
    (<https://github.com/shinchiro/mpv-winbuild-cmake/blob/master/packages/ffmpeg.cmake>),
    and Homebrew's `ffmpeg` formula, which the macOS bundle copies from, declares
    `GPL-3.0-or-later` with the same two flags
    (<https://github.com/Homebrew/homebrew-core/blob/HEAD/Formula/f/ffmpeg.rb>).
    Debian's standard FFmpeg build, which the AppImage copies from, is
    configured for "effective licensing of GPLv2+"
    (<https://salsa.debian.org/multimedia-team/ffmpeg/-/blob/debian/master/debian/rules>).
  * Eleven compiled crates are `Apache-2.0` only (`winit`, `glutin`,
    `ab_glyph` and others in Table A). Apache-2.0 is compatible with GPLv3 but
    not with GPLv2, so the linked program can only be conveyed under GPLv3.
  * The Jellyfin icon is CC BY-SA 4.0, which is one-way compatible with GPLv3.
  This is the project's reading of those licences, not legal advice.
* **Crates that offer a copyleft option** (recorded so a later `cargo add`
  cannot hide one):
  * `self_cell` 1.3.0, `Apache-2.0 OR GPL-2.0-only`, compiled as a dependency
    of `epaint`. Taken under **Apache-2.0**.
  * `r-efi` 5.3.0 and 6.0.0, `MIT OR Apache-2.0 OR LGPL-2.1-or-later`, locked
    but not compiled (Table B). Taken under **MIT/Apache-2.0**.
* **Policy enforced by the test**: a third-party crate must offer at least one
  licence option outside the GPL, AGPL, LGPL and SSPL families. A crate that
  offers none is rejected until it is reviewed and named in this section.
  A new dependency is pinned in the workspace manifest, frozen in `Cargo.lock`
  and added to the tables by the generator.
* **Bundled assets** have their own licences, credited in the app under
  Settings > About and in `THIRD-PARTY-NOTICES.md`: the backdrop photo
  (CC BY 3.0), Plus Jakarta Sans (SIL OFL 1.1) and the Jellyfin icon
  (CC BY-SA 4.0). The format, rating and service logos are trademarks of their
  owners, shown only to label a title's format or rating or the service a
  library belongs to.
