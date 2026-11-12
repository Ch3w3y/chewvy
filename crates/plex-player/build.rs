//! libmpv readiness guard + committed-bindings stamp.
//!
//! Two responsibilities, in order:
//!
//! 1. **Version guard.** libmpv must be at least 2.5.0 — as reported by
//!    `pkg-config`, or by `MPV_CLIENT_API_VERSION` in `client.h` when `MPV_DIR`
//!    names an unpacked dev package — or this build script panics, so the
//!    player crate cannot be built against an older/absent libmpv. `MPV_DIR` is
//!    consulted first because the Windows/macOS packages ship `include/mpv/*.h`
//!    plus an import library but no `.pc` file to probe (`PLEX_DISABLE_MPV_GUARD=1`
//!    skips the whole guard). The probe also emits the `-lmpv` link directives
//!    used by the display-free probe session (`src/probe.rs`) and the tests.
//! 2. **Bindings.** The FFI is bindgen 0.72.1 output over the installed headers,
//!    *committed* at `src/mpv_bindings.rs` so the routine `build/check/test`
//!    gates need neither bindgen nor libclang. Regenerate explicitly with:
//!
//!    ```sh
//!    source scripts/env.sh
//!    cargo build --offline --locked --workspace \
//!        --features plex-player/regenerate-mpv-bindings
//!    ```
//!
//!    With that feature enabled bindgen is a real build-dependency and its
//!    failure is fatal (an operator asked for regeneration). Without it the
//!    committed snapshot is used and bindgen is **not compiled at all** - the
//!    bindgen code below is `#[cfg(feature = "regenerate-mpv-bindings")]`, so
//!    the plain gate invocations compile this script with no bindgen in scope.
//!    `PLEX_REGENERATE_BINDINGS=1` without that feature is reported as a
//!    warning, never as a silent regeneration.
//!
//! Machine-readable stamps emitted for every build (consumed by
//! `plex_cli::smoke` and the player probe report): `PLEX_MPV_VERSION`,
//! `PLEX_MPV_PKGCONFIG`, `PLEX_MPV_INCLUDES`, `PLEX_MPV_LINK_PATHS`,
//! `PLEX_MPV_BINDINGS`, `PLEX_MPV_BINDGEN_VERSION`, `PLEX_MPV_FFI_FNS`,
//! `PLEX_MPV_FFI_CONSTS`, `PLEX_MPV_REGEN_FEATURE`, `PLEX_MPV_FFI_FEATURE`.

use std::env;
use std::path::{Path, PathBuf};

/// Researched minimum libmpv; also the floor enforced on `MPV_DIR`.
const MIN_MPV_VERSION: &str = "2.5.0";
const MPV_PKG: &str = "mpv";
/// Names a linker resolves for `-lmpv` once the search directory is on its
/// path: a MinGW import library (`libmpv.dll.a`, first in GNU ld's `-l` search
/// order), an MSVC import library, or the shared object of a package that ships
/// no import library.
const MPV_LIB_NAMES: &[&str] = &[
    "libmpv.dll.a",
    "mpv.dll.a",
    "mpv.lib",
    "libmpv.lib",
    "libmpv.so",
    "libmpv.dylib",
    "libmpv.a",
];
/// Committed bindgen output (relative to this crate).
const COMMITTED_BINDINGS: &str = "src/mpv_bindings.rs";
/// Translation unit handed to bindgen.
const FFI_HEADER: &str = "src/mpv_ffi.h";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={FFI_HEADER}");
    println!("cargo:rerun-if-changed={COMMITTED_BINDINGS}");
    println!("cargo:rerun-if-env-changed=PLEX_DISABLE_MPV_GUARD");
    println!("cargo:rerun-if-env-changed=PLEX_REGENERATE_BINDINGS");
    println!("cargo:rerun-if-env-changed=LIBCLANG_PATH");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_LIBMPV_FFI");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_REGENERATE_MPV_BINDINGS");
    println!("cargo:rerun-if-env-changed=MPV_DIR");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    emit_pkg_config_reruns();
    println!("cargo:rerun-if-env-changed=HOST_PKG_CONFIG_PATH");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_ALL_STATIC");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_ALLOW_SYSTEM_CFLAGS");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_ALLOW_SYSTEM_LIBS");

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into()));

    // ---- 1. libmpv guard (MPV_DIR, else pkg-config) -----------------------
    if env::var_os("PLEX_DISABLE_MPV_GUARD").is_some() {
        println!(
            "cargo:warning=libmpv guard skipped via PLEX_DISABLE_MPV_GUARD: the real \
             player needs libmpv {} or newer plus its render+client headers",
            MIN_MPV_VERSION
        );
        emit("PLEX_MPV_VERSION", "guard-disabled");
        emit("PLEX_MPV_PKGCONFIG", "skipped");
        emit("PLEX_MPV_INCLUDES", "");
        bindings_status(&manifest, Vec::new());
        return;
    }

    let includes = match env::var_os("MPV_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => probe_mpv_dir(Path::new(&dir)),
        None => probe_pkg_config(),
    };
    bindings_status(&manifest, includes);
}

/// Target-specific `PKG_CONFIG_PATH` spellings understood by the `pkg-config`
/// crate (plain target and target with `-` replaced by `_`).
fn emit_pkg_config_reruns() {
    if let Ok(target) = env::var("TARGET") {
        println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH_{target}");
        println!(
            "cargo:rerun-if-env-changed=PKG_CONFIG_PATH_{}",
            target.replace('-', "_")
        );
    }
}

/// pkg-config probe: the normal Linux/Homebrew discovery path.
///
/// Returns the include directories to hand to bindgen.
fn probe_pkg_config() -> Vec<String> {
    match pkg_config::Config::new()
        .atleast_version(MIN_MPV_VERSION)
        .probe(MPV_PKG)
    {
        Ok(lib) => {
            // include_paths is empty when pkg-config reports no -I flags, which
            // is the case for the distro mpv.pc: the headers then come from the
            // compiler's default search path.
            emit("PLEX_MPV_VERSION", &lib.version);
            emit("PLEX_MPV_PKGCONFIG", "ok");
            emit("PLEX_MPV_INCLUDES", &paths(&lib.include_paths));
            if !lib.link_paths.is_empty() {
                emit("PLEX_MPV_LINK_PATHS", &paths(&lib.link_paths));
            }
            resolve_includes(&lib.include_paths)
        }
        Err(err) => panic!(
            "libmpv {MIN_MPV_VERSION} or newer is required for crates/plex-player \
             (pkg-config probe of '{MPV_PKG}' failed: {err}). Install the libmpv \
             development package, point MPV_DIR at an unpacked libmpv dev package, \
             or set PLEX_DISABLE_MPV_GUARD=1 to build the display-free skeleton on a \
             machine without it.",
        ),
    }
}

/// `MPV_DIR` probe for a libmpv dev package laid out as
/// `include/mpv/*.h` + import library (optionally under `lib/`), which is what
/// the shinchiro Windows builds and the equivalent macOS/Linux tarballs ship.
///
/// Returns the include directory to hand to bindgen.
fn probe_mpv_dir(dir: &Path) -> Vec<String> {
    let include = [dir.join("include"), dir.to_path_buf()]
        .into_iter()
        .find(|cand| cand.join("mpv/client.h").is_file())
        .unwrap_or_else(|| {
            panic!(
                "MPV_DIR={} has no mpv/client.h (checked {} and {}): point it at an \
                 unpacked libmpv development package, e.g. the shinchiro \
                 mpv-dev-x86_64-*.7z Windows build.",
                dir.display(),
                dir.join("include").display(),
                dir.display(),
            )
        });
    let header = include.join("mpv/client.h");
    let text = std::fs::read_to_string(&header)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", header.display()));
    let Some((major, minor)) = parse_client_api_version(&text) else {
        panic!(
            "no usable `#define MPV_CLIENT_API_VERSION MPV_MAKE_VERSION(major, minor)` \
             in {}: an unexpected client.h layout, or not a libmpv header.",
            header.display()
        );
    };
    if (major, minor) < min_client_api() {
        panic!(
            "libmpv {MIN_MPV_VERSION} or newer is required for crates/plex-player \
             (MPV_DIR={} reports client API {major}.{minor}). Install a newer \
             libmpv development package, or set PLEX_DISABLE_MPV_GUARD=1 to build \
             the display-free skeleton without it.",
            dir.display(),
        );
    }
    let link_dir = [dir.to_path_buf(), dir.join("lib")]
        .into_iter()
        .find(|cand| MPV_LIB_NAMES.iter().any(|name| cand.join(name).is_file()))
        .unwrap_or_else(|| {
            panic!(
                "MPV_DIR={} has no libmpv import library (checked {} and {} for one \
                 of {}): the package must provide what the linker resolves for \
                 -lmpv, e.g. libmpv.dll.a (MinGW) or mpv.lib (MSVC).",
                dir.display(),
                dir.display(),
                dir.join("lib").display(),
                MPV_LIB_NAMES.join(", "),
            )
        });
    println!("cargo:rustc-link-search=native={}", link_dir.display());
    println!("cargo:rustc-link-lib=dylib=mpv");
    emit("PLEX_MPV_VERSION", &format!("{major}.{minor}.0"));
    emit("PLEX_MPV_PKGCONFIG", "MPV_DIR");
    emit("PLEX_MPV_INCLUDES", &include.display().to_string());
    emit("PLEX_MPV_LINK_PATHS", &link_dir.display().to_string());
    vec![include.display().to_string()]
}

/// `MIN_MPV_VERSION` as the `(major, minor)` pair that `mpv/client.h` reports.
fn min_client_api() -> (u32, u32) {
    let mut parts = MIN_MPV_VERSION
        .split('.')
        .map(|part| part.trim().parse::<u32>().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

/// `(major, minor)` read off the
/// `#define MPV_CLIENT_API_VERSION MPV_MAKE_VERSION(major, minor)` line of a
/// libmpv `client.h`, tolerating spacing and trailing comments.
fn parse_client_api_version(text: &str) -> Option<(u32, u32)> {
    for line in text.lines() {
        let Some(rest) = line
            .trim()
            .strip_prefix('#')
            .and_then(|line| line.trim().strip_prefix("define"))
            .map(str::trim)
            .and_then(|line| line.strip_prefix("MPV_CLIENT_API_VERSION"))
        else {
            continue;
        };
        // Skip longer neighbours such as a hypothetical MPV_CLIENT_API_VERSION_X.
        if rest.chars().next().is_some_and(|c| !c.is_whitespace()) {
            continue;
        }
        let Some((_, args)) = rest.split_once('(') else {
            continue;
        };
        // Drop any trailing `) /* comment */` before splitting the arguments.
        let args = args.split(')').next().unwrap_or_default();
        let Some((major, minor)) = args.split_once(',') else {
            continue;
        };
        if let (Ok(major), Ok(minor)) = (major.trim().parse(), minor.trim().parse()) {
            return Some((major, minor));
        }
    }
    None
}

fn emit(key: &str, value: &str) {
    println!("cargo:rustc-env={key}={value}");
}

/// `:`-joined display of pkg-config reported paths (PathBuf is not `Borrow<str>`).
fn paths(items: &[PathBuf]) -> String {
    items
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<String>>()
        .join(":")
}

/// Include directories handed to bindgen: whatever pkg-config reported, plus
/// the default system locations that mpv.pc relies on (`/opt/homebrew/include`
/// is Apple Silicon Homebrew, `/usr/local/include` Intel Homebrew and the usual
/// fallback on other Unixes).
fn resolve_includes(from_pkg: &[PathBuf]) -> Vec<String> {
    let mut dirs: Vec<String> = from_pkg.iter().map(|p| p.display().to_string()).collect();
    for fallback in [
        "/usr/include",
        "/usr/local/include",
        "/opt/homebrew/include",
    ] {
        if Path::new(fallback).join("mpv/client.h").is_file() && !dirs.iter().any(|d| d == fallback)
        {
            dirs.push(fallback.to_string());
        }
    }
    dirs
}

fn bindings_status(manifest: &Path, includes: Vec<String>) {
    let committed = manifest.join(COMMITTED_BINDINGS);
    emit(
        "PLEX_MPV_REGEN_FEATURE",
        if cfg!(feature = "regenerate-mpv-bindings") {
            "on"
        } else {
            "off"
        },
    );
    emit(
        "PLEX_MPV_FFI_FEATURE",
        if cfg!(feature = "libmpv-ffi") {
            "on"
        } else {
            "off"
        },
    );

    let asked_for_regen = env::var_os("PLEX_REGENERATE_BINDINGS").is_some();
    if asked_for_regen && !cfg!(feature = "regenerate-mpv-bindings") {
        println!(
            "cargo:warning=PLEX_REGENERATE_BINDINGS is set but the \
             'regenerate-mpv-bindings' feature is off, so bindgen is not in the \
             build graph: {COMMITTED_BINDINGS} is used as committed, not regenerated"
        );
    }

    if cfg!(feature = "regenerate-mpv-bindings") {
        regenerate(manifest, &includes);
    }
    stamp(&committed);
}

/// bindgen is only compiled/used when the regeneration feature is on.
#[cfg(feature = "regenerate-mpv-bindings")]
fn regenerate(manifest: &Path, includes: &[String]) {
    let header = manifest.join(FFI_HEADER);
    let builder = bindgen::Builder::default()
        .header(header.to_str().unwrap_or(FFI_HEADER).to_string())
        .allowlist_type("mpv_.*")
        .allowlist_var("MPV_.*")
        .allowlist_function("mpv_.*")
        .derive_debug(true)
        .impl_debug(true)
        .derive_partialeq(true)
        .layout_tests(false)
        .default_enum_style(bindgen::EnumVariation::Consts);
    let builder = includes
        .iter()
        .fold(builder, |b, dir| b.clang_arg(format!("-I{dir}")));

    match builder.generate() {
        Ok(bindings) => {
            let text = bindings.to_string();
            let committed = manifest.join(COMMITTED_BINDINGS);
            std::fs::write(&committed, &text).unwrap_or_else(|err| {
                panic!(
                    "regenerate-mpv-bindings: cannot write {}: {err}",
                    committed.display()
                )
            });
            println!(
                "cargo:warning=regenerated {} from the installed libmpv headers",
                COMMITTED_BINDINGS
            );
        }
        Err(err) => panic!(
            "regenerate-mpv-bindings is enabled but bindgen could not generate \
             libmpv bindings from {FFI_HEADER}: {err} (install libclang, or build \
             without that feature to use the committed snapshot)"
        ),
    }
}

/// No bindgen in this build at all: the committed snapshot is the FFI.
#[cfg(not(feature = "regenerate-mpv-bindings"))]
fn regenerate(_manifest: &Path, _includes: &[String]) {}

/// Report the committed snapshot plus a machine-readable symbol/version stamp.
fn stamp(path: &Path) {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            // bindgen 0.72 writes `/* automatically generated by rust-bindgen 0.72.1 */`.
            let bindgen_version = text
                .lines()
                .find_map(|line| {
                    let t = line.trim();
                    let rest = t
                        .strip_prefix("/* automatically generated by rust-bindgen ")?
                        .strip_suffix(" */")?;
                    Some(rest.trim().to_owned())
                })
                .unwrap_or_else(|| "unknown".to_string());
            // bindgen 0.72 emits `unsafe extern "C" { pub fn mpv_(...) }`, so the
            // function lines are indented: match on the trimmed token.
            let fns = text
                .lines()
                .filter(|line| {
                    let t = line.trim();
                    t.starts_with("pub fn mpv_") || t.starts_with("pub unsafe fn mpv_")
                })
                .count();
            let consts = text
                .lines()
                .filter(|line| {
                    let t = line.trim();
                    t.starts_with("pub const MPV_") || t.starts_with("pub const mpv_")
                })
                .count();
            emit("PLEX_MPV_BINDINGS", "committed");
            emit("PLEX_MPV_BINDGEN_VERSION", &bindgen_version);
            emit("PLEX_MPV_FFI_FNS", &fns.to_string());
            emit("PLEX_MPV_FFI_CONSTS", &consts.to_string());
        }
        Err(err) => panic!(
            "missing committed libmpv bindings at {}: {err}. Generate them with \
             'cargo build --workspace --features plex-player/regenerate-mpv-bindings' \
             (requires bindgen + libclang).",
            path.display()
        ),
    }
}

/// These only run when the script itself is built as a test harness:
/// ```sh
/// rustc --edition 2021 --test --cfg 'feature = "libmpv-ffi"' \
///     --extern pkg_config=target/debug/deps/libpkg_config-*.rlib \
///     crates/plex-player/build.rs -o target/mpv-build-test && target/mpv-build-test
/// ```
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_version_line_of_a_real_client_h() {
        let header = "/* The API is versioned (see MPV_CLIENT_API_VERSION). */\n\
                      #define MPV_MAKE_VERSION(major, minor) (((major) << 16) | (minor) | 0UL)\n\
                      #define MPV_CLIENT_API_VERSION MPV_MAKE_VERSION(2, 5)\n";
        assert_eq!(parse_client_api_version(header), Some((2, 5)));
    }

    #[test]
    fn tolerates_spacing_and_a_trailing_comment() {
        assert_eq!(
            parse_client_api_version(
                "#  define   MPV_CLIENT_API_VERSION  MPV_MAKE_VERSION( 4 , 1 ) /* api */"
            ),
            Some((4, 1))
        );
    }

    #[test]
    fn ignores_comment_mentions_and_unusable_macros() {
        assert_eq!(
            parse_client_api_version(" * @see MPV_CLIENT_API_VERSION\n"),
            None
        );
        assert_eq!(
            parse_client_api_version("#define MPV_CLIENT_API_VERSION MPV_MAKE_VERSION(a, b)"),
            None
        );
        assert_eq!(
            parse_client_api_version(
                "#define MPV_CLIENT_API_VERSION_LATEST MPV_MAKE_VERSION(2, 5)"
            ),
            None
        );
    }

    #[test]
    fn parses_the_installed_header_when_one_is_present() {
        for dir in [
            "/usr/include",
            "/usr/local/include",
            "/opt/homebrew/include",
        ] {
            let path = Path::new(dir).join("mpv/client.h");
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let found = parse_client_api_version(&text);
            assert!(
                found.is_some_and(|api| api >= min_client_api()),
                "unexpected MPV_CLIENT_API_VERSION in {}: {found:?}",
                path.display()
            );
            return;
        }
    }

    #[test]
    fn enforces_the_same_floor_as_pkg_config() {
        assert_eq!(min_client_api(), (2, 5));
        let floor =
            parse_client_api_version("#define MPV_CLIENT_API_VERSION MPV_MAKE_VERSION(2, 5)");
        assert!(floor >= Some(min_client_api()));
        let older =
            parse_client_api_version("#define MPV_CLIENT_API_VERSION MPV_MAKE_VERSION(1, 9)");
        assert!(older < Some(min_client_api()));
    }
}
