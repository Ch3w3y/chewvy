/* Translation unit for the libmpv FFI.
 *
 * Only the *client* and *render* APIs are exposed; the stream-callback API
 * (mpv_stream_cb.h) is intentionally not bound. `mpv/render_gl.h` is pulled in
 * because the render API's GL interop types are part of the researched render
 * path. As of milestone M5 the Rust wrapper in `src/render_link.rs` really does
 * call the render API (`mpv_render_context_create`, `mpv_render_context_render`,
 * `mpv_render_context_update`), so `mpv/render_gl.h` is now part of the bound
 * API surface: it declares `mpv_opengl_init_params`, `mpv_opengl_fbo` and
 * `mpv_render_gl_format`, which are the GL interop types used by the
 * MPV_RENDER_API_TYPE_OPENGL path.
 *
 * Regenerate the Rust bindings from this header:
 *   source scripts/env.sh
 *   PLEX_REGENERATE_BINDINGS=1 cargo build --offline --locked \
 *       --workspace --features plex-player/libmpv-ffi
 *
 * `MPV_ENABLE_DEPRECATED` is deliberately left at its default (deprecated
 * events/properties are compiled but the Rust wrapper does not use them).
 */
#include <mpv/client.h>
#include <mpv/render.h>
#include <mpv/render_gl.h>
