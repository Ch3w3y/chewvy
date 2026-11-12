//! Frosted glass: a panel that shows what is behind it blurred, brightened and
//! tinted, with a hairline rim — the app's navigation and overlay material.
//!
//! [`panel`] adds an `egui_glow` paint callback. When the painter reaches it,
//! everything drawn behind the panel is already in the target framebuffer, so
//! the callback copies that region (plus a margin the blur reaches into) down
//! to a quarter-size texture with a linear blit, runs a separable Gaussian over
//! it twice, and composites the result back into the panel's rounded rectangle
//! with extra saturation and the caller's tint. The rim and the specular top
//! edge are ordinary egui shapes drawn after it.
//!
//! When the shaders cannot be built (a GL without GLSL ES 3.00), [`panel`]
//! falls back to a plain fill of the tint made more opaque.
//!
//! The GL calls are the module's only `unsafe`, as in `crate::gl::fbo`: each
//! one names objects this module created on the context `egui_glow` passes to
//! the callback, and the callback restores the framebuffer bindings it found;
//! `egui_glow` restores the rest of its own state after every callback.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use egui::{pos2, Color32, CornerRadius, Rect, Stroke};
use glow::HasContext as _;

/// How far the blurred copy is scaled down before blurring.
const DOWNSCALE: i32 = 4;
/// Margin, in screen pixels, read around the panel so its edges blur over
/// what is next to it rather than over a clamped border.
const MARGIN: i32 = 48;
/// Separable blur passes (horizontal + vertical each).
const PASSES: usize = 2;
/// Colour saturation applied to the blurred backdrop (1 is unchanged).
const SATURATION: f32 = 1.35;
/// The rim drawn around every glass panel.
const RIM: Color32 = Color32::from_rgba_premultiplied(30, 30, 30, 30);
/// The brighter line along the top edge, where light catches the glass.
const SHEEN: Color32 = Color32::from_rgba_premultiplied(46, 46, 46, 46);

/// 0: not tried yet, 1: works, 2: shaders failed (plain fills from now on).
static STATE: AtomicU8 = AtomicU8::new(0);
const WORKS: u8 = 1;
const FAILED: u8 = 2;

/// The GL objects, made on first use on the context `egui_glow` paints with.
static GL: Mutex<Option<Objects>> = Mutex::new(None);

/// A glass panel filling `rect` with corner `radius` (points), tinted by
/// `tint` (premultiplied: its alpha is how much tint covers the blur).
pub fn panel(painter: &egui::Painter, rect: Rect, radius: f32, tint: Color32) {
    let rounding = CornerRadius::same(radius.clamp(0.0, 255.0) as u8);
    if STATE.load(Ordering::Relaxed) == FAILED {
        let [r, g, b, a] = tint.to_array();
        let solid = Color32::from_rgba_premultiplied(r, g, b, a.saturating_add(70));
        painter.rect_filled(rect, rounding, solid);
    } else {
        let tint = tint.to_normalized_gamma_f32();
        let callback = egui_glow::CallbackFn::new(move |info, painter| {
            draw(painter.gl(), &info, radius, tint);
        });
        painter.add(egui::PaintCallback {
            rect,
            callback: Arc::new(callback),
        });
    }
    painter.rect_stroke(
        rect,
        rounding,
        Stroke::new(1.0, RIM),
        egui::StrokeKind::Inside,
    );
    let inset = radius.max(1.0);
    painter.line_segment(
        [
            pos2(rect.left() + inset, rect.top() + 0.5),
            pos2(rect.right() - inset, rect.top() + 0.5),
        ],
        Stroke::new(1.0, SHEEN),
    );
}

const VERTEX: &str = r"#version 300 es
out vec2 v_uv;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_uv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
";

/// A 9-tap Gaussian in five bilinear samples along `u_step`.
const BLUR: &str = r"#version 300 es
precision mediump float;
uniform sampler2D u_tex;
uniform vec2 u_step;
in vec2 v_uv;
out vec4 o;
void main() {
    vec4 c = texture(u_tex, v_uv) * 0.2270270270;
    c += (texture(u_tex, v_uv + u_step * 1.3846153846)
        + texture(u_tex, v_uv - u_step * 1.3846153846)) * 0.3162162162;
    c += (texture(u_tex, v_uv + u_step * 3.2307692308)
        + texture(u_tex, v_uv - u_step * 3.2307692308)) * 0.0702702703;
    o = c;
}
";

/// The blurred backdrop inside a rounded rectangle, saturated and tinted.
const COMPOSE: &str = r"#version 300 es
precision mediump float;
uniform sampler2D u_tex;
uniform vec4 u_uv;
uniform vec2 u_size;
uniform float u_radius;
uniform vec4 u_tint;
uniform float u_saturation;
in vec2 v_uv;
out vec4 o;
float rounded(vec2 p, vec2 half_size, float r) {
    vec2 q = abs(p) - half_size + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
void main() {
    vec3 c = texture(u_tex, mix(u_uv.xy, u_uv.zw, v_uv)).rgb;
    float luma = dot(c, vec3(0.2126, 0.7152, 0.0722));
    c = mix(vec3(luma), c, u_saturation);
    c = c * (1.0 - u_tint.a) + u_tint.rgb;
    float d = rounded((v_uv - 0.5) * u_size, u_size * 0.5, u_radius);
    float a = clamp(0.5 - d, 0.0, 1.0);
    o = vec4(c * a, a);
}
";

/// Everything the effect keeps between frames.
struct Objects {
    blur: glow::Program,
    compose: glow::Program,
    vao: glow::VertexArray,
    /// Two ping-pong targets at the downscaled size.
    targets: [(glow::Framebuffer, glow::Texture); 2],
    size: (i32, i32),
}

fn compile(gl: &glow::Context, fragment: &str) -> Result<glow::Program, String> {
    // SAFETY: shader and program objects created here on `gl`, the context the
    // callback runs on; failed objects are deleted before returning.
    unsafe {
        let program = gl.create_program()?;
        let mut shaders = Vec::new();
        for (kind, source) in [
            (glow::VERTEX_SHADER, VERTEX),
            (glow::FRAGMENT_SHADER, fragment),
        ] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let log = gl.get_shader_info_log(shader);
                gl.delete_shader(shader);
                gl.delete_program(program);
                return Err(log);
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        if gl.get_program_link_status(program) {
            Ok(program)
        } else {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            Err(log)
        }
    }
}

fn target(gl: &glow::Context) -> Result<(glow::Framebuffer, glow::Texture), String> {
    // SAFETY: a texture and framebuffer created and configured on `gl`.
    unsafe {
        let texture = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        for (key, value) in [
            (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
            (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, key, value as i32);
        }
        let framebuffer = gl.create_framebuffer()?;
        Ok((framebuffer, texture))
    }
}

impl Objects {
    fn new(gl: &glow::Context) -> Result<Self, String> {
        let blur = compile(gl, BLUR)?;
        let compose = compile(gl, COMPOSE)?;
        // SAFETY: a vertex array object created on `gl` (GLES 3 needs one bound
        // to draw, even with no attributes).
        let vao = unsafe { gl.create_vertex_array()? };
        Ok(Self {
            blur,
            compose,
            vao,
            targets: [target(gl)?, target(gl)?],
            size: (0, 0),
        })
    }

    /// Make both targets `size` (downscaled pixels), reallocating on change.
    fn resize(&mut self, gl: &glow::Context, size: (i32, i32)) {
        if self.size == size {
            return;
        }
        for (framebuffer, texture) in self.targets {
            // SAFETY: this module's own texture and framebuffer on `gl`.
            unsafe {
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    size.0,
                    size.1,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                gl.framebuffer_texture_2d(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::TEXTURE_2D,
                    Some(texture),
                    0,
                );
            }
        }
        self.size = size;
    }
}

/// The callback body: blur what is behind `info.viewport` and composite it.
fn draw(gl: &glow::Context, info: &egui::PaintCallbackInfo, radius: f32, tint: [f32; 4]) {
    // SAFETY: plain state queries on the callback's context. Read before
    // anything below binds this module's own framebuffers.
    let (draw_fb, read_fb, scissor) = unsafe {
        (
            gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING),
            gl.get_parameter_framebuffer(glow::READ_FRAMEBUFFER_BINDING),
            gl.is_enabled(glow::SCISSOR_TEST),
        )
    };
    draw_into(gl, info, radius, tint, draw_fb);
    // SAFETY: binding back exactly what was bound on entry.
    unsafe {
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, draw_fb);
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, read_fb);
        if scissor {
            gl.enable(glow::SCISSOR_TEST);
        } else {
            gl.disable(glow::SCISSOR_TEST);
        }
    }
}

/// [`draw`] once the entry bindings are saved: `target` is the framebuffer the
/// panel is composited into.
fn draw_into(
    gl: &glow::Context,
    info: &egui::PaintCallbackInfo,
    radius: f32,
    tint: [f32; 4],
    target: Option<glow::Framebuffer>,
) {
    let Ok(mut slot) = GL.lock() else {
        return;
    };
    if slot.is_none() {
        match Objects::new(gl) {
            Ok(objects) => {
                *slot = Some(objects);
                STATE.store(WORKS, Ordering::Relaxed);
            }
            Err(error) => {
                log::warn!("glass: shaders unavailable, using plain fills: {error}");
                STATE.store(FAILED, Ordering::Relaxed);
                return;
            }
        }
    }
    let Some(objects) = slot.as_mut() else {
        return;
    };

    let screen = info.screen_size_px;
    let (screen_w, screen_h) = (screen[0] as i32, screen[1] as i32);
    let panel = info.viewport_in_pixels();
    if panel.width_px <= 0 || panel.height_px <= 0 {
        return;
    }
    // The source region in GL window coordinates (origin bottom-left).
    let x0 = (panel.left_px - MARGIN).max(0);
    let y0 = (panel.from_bottom_px - MARGIN).max(0);
    let x1 = (panel.left_px + panel.width_px + MARGIN).min(screen_w);
    let y1 = (panel.from_bottom_px + panel.height_px + MARGIN).min(screen_h);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let small = (
        ((x1 - x0) / DOWNSCALE).max(1),
        ((y1 - y0) / DOWNSCALE).max(1),
    );
    objects.resize(gl, small);
    let [(fb_a, tex_a), (fb_b, tex_b)] = objects.targets;

    // SAFETY: every object below was created by this module on `gl`; [`draw`]
    // binds the entry framebuffers again afterwards, and egui_glow resets
    // program, blend, viewport, scissor rect and its vertex array after the
    // callback.
    unsafe {
        // 1. Copy the backdrop down into target A.
        gl.disable(glow::SCISSOR_TEST);
        gl.disable(glow::BLEND);
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, target);
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(fb_a));
        gl.blit_framebuffer(
            x0,
            y0,
            x1,
            y1,
            0,
            0,
            small.0,
            small.1,
            glow::COLOR_BUFFER_BIT,
            glow::LINEAR,
        );

        // 2. Blur A -> B -> A, PASSES times.
        gl.bind_vertex_array(Some(objects.vao));
        gl.use_program(Some(objects.blur));
        gl.viewport(0, 0, small.0, small.1);
        gl.active_texture(glow::TEXTURE0);
        let tex_loc = gl.get_uniform_location(objects.blur, "u_tex");
        let step_loc = gl.get_uniform_location(objects.blur, "u_step");
        gl.uniform_1_i32(tex_loc.as_ref(), 0);
        let texel = (1.0 / small.0 as f32, 1.0 / small.1 as f32);
        for _ in 0..PASSES {
            for (from, into, step) in [(tex_a, fb_b, (texel.0, 0.0)), (tex_b, fb_a, (0.0, texel.1))]
            {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(into));
                gl.bind_texture(glow::TEXTURE_2D, Some(from));
                gl.uniform_2_f32(step_loc.as_ref(), step.0, step.1);
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            }
        }

        // 3. Composite into the panel on the original target.
        gl.bind_framebuffer(glow::FRAMEBUFFER, target);
        gl.viewport(
            panel.left_px,
            panel.from_bottom_px,
            panel.width_px,
            panel.height_px,
        );
        gl.enable(glow::SCISSOR_TEST);
        gl.enable(glow::BLEND);
        gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
        gl.use_program(Some(objects.compose));
        gl.bind_texture(glow::TEXTURE_2D, Some(tex_a));
        let span = ((x1 - x0) as f32, (y1 - y0) as f32);
        let u0 = (panel.left_px - x0) as f32 / span.0;
        let v0 = (panel.from_bottom_px - y0) as f32 / span.1;
        let u1 = (panel.left_px + panel.width_px - x0) as f32 / span.0;
        let v1 = (panel.from_bottom_px + panel.height_px - y0) as f32 / span.1;
        let loc = |name: &str| gl.get_uniform_location(objects.compose, name);
        gl.uniform_1_i32(loc("u_tex").as_ref(), 0);
        gl.uniform_4_f32(loc("u_uv").as_ref(), u0, v0, u1, v1);
        gl.uniform_2_f32(
            loc("u_size").as_ref(),
            panel.width_px as f32,
            panel.height_px as f32,
        );
        gl.uniform_1_f32(loc("u_radius").as_ref(), radius * info.pixels_per_point);
        gl.uniform_4_f32(loc("u_tint").as_ref(), tint[0], tint[1], tint[2], tint[3]);
        gl.uniform_1_f32(loc("u_saturation").as_ref(), SATURATION);
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
    }
}
