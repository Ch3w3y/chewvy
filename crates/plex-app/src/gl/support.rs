//! What a display must offer for Chewvy to start, and the one message shown
//! when it does not.
//!
//! The UI, the video target and the window surface are all GL, so a driver that
//! cannot hand the app a usable context leaves the user with no window at all
//! and nothing else to read. That is why the version gate, the message text and
//! the way it is raised live together here instead of at each failure site.

/// The desktop GL version Chewvy asks the window system for:
/// `glutin::context::Version` in the window host's context attributes, and the
/// version the user-facing requirement is stated with.
///
/// The request is versioned so that a capable driver is asked for it by name; a
/// Windows install whose display driver provides no vendor OpenGL still answers
/// with the built-in GL 1.1 rather than an error (measured on the Windows 11
/// test VM), which is why the version is checked after creation, not trusted.
pub const GL_REQUEST: (u8, u8) = (3, 2);

/// The floor Chewvy starts on, per API family.
///
/// Both numbers are what the two things Chewvy cannot do without require:
/// `egui_glow`'s painter refuses anything below GL 2.0 outright (`painter.rs`,
/// `gl.version().major < 2`), and mpv's OpenGL renderer refuses with "At least
/// OpenGL 2.1 or OpenGL ES 2.0 required." (`video/out/opengl/ra_gl.c`).
/// Anything older has never been run here, so it is refused with a message
/// rather than started and hoped for.
const GL_MIN_DESKTOP: (u32, u32) = (2, 1);
const GL_MIN_ES: (u32, u32) = (2, 0);

/// The underlying error used when the version gate is what stopped the run:
/// it names the two components that cannot work below the floor.
pub const GL_FLOOR_ERROR: &str =
    "egui_glow's painter and mpv's OpenGL renderer need a shader-capable context";

/// How a driver names itself to Chewvy: `OpenGL 1.1` or `OpenGL ES 3.2`.
#[must_use]
pub fn version_text(major: u32, minor: u32, is_gles: bool) -> String {
    let api = if is_gles { "OpenGL ES" } else { "OpenGL" };
    format!("{api} {major}.{minor}")
}

/// `true` when a context of this version can run Chewvy's UI and player.
#[must_use]
pub fn version_supported(major: u32, minor: u32, is_gles: bool) -> bool {
    let (min_major, min_minor) = if is_gles { GL_MIN_ES } else { GL_MIN_DESKTOP };
    (major, minor) >= (min_major, min_minor)
}

/// The message to show when this freshly created context is too old to run
/// Chewvy, `None` when it can.
///
/// The check belongs here because `egui_glow::EguiGlow::new` **unwraps** its
/// painter and `EguiGlow`'s remaining fields are private, so a caller cannot
/// build it around a `Painter` it created itself and inspect the `Result`.
#[must_use]
pub fn context_failure(gl: &glow::Context) -> Option<String> {
    let version = glow::HasContext::version(gl);
    if version_supported(version.major, version.minor, version.is_embedded) {
        return None;
    }
    Some(graphics_failure_text(
        Some(&version_text(
            version.major,
            version.minor,
            version.is_embedded,
        )),
        GL_FLOOR_ERROR,
    ))
}

/// The single user-facing message for "Chewvy could not start its graphics".
///
/// `reported` is what the driver said it is ([`version_text`]) when a context
/// exists, `None` when the context or surface itself could not be created.
/// `underlying` is the driver/backend error text, kept verbatim.
#[must_use]
pub fn graphics_failure_text(reported: Option<&str>, underlying: &str) -> String {
    let asked = format!("OpenGL {}.{}", GL_REQUEST.0, GL_REQUEST.1);
    let floor = format!(
        "OpenGL {}.{}/OpenGL ES {}.{}",
        GL_MIN_DESKTOP.0, GL_MIN_DESKTOP.1, GL_MIN_ES.0, GL_MIN_ES.1
    );
    let display = match reported {
        Some(reported) => format!(
            "it asks the window system for {asked} core and cannot start below {floor}; this \
             display reported {reported}"
        ),
        None => {
            format!("it asks the window system for {asked} core and cannot start below {floor}")
        }
    };
    format!(
        "Chewvy could not start its graphics: {display}. Update your graphics driver and start \
         Chewvy again. Underlying error: {underlying}"
    )
}

/// Report `text` where the user will see it, and in the log.
///
/// The `log::error!` line reaches stderr on every OS (see the binary's own
/// `StderrLogger`). Windows gets a native modal dialog on top: a double-clicked
/// `chewvy.exe` has no console to read an error line from.
pub fn report_graphics_failure(text: &str) {
    log::error!("{text}");
    #[cfg(target_os = "windows")]
    message_box("Chewvy", text);
}

/// A modal `MB_ICONERROR` dialog with no owner window.
#[cfg(target_os = "windows")]
fn message_box(title: &str, text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR};

    let utf16 = |value: &str| -> Vec<u16> {
        value
            .encode_utf16()
            .filter(|unit| *unit != 0)
            .chain(std::iter::once(0))
            .collect()
    };
    let title = utf16(title);
    let body = utf16(text);
    // SAFETY: MessageBoxW reads the two NUL-terminated UTF-16 buffers, and both
    // vectors live until after the call; the null HWND means "no owner window",
    // which the API accepts for a standalone dialog. The returned button id is
    // meaningless here because nothing follows this dialog but process exit.
    let _button: i32 = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            body.as_ptr(),
            title.as_ptr(),
            MB_ICONERROR,
        )
    };
}

#[cfg(test)]
mod tests {
    use super::{
        graphics_failure_text, version_supported, version_text, GL_FLOOR_ERROR, GL_REQUEST,
    };

    #[test]
    fn a_windows_builtin_gl_1_1_context_is_refused() {
        // The measured Windows 11 failure: the display adapter offers only
        // Windows' built-in OpenGL 1.1, WGL answers the 3.2-core request with
        // it, and egui_glow's painter unwrap then panicked the process.
        assert!(!version_supported(1, 1, false));
        assert_eq!(version_text(1, 1, false), "OpenGL 1.1");
    }

    #[test]
    fn the_floors_are_accepted_and_everything_below_them_refused() {
        assert!(version_supported(3, 2, false));
        assert!(version_supported(4, 6, false));
        assert!(version_supported(2, 1, false));
        assert!(!version_supported(2, 0, false));
        assert!(version_supported(2, 0, true));
        assert!(version_supported(3, 2, true));
        assert!(!version_supported(1, 1, true));
        assert_eq!(version_text(3, 2, true), "OpenGL ES 3.2");
    }

    #[test]
    fn the_message_names_app_requirement_driver_and_error() {
        let reported = version_text(1, 1, false);
        let text = graphics_failure_text(Some(&reported), GL_FLOOR_ERROR);
        assert!(
            text.starts_with("Chewvy could not start its graphics"),
            "{text}"
        );
        assert!(text.contains("OpenGL 3.2"), "{text}");
        assert!(text.contains("Update your graphics driver"), "{text}");
        assert!(text.contains("OpenGL 1.1"), "{text}");
        assert!(text.ends_with(GL_FLOOR_ERROR), "{text}");
        assert_eq!(GL_REQUEST, (3, 2));
    }

    #[test]
    fn the_message_works_without_a_context_version() {
        let text = graphics_failure_text(None, "failed to create window surface: bad config");
        assert!(text.contains("failed to create window surface"), "{text}");
        assert!(!text.contains("reported"), "{text}");
    }
}
