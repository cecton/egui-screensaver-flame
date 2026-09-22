//! Flame screensaver for [egui](https://github.com/emilk/egui).
//!
//! A windswept fire rendered from a fractal-noise fragment shader, ported from
//! [Blake Bowen's "Flame in the wind" CodePen](https://codepen.io/osublake/pen/pqNXoq),
//! itself adapting a shader by Shadertoy user
//! [@kuvkar](https://www.shadertoy.com/view/4tXXRn).
//!
//! Like the original, this runs the shader on the GPU: a fullscreen triangle
//! is drawn through an [`egui::PaintCallback`] using [`egui_glow`], so the
//! per-pixel math runs in parallel across the GPU's shader cores instead of
//! serially on the CPU. **This requires the host `eframe::App` to use the
//! `glow` rendering backend** (not `wgpu`) — [`FlameBackground::paint`]
//! simply skips drawing if no `glow::Context` is available.
//!
//! Rendering is capped at 30 FPS. If the hardware cannot sustain that rate,
//! frames are painted as fast as possible without any artificial delay.
//!
//! # Usage
//!
//! ```rust,no_run
//! use egui_screensaver_flame::FlameBackground;
//!
//! struct MyApp {
//!     flame: FlameBackground,
//! }
//!
//! impl eframe::App for MyApp {
//!     fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
//!         let ctx = ui.ctx().clone();
//!         // Call paint once per frame before drawing any UI windows so the
//!         // screensaver sits on the background layer behind everything else.
//!         self.flame.paint(&ctx, frame.gl());
//!     }
//! }
//! ```

use std::sync::Arc;
use std::time::Duration;

use egui::{Color32, Context, LayerId, Painter, Shape};
use egui_glow::{CallbackFn, ShaderVersion};
use glow::HasContext as _;

/// Side length of the tileable value-noise texture used for the fractal
/// Brownian motion, in texels.
const NOISE_SIZE: usize = 64;

/// Matches the original's `time += 0.1 * delta` running on a ~60 fps Pixi
/// ticker (`delta` ~= 1 at 60 fps), i.e. roughly 6 time-units per second.
const TIME_SPEED: f32 = 6.0;

const REPAINT_MS: u64 = 33;

const VERTEX_SRC: &str = include_str!("shader/vertex.glsl");
const FRAGMENT_SRC: &str = include_str!("shader/fragment.glsl");

/// A tileable value-noise texture uploaded once to the GPU, standing in for
/// the original's noise PNG. Small xorshift64 PRNG so this crate needs no
/// extra dependency for it.
fn generate_noise_texels(size: usize) -> Vec<u8> {
    let mut state = 0x9E3779B97F4A7C15u64;
    (0..size * size)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 56) as u8
        })
        .collect()
}

unsafe fn compile_shader(
    gl: &glow::Context,
    shader_type: u32,
    source: &str,
) -> Result<glow::Shader, String> {
    unsafe {
        let shader = gl.create_shader(shader_type)?;
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if gl.get_shader_compile_status(shader) {
            Ok(shader)
        } else {
            Err(gl.get_shader_info_log(shader))
        }
    }
}

unsafe fn link_program(
    gl: &glow::Context,
    shaders: &[glow::Shader],
) -> Result<glow::Program, String> {
    unsafe {
        let program = gl.create_program()?;
        for &shader in shaders {
            gl.attach_shader(program, shader);
        }
        gl.link_program(program);
        let result = if gl.get_program_link_status(program) {
            Ok(program)
        } else {
            Err(gl.get_program_info_log(program))
        };
        for &shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        result
    }
}

/// Compiled GPU resources for the flame shader: program, fullscreen-triangle
/// VBO/VAO, and the one-time noise texture. Built once, lazily, on the first
/// frame a `glow::Context` is available (see [`FlameBackground::paint`]).
/// Nothing here mutates after construction, so this is shared across frames
/// as a plain `Arc` with no interior mutability needed.
struct GlResources {
    gl: Arc<glow::Context>,
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    noise_texture: glow::Texture,
    u_time: Option<glow::UniformLocation>,
    u_dimensions: Option<glow::UniformLocation>,
    u_scale: Option<glow::UniformLocation>,
}

impl GlResources {
    fn new(gl: Arc<glow::Context>) -> Result<Self, String> {
        unsafe {
            let shader_version = ShaderVersion::get(&gl);
            let new_interface = shader_version.is_new_shader_interface() as i32;
            let header = format!(
                "{}\n#define NEW_SHADER_INTERFACE {new_interface}\n",
                shader_version.version_declaration()
            );

            let vertex =
                compile_shader(&gl, glow::VERTEX_SHADER, &format!("{header}{VERTEX_SRC}"))?;
            let fragment = compile_shader(
                &gl,
                glow::FRAGMENT_SHADER,
                &format!("{header}{FRAGMENT_SRC}"),
            )?;
            let program = link_program(&gl, &[vertex, fragment])?;

            // A single triangle covering the whole screen (see vertex.glsl).
            let vertices: [f32; 6] = [-1.0, -1.0, 3.0, -1.0, -1.0, 3.0];
            let vbo = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                f32_slice_as_bytes(&vertices),
                glow::STATIC_DRAW,
            );

            let vao = gl.create_vertex_array()?;
            gl.bind_vertex_array(Some(vao));
            let a_pos = gl.get_attrib_location(program, "a_pos");
            if let Some(a_pos) = a_pos {
                gl.enable_vertex_attrib_array(a_pos);
                gl.vertex_attrib_pointer_f32(a_pos, 2, glow::FLOAT, false, 0, 0);
            }
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);

            // Single-channel noise value replicated across RGBA8, which is
            // supported everywhere (unlike `LUMINANCE`, deprecated in
            // desktop core-profile GL, or `RED`/`R8`, unavailable on
            // WebGL1/GLES2) — the shader only reads the `.r` channel.
            let texels = generate_noise_texels(NOISE_SIZE);
            let mut rgba = Vec::with_capacity(texels.len() * 4);
            for t in texels {
                rgba.extend_from_slice(&[t, t, t, t]);
            }
            let noise_texture = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(noise_texture));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::REPEAT as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::REPEAT as i32);
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                NOISE_SIZE as i32,
                NOISE_SIZE as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&rgba)),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);

            gl.use_program(Some(program));
            if let Some(loc) = gl.get_uniform_location(program, "mapSampler") {
                gl.uniform_1_i32(Some(&loc), 0);
            }
            let u_time = gl.get_uniform_location(program, "time");
            let u_dimensions = gl.get_uniform_location(program, "dimensions");
            let u_scale = gl.get_uniform_location(program, "scale");
            gl.use_program(None);

            Ok(Self {
                gl,
                program,
                vao,
                vbo,
                noise_texture,
                u_time,
                u_dimensions,
                u_scale,
            })
        }
    }

    fn draw(&self, time: f32, scale: f32, screen_size_px: [u32; 2]) {
        unsafe {
            let gl = &self.gl;
            gl.use_program(Some(self.program));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.noise_texture));
            gl.uniform_1_f32(self.u_time.as_ref(), time);
            gl.uniform_1_f32(self.u_scale.as_ref(), scale);
            gl.uniform_2_f32(
                self.u_dimensions.as_ref(),
                screen_size_px[0] as f32,
                screen_size_px[1] as f32,
            );
            gl.bind_vertex_array(Some(self.vao));
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.bind_vertex_array(None);
        }
    }
}

impl Drop for GlResources {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_program(self.program);
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_buffer(self.vbo);
            self.gl.delete_texture(self.noise_texture);
        }
    }
}

fn f32_slice_as_bytes(floats: &[f32]) -> &[u8] {
    // SAFETY: `f32` has no padding/invalid bit patterns, and the resulting
    // slice's lifetime and length are derived directly from `floats`.
    unsafe {
        std::slice::from_raw_parts(floats.as_ptr().cast::<u8>(), std::mem::size_of_val(floats))
    }
}

/// Flame screensaver state.
///
/// Create one instance (e.g. as a field of your `eframe::App` struct) and
/// call [`FlameBackground::paint`] every frame from your `ui` method.
pub struct FlameBackground {
    time: f32,
    last_time: Option<f64>,
    gl_resources: Option<Arc<GlResources>>,
    /// Widens the flame's silhouette horizontally. `1.0` is the original
    /// width; larger values make it wider, smaller values narrower. Height,
    /// turbulence pattern, and the background halo are unaffected, so the
    /// flame doesn't grow taller or get cut off at the top. Must be
    /// positive (`0.0` would divide by zero in the shader).
    pub scale: f32,
    /// Multiplies the animation rate. `1.0` is the original speed, `2.0`
    /// doubles it, `0.5` halves it, `0.0` freezes the animation.
    pub speed: f32,
}

impl FlameBackground {
    /// Create a new flame state with the given user-settable parameters.
    pub fn new(scale: f32, speed: f32) -> Self {
        Self {
            time: 0.0,
            last_time: None,
            gl_resources: None,
            scale,
            speed,
        }
    }

    /// Paint the screensaver onto the egui background layer for this frame.
    ///
    /// Call this once per frame **before** drawing any UI panels or windows
    /// so the animation appears behind all other content. `gl` should come
    /// from `eframe::Frame::gl()` — if it's `None` (e.g. the host app uses
    /// the `wgpu` backend instead of `glow`), this draws nothing but a black
    /// background.
    pub fn paint(&mut self, ctx: &Context, gl: Option<&Arc<glow::Context>>) {
        ctx.request_repaint_after(Duration::from_millis(REPAINT_MS));

        let now = ctx.input(|i| i.time);
        let dt = match self.last_time {
            Some(last) => (now - last).clamp(0.0, 0.1) as f32,
            None => 0.0,
        };
        self.last_time = Some(now);
        self.time += dt * TIME_SPEED * self.speed;

        if self.gl_resources.is_none()
            && let Some(gl) = gl
        {
            match GlResources::new(gl.clone()) {
                // On wasm32, `GlResources` (via `glow::Context`) isn't `Sync`
                // — see `AssertSendSync` below, applied where this `Arc` is
                // captured into the `Fn + Sync + Send` paint callback.
                #[allow(clippy::arc_with_non_send_sync)]
                Ok(resources) => self.gl_resources = Some(Arc::new(resources)),
                Err(err) => log::error!("egui-screensaver-flame: GL setup failed: {err}"),
            }
        }

        let screen_rect = ctx.viewport_rect();
        let painter = Painter::new(ctx.clone(), LayerId::background(), screen_rect);
        painter.rect_filled(screen_rect, 0.0, Color32::BLACK);

        if let Some(resources) = self.gl_resources.clone() {
            let time = self.time;
            let scale = self.scale;
            let resources = AssertSendSync(resources);
            painter.add(Shape::Callback(egui::PaintCallback {
                rect: screen_rect,
                callback: Arc::new(CallbackFn::new(move |info, _painter| {
                    resources.get().draw(time, scale, info.screen_size_px);
                })),
            }));
        }
    }
}

/// `CallbackFn` requires `Fn + Sync + Send` on every target, but on wasm32
/// (WebGL) `glow::Context` isn't `Sync` — it wraps `web_sys` handles behind
/// `RefCell`s, since JS objects aren't natively thread-safe. `wasm32`
/// without the `atomics` target feature (which eframe doesn't enable) never
/// actually runs on more than one thread, so this assertion is honest: the
/// wrapped value never crosses a real thread boundary on any target.
struct AssertSendSync<T>(T);
unsafe impl<T> Send for AssertSendSync<T> {}
unsafe impl<T> Sync for AssertSendSync<T> {}

impl<T> AssertSendSync<T> {
    /// A method (rather than direct `.0` field access) so closures capture
    /// this wrapper as a whole — Rust's disjoint closure captures would
    /// otherwise capture just the inner `T` from a bare `resources.0`,
    /// silently defeating the wrapper.
    fn get(&self) -> &T {
        &self.0
    }
}
