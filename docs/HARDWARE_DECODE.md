# Hardware decoding and video output (Linux, Windows, macOS)

The question: **is OpenGL the best setup for hardware-accelerated decoding
of all supported formats on Linux, Windows and macOS?**

Short answer: **for this client's design (libmpv render API, mpv draws into an
app-owned GL FBO, egui on top), OpenGL is the best documented and the most
portable choice as of 2026-11-10 — it is the only mpv render-API backend with
hardware-decoding guidance for all three desktop platforms.** It is not the
feature ceiling: mpv's Vulkan path (`--vo=gpu-next`, `--hwdec=vulkan`) is where
Dolby Vision reshaping and HDR output live, and the render API does not support
gpu-next yet. Everything below distinguishes **H** (verified against a primary
source quoted or named here) from **U** (reasoned, or to be measured on the
real host by `scripts/hwdec-probe.sh`).

Sources were read on **2026-11-10**: mpv `master` on GitHub
(`include/mpv/render.h`, `include/mpv/render_gl.h`, `include/mpv/render_d3d11.h`,
`DOCS/man/options.rst`, `video/out/opengl/`, `video/out/hwdec/`, the
"GPU Next vs GPU" wiki page and the 0.41.0/0.42.0 release notes), FFmpeg
`master` on GitHub (`libavcodec/hwaccels.h`), and libplacebo release notes.
Quoted text is verbatim (including mpv's spelling of "fallback").
`trac.ffmpeg.org/wiki/HWAccelIntro` is a wiki and is cited only as a secondary
pointer where marked.

Current app configuration:
`crates/plex-app/src/player/mod.rs` sets `vo=libmpv`,
`hwdec=auto-safe` (else `no`), `hwdec-extra-frames=3`; the render context is
OpenGL through `GlutinResolver` (WGL/EGL/CGL via
`glutin Display::get_proc_address`).

## a) How hardware decode and GL rendering actually connect

Decoding and rendering are separate stages: `--hwdec` selects the decode API
(vaapi/nvdec/videotoolbox/d3d11va/…), and the selected API's **zero-copy**
mode only works when mpv can import the decoder's surfaces into the renderer's
textures. mpv's own docs state the asymmetry directly
(`DOCS/man/options.rst`, master):

> Most non-copy methods only work with the OpenGL GPU backend. Currently, only
> the `vaapi`, `nvdec`, `cuda` and `vulkan` methods work with Vulkan. [H]

Modes and fallback (same file, master) [H]:

- `<api>` — direct (zero-copy) decode into GPU surfaces.
- `<api>-copy` — decode on GPU, copy frames back to system RAM:
  "Because these copy the decoded video back to system RAM, they're often less
  efficient than the direct modes, and may not help too much over software
  decoding if you are short on CPU resources."
- `auto` / `auto-safe` — cascade: "It will go down the list of available
  hwdecs until one is successfully initialised. If all of them fail, it will
  fallback to software decoding."
- Even after a chosen decoder fails at runtime:
  `--hwdec-software-fallback=<yes|no|N>` "Fallback to software decoding if the
  hardware-accelerated decoder fails (default: 3)."

The render API only has three backends (`include/mpv/render.h`, master) [H]:

> Direct3D 11: via MPV_RENDER_API_TYPE_D3D11, see render_d3d11.h header.
> OpenGL: via MPV_RENDER_API_TYPE_OPENGL, see render_gl.h header.
> Software: via MPV_RENDER_API_TYPE_SW

and it explicitly supports hwdec, with per-platform interop conditions
(`include/mpv/render_gl.h`, master) [H]:

> Hardware decoding via this API is fully supported, but requires some
> additional setup. (At least if direct hardware decoding modes are wanted,
> instead of copying back surface data from GPU to CPU RAM.)
>
> - Windows: ANGLE is required (although in theory GL/DX interop could be used)
> - Intel/Linux: EGL is required, and also the native display resource needs
>   to be provided (e.g. MPV_RENDER_PARAM_X11_DISPLAY for X11 and
>   MPV_RENDER_PARAM_WL_DISPLAY for Wayland)
> - nVidia/Linux: Both GLX and EGL should work (GLX is required if vdpau is
>   used, e.g. due to old drivers.)
> - macOS: CGL is required (CGLGetCurrentContext() returning non-NULL)
> - iOS: EAGL is required (EAGLContext.currentContext returning non-nil)

`render.h` reinforces this for Linux: `MPV_RENDER_PARAM_X11_DISPLAY` is
documented as "X11 Display, sometimes used for hwdec" and
`MPV_RENDER_PARAM_WL_DISPLAY` as "Wayland display, sometimes used for hwdec"
[both H].

**Native display parameter (implemented, confirmed on the development host):**
`RenderContext::create` (`crates/plex-player/src/render_link.rs`) now passes the
window server display behind the window's raw-window-handle 0.6 handle as
`MPV_RENDER_PARAM_WL_DISPLAY` (Wayland) / `MPV_RENDER_PARAM_X11_DISPLAY` (X11)
alongside `API_TYPE`, `OPENGL_INIT_PARAMS` and `ADVANCED_CONTROL`, which
`render_gl.h` requires for Intel/Linux EGL zero-copy hwdec [H requirement, code
fact verified 2026-11-10]. Other display-handle variants (Windows, macOS,
headless SW sessions) pass nothing, as before. The player logs mpv's
`hwdec-current` once per item at info level, so the app's own render path
reports which decoder it picked; on the development host that line reads
`vaapi` (zero-copy EGL dmabuf) under an EGL context and `vulkan-copy` when the
native display is withheld or the context is GLX — see "Host measurements".

The interop mechanisms themselves live in mpv source (file existence = H;
the two content quotes were read in source) [H]:

| decode API → GL | mechanism | mpv source |
| --- | --- | --- |
| vaapi / amdgpu | EGLImage from drm-lease/dmabuf | `video/out/hwdec/dmabuf_interop_gl.c`, `dmabuf_interop.h` |
| nvdec/cuda | CUDA–GL interop | `video/out/hwdec/hwdec_cuda_gl.c` (and `hwdec_cuda_vk.c` for Vulkan) |
| videotoolbox (macOS) | IOSurface mapped into a GL texture via CGL (`CGLIOSurface.h`, `CGLGetCurrentContext()`, requires OpenGL ≥ 3.0 and a live CGL context) | `video/out/hwdec/hwdec_mac_gl.c` |
| d3d11va (under ANGLE) | EGL–D3D11 interop | `video/out/opengl/hwdec_d3d11egl.c` |
| dxva2 (native WGL) | `WGL_NV_DX_interop` (`DXRegisterObjectNV`, needs `MPGL_CAP_DXINTEROP`, D3D9Ex surface) | `video/out/opengl/hwdec_dxva2gldx.c` |
| vdpau | GLX/VDPAU interop | `video/out/opengl/hwdec_vdpau.c` |

## b) Render-API alternatives, per platform

| option | what it is | status for an embedded libmpv client (2026-11-10) |
| --- | --- | --- |
| **OpenGL (render GL)** | our current `vo=libmpv` + `MPV_RENDER_API_TYPE_OPENGL` | only backend with documented hwdec setup for **all three** desktop OSes (quotes in a) [H] |
| **Vulkan (gpu-next)** | mpv's newer renderer built on libplacebo | "There is currently no support for the libmpv / render API in vo_gpu_next." (mpv wiki, GPU Next vs GPU) [H]; `--hwdec=vulkan` "requires `--vo=gpu-next`" (`options.rst`) [H] → not available to us at all today |
| **D3D11 (render_d3d11.h)** | newer render-API backend, renders to a D3D11 texture | Windows-only; header present in master (`© 2024`), so this is recent [H existence; maturity U] |
| **Software (render SW)** | CPU blit to memory surfaces | render.h: "an extremely simple (but slow) renderer"; "certain multimedia job creation measures like HDR may not work properly" [H] — a last resort, and it forfeits GPU decode→present zero-copy |

Per-platform notes:

- **Linux (X11/Wayland, Mesa: Intel/AMD).** EGL path requires the native
  display parameter (render_gl.h, above) [H]. `vaapi` itself:
  "most likely works with Intel and AMD GPUs only. It requires the opengl EGL
  backend if the GPU does not support drm modifiers." (`options.rst`) [H].
  nVidia: `nvdec`/`cuda` via CUDA–GL interop; GLX or EGL both acceptable
  (render_gl.h) [H]. vdpau is legacy (`--gpu-context=x11` or `--vo=vdpau`) [H].
- **Windows.** `options.rst` gates the modern API hard: ":d3d11va: requires
  `--vo=gpu` or `--vo=gpu-next` with `--gpu-context=d3d11` or
  `--gpu-context=angle` (Windows 8+ only)", and dxva2 additionally allows
  `--gpu-context=dxinterop` [H]. Under the render API, render_gl.h says ANGLE
  is required for zero-copy (native WGL would need GL/DX interop "in theory")
  [H]. **Consequence [U until probed]:** our current native-WGL glutin context
  most likely ends up at `d3d11va-copy`/`vaapi-copy` or software, not
  zero-copy — the probe must record `hwdec-current` on a Windows host.
  A glutin/WGL + ANGLE experiment (`gpu-context=angle` over ANGLE's EGL) is
  the route to Windows zero-copy without leaving GL [U].
- **macOS.** render_gl.h requires a current CGL context [H];
  `hwdec_mac_gl.c` is the actual videotoolbox→GL IOSurface bridge and requires
  OpenGL ≥ 3.0 [H content]. Note a documentation wrinkle: `options.rst`
  literally reads ":videotoolbox: requires `--vo=gpu` or `--vo=gpu-next`
  (macOS only), or `--vo=libmpv` (iOS 9.0 and up)" — macOS-via-libmpv is not
  spelled out, even though the mac GL interop module exists [H text; whether
  `videotoolbox` initializes under the macOS render API is U — probe it].
  Apple's OpenGL deprecation is real but CGL/GL still ships and mpv still uses
  it here [U — vendor status page not re-read].
- **Wayland.** Passing `MPV_RENDER_PARAM_WL_DISPLAY` is the documented hwdec
  hook for Wayland (render.h) [H]; the app contract in
  `crates/plex-app/src/lib.rs` already states "WL_DISPLAY is used only for
  hardware decode interop" [code fact].

## c) Codec coverage — what "all supported formats" honestly means

Layer 1 — mpv's own gating of each hwdec API (verbatim requirements,
`options.rst`, master) [H]:

| hwdec | requires | platform |
| --- | --- | --- |
| d3d11va(-copy) | vo=gpu/gpu-next + gpu-context=d3d11 or angle | Windows 8+ |
| dxva2(-copy) | vo=gpu + gpu-context=d3d11/angle/dxinterop | Windows |
| nvdec(-copy) | vo=gpu or gpu-next | any (CUDA) |
| vaapi(-copy) | vo=gpu/gpu-next/vaapi/dmabuf-wayland | Linux (copy: also some-GPU Linux/Windows) |
| vdpau(-copy) | vo=gpu + gpu-context=x11, or vo=vdpau | Linux |
| drm(-copy) | vo=gpu or gpu-next | Linux |
| videotoolbox(-copy) | vo=gpu/gpu-next (macOS) or vo=libmpv (iOS 9+) | macOS/iOS |
| vulkan(-copy) | vo=gpu-next only | any with Vulkan Video |

Layer 2 — which **codec × API** combinations exist in FFmpeg
(`libavcodec/hwaccels.h`, master, read 2026-11-10; every row verbatim from the
`ff_<codec>_<api>_hwaccel` externs) [H]:

| codec | vaapi | nvdec | d3d11va | dxva2 | vdpau | videotoolbox | vulkan |
| --- | :-: | :-: | :-: | :-: | :-: | :-: | :-: |
| h264 | ✓ | ✓ | ✓ (1/2, 12) | ✓ | ✓ | ✓ | ✓ |
| hevc | ✓ | ✓ | ✓ (1/2, 12) | ✓ | ✓ | ✓ | ✓ |
| av1 | ✓ | ✓ | ✓ (12) | ✓ | ✓ | ✓ | ✓ |
| vp9 | ✓ | ✓ | ✓ (12) | ✓ | ✓ | ✓ | ✓ |
| vp8 | ✓ | ✓ | — | — | — | — | — |
| mpeg1/2 | 2 only | ✓ | 2 only | 2 only | ✓ | ✓ | — |
| mpeg4 | ✓ | ✓ | — | — | ✓ | ✓ | — |
| h263 | ✓ | — | — | — | — | ✓ | — |
| vc1 / wmv3 | ✓ | ✓ | ✓ (12) | ✓ | ✓ | — | — |
| mjpeg | ✓ | ✓ | — | — | — | — | — |
| prores | — | — | — | — | — | ✓ | ✓ |
| vvc (H.266) | ✓ | — | — | — | — | — | — |

Honest caveats before claiming "all formats":

1. `hwaccels.h` lists what FFmpeg *can* build; whether the libavcodec in a
   given distro/libmpv bundle actually enabled each one is a build question [U
   — probe: `ffmpeg -hwaccels` + `ffmpeg -decoders`].
2. GPU vintage gates everything: AV1 decode needs roughly Intel Arc / AMD
   RDNA2+ / NVIDIA RTX 30-series and later; 10-bit VP9 (profile 2) and 12-bit
   HEVC are narrower; pre-Turing NVIDIA has no AV1 decode [U — vendor tables
   not re-quoted; probe + capability matrix rows decide].
3. Gaps no GPU can paper over (table above [H]): VP8 has no zero-copy path on
   d3d11/vdpau/videotoolbox/vulkan; MJPEG is nvdec/vaapi only; VC1/WMV3 have
   no VideoToolbox path — exactly the legacy-Plex codecs where
   `-copy`/cascade/server-transcode must remain honest fallbacks.
4. mpv's auto-safe cascade + `--hwdec-software-fallback` guarantee playback,
   not zero-copy [H]. So the project keeps an explicit tested capability
   matrix and a server-transcode fallback, and never claims every codec.

### Host measurements (2026-11-10, development host)

`scripts/hwdec-probe.sh` was run, plus a manual follow-up, on the
development host: AMD Radeon RX 9070 XT, Mesa 27.0.0-devel (radeonsi), mpv 0.41.0,
ffmpeg n9.0.2, **no display** attached during the runs — so `vo=null` only
[U, host-specific, re-run per host]:

| probe | result |
| --- | --- |
| `hwdec=auto-safe`, `vo=null` | h264, hevc (main10), av1 and vp9 1080p clips all report "Using hardware decoding (vulkan-copy)" |
| `hwdec=vaapi-copy`, `vo=null` | all four codecs → `vaapi-copy` |
| `hwdec=vaapi`, `vo=null` | all four codecs → software (as expected: no GL/EGL interop without a VO) |
| `vainfo` VLD entrypoints | H264 ConstrainedBaseline/Main/High, HEVC Main/Main10, VP9 Profile0/Profile2, AV1 Profile0/Profile2 |

This host's VA-API therefore covers the four common codecs (and Vulkan Video
decodes them too — that is what auto-safe picked), but every `null`-VO result is
a `-copy` result by construction.

**The app's own `vo=libmpv` OpenGL path, measured 2026-11-10 on the same host**
(chewvy run headless under gamescope against a local mock server serving
generated 1080p HEVC Main10 and H.264 clips, `hwdec=auto-safe`, reading the
`hwdec-current` info line). The app issues `loadfile` only once the render
context exists: issued straight after `mpv_initialize`, a fast local source
opened first, mpv logged "[vo/libmpv] No render context set" and the item ended
with `MPV_ERROR_NOTHING_TO_PLAY` (-16) [U, host-specific]:

| app session | context | `hwdec-current` |
| --- | --- | --- |
| Wayland/Xwayland under gamescope, window's native `wl_display` passed | EGL | `vaapi` for HEVC Main10 **and** H.264 — zero-copy, mpv logs "Using EGL dmabuf interop via GL_EXT_EGL_image_storage" |
| same, native display withheld (A/B) | EGL | `vulkan-copy` |
| X11/Xwayland with a GLX context | GLX | `vulkan-copy` for both codecs; `vaapi`, `vdpau-gl`, `drmprime` and `vulkan` interop backends all failed to load under GLX |

So on this AMD/Mesa host the native-display parameter is exactly what
switches the real app path from `vulkan-copy` to zero-copy `vaapi`, and an EGL
(now preferred on Linux, `crates/plex-app/src/shell/host.rs`) is what the
X11 session was missing — X11 with EGL is not re-measured yet.
`vo=gpu`, AV1/VP9 through the app, 4K, HDR/Dolby-Vision, NVIDIA, Windows and
macOS remain unmeasured. The HDR *metadata/detection* half was measured the
next day — see the HDR toolchain block below.

**HDR toolchain / detection measurements (2026-11-11, same host, no display
attached).** `scripts/hdr-fixtures.sh` generates five synthetic clips (HDR10,
HLG, DV P8.1, DV P5, HDR10+; 1080p24, 72 frames, deterministic `testsrc2`)
and verifies each with ffprobe, dovi_tool/hdr10plus_tool, and mpv with
`vo=null`; the full verified table is in section f [U, host-specific]:

| probe | result |
| --- | --- |
| toolchain | ffmpeg n9.0.2, x265 4.3, dovi_tool 2.3.4, hdr10plus_tool 1.7.2, mpv 0.41.0, `kwin_wayland --version` → kwin 6.8.2 (Plasma ≥6.4 ships color-management-v1) |
| mpv 0.41 detection, `vo=null`, MP4 | HDR10 line: `Decoder format: … bt.2020-ncl/bt.2020/pq/limited`; HLG: `…/bt.2020/hlg/…`; **both DV clips: `dolbyvision/bt.2020/pq/…`** — mpv reads the DV configuration even without gpu-next; HDR10+ adds nothing visible to this line (its metadata is frame side data only, consumed by gpu-next tone-mapping) |
| x265 4.3 | writes DV P5/8.1 RPUs + config record (`--dolby-vision-profile`, `--dolby-vision-rpu`), but refuses: "Dolby Vision requires VBV settings to enable HRD" unless `--vbv-*` are given |
| dovi_tool 2.3.4 | `generate` builds valid P8.1 and P5 RPUs (P5 via `level2`); `info`/`extract-rpu` accept raw `.hevc`/`.rpu`, reject `.mp4` |
| hdr10plus_tool 1.7.2 | **no metadata generator** (extract/inject/remove only) — the fixture script authors the extract-schema JSON itself, and `inject` requires a `ToolInfo` field; `extract`/`inject` reject `.mp4` |

On-screen HDR *presentation* (PQ reaching a monitor under KWin) is still
unmeasured — these rows settle metadata authoring and mpv's detection, not
display behaviour.

## d) HDR (HDR10, HDR10+, Dolby Vision, HLG) under the render API

- **Tone-mapping ≠ HDR output.** mpv on the OpenGL path tone-maps HDR into the
  destination dynamic range; pushing a real PQ signal to the display (swapchain
  colourspace, compositor/DRM negotiation) is a separate feature concentrated
  in gpu-next.
- Our options already follow the documented defaults: `tone-mapping=auto`,
  `hdr-compute-peak=auto` (`crates/plex-app/src/player/mod.rs`). `options.rst`:
  tone-mapping `auto` "Maps to `bt.2390` when using `--vo=gpu`, and to `spline`
  with `--vo=gpu-next`. (Default)" [H] — so the GL render API gets bt.2390.
- `--hdr-compute-peak` (dynamic per-scene tone-mapping) "Requires compute
  shaders, which is a fairly recent OpenGL feature" [H]. Compute shaders mean
  OpenGL 4.3+ [U common knowledge]; the app requests GL 3.2 core explicitly on
  Windows/macOS and an unversioned context on Linux, so on some hosts
  hdr-compute-peak silently degrades to metadata-only [U — probe: `hwdec`/GL
  version lines].
- **HDR output** is a gpu-next feature: `--target-colorspace-hint` ("This can
  be used for 'HDR passthrough'") is documented with "Requires a supporting
  driver and `--vo=gpu-next`" nearby, and its `*-mode` sibling is marked
  "(Only for `--vo=gpu-next`)" [H]. Under the GL render API mpv renders into
  *our* FBO; the app owns the swapchain, so a true HDR present would be our
  engineering plus platform plumbing [U].
- **Wayland colour management** is moving: mpv 0.41.0 release notes list
  "wayland: support color-management-v1 version 2", "wayland: add support for
  wp-color-representation-v1" and "Improved default color management for
  gpu-next, and handling on Wayland" [H]. A still-reported symptom class: BT.
  2020 content "washed out" on `dmabuf-wayland` (mpv issue 16825) — i.e. even
  in mpv standalone, Wayland HDR depends on a KDE/Hyprland-style
  colour-managed compositor [H issue exists; our experience U].
- **Windows HDR**: the d3d11 context has composition-mode plumbing
  (`--d3d11-composition-size`: "Windows with `--gpu-context=d3d11` and
  `--d3d11-output-mode=composition` only" [H]), i.e. PQ presentation is d3d11-
  centric, not GL-centric [U conclusion].
- **Dolby Vision**: libplacebo integrated libdovi — "add
  `<libplacebo/utils/dolbyvision.h>` for parsing DV RPUs (based on libdovi)"
  (libplacebo v5.264.0 release notes) [H]. mpv's DV support (RPU-driven
  reshaping) is listed under gpu-next's new functionality ("Support for Dolby
  Vision content (reshaping only)" — GPU Next vs GPU wiki [H]); "Dolby Vision
  Profile 7 enhancement layer support in gpu-next passthrough" arrived with
  0.42.0 (which "requires FFmpeg 6.1 or newer and libplacebo 7.387.0 or
  newer") [H]. On the OpenGL/gpu path DV plays via its base layer, but FEL
  (P7 dual-layer) and RPU-driven dynamic metadata are gpu-next territory [U
  reading of the same sources]. Profile 5 (single-layer, no HDR10 base) is the
  risk case worth a matrix row [U].
- **HLG** is just BT.2020+PQ-family tone-mapping handled by the same machinery
  as HDR10 on gpu/GL [U by construction; probe fixture will confirm].

**Stance for the capability matrix:** claim "HDR10/HLG tone-mapped on the GL
path (bt.2390 auto)"; leave "HDR passthrough", "DV P7 FEL", "DV P5",
"HDR on Wayland/KDE" as **not claimed** until measured — consistent with the
project's no-overclaim rule.

## e) Recommendation

1. **Keep `vo=libmpv` + OpenGL + `hwdec=auto-safe` as the default.** For an
   embedded, egui-overlay client on all three OSes, OpenGL is not merely
   "good enough": per mpv's own headers it is the *only* render-API backend
   that documents hardware-decoding interop on Linux, Windows and macOS at all
   (render.h backend list + render_gl.h platform list) [H]. Alternatives today
   would cost either portability (D3D11 backend = Windows only) or hardware
   decoding outright (gpu-next/Vulkan = no render API; SW = no GPU decode
   path, slow).
2. **So "yes, OpenGL — with eyes open".** "All supported formats" is delivered
   by the cascade + software fallback + Plex server transcoding, not by OpenGL
   magic: expect nvdec/cuda (nVidia everywhere), vaapi/EGL-dmabuf (Intel/AMD
   Linux), videotoolbox/IOSurface (Apple silicon), and `-copy` or software
   where zero-copy is undocumented (native-WGL Windows, legacy codecs) [H for
   the doc statements, U for per-host outcome — that is what the probe settles].
3. **Engineering items:**
   - **Native display parameter (implemented, confirmed on the development
     host):** pass `MPV_RENDER_PARAM_X11_DISPLAY` /
     `MPV_RENDER_PARAM_WL_DISPLAY` at render-context creation (required for
     Intel/Linux EGL zero-copy) — `RenderContext::create` takes the window's
     raw display handle now, and the player logs `hwdec-current` once per
     item; on the development host that line flips from `vulkan-copy` to
     `vaapi` exactly when the native display is passed.
   - **Windows:** record `hwdec-current` on a real Windows host under native
     WGL (expected `*‑copy`/software). Only if zero-copy matters later, try
     ANGLE (`gpu-context=angle`) before any other render backend.
   - **macOS:** measure whether `videotoolbox` initializes under the render
     API despite the "(macOS only)" docs wording above.
   - **Matrix rows:** one capability-matrix entry per (OS, GPU family, codec,
     profile) actually played, sourced from `hwdec-current` + probe output;
     keep AV1-on-old-GPU, VP8, VC1/WMV3, MJPEG, DV-P7-FEL, HDR-passthrough as
     explicitly unclaimed until measured.
   - **HDR:** stay tone-mapped (`tone-mapping=auto`) on GL; revisit HDR
     passthrough/DV-reshaping when mpv's render API gains gpu-next/D3D11
     maturity — re-check at each mpv update.
4. **Per-GPU claims need per-host measurement:** every per-GPU statement
   here stays [U] until `scripts/hwdec-probe.sh` has run on that GPU family
   and operating system.

## f) HDR / Dolby Vision output mode (2026-11-11)

**The question.** The GL render API tone-maps HDR into SDR inside our own
window; real HDR output and Dolby Vision RPU reshaping live in gpu-next,
which the render API does not support — "There is currently no support for
the libmpv / render API in `vo_gpu_next`" [H wiki, re-accessed 2026-11-11]
— and `render.h` on master still lists only D3D11/OpenGL/SW (no Vulkan
backend) [H]. An opt-in "HDR / Dolby Vision output" mode must therefore
either hand the window to mpv, accept tone-mapping, or move presentation
into the app.

### Fixtures (`scripts/hdr-fixtures.sh`, run 2026-11-11, exit 0, ~6 s)

Output goes to `target/hdr-fixtures/` (git-ignored; per-fixture full mpv
logs under `target/hdr-fixtures/logs/`). Table as printed by that run:

| fixture | ffprobe colour tags | frame side data | mpv decoder format |
|---|---|---|---|
| hdr10 | hevc,Main 10,yuv420p10le,bt2020nc,smpte2084,bt2020 | Content light level metadata; Mastering display metadata | 1920x1080 yuv420p10 bt.2020-ncl/bt.2020/pq/limited/auto |
| hlg | hevc,Main 10,yuv420p10le,bt2020nc,arib-std-b67,bt2020 | — | 1920x1080 yuv420p10 bt.2020-ncl/bt.2020/hlg/limited/auto |
| dv_p81 | hevc,Main 10,yuv420p10le,bt2020nc,smpte2084,bt2020 | Content light level metadata; Dolby Vision Metadata; Dolby Vision RPU Data; Mastering display metadata | 1920x1080 yuv420p10 dolbyvision/bt.2020/pq/limited/auto |
| dv_p5 | hevc,Main 10,yuv420p10le,bt709,bt709,bt709 | Dolby Vision Metadata; Dolby Vision RPU Data | 1920x1080 yuv420p10 dolbyvision/bt.2020/pq/full/auto |
| hdr10plus | hevc,Main 10,yuv420p10le,bt2020nc,smpte2084,bt2020 | Content light level metadata; HDR Dynamic Metadata SMPTE2094-40 (HDR10+); Mastering display metadata | 1920x1080 yuv420p10 bt.2020-ncl/bt.2020/pq/limited/auto |

(The DV fixtures additionally verify with dovi_tool summaries `Profile: 8`
with L6 1000/400 nits and `Profile: 5`; the HDR10+ stream verifies with
`hdr10plus_tool --verify` → "Dynamic HDR10+ metadata detected.")

What matters for the design:

- mpv 0.41 tags both DV clips `dolbyvision` even with `vo=null`: *detection*
  does not need gpu-next; only *reshaping* does [H wiki + measured line,
  section "Host measurements" 2026-11-11].
- The P5 fixture's DV config record overrides the base layer's VUI tags in
  mpv's view (`bt.2020/pq/full` where the stream VUI says bt709). Real P5
  masters encode pixels in a proprietary IPT-like space (IPTPQc2) with no
  HDR10 fallback — the green/purple shift non-DV players show — so the
  synthetic BT.709 base exercises metadata and profile plumbing, not the
  colour shift [S: mpv issue 10700; Plex forum].
- HDR10+ arrives as per-frame `HDR Dynamic Metadata SMPTE2094-40 (HDR10+)`
  side data in an MP4; gpu-next consumes it (`--tone-mapping=st2094-10/40`,
  dynamic tone-mapping) [H wiki]; the GL renderer does not [H wiki].
- P8.x plays as plain HDR10 on non-DV renderers because its base layer is
  HDR10-compatible (`DOVIBLCompatID` in PMS metadata names that
  compatibility) [S]; the fixture reproduces the HDR10-equivalent tag set.

### Design options

**A — hand playback to an mpv-owned window (`vo=gpu-next`).** Keep the
libmpv core in-process (all `mpv_*` control stays as today — no second
process, no IPC), but do not create a render context and let mpv create and
own its window. This is mpv's documented "older way": `client.h` — "set it
as `wid` option. This works on X11, win32, and macOS only" — while
`options.rst` documents `--wid` for X11/win32/Android; Wayland has no
embedding, so there the mpv window is a separate top-level (our dev
host's session) [H]. Gains the full gpu-next set: DV reshaping, HDR10+
curves, and `--target-colorspace-hint` HDR passthrough ("In `auto` mode,
the target colorspace is only set if the current display parameters are
known. Currently, this is supported on Wayland, D3D11 and winvk contexts …
Requires a supporting driver and `--vo=gpu-next`"), with mpv itself doing
the Wayland color-management-v1 surface tagging (source 7). What must be
re-solved: the egui overlay cannot composite over an mpv-owned window, and
the libmpv profile disables the built-in UI/input by default — measured on
the development host, `mpv --show-profile=libmpv` prints `osc=no`,
`input-default-bindings=no`, `input-vo-keyboard=no`; `client.h` names
`load-scripts`/`input-conf` among the options libmpv ignores at
`mpv_initialize()` [H] — so controls either stay in our app window (which
remains the browse/control surface) or we re-enable `osc` + VO keyboard
before init. Untested here [U]: gpu-next as the VO of a live libmpv handle
(only the *render API* is documented as unsupported), and switching `vo`
between modes without restarting the handle.

**B — stay in the render API, tone-map only (status quo).** Today:
HDR10/HLG tone-mapped (bt.2390 on GL), DV P8.x via its HDR10 base layer,
no HDR output, DV P5 colours unreliable. Zero new plumbing; the honest
default and the fallback for A.

**C — HDR/DV inside the render API.** Three sub-variants, none free:

- **C1 OpenGL render API + app-side colour management.** The app owns PQ
  presentation: 10-bit-capable FBO/swapchain, color-management-v1 tagging
  of *our* surface (protocol merged upstream [H]; KWin shipped it in Plasma
  6.4 [S]; the development host runs kwin 6.8.2), while DV reshaping remains absent
  because mpv's gpu renderer has no libdovi path [H wiki] and mpv sets no
  swapchain colourspace for the GL render API (`--target-colorspace-hint`
  is gpu-next-only [H]).
- **C2 D3D11 render API (Windows only).** A supported backend [H render.h],
  and Windows HDR presentation is d3d11/DXGI-advanced-colour-centric [H
  Microsoft]; `--d3d11-output-mode=composition` + `display-swapchain` are
  the documented WinUI embedding route [H]. Still the gpu renderer → no DV
  reshaping.
- **C3 link libplacebo directly and render ourselves.** DV reshaping,
  HDR10+ and output hints all live in libplacebo (source 10), but the
  app then owns decode→display end to end (frame delivery, OSD, subtitles,
  cache) — a new renderer subsystem, not a mode flag [U].

### Auto-detection from Plex metadata

PMS exposes per-video-stream attributes on `<Stream>` that third parties
parse today — `DOVIPresent`, `DOVIProfile`, `DOVIRPUPresent`,
`DOVIBLPresent`, `DOVIELPresent`, `DOVIBLCompatID`, `DOVILevel`,
`DOVIVersion`, beside `colorPrimaries`/`colorTrc`/`colorSpace`/`bitDepth`
[secondary: python-plexapi parses `DOVIPresent`/`DOVIProfile`; PMS XML
snippets carrying the full set appear in Plex forum captures]. Plex
publishes no formal spec for these names — exact spelling/versioning is still to be
confirmed against real servers [U]. Proposed rule, matching the fixture taxonomy:

- HDR/DV candidate ⇒ `DOVIPresent=1` OR `colorTrc ∈ {smpte2084,
  arib-std-b67}` OR `colorPrimaries=bt2020`.
- DV specifics ⇒ `DOVIProfile` (8 + `DOVIRPUPresent=1` = P8.x with
  HDR10-compatible base; 5 = IPT-only, no HDR10 fallback; 7 = dual-layer —
  `DOVIELPresent=1`, EL passthrough needs mpv ≥0.42 gpu-next [H releases]).
- No DV fields and `colorTrc=bt709` ⇒ SDR: never enter the mode.

**Status:** option A is what the app ships as its HDR window
(`crates/plex-app/src/player/hdr.rs`, Settings > HDR & Dolby Vision). HDR
reaching a display through it has not been measured on Windows or macOS.

### Recommendation (phased, honest)

1. **Default stays B.** One user-facing opt-in setting ("HDR / Dolby Vision
   output", off by default); per-item auto-detection above only *enables*
   the opt-in, it never forces a mode.
2. **First implementation of the opt-in is A on Wayland/KDE**: an mpv-owned
   top-level window (`vo=gpu-next`, `--target-colorspace-hint=auto`,
   `--hwdec` unchanged), driven by the existing in-process libmpv handle,
   while our app window keeps browse/control UI. That is exactly the
   configuration mpv documents for HDR output on Wayland, and the development
   host (Plasma/KWin 6.8) is the test bed. X11/Xwayland can embed via `--wid`
   later [H]; Windows (C2) and macOS stay on B until a real host measures
   them.
3. **Honest claims:** "HDR10/HLG/DV-P8 playback, tone-mapped by default;
   opt-in HDR output on colour-managed Wayland compositors; DV reshaping
   (P8/P5) requires gpu-next mode; tone-mapped DV P5 is *labelled*, not
   colour-correct; DV P7 FEL needs mpv ≥0.42." Everything else stays
   unclaimed until fixture-backed probes run on the real desktop.
4. **Re-check at every mpv update** whether the render API gained gpu-next
   — if/when it does, A collapses back into B and the window hand-off
   disappears.

## Sources (all accessed 2026-11-10)

1. mpv master, `include/mpv/render.h` — render backends, X11/WL display params,
   SW renderer caveats.
   https://raw.githubusercontent.com/mpv-player/mpv/master/include/mpv/render.h
2. mpv master, `include/mpv/render_gl.h` — "Hardware decoding" section,
   per-platform requirements.
   https://raw.githubusercontent.com/mpv-player/mpv/master/include/mpv/render_gl.h
3. mpv master, `include/mpv/render_d3d11.h` — D3D11 render backend (© 2024).
   https://raw.githubusercontent.com/mpv-player/mpv/master/include/mpv/render_d3d11.h
4. mpv master, `DOCS/man/options.rst` — `--hwdec` API list/requirements,
   auto cascade, `-copy` notes, `--hwdec-software-fallback`,
   `--tone-mapping`, `--hdr-compute-peak`, `--target-colorspace-hint*`,
   `--d3d11-composition-size`.
   https://raw.githubusercontent.com/mpv-player/mpv/master/DOCS/man/options.rst
5. mpv master, `video/out/opengl/` and `video/out/hwdec/` — interop modules
   (`hwdec_mac_gl.c`, `hwdec_dxva2gldx.c`, `hwdec_cuda_gl.c`,
   `dmabuf_interop_gl.c`, `hwdec_d3d11egl.c`, `hwdec_vdpau.c`).
   https://github.com/mpv-player/mpv/tree/master/video/out/hwdec
6. mpv wiki, "GPU Next vs GPU" — no libmpv/render-API support in vo_gpu_next;
   DV reshaping, `--hwdec=vulkan` in gpu-next.
   https://github.com/mpv-player/mpv/wiki/GPU-Next-vs-GPU
7. mpv releases — v0.41.0 (Wayland color-management-v1 v2,
   wp-color-representation-v1) and v0.42.0 (DV P7 EL passthrough in gpu-next;
   requires FFmpeg 6.1+, libplacebo 7.387.0+).
   https://github.com/mpv-player/mpv/releases
8. mpv issue 16825 — BT.2020 washed out on dmabuf-wayland (symptom class).
   https://github.com/mpv-player/mpv/issues/16825
9. FFmpeg master, `libavcodec/hwaccels.h` — codec × hwaccel matrix reproduced
   in section c.
   https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/libavcodec/hwaccels.h
10. libplacebo v5.264.0 release notes — libdovi-based DV RPU parsing.
    https://code.videolan.org/videolan/libplacebo/-/tags/v5.264.0
11. (secondary) FFmpeg wiki HWAccelIntro — API/platform overview.
    https://trac.ffmpeg.org/wiki/HWAccelIntro

### HDR sources (all accessed 2026-11-11)

12. mpv master, `DOCS/man/options.rst` (re-read) — `--wid` platform list
    (X11/win32/Android; no Wayland entry), `--force-window`, `--osc`,
    `--target-colorspace-hint=<auto|yes|no>` and `--target-colorspace-hint-mode`
    ("Requires … `--vo=gpu-next`"; `source-dynamic` "does not send full
    HDR10+ or Dolby Vision metadata"), `--d3d11-output-mode`.
    https://raw.githubusercontent.com/mpv-player/mpv/master/DOCS/man/options.rst
13. mpv master, `include/mpv/client.h` — embedding the native mpv window
    via `wid` "works on X11, win32, and macOS only"; libmpv init-time
    differences (`load-scripts`, `input-conf`); `mpv --show-profile=libmpv`
    measured locally prints `osc=no`/`input-vo-keyboard=no`.
    https://raw.githubusercontent.com/mpv-player/mpv/master/include/mpv/client.h
14. mpv wiki, "GPU Next vs GPU" (re-read) — "There is currently no support
    for the libmpv / render API in `vo_gpu_next`"; DV reshaping,
    `--target-colorspace-hint`, `--tone-mapping=st2094-10/40` gpu-next-only.
    https://github.com/mpv-player/mpv/wiki/GPU-Next-vs-GPU
15. mpv master, `include/mpv/render.h` (re-read) — backends remain
    D3D11/OpenGL/SW; no Vulkan render API on master. Releases: latest tag
    v0.42.0 (host here runs 0.41.0).
    v0.42.0 (released 2026-11-05) lists "Dolby Vision Profile 7 enhancement
    layer support in `gpu-next`" and requires libplacebo 7.387.0, the release
    that added P7 FEL merging (checked 2026-11-11).
    https://raw.githubusercontent.com/mpv-player/mpv/master/include/mpv/render.h
    https://github.com/mpv-player/mpv/releases
16. wayland-protocols color-management-v1 spec page — the colour-management
    protocol (merged upstream in 2025); KWin shipped it in Plasma 6.4
    (secondary shipping timeline: https://youngju.dev/blog/2026-07-17-wayland-color-management-hdr-shipping-timeline-xserver.en).
    https://wayland.app/protocols/color-management-v1
17. Microsoft, "Use DirectX with Advanced Color on high/standard dynamic
    range displays" — DXGI advanced-colour/DXGI 1.5 composition behind
    Windows HDR presentation.
    https://learn.microsoft.com/en-us/windows/win32/direct3darticles/high-dynamic-range
18. (secondary) mpv issue 10700 + Plex forum thread on DV P5 → 8.1 —
    DV P5's base layer is a proprietary IPT-like space (IPTPQc2) misread as
    BT.2020/PQ by non-DV players (green/purple shift).
    https://github.com/mpv-player/mpv/issues/10700
    https://forums.plex.tv/t/dolby-video-profile-5-to-8-1-mkv-conversion-script-get-rid-of-the-green-purple/936976
19. (secondary) python-plexapi `plexapi.media` source — parses PMS
    `DOVIPresent`/`DOVIProfile`; full `DOVIBLCompatID`/`DOVIBLPresent`/
    `DOVIELPresent`/`DOVILevel`/`DOVIRPUPresent`/`DOVIVersion` spellings
    from PMS XML captures on Plex forums (e.g.
    https://forums.plex.tv/t/4k-dovi-atmos-direct-play-buffering-nvidia-shield/801841).
    https://python-plexapi.readthedocs.io/en/latest/_modules/plexapi/media.html
20. Fixture tool references — dovi_tool (RPU generator/inspector; repo now
    under quietvoid) and hdr10plus_tool 1.7.2 (extract/inject only; inject
    consumes the extract-schema JSON, `ToolInfo` field required; rejects
    MP4 input).
    https://github.com/quietvoid/dovi_tool
    https://github.com/quietvoid/hdr10plus_tool
