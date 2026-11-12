//! Command and property vocabulary for the libmpv control layer.
//!
//! Everything here is pure Rust: no libmpv call happens in this module, which is
//! what makes the control layer testable against the mock link
//! ([`crate::mock`]) and lets the smoke report show the exact argument lists that
//! would be handed to `mpv_command_async` / `mpv_set_property_async`.
//!
//! # Verification labels
//!
//! A name used by this wrapper is only as trustworthy as its source:
//!
//! * [`SourceLabel::Header`] - the name appears as a literal string inside the
//!   installed libmpv 2.5.0 headers (`/usr/include/mpv/client.h`), which is the
//!   only name documentation this repository can check offline;
//! * [`SourceLabel::Unverified`] - the name comes from upstream mpv command/property
//!   documentation (URL + access date recorded) or from our own choice, and is
//!   **not** a header symbol. It stays unverified until a real libmpv run observes
//!   it ([`crate::probe`] upgrades it with evidence from a real session).
//!
//! No command name is a header symbol: `client.h` documents the *existence* of
//! `mpv_command_async` but never names `load`/`seek`/`stop`. Those are marked
//! `Unverified` here and are only claimed once a run reports them.

use std::fmt;

/// Where a command or property name's meaning was taken from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceLabel {
    /// Literal string in the installed libmpv headers. Each such label carries
    /// the file+line it was read from by hand in the test environment copy of the headers
    /// (mpv 2.5.0). The header-derived FFI surface around them is checked by
    /// `build.rs` (pkg-config probe + generated binding/function counts) and by
    /// `tests/native_contract.rs`; a per-name header-symbol test does **not**
    /// exist yet (no such test file exists in this crate - an earlier citation
    /// that named one was wrong). Names observed at run time are re-checked
    /// against this table by
    /// `lookup_property` (see `src/mpv_link.rs::next_event`).
    Header {
        /// Header file, e.g. `client.h`.
        file: &'static str,
        /// Line in that header, as installed when the table was written.
        line: u32,
    },
    /// Documented upstream (or chosen by us) but **not** a header symbol.
    Unverified {
        /// Upstream source URL, or `repo:` for a wrapper-internal decision.
        source: &'static str,
        /// Access date of that source, `YYYY-MM-DD`.
        accessed: &'static str,
    },
}

impl fmt::Display for SourceLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Header { file, line } => write!(f, "header:{file}:{line}"),
            Self::Unverified { source, accessed } => {
                write!(f, "unverified:{source}(accessed {accessed})")
            }
        }
    }
}

/// A name the wrapper can pass to libmpv, with its provenance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Named {
    /// Name handed to libmpv verbatim.
    pub name: &'static str,
    /// What justifies that name.
    pub label: SourceLabel,
}

impl Named {
    /// `true` when the name is backed by the installed headers.
    #[must_use]
    pub fn is_header_backed(&self) -> bool {
        matches!(self.label, SourceLabel::Header { .. })
    }

    /// Machine-readable id, e.g. `time-pos=header:client.h:610`.
    #[must_use]
    pub fn fact(&self) -> String {
        format!("{}={}", self.name, self.label)
    }
}

/// Find a property name this wrapper knows, for provenance labelling of values
/// delivered by libmpv. `None` means the name never came from our own table.
#[must_use]
pub fn lookup_property(name: &str) -> Option<Named> {
    props::ALL.iter().copied().find(|named| named.name == name)
}

/// Find a command name this wrapper issues.
#[must_use]
pub fn lookup_command(name: &str) -> Option<Named> {
    cmds::ALL.iter().copied().find(|named| named.name == name)
}

/// Properties this wrapper observes or sets.
///
/// Only `time-pos`, `idle-active` and `playback-time` are header-mentioned names
/// (they appear as string literals in `client.h`); everything else is an
/// upstream-documentation name and is labelled unverified until the probe run
/// reports it from a real libmpv session.
pub mod props {
    use super::{Named, SourceLabel};

    /// Header-mentioned: the literal appears in `client.h`.
    const fn header(name: &'static str, line: u32) -> Named {
        Named {
            name,
            label: SourceLabel::Header {
                file: "client.h",
                line,
            },
        }
    }

    /// Upstream mpv property list (<https://mpv.io/manual/stable/>), not a header symbol.
    const fn docs(name: &'static str) -> Named {
        Named {
            name,
            label: SourceLabel::Unverified {
                source: "https://mpv.io/manual/stable/#properties",
                accessed: "2026-11-04",
            },
        }
    }

    /// Documented in the man page **shipped with the installed libmpv**
    /// (`/usr/share/man/man1/mpv.1.gz`, mpv v0.41.0 - the copy the headless test environment can
    /// read offline), still not a header symbol and **not measured as a settable
    /// property by any run in this repository** (see [`props::AID`]).
    const fn manpage(name: &'static str) -> Named {
        Named {
            name,
            label: SourceLabel::Unverified {
                source: "man:/usr/share/man/man1/mpv.1.gz:2002,2061 (--aid=<ID|auto|no>, --sid=<ID|auto|no>) and :18994 (vid/aid/sid listed as properties)",
                accessed: "2026-11-06",
            },
        }
    }

    /// Observe: playback position in seconds (header-mentioned).
    pub const TIME_POS: Named = header("time-pos", 610);
    /// Observe: last reported playback time (header-mentioned).
    pub const PLAYBACK_TIME: Named = header("playback-time", 1309);
    /// Observe: core idle state (header-mentioned; used by the deprecated
    /// `MPV_EVENT_IDLE` documentation, so we observe it instead of the event).
    pub const IDLE_ACTIVE: Named = header("idle-active", 1296);

    /// Set/observe: paused flag.
    pub const PAUSE: Named = docs("pause");
    /// Observe: reached end of file.
    pub const EOF_REACHED: Named = docs("eof-reached");
    /// Observe: why playback ended (`normal|error|stop`).
    pub const EOF_REASON: Named = docs("eof-reason");
    /// Observe: total duration in seconds.
    pub const DURATION: Named = docs("duration");
    /// Set: output volume, 0-100 (100 = unity gain).
    pub const VOLUME: Named = docs("volume");
    /// **Candidate only - `[U]`, no run in this repository has issued it.**
    /// Set: selected audio track id (or `no` to mute audio).
    ///
    /// Documentation and measurement are separate facts and must not be merged:
    ///
    /// * the man page installed with this libmpv (mpv 0.41.0, client API 2.5,
    ///   `/usr/share/man/man1/mpv.1.gz`, read 2026-11-06) documents
    ///   `--aid=<ID|auto|no>` (line 2002), uses `set aid 2` at runtime (line
    ///   2053) and lists `vid`/`aid`/`sid` under "Inconsistencies between
    ///   options and properties" (lines 18988-18994);
    /// * **no captured run in this tree ever sent `aid`**. the `--probe-json` report
    ///   records `set audio-id` and `set sub-id` being rejected with
    ///   `MPV_ERROR_PROPERTY_NOT_FOUND`, and `crates/plex-player/tests/ffi_native_session.rs`
    ///   drives those same two names through the control gate - never `aid`/`sid`.
    ///   So whether this libmpv accepts `aid` as a writable property is **[U]**
    ///   and is recorded as such in `docs/CAPABILITIES.md` ("player
    ///   controls").
    ///
    /// Consequently `AID`/`SID` are **not** the names this client sends: the
    /// selection requests built by `src/tracks.rs` still carry
    /// [`Self::AUDIO_ID`]/[`Self::SUB_ID`]. Re-routing the control set onto
    /// `aid`/`sid` needs a measured run first.
    pub const AID: Named = manpage("aid");
    /// **Candidate only - `[U]`**; same status as [`Self::AID`] (`--sid` at
    /// mpv.1.gz line 2061, `sid` listed among properties at 18988-18994, never
    /// sent by a captured run in this tree).
    pub const SID: Named = manpage("sid");
    /// **Kept only as recorded rejection evidence - do not use for selection.**
    ///
    /// `audio-id` is **not** a property of the installed libmpv: the retained
    /// Run (the `--probe-json` report `items[].errors`) recorded
    /// `set audio-id = string` -> `MPV_ERROR_PROPERTY_NOT_FOUND` on every probed
    /// item, and the man page installed with that libmpv contains the string
    /// `audio-id` zero times. That probe is **earlier-session evidence retained
    /// from M4, not a run of this tree**; whether the rejection is about the
    /// name, the value or the locally generated single-stream items is **[U]**
    /// (`docs/CAPABILITIES.md` track-selection rows).
    /// It stays in [`Self::ALL`] so the M4 probe vocabulary and
    /// `tests/ffi_native_session.rs` keep recording that rejection.
    pub const AUDIO_ID: Named = docs("audio-id");
    /// Same as [`Self::AUDIO_ID`]: recorded rejected name, not a selection name.
    pub const SUB_ID: Named = docs("sub-id");
    /// Observe: track list as a node array (not decoded in M4).
    pub const TRACK_LIST: Named = docs("track-list");
    /// Observe: cache/stall state.
    pub const PAUSED_FOR_CACHE: Named = docs("paused-for-cache");
    /// Observe: current media title.
    pub const MEDIA_TITLE: Named = docs("media-title");

    /// Every name [`crate::engine::PlayerSession::open`] **observes**, and the
    /// list the probe report and the header cross-check are built from.
    ///
    /// `ALL` is not documentation: `open` issues one `mpv_observe_property` per
    /// entry (`src/engine.rs`, and `src/mpv_link.rs` turns a nonzero return code
    /// into a `PlayerError`). An unmeasured name in this list therefore makes a
    /// real session's `open` depend on whether this libmpv happens to accept
    /// that name - which is exactly the `[U]` status of `aid`/`sid` documented on
    /// [`Self::AID`]. They are deliberately **not** here (they are also not in
    /// [`crate::state::observed_format`]'s table); they stay available as named,
    /// man-page-labelled candidates for the measured track-selection step.
    /// [`crate::tracks`] keeps sending the recorded-rejected
    /// [`Self::AUDIO_ID`]/[`Self::SUB_ID`] names, which is the state a captured
    /// run actually measured.
    ///
    /// The list is pinned row by row - name, requested format, provenance - by
    /// `crates/plex-player/tests/control_loop.rs::the_observed_vocabulary_is_pinned_and_every_name_has_a_declared_format`,
    /// which also pins that `AID`/`SID` are absent from it.
    pub const ALL: &[Named] = &[
        TIME_POS,
        PLAYBACK_TIME,
        IDLE_ACTIVE,
        PAUSE,
        EOF_REACHED,
        EOF_REASON,
        DURATION,
        VOLUME,
        AUDIO_ID,
        SUB_ID,
        TRACK_LIST,
        PAUSED_FOR_CACHE,
        MEDIA_TITLE,
    ];
}

/// Command names accepted by `mpv_command_async` (argument 0).
///
/// None of these is a header symbol: `client.h` never names a command. Each is
/// labelled unverified until a real display-free run confirms it
/// (the `--probe-json` report -> `commands`).
///
/// `load` is deliberately **not** in this vocabulary: no probe run has ever
/// confirmed a `load` command on the installed libmpv, while `loadfile` was
/// confirmed (see the `--probe-json` report `commands.replies`).
pub mod cmds {
    use super::{Named, SourceLabel};

    const fn docs(name: &'static str) -> Named {
        Named {
            name,
            label: SourceLabel::Unverified {
                source: "https://mpv.io/manual/stable/#commands",
                accessed: "2026-11-04",
            },
        }
    }

    /// Load a URI as the current (and only queued) item.
    pub const LOADFILE: Named = docs("loadfile");
    /// Stop playback and drop the current item.
    pub const STOP: Named = docs("stop");
    /// Seek.
    pub const SEEK: Named = docs("seek");

    /// Every command name this wrapper can issue.
    pub const ALL: &[Named] = &[LOADFILE, STOP, SEEK];
}

/// Value type requested from/for a property (maps to `mpv_format`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    /// `MPV_FORMAT_FLAG`.
    Flag,
    /// `MPV_FORMAT_INT64`.
    Int64,
    /// `MPV_FORMAT_DOUBLE`.
    Double,
    /// `MPV_FORMAT_STRING`.
    String,
    /// `MPV_FORMAT_NONE`.
    None,
}

/// Concrete property values used by set-property requests.
#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    /// Boolean property (`pause`, `eof-reached`, ...).
    Flag(bool),
    /// Integer property (`volume` is set as double; ids are strings in mpv).
    Int64(i64),
    /// Floating point property (`time-pos`, `volume`).
    Double(f64),
    /// String property (`audio-id`/`sub-id` accept `"no"` or numeric strings).
    Str(String),
    /// Explicit clear.
    None,
}

impl PropertyValue {
    /// The `mpv_format` this value would be sent as.
    #[must_use]
    pub fn kind(&self) -> ValueKind {
        match self {
            Self::Flag(_) => ValueKind::Flag,
            Self::Int64(_) => ValueKind::Int64,
            Self::Double(_) => ValueKind::Double,
            Self::Str(_) => ValueKind::String,
            Self::None => ValueKind::None,
        }
    }
}

/// Seek target semantics (`seek` command flags).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SeekMode {
    /// `absolute` seek.
    Absolute,
    /// `relative` seek (offset may be negative).
    Relative,
    /// Absolute seek with `exact` flag.
    AbsoluteExact,
}

impl SeekMode {
    /// The mpv flag string for this mode.
    ///
    /// Flag strings are not header symbols; see the `--probe-json` report for the
    /// command/flag run that exercised them.
    #[must_use]
    pub fn flag(self) -> &'static str {
        match self {
            Self::Absolute => "absolute",
            Self::Relative => "relative",
            Self::AbsoluteExact => "absolute+exact",
        }
    }
}

/// An item to play.
///
/// The player layer accepts local paths and http/https URLs (the direct-play URL
/// shape the app derives from library metadata). Anything else - a Plex
/// scheme, an optical disc device, a missing URI - is rejected with
/// [`crate::PlayerError::UnsupportedItem`] rather than handed to libmpv.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemSource {
    /// Local file path (fixture/attachment file).
    File {
        /// Absolute or relative path.
        path: String,
    },
    /// http(s) URL (direct-play stream).
    Url {
        /// Full URL.
        url: String,
    },
}

impl ItemSource {
    /// Validate the source, or explain why this item cannot be played here.
    ///
    /// ```
    /// use plex_player::commands::ItemSource;
    /// let item = ItemSource::parse("https://example.invalid/stream.mkv").expect("parsed");
    /// assert!(matches!(item, ItemSource::Url { .. }));
    /// assert!(ItemSource::parse("dvd://1").is_err());
    /// ```
    pub fn parse(raw: &str) -> Result<Self, crate::PlayerError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(crate::PlayerError::UnsupportedItem {
                item: "<empty>".to_owned(),
                reason: "no source given".to_owned(),
            });
        }
        if let Some(rest) = trimmed.strip_prefix("file://") {
            return Ok(Self::File {
                path: rest.to_owned(),
            });
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            return Ok(Self::Url {
                url: trimmed.to_owned(),
            });
        }
        if trimmed.starts_with('/') || trimmed.starts_with("./") || trimmed.starts_with("../") {
            return Ok(Self::File {
                path: trimmed.to_owned(),
            });
        }
        // Anything with an explicit scheme we do not handle (dvd://, cdda://,
        // rtmp://, plex-scheme ids) or an unparseable bare token.
        let scheme = lower
            .split_once("://")
            .map(|(s, _)| s.to_owned())
            .unwrap_or_else(|| "path".to_owned());
        Err(crate::PlayerError::UnsupportedItem {
            item: trimmed.to_owned(),
            reason: if scheme == "path" {
                "not an absolute path and not a supported URL scheme".to_owned()
            } else {
                format!("scheme {scheme}:// is not supported by the player layer")
            },
        })
    }

    /// The string handed to the `loadfile` command.
    #[must_use]
    pub fn load_arg(&self) -> &str {
        match self {
            Self::File { path } => path,
            Self::Url { url } => url,
        }
    }
}

/// A request the engine issues asynchronously with a reply token.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// `mpv_command_async(token, args)` - `args[0]` is the command name.
    Command {
        /// Command name (argument 0).
        name: Named,
        /// Remaining arguments.
        args: Vec<String>,
    },
    /// `mpv_set_property_async(token, name, format, value)`.
    SetProperty {
        /// Property name.
        name: Named,
        /// Value to set.
        value: PropertyValue,
    },
}

impl Request {
    /// Stable command label used in events, logs and the probe report.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Command { name, args } => {
                let mut out = String::from(name.name);
                for a in args {
                    out.push(' ');
                    out.push_str(a);
                }
                out
            }
            Self::SetProperty { name, value } => {
                format!("set {} = {}", name.name, value.kind())
            }
        }
    }

    /// Which libmpv async call this request is issued through.
    ///
    /// This is a fact about **our own** request shape (the variant decides it),
    /// recorded at issue time so a failed reply can say which call it came back
    /// from ([`crate::PlayerError::Command`]). The two names are the async
    /// entry points declared by `client.h`; they are not property/command names.
    #[must_use]
    pub fn call_name(&self) -> &'static str {
        match self {
            Self::Command { .. } => "mpv_command_async",
            Self::SetProperty { .. } => "mpv_set_property_async",
        }
    }

    /// The property name, when this is a property set.
    #[must_use]
    pub fn property_name(&self) -> Option<&'static str> {
        match self {
            Self::SetProperty { name, .. } => Some(name.name),
            Self::Command { .. } => None,
        }
    }
}

impl fmt::Display for ValueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Flag => "flag",
            Self::Int64 => "int64",
            Self::Double => "double",
            Self::String => "string",
            Self::None => "none",
        };
        f.write_str(text)
    }
}
