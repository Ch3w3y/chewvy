//! The playback-source half of the shell: the key that identifies one media
//! part, the resolved source the player is handed, and the redaction helpers
//! that keep a credential out of any label built from that source.

use std::fmt;

use plex_core::log::sanitize;

/// Redact the credential part of a media URL for display/logging.
///
/// **This is defence in depth, not this client's request design.** Tokens
/// travel as a **header only, never a query string**, and `plex_core::http`
/// refuses a query key named `X-Plex-Token` before a request leaves the process,
/// so the sources this shell caches are query-free. This function exists anyway
/// because the shell cannot prove what a *foreign* string handed to it (a
/// server-supplied `parts[].key` URL, a player-layer error quoting its item, a
/// future upstream change) contains: masking the value of any token-shaped query
/// parameter (`X-Plex-Token`, `token`) costs nothing and keeps the useful part of
/// the line ("the path is /library/12345/1.mkv") readable.
///
/// Open consequence of header-only tokens: whatever direct-play URL is handed to
/// libmpv carries no credential of its own, so the window host has to present one
/// to mpv (mpv's `--http-header-fields`, or a server-issued short-lived URL). That
/// is not implemented here and is not claimed by this code.
#[must_use]
fn redact_url(url: &str) -> String {
    let Some((path, query)) = url.split_once('?') else {
        return url.to_owned();
    };
    let masked: Vec<String> = query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((name, _)) if name.eq_ignore_ascii_case("X-Plex-Token") || name == "token" => {
                format!("{name}=[redacted]")
            }
            _ => pair.to_owned(),
        })
        .collect();
    format!("{path}?{}", masked.join("&"))
}

/// Mask every token-shaped `name=value` pair in an arbitrary line of text.
///
/// [`redact_url`] understands a whole URL; error messages quote things *inside*
/// a sentence (`unsupported item plex://host/x.mkv?X-Plex-Token=…: …`), which is
/// exactly the shape a refusal line takes. This masks the value half of any
/// pair whose name is `X-Plex-Token` / `access_token` / `accessToken` / `token`
/// (case-insensitive, name must start at a word boundary so `X-Plex-Token` is
/// masked once, not twice) and leaves the rest of the line alone.
///
/// Only ASCII names are matched, on an ASCII-lowercased copy, which preserves
/// byte offsets, so the masking never splits a multi-byte character.
#[must_use]
fn mask_secret_pairs(text: &str) -> String {
    const NAMES: [&str; 4] = ["x-plex-token", "access_token", "accesstoken", "token"];
    fn is_delim(c: char) -> bool {
        c.is_whitespace()
            || c.is_control()
            || matches!(
                c,
                ',' | ';' | '"' | '\'' | ')' | ']' | '}' | '>' | '<' | '|'
            )
    }
    fn is_name_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '-' || c == '_'
    }
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut rest = 0_usize; // byte offset into `text` (names are ASCII)
    'scan: while rest < text.len() {
        let boundary = rest == 0 || {
            let previous = text[..rest].chars().next_back().unwrap_or(' ');
            !is_name_char(previous)
        };
        if boundary {
            for name in NAMES {
                if !lower[rest..].starts_with(name) {
                    continue;
                }
                let name_end = rest + name.len();
                if !text[name_end..].starts_with('=') {
                    continue;
                }
                // Name as given (original case) + the mask, then skip the value.
                out.push_str(&text[rest..name_end]);
                out.push_str("=[redacted]");
                let mut at = name_end + 1;
                while at < text.len() {
                    let Some(character) = text[at..].chars().next() else {
                        break;
                    };
                    if is_delim(character) {
                        break;
                    }
                    at += character.len_utf8();
                }
                rest = at;
                continue 'scan;
            }
        }
        let character = text[rest..].chars().next().unwrap_or(' ');
        out.push(character);
        rest += character.len_utf8();
    }
    out
}

/// A key: which part of which item a resolved source belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CacheKey {
    /// Item rating key.
    pub rating_key: String,
    /// Media part index.
    pub part: usize,
}

/// A resolved source the shell is allowed to hand to the player.
///
/// `source` is the raw `loadfile` argument and is deliberately **not** printed
/// by `Debug`: a media source may carry a credential, and resolved sources show
/// up in assertion failures. `Debug` shows the key, the redacted label and
/// whether the source is local, which is everything a diagnostic needs and
/// nothing a credential needs.
#[derive(Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The key it was resolved for.
    pub key: CacheKey,
    /// The raw source handed to libmpv (`loadfile` argument).
    pub source: String,
    /// Human-readable label the UI/tests may show - never the token.
    pub label: String,
    /// `true` when the source came from a local fixture rather than a server.
    pub local: bool,
}

impl Resolved {
    /// Build a resolved entry, labelling it from the redacted form of its source.
    #[must_use]
    pub fn new(rating_key: &str, part: usize, source: &str) -> Self {
        let redacted = mask_secret_pairs(&redact_url(source));
        Self {
            key: CacheKey {
                rating_key: rating_key.to_owned(),
                part,
            },
            source: source.to_owned(),
            label: sanitize(&redacted, 120),
            local: !source.contains("://"),
        }
    }

    /// The displayable label (already redacted at construction).
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// `Debug` without the raw source: see the type-level note on [`Resolved`].
impl fmt::Debug for Resolved {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Resolved")
            .field("key", &self.key)
            .field("label", &self.label)
            .field("local", &self.local)
            .field("source", &"[redacted]")
            .finish()
    }
}
