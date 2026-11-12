# Mobile port plan: Android, then iOS

Planning document only. Nothing here has been executed on a phone or tablet, and
no mobile statement in it is a measurement: each is either quoted from a primary
source in §9 (all accessed 2026-11-10) or explicitly marked **unconfirmed**. The
access date matters — two of the tooling statements have changed status recently
(MoltenVK's Khronos deprecation notice, and the Rust-Android packaging tooling
`glutin`'s example still documents).

Target: the same client that runs on the Linux desktop, on a phone — sign-in stays the PIN flow (no on-screen keyboard needed),
and playback stays on libmpv with an explicit tested capability matrix rather
than a "plays everything" claim.

## 1. What the desktop build actually is today

Three crates, already cleanly layered:

| crate | job | OS coupling today |
|---|---|---|
| `plex-core` | HTTP client, PIN auth, token storage, library/resources, server discovery | XDG paths, `std::os::unix` file permissions, D-Bus Secret Service |
| `plex-player` | libmpv via FFI: `mpv_render_context` + GL render API (`render_link.rs`), core-ownership pinning (`mpv_link.rs`) | none in Rust; requires a GL context and a `get_proc_address` resolver supplied by the caller |
| `plex-app` | winit 0.30 event loop, glutin EGL display, egui 0.36 + `egui_glow`, shell/watch state machines, mpv options | EGL loader, XDG cache dir, `xdg-open`, `ffmpeg` spawn, desktop window geometry |

The two structural facts that make a mobile port tractable, both already true in
the code:

1. The player does **not** own a window: `player/mod.rs` starts mpv with
   `("vo", "libmpv")` (line 178) and draws through `MPV_RENDER_API_TYPE_OPENGL`
   into an app-owned FBO (`plex-player/src/render_link.rs`, with a documented
   `MPV_RENDER_API_TYPE_SW` last resort). A port therefore needs no `--wid`, no
   `Surface`, and no mpv-side VO at all.
2. `plex-core` has no UI or windowing dependency, and token storage is already
   behind a `TokenStore` trait with two implementations and probe-based
   selection (`plex-core/src/tokens.rs:218`, `:306`, `:458`).

The concrete Linux coupling to be replaced, by file:

| site | what it assumes | mobile consequence |
|---|---|---|
| `plex-core/src/tokens.rs:36-37,145-161` | `~/.config/plex-rust-client` via the `xdg` crate, `0600` via `OpenOptionsExt` | no XDG on either OS; a file store must move to an app data dir (Android) or Keychain (iOS) |
| `plex-core/src/tokens.rs` `SecretServiceStore` | session D-Bus + secrets daemon | absent on Android and iOS; primary store silently unavailable |
| `plex-core/src/client_info.rs:31` + `device_name()`/`platform_version()` (`:279-299`) | `X-Plex-Platform: Linux`; device name from `/proc/sys/kernel/hostname`; version from `/etc/os-release` or `/proc/sys/kernel/osrelease` | neither path is reliable on iOS (no `/proc`), so a phone would report itself as `linux-desktop`/`Linux`; accepted header values for a third-party mobile client are still unverified (see `docs/AUTH_RESEARCH.md` §5) |
| `plex-app/src/ui/art_cache.rs:74`, `ui/settings.rs:10-11`, `shell/connect.rs:11`, `player/memory.rs:8` | XDG cache dir, unix `PermissionsExt` | both need an app-data/cache path instead |
| `plex-app/src/ui/app.rs:717` | `std::process::Command::new("xdg-open")`, error swallowed by `let _ =` | no `xdg-open`, and on iOS no process spawning at all: the sign-in button currently becomes a silent no-op |
| `plex-app/src/main.rs:171` | spawns `ffmpeg` to make a demo fixture | desktop-only affordance; mobile needs a bundled sample or no sample at all |
| `plex-app/src/gl/egl.rs` | dlopen'd libglvnd EGL, `EGL_PLATFORM` env selector (`:692`), surfaceless/PBUFFER probe configs | Android EGL display comes from the NDK, not an env var; iOS has no EGL |
| `plex-app/src/shell/host.rs` | `glutin_winit::DisplayBuilder` + `finalize_window` (`:206`, `:259`) [S8], explicit glutin context/window-surface creation, `window_event` treating `CloseRequested`/`Destroyed` as exit (`:897`) [S5], `Drop` order surface → context → display → window (`:150`) | the shape survives; only the display/config bootstrap changes (and D13 warns against relying on default configs) |

One gap is mobile-specific and worth naming before any porting work, because it
is a defect rather than a platform difference: `HostApp` implements
`ApplicationHandler::resumed` (`shell/host.rs:844`), which rebuilds display,
context, window surface and `egui_glow` painter every time it runs, and the crate
implements **no** `suspended` — winit's `suspended` is a provided method whose
default body does nothing [S4]. On a desktop that is invisible; on Android winit
says that once `Suspended` arrives the `SurfaceView` is destroyed and
"applications must drop all render surfaces before the event callback
completes" [S4]. So the port must add a real `suspended`, and must make `resumed`
safe to call repeatedly: winit recommends handling "redundant (i.e. back-to-back)
`Suspended` or `Resumed` events" gracefully [S4].

## 2. Reuse / adapt / replace

| area | verdict |
|---|---|
| auth (PIN pair/poll, headers, HTTP, redaction), library models, resource state machines, UI screens, shell/watch logic, egui widgets, mpv property/command plumbing, render-context ownership rules | **reuse as-is** |
| GL display bootstrap (`gl/egl.rs`), surface lifecycle, DPI/scale factor, resize handling | **adapt**: one backend per OS behind the existing seam |
| `TokenStore` backends | **replace per OS**: Android Keystore-backed store, iOS Keychain store; keep the file store as a fallback on Android only |
| `X-Plex-Platform` / `Platform-Version` / paths | **replace** with per-target values |
| browser launch (`xdg-open`), `ffmpeg` fixture spawn, KDE Wallet, surfaceless EGL probe, window title/decorations | **drop or reimplement per OS** |
| `winit` + `glutin` + `egui_glow` stack | **Android: keep. iOS: keep for a first spike, then decide (see §4.1)** |

A `PlatformServices` seam (paths + token store + open-URL) is justified because
each of the three already has two real implementations; no further trait or
generic should be added speculatively.

## 3. Android

### 3.1 Toolchain and packaging

`winit`'s Android backend builds on the `android-activity` crate as its "glue"
and re-exports it as `winit::platform::android::activity::*`; the glue version
must match winit's (`winit 0.30` ↔ `android-activity = "0.6"`), which our
`Cargo.lock` already satisfies (android-activity 0.6.1, ndk 0.9) [S1].
Requirements from the same page: a `crate-type = ["cdylib"]` lib target, an
`android_main` entrypoint, and a base-class feature flag —
`android-native-activity` (NativeActivity, "possible to use without compiling
any Java or Kotlin code") or `android-game-activity` (GameActivity, derived from
`AppCompat`, needs a Java/Kotlin-capable build with Maven deps) [S1].

Consequence for this repo: `plex-app` already has `main.rs` *and* `lib.rs`, so
the change is additive — add `cdylib` on Android plus a `#[no_mangle]
android_main` gated by `cfg(target_os = "android")`, and let `main.rs` stay the
desktop entry.

Build/packaging reality: `cargo-ndk` builds the Rust cdylib and drops it into a
Gradle module's `jniLibs/` layout, and does not produce an APK or AAB (Gradle's
job) [S11]. `cargo-apk` is what glutin's own Android example still documents
[S7], but the surrounding Rust-Android tooling is stale — the successor `xbuild`
repo is titled "(unmaintained)" [S13] — so the honest plan is **Gradle +
`cargo-ndk`**, with `cargo-mobile2` (maintained under Tauri, for Tauri's own
needs) as an optional wrapper [S12]. **Unconfirmed:** whether `cargo-mobile2`
still works unchanged with a winit-only (non-Tauri) app; re-read its README
before adopting it.

`android-activity` warns that the native entrypoints are tied to the *Activity*
lifecycle, not the process: an Activity can be created and destroyed repeatedly,
and by default some configuration changes (e.g. device rotation) destroy and
re-create it [S6]. Two concrete decisions follow: declare
`android:configChanges` for orientation/screenSize in `AndroidManifest.xml` (so
rotation does not tear down mpv), and still make surface teardown/recreation
idempotent, because the manifest is a mitigation, not a guarantee.

### 3.2 Window, GL and surface lifecycle

`glutin`'s README states the rule for exactly our stack: create the GL surface
only after winit raises `Resumed` and destroy it again on `Suspended`, and points
at its `android.rs` example, which uses `ContextSurfaceAndroidExt` to build the
surface from the `ANativeWindow` [S7]. winit's `ApplicationHandler` docs are the
stricter contract to build against: on Android "the `Resumed` event is sent when
a new `SurfaceView` has been created", applications "must wait until they have
been `Resumed` before they will be able to create a render surface (such as an
`EGLSurface`…)", must "drop all render surfaces before the event callback
completes" after `Suspended`, and should tolerate back-to-back duplicates [S4].

So the work in `shell/host.rs` is concrete rather than open-ended: keep
`create_display` (glutin `DisplayBuilder` → context → window surface, `:206-281`)
but reach it from a guarded `resumed`, add a `suspended` that drops painter,
surface and context in the existing teardown order, and replace the EGL display
bootstrap in `gl/egl.rs` (dlopen + `EGL_PLATFORM` env + surfaceless/PBUFFER
probing, `:692`) with an Android EGL display plus a window surface from the
`ANativeWindow`. Pick an explicit `EGLConfig` + GLES2/3 spec rather than trusting
defaults — glutin failing when no config matches is a failure the desktop
build already had to turn into a reported error.

Android's own surface contract is the same shape one frame later:
`SurfaceHolder.Callback` gives `surfaceCreated`/`surfaceChanged`/
`surfaceDestroyed`, and the surface is only usable between created and destroyed
[S10]. That matters if we ever hand a `Surface` to mpv (§3.3), not for the
render-API path.

### 3.3 libmpv on Android

There is no importable libmpv artifact: `mpv-android` states plainly that it is
"not a library/module (AAR) you can import into your app" and recommends using
its `MPVLib.kt`, `BaseMPVView.kt`, native code and `buildscripts/` as
inspiration [S21]. Its build scripts are a separate, Gradle-independent step
("the app won't work if you skip this"), supported on Linux and macOS only,
installing SDK+NDK via `download.sh` and building with `buildall.sh`, which
defaults to 32-bit `armv7l` and needs explicit `--arch arm64` / `x86` /
`x86_64` passes [S22]. Debugging is `adb logcat -s mpv` [S22].

Video output decision: we keep `vo=libmpv` + render API, so we do **not** need
`--wid`/`--android-surface-size` (documented for embedding via a `Surface`
[S15]). The cost is in hardware decode. mpv's own hwdec list says `mediacodec`
requires `--vo=gpu --gpu-context=android` or `--vo=mediacodec_embed` (Android
only), while `mediacodec-copy` "copies video back to system RAM" with no stated
VO requirement [S17]. `vo=mediacodec_embed` additionally documents that native
decode/render loses mpv features — subtitles, OSD/OSC, filters [S16] — which for
a Plex client means our subtitle and OSD overlay story would change.

So the realistic Android ladder, each step separately tested:
`hwdec=no` (baseline, must work) → `hwdec=mediacodec-copy` (hardware decode with
a CPU copy; **unconfirmed** whether it engages under `vo=libmpv` — verify on
device) → `hwdec=mediacodec` + `--gpu-context=android` (needs a mpv-side GL
context or `--vo=gpu`, i.e. a different integration shape) →
`--vo=mediacodec_embed` (only if we accept losing subtitles/OSD).

Audio output: current mpv master documents no Android AO at all in
`DOCS/man/ao.rst` — the only mobile entry there is `audiounit` "(iOS only)"
[S23] — yet `audio/out/ao_aaudio.c`, `ao_audiotrack.c` and `ao_opensles.c` exist
in the same tree [S24]. Which AO an Android libmpv build selects, and how it
interacts with audio focus/ducking, is **unconfirmed**; the port must print
`ao` after load rather than assume it.

### 3.4 Token storage

Android Keystore stores *cryptographic keys*, whose material stays
non-exportable, via the `AndroidKeyStore` provider (API 18+) [S9]; it is not an
arbitrary-secret blob store. The practical shape for a Plex token is therefore:
key in Keystore, ciphertext in app-private storage — or a Java-side
encrypted-preferences store reached over JNI. `android-activity` exposes JNI
hooks (its optional `android_on_create` entrypoint runs on the Java main thread
and is documented as a good place to initialise JNI) [S6], which is the seam to
use instead of owning a Java layer. **Unconfirmed:** the name and maintenance
status of any existing Rust keystore crate (candidates exist on crates.io; none
verified here), versus ~150 lines of our own JNI. Keep `FileTokenStore` (its
`std::os::unix` permissions work on Android) as an explicit fallback, and record
which backend was used in `docs/`, as the desktop code already does.

### 3.5 Sign-in without a browser process

The PIN flow is a good mobile fit: the 4-character code is *displayed* and typed
in the browser at `plex.tv/link` (`plex-app/src/watch.rs:597`,
`shell/live.rs:32`), so no text entry is needed to sign in. What must change is
the launch mechanism: replace `Command::new("xdg-open")`
(`ui/app.rs:717`) with an Intent-style call through JNI on Android, and give the
UI a visible error instead of `let _ =` — on mobile a failed launch currently
looks like a dead button. **Unconfirmed:** the exact Intent flags
(`FLAG_ACTIVITY_NEW_TASK` from a non-Activity context) and whether
`android-activity` needs the call marshalled to the UI thread.

### 3.6 Touch, text input, accessibility

egui takes touch through its integration ("Gather input (mouse, touches,
keyboard, screen size…)") and lists Android among `eframe`'s native targets
[S29][S30]; `winit` 0.30 added Android `KeyEvent::text` population and "basic
iOS IME support" via `Window::set_ime_allowed` [S3]. But egui 0.36.0's headline
"Autocomplete, autocorrect and IMEs now work correctly on iOS and android" is
qualified "(on eframe web)" [S31] — i.e. native soft-keyboard quality is
plausible, not promised. Plan consequence: keep the mobile UI working with zero
required text entry (PIN typed in browser, library browsing by tap), and treat
search-filter typing as a feature that may need a native text-field escape
hatch.

Accessibility is weak on mobile and should be stated honestly: egui supports
AccessKit, but AccessKit "currently implements the native accessibility APIs on
Windows and macOS"; other platforms get an experimental built-in screen reader
[S29].

## 4. iOS

### 4.1 The OpenGL decision (the expensive one)

Apple deprecated OpenGL ES and OpenGL on iOS/tvOS starting iOS 12 [S25], and
App Store guideline 2.5.1 requires public APIs only, on the currently shipping
OS, with deprecated APIs being phased out [S26]. Meanwhile MoltenVK's README now
carries a Khronos deprecation notice for MoltenVK itself, while noting that
"MoltenVK remains the recommended option" for iOS [S32]. Read together with
`winit`'s iOS backend (which hands you a UIKit-hosted layer rather than an EGL
context) and `glutin`'s README (which documents Android and never mentions iOS
[S7]), this means: **on iOS there is no low-friction EGL story.** Options:

| option | what it buys | what it costs |
|---|---|---|
| A. Keep OpenGL ES: EAGL/UIKit-created GLES context, existing `egui_glow` + mpv GL render API | smallest diff from desktop; our FBO render design is unchanged | deprecated public API; own the EAGL layer plumbing that glutin does not give us |
| B. Metal for everything: `egui-wgpu` + mpv with the Metal VO/render support (MPVKit's patched mpv) | aligned with Apple's direction; no deprecated API | a non-upstream mpv patch (upstream PR 7857, "not officially supported yet"), and a second GPU stack to debug [S33] |
| C. Split: mpv renders (Metal/native), egui via wgpu/Metal, composited | independence from mpv's GL state | shared-texture or CPU-copy sync between two renderers; most code |
| D. Drop libmpv on iOS for an AVPlayer-style native player | Apple-supported decode/audio stack | a second playback engine: our property/command control loop, subtitles and quality matrix all move into new code |

Recommendation: **A first, as a spike; B as a separate project with its own
capability matrix; D not before the API/platform work is proven.** This keeps
one code path across Android and iOS for as long as possible, and turns the
Metal question into a visible, reversible decision gate rather than a
prerequisite.

### 4.2 Toolchain and packaging

`winit`'s iOS backend requires iOS 8+ and is regularly tested on 9.3; windows
should be created inside `ApplicationHandler::resumed`; the recommended
integration is a static lib with a wrapped exported `main` (`start_winit_app`)
dragged into an Xcode project with a generated header [S2]. Lifecycle mapping,
from the same page: `applicationDidBecomeActive` → `Resumed`,
`applicationWillResignActive` → `Suspended`, `applicationWillTerminate` →
`LoopExiting` — with two warnings that bite a GL app directly: after
`LoopExiting` "every attempt to draw with opengl will result in segfault", and
the app "may not receive the `LoopExiting` event if suspended; it might be
SIGKILL'ed" [S2]. Both map onto rules `host.rs` already encodes (never draw on a
dead surface; tear down the render context before the mpv core,
`render_link.rs`/`mpv_link.rs`), but iOS is where a missed ordering becomes a
crash instead of a warning. winit also guarantees it registers no
`UIApplicationDelegate`, so the app can install its own for
background/foreground work [S2].
Xcode project generation is possible from `Cargo.toml` with `cargo-xcode`
("Make Xcode project files from Cargo projects", 1.11.2) [S14], or by hand;
`cargo-mobile2` offers `cargo apple run`/`open` [S12]. **Unconfirmed:** which of
these survives contact with a signing identity + entitlements workflow in 2026;
budget a day of plumbing either way.

App-store constraints that change design, not just build: 2.5.2 forbids
downloading and executing new code (no runtime-fetched codec pack); 2.5.13 keeps
us inside our own sandbox; 4.2 minimum functionality (a repackaged Plex web page
can be rejected) [S26]. Background audio needs `UIBackgroundModes: audio` and
"must play audible content" [S27]; whether iOS suspends mpv's AO mid-playback
without it is **unconfirmed** and belongs in the first on-device spike.

### 4.3 libmpv on iOS

mpv documents `videotoolbox` hwdec as requiring `--vo=libmpv` "(iOS 9.0 and up)"
and `videotoolbox-copy` as copying to system RAM on "macOS 10.15 or iOS 9.0 and
up" [S17] — both compatible with our existing `vo=libmpv` design, which is the
good news. Audio has exactly one documented mobile AO for Apple: `audiounit`
"(iOS only)" [S23].

The bad news is availability: mpv's own docs describe no iOS libmpv artifact.
`MPVKit` maintains iOS/macOS/tvOS mpv builds with a Metal patch that "is not
officially supported yet" upstream (mpv PR 7857), offers bundles under LGPL-3.0
(or GPL-3.0 with `enable-gpl`), and warns that the project "is only suitable for
learning libmpv", recommending its AetherEngine wrapper for production [S33].
**Unconfirmed:** MPVKit's API stability, and whether a GLES-only mpv build
(option A, no Metal patch) is available there at all. Honest position: MPVKit is
the fastest route to a *spike*; expect to own a `buildscripts`-style
mpv+FFmpeg cross-build for shipping (the Android `buildscripts` are the
working precedent, though Android-specific [S22]).

### 4.4 Token storage (Keychain)

Apple's Keychain services exist for exactly this: "small secrets … stored in an
encrypted database", not limited to passwords [S28]. So iOS gets a
`KeychainTokenStore` implementing the existing `TokenStore` trait, and the
file-store fallback should be disabled-by-default there (plaintext at rest).
**Unconfirmed:** which crate to use (`security-framework` wraps Security
framework APIs and its docs index lists a `passwords` module that "Works on both
iOS and macOS", https://docs.rs/security-framework/latest/security_framework/ ,
accessed 2026-11-10 — its wider keychain surface was not verified here) versus a
thin `objc2`-level call. The file-store path must not silently become the mobile
default.

### 4.5 Sign-in and touch

Same PIN flow, launched by the app delegate / `openURL` path instead of a
process spawn; iOS forbids spawning processes, so `ui/app.rs:717` cannot even
fail gracefully there — it needs an injected opener. Touch works the same way
Android does for egui; text entry has the same caveat as §3.6 (native IME is
"basic" per winit [S3]), and iOS additionally hides the keyboard, so any
text-entry screen needs explicit show/hide handling.

### 4.6 Licensing risk (this is a product decision, not a footnote)

- Chewvy is `GPL-3.0-or-later`. mpv as a whole is GPLv2+, or LGPLv2.1+ if
  built with no GPL-only files (`-Dgpl=false`); that mode disables a documented
  feature list (Linux X11 VO, vdpau/nvdec-ish paths, jack/DVD/…) and mpv's
  `Copyright` warns that "Linked libraries still can affect the final license
  (for example if FFmpeg was built as GPL)" [S19]. FFmpeg is LGPL 2.1+ by
  default but GPL 2+ if configured `--enable-gpl`, and its LGPL checklist says:
  avoid `--enable-gpl`, build shared, distribute source [S18]. mpv-android calls
  itself GPL-2.0-or-later for exactly this combination [S21].
- App Store friction is about GPLv2 §6's "no further restrictions": the FSF
  analysed Apple's terms that way and pushed Apple to accommodate GPL [S20].
  Chewvy is GPL-3.0-or-later and mpv is "2 or later", so the combined work is
  GPLv3, whose §11 was written for this situation — but that is a
  licence-interpretation step, not something to assume. **Unconfirmed** as legal advice: get a review before iOS submission;
  Android (Play or F-Droid) carries none of this pressure.
- Mitigation if the review is uneasy: mpv `-Dgpl=false` + FFmpeg without
  `--enable-gpl`, publish the corresponding source, accept the disabled features
  (mostly Linux-only in that list, so cheap for mobile).

**Never** claim "all codecs work": on mobile the claim is "whatever this
mpv+FFmpeg build and this device's MediaCodec/VideoToolbox actually decode, plus
server transcoding when mpv declines", tracked in the capability matrix
(`docs/CAPABILITIES.md`).

## 5. Code changes, in the order they pay off

1. `plex-core`: per-target `ClientInfo` (`PLATFORM` at `client_info.rs:31`, plus
   `device_name()`/`platform_version()`, which read `/proc` and `/etc/os-release`
   and would report `linux-desktop`/`Linux` on an iPhone), a `Paths` seam
   replacing `xdg` calls (`tokens.rs:145-161`, `client_info.rs:335`,
   `art_cache.rs:74`), and an `OpenUrl` seam replacing
   `Command::new("xdg-open")` (`ui/app.rs:717`) — with the error surfaced in the
   UI.
2. `plex-app`: add `cdylib` to the lib target on Android; add `android_main`;
   move `main.rs`'s `ffmpeg`-fixture path behind a desktop-only flag.
3. `plex-app/src/shell/host.rs`: guard `resumed` so a second call rebuilds
   instead of leaking the first set of GL objects, and add `suspended`, dropping
   painter → surface → context before returning (the same order as the existing
   `Drop`, `:150`).
4. `plex-app/src/gl/`: split the EGL loader into `gl/egl_desktop.rs` and
   `gl/egl_android.rs` (one display source each) rather than adding `cfg`
   branches inside the existing functions; specify the EGL config explicitly.
5. `plex-player`: no API change expected. Add a capability probe that reports
   `hwdec`, `hwdec-current`, `ao` and `gpu-context` after load, so mobile
   sessions are self-documenting in the log (this is the same discipline that
   made the desktop matrix trustworthy).
6. Per-OS `TokenStore`: `KeystoreTokenStore` (Android), `KeychainTokenStore`
   (iOS). Keep `FileTokenStore` selectable for tests, never as the silent
   default.
7. Only for iOS option A: an EAGL/CAEAGLLayer context owner in
   `plex-app/src/gl/`, replacing what glutin would have done.
8. Delete the superseded path in the same change wherever a spike replaces it
   (no desktop EGL code kept "for later" once each platform has its own).

## 6. Phases

| phase | size | content | exit criterion |
|---|---|---|---|
| **A1** cargo-ndk + Gradle skeleton, empty winit window on device | S | cdylib, `android_main`, manifest, one ABI (`arm64-v8a`) | window visible; `Resumed`/`Suspended` observed across a rotation and a home-button round trip |
| **A2** egui on device | S | glutin EGL surface + `egui_glow`, scale factor from winit, real `suspended` implementation | the existing splash/pairing screen renders and takes taps; no surface object outlives a pause; no D13-style config crash |
| **A3** libmpv Android build + silent playback | M | `buildscripts`-derived mpv+FFmpeg for `arm64-v8a`, `vo=libmpv`, `hwdec=no`, a bundled sample file | sample plays (frame + log evidence); ABI match between Rust cdylib and `libmpv.so` verified |
| **A4** audio + real content | M | AO probe and fix, HTTP stream from a real server via `plex-core` | audible playback from a Plex server; `ao` name recorded |
| **A5** sign-in + token persistence | S | Intent-based URL open, Keystore store, `X-Plex-Platform: Android` | PIN sign-in on the phone with the existing polling flow; token survives app restart; no token in logs (existing `tests/redaction.rs` discipline) |
| **A6** library browse + capability matrix | M | real libraries, subtitles, `hwdec` ladder per §3.3 | matrix rows filled with tested/unsupported, no aspirational rows |
| **I1** iOS spike, option A | M | static lib + Xcode project, EGL-free GL context, egui renders | screen renders on device; no GL draw after `LoopExiting` |
| **I2** mpv on iOS + playback | L | MPVKit-style build for spike or own cross-build; `audiounit`; `videotoolbox`/`videotoolbox-copy` | sample plays audibly; `hwdec-current` reported; background-audio decision recorded |
| **I3** sign-in, Keychain, packaging | M | `openURL`, Keychain store, `UIBackgroundModes` decision, App Store metadata | installable build, signed-in, token in Keychain |
| **I4** Metal decision gate | L | evaluate option B (mpv Metal patch + `egui-wgpu`) against the matrix, or record "stay on GLES" | written decision + citation, or the deprecation risk explicitly accepted |

Sizes: S is days, M is weeks, L is its own project. The order is deliberate: Android first proves the
architecture (its ABI/NDK/libmpv-build problem is the same class of problem iOS
has, at lower risk and with no deprecation question), and iOS then inherits a
platform layer that is already real.

## 7. Explicitly not claimed

- Not every codec works: only tested matrix rows, plus server transcoding where
  available.
- No claim that `mediacodec`, `videotoolbox`, `audiounit` or AAudio work with
  *our* `vo=libmpv` build before A3/A4/I2 produce logs.
- No claim that winit's mobile backends drive our glutin+glow stack without
  patching — glutin's Android example proves the pattern, not this app.
- No claim that egui text input is comfortable on a soft keyboard, no claim of a
  store-ready iOS binary before I3, and no legal conclusion before a review.

## 8. Open questions

1. iOS: is the OpenGL ES deprecation acceptable for a first shipping release
   (option A), or is Metal (option B) a precondition? This single answer decides
   whether I4 comes later or first.
2. Licensing for iOS distribution: does GPLv3 (for App Store
   §11 reasons) hold for this workspace + a `-Dgpl=false` mpv + LGPL FFmpeg?
   Needs a reviewer, not a grep.
3. Is Play Store distribution expected to require background audio and
   Picture-in-Picture (both used by mpv-android's feature list [S21]), or is
   foreground-only playback acceptable for v1 on mobile?
4. Which minimum OS versions to support — this sets the ABI/SDK matrix
   (Android) and the deployment target (iOS). Neither is currently constrained
   by this repo.
5. Do we accept owning a cross-compiled mpv+FFmpeg build (Android and iOS), or
   should we prefer a third-party bundle (MPVKit for iOS) and its update cadence?
6. Touch UX scope: is a phone-shaped re-layout (bottom navigation, larger hit
   targets in `plex-app/src/ui/`) in scope for the port, or is "the desktop
   layout, scaled" the accepted v1?

## 9. Sources

All URLs accessed 2026-11-10. In-repo companions: `docs/ARCHITECTURE.md`
(sign-in and layering), `docs/DEPENDENCIES.md` (crate/system library and licence
inventory), `docs/CAPABILITIES.md` (capability matrix).

| ref | source |
|---|---|
| S1 | winit 0.30.13, `winit::platform::android` module docs — https://docs.rs/winit/0.30.13/winit/platform/android/index.html |
| S2 | winit 0.30.13, `winit::platform::ios` module docs — https://docs.rs/winit/0.30.13/winit/platform/ios/index.html |
| S3 | winit 0.30 changelog (iOS basic IME via `set_ime_allowed`, Android `KeyEvent::text`) — https://docs.rs/winit/latest/winit/changelog/v0_30 |
| S4 | winit 0.30.13 `ApplicationHandler` docs (`resumed`/`suspended` contract, Android `SurfaceView`/`EGLSurface` rules, iOS `applicationDidBecomeActive`) — https://docs.rs/winit/0.30.13/winit/application/trait.ApplicationHandler.html |
| S5 | winit `WindowEvent` docs (variants our `window_event` handles: `CloseRequested`, `Destroyed`, `Resized`) — https://docs.rs/winit/0.30.13/winit/event/enum.WindowEvent.html |
| S6 | `android-activity` crate docs (entrypoints, Activity-vs-process lifecycle, JNI on the Java main thread) — https://docs.rs/android-activity/latest/android_activity/index.html |
| S7 | glutin README (Android EGL support, `Resumed`/`Suspended` rule, `android.rs` example, cargo-apk; no iOS section) — https://github.com/rust-windowing/glutin |
| S8 | glutin-winit 0.5.0 `DisplayBuilder` docs (the window+config builder our `host.rs:206` uses) — https://docs.rs/glutin-winit/latest/glutin_winit/struct.DisplayBuilder.html |
| S9 | Android Keystore system (keys, non-exportable key material, `AndroidKeyStore` API 18+) — https://developer.android.com/privacy-and-security/keystore |
| S10 | `SurfaceHolder.Callback` reference (`surfaceCreated`/`Changed`/`Destroyed`) — https://developer.android.com/reference/android/view/SurfaceHolder.Callback |
| S11 | `cargo-ndk` docs (cdylib + jniLibs; APK/AAB left to Gradle) — https://docs.rs/cargo-ndk/latest/cargo_ndk/ |
| S12 | `cargo-mobile2` (`cargo apple`/`cargo android` run/open; maintained under Tauri) — https://github.com/tauri-apps/cargo-mobile2 |
| S13 | `xbuild` repo, titled "Cross compile rust to any platform (unmaintained)" — https://github.com/rust-mobile/xbuild |
| S14 | `cargo-xcode` on crates.io (1.11.2, "Make Xcode project files from Cargo projects") — https://crates.io/crates/cargo-xcode |
| S15 | mpv master `DOCS/man/options.rst` (`--wid` as window handle, `--android-surface-size`) — https://raw.githubusercontent.com/mpv-player/mpv/master/DOCS/man/options.rst |
| S16 | mpv master `DOCS/man/vo.rst` (`mediacodec_embed`, its subtitle/OSD/filters limitation, `--gpu-context=android` alternative) — https://raw.githubusercontent.com/mpv-player/mpv/master/DOCS/man/vo.rst |
| S17 | mpv master `DOCS/man/options.rst` hwdec list (`videotoolbox` with `--vo=libmpv` on iOS 9+, `videotoolbox-copy`, `mediacodec`/`mediacodec-copy`) — same URL as S15 |
| S18 | FFmpeg legal notes (LGPL default, GPL when `--enable-gpl`, LGPL compliance checklist) — https://ffmpeg.org/legal.html |
| S19 | mpv `Copyright` (GPLv2+ default, LGPLv2.1+ via `-Dgpl=false`, disabled feature list, linked-library effect) — https://raw.githubusercontent.com/mpv-player/mpv/master/Copyright |
| S20 | FSF, "More about the App Store GPL Enforcement" (GPLv2 §6 further restrictions vs Apple terms) — https://fsf.org/blogs/licensing/more-about-the-app-store-gpl-enforcement |
| S21 | mpv-android README (not an importable AAR; `MPVLib`/`BaseMPVView`/native/buildscripts as inspiration; GPL-2.0-or-later; PiP/background playback features) — https://github.com/mpv-android/mpv-android |
| S22 | mpv-android `buildscripts/README.md` (separate from Gradle; Linux/macOS hosts only; `download.sh`, `buildall.sh`, default `armv7l`, `--arch arm64/x86/x86_64`; `adb logcat -s mpv`) — https://raw.githubusercontent.com/mpv-android/mpv-android/master/buildscripts/README.md |
| S23 | mpv master `DOCS/man/ao.rst` (`audiounit` "(iOS only)"; no Android AO documented in master) — https://raw.githubusercontent.com/mpv-player/mpv/master/DOCS/man/ao.rst |
| S24 | mpv `audio/out` directory listing in master (`ao_aaudio.c`, `ao_audiotrack.c`, `ao_opensles.c`) — https://github.com/mpv-player/mpv/tree/master/audio/out |
| S25 | Apple `EAGLContext` docs (OpenGL ES/OpenGL deprecated on iOS and tvOS starting iOS 12.0) — https://developer.apple.com/documentation/uikit/eaglcontext |
| S26 | App Store Review Guidelines (2.5.1 public APIs / currently shipping OS, 2.5.2 no downloaded executable code, 2.5.13 no foreign file access, 4.2 minimum functionality, 5.1.1 privacy/permissions) — https://developer.apple.com/app-store/review/guidelines/ |
| S27 | Apple "Playing Background Audio" (`UIBackgroundModes: audio`, must play audible content) — https://developer.apple.com/documentation/watchkit/playing_background_audio |
| S28 | Apple Keychain services overview (small secrets in an encrypted database) — https://developer.apple.com/documentation/security/keychain-services |
| S29 | egui README (touch input contract, `eframe` targets Web/Linux/Mac/Windows/Android, AccessKit native APIs on Windows and macOS) — https://github.com/emilk/egui |
| S30 | eframe README ("natively (for Linux, Mac, Windows, and Android) or as a web app") — https://github.com/emilk/egui/blob/main/crates/eframe/README.md |
| S31 | egui 0.36.0 release notes (mobile keyboard/IME improvements, scoped "on eframe web") — https://github.com/emilk/egui/releases/tag/0.36.0 |
| S32 | MoltenVK README (Khronos deprecation notice; MoltenVK still recommended for iOS) — https://github.com/KhronosGroup/MoltenVK |
| S33 | MPVKit README (iOS/macOS/tvOS mpv builds; Metal patch not officially supported upstream, mpv PR 7857; LGPL-3.0 / GPL-3.0-with-enable-gpl bundles; "only suitable for learning libmpv", AetherEngine suggested) — https://github.com/mpvkit/MPVKit |

mpv render-API contracts used by the existing player code are cited from the
system headers already recorded in `docs/DEPENDENCIES.md`
(`/usr/include/mpv/render.h`, `/usr/include/mpv/render_gl.h`), because current
mpv master no longer ships `DOCS/man/render.rst`.
