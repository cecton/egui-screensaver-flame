//! Flame screensaver for [egui](https://github.com/emilk/egui).
//!
//! A windswept fire rendered from a fractal-noise fragment shader, ported from
//! [Blake Bowen's "Flame in the wind" CodePen](https://codepen.io/osublake/pen/pqNXoq),
//! itself adapting a shader by Shadertoy user
//! [@kuvkar](https://www.shadertoy.com/view/4tXXRn).
//!
//! The original runs the shader on the GPU as a full-screen PixiJS filter.
//! Here the same per-pixel math is evaluated on the CPU into a small
//! downsampled buffer (sized to match the window's aspect ratio so the flame
//! isn't stretched), then scaled up to fill the screen — the same technique
//! `sea_screensaver` uses for its wave animation.
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
//!     fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
//!         let ctx = ui.ctx().clone();
//!         // Call paint once per frame before drawing any UI windows so the
//!         // screensaver sits on the background layer behind everything else.
//!         self.flame.paint(&ctx);
//!     }
//! }
//! ```

use std::time::Duration;

use egui::{Color32, ColorImage, Context, LayerId, Painter, TextureHandle, TextureOptions};

// ── Tunables ──────────────────────────────────────────────────────────────────

/// Side length of the tileable value-noise texture used for the fractal
/// Brownian motion, in texels.
const NOISE_SIZE: usize = 64;

/// Height of the CPU-rasterized buffer, in pixels. The width is derived from
/// the current window aspect ratio each frame so the flame's proportions
/// never get stretched.
const BUFFER_HEIGHT: usize = 220;
const MIN_BUFFER_WIDTH: usize = 120;
const MAX_BUFFER_WIDTH: usize = 640;

/// Matches the original's `time += 0.1 * delta` running on a ~60 fps Pixi
/// ticker (`delta` ~= 1 at 60 fps), i.e. roughly 6 time-units per second.
const TIME_SPEED: f32 = 6.0;

const REPAINT_MS: u64 = 33;

// ── Small math helpers (mirroring the GLSL source line-for-line) ────────────

/// GLSL `fract(x)`, which is always non-negative — unlike `f32::fract`, which
/// keeps the sign of `x` for negative inputs.
fn glsl_fract(x: f32) -> f32 {
    x - x.floor()
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn mix4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        mix(a[0], b[0], t),
        mix(a[1], b[1], t),
        mix(a[2], b[2], t),
        mix(a[3], b[3], t),
    ]
}

#[allow(clippy::excessive_precision)]
fn rand(x: f32, y: f32) -> f32 {
    glsl_fract((x * 12.9898 + y * 78.233).sin() * 43758.5453)
}

/// `rotz` from the shader: a 2D rotation matrix (built the same way GLSL's
/// column-major `mat2` indexing does) applied to `(x, y)`.
fn rotz(angle: f32, x: f32, y: f32) -> (f32, f32) {
    let (s, c) = angle.sin_cos();
    (x * c + y * s, -x * s + y * c)
}

// ── Tileable value-noise texture (stands in for the original's noise PNG) ───

struct NoiseTexture {
    size: usize,
    texels: Vec<f32>,
}

impl NoiseTexture {
    fn generate() -> Self {
        let size = NOISE_SIZE;
        // Small xorshift64 PRNG so this crate has no dependency beyond egui.
        let mut state = 0x9E3779B97F4A7C15u64;
        let texels = (0..size * size)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 11) as f32 / (1u64 << 53) as f32
            })
            .collect();
        Self { size, texels }
    }

    /// Bilinear-filtered sample with `GL_REPEAT` wrapping, matching the
    /// original's tiled, linearly-filtered noise texture lookup.
    fn sample(&self, u: f32, v: f32) -> f32 {
        let size = self.size;
        let x = glsl_fract(u) * size as f32;
        let y = glsl_fract(v) * size as f32;
        let x0 = x.floor();
        let y0 = y.floor();
        let fx = x - x0;
        let fy = y - y0;
        let wrap = |i: isize| -> usize { i.rem_euclid(size as isize) as usize };
        let texel = |xi: isize, yi: isize| self.texels[wrap(yi) * size + wrap(xi)];
        let (x0, y0) = (x0 as isize, y0 as isize);
        let top = mix(texel(x0, y0), texel(x0 + 1, y0), fx);
        let bottom = mix(texel(x0, y0 + 1), texel(x0 + 1, y0 + 1), fx);
        mix(top, bottom, fy)
    }

    /// Fractal Brownian motion: three octaves of the noise texture, matching
    /// the shader's `fbm()`.
    fn fbm(&self, u: f32, v: f32) -> f32 {
        let mut n = (self.sample(u, v) - 0.5) * 0.5;
        n += (self.sample(u * 2.0, v * 2.0) - 0.5) * 0.25;
        n += (self.sample(u * 3.0, v * 3.0) - 0.5) * 0.125;
        n + 0.5
    }
}

/// Evaluate the flame fragment shader for one pixel, given its position
/// within a `buf_w` x `buf_h` buffer. Returns straight-through-premultiplied
/// RGBA in `[0, 1]`, mirroring `gl_FragColor` at the end of the GLSL source.
fn shade_pixel(
    noise: &NoiseTexture,
    buf_w: f32,
    buf_h: f32,
    px: f32,
    py: f32,
    time: f32,
) -> [f32; 4] {
    let u = (px + 0.5) / buf_w;
    let v = (py + 0.5) / buf_h;

    let mut uv_x = u;
    let mut uv_y = 1.0 - v;
    let unshifted_y = uv_y;

    uv_x -= 0.5;
    uv_y -= 0.5;
    uv_y /= buf_w / buf_h;

    let center_uv_x = uv_x;
    let center_uv_y = uv_y;

    let variation_h = noise.fbm(time * 0.3, time * 0.3) * 1.1;
    let offset_y = -time * 0.05;

    let f = noise.fbm(uv_x * 0.1, uv_y * 0.1 + offset_y);
    let l = (uv_x * uv_x + uv_y * uv_y).sqrt().max(0.1);
    let angle = ((f - 0.5) / l) * smoothstep(-0.2, 0.4, unshifted_y) * 0.45;
    let (rx, ry) = rotz(angle, uv_x, uv_y);
    uv_x += rx;
    uv_y += ry;

    let mut flame = 1.3 - uv_x.abs() * 5.0;

    let mut blueflame = (flame * 0.9).max(0.0).powf(15.0);
    blueflame *= smoothstep(0.2, -1.0, unshifted_y);
    blueflame /= (uv_x * 2.0).abs().max(1e-4);
    blueflame = blueflame.clamp(0.0, 1.0);

    flame *= smoothstep(1.0, variation_h * 0.5, unshifted_y);
    flame = flame.clamp(0.0, 1.0);
    flame = flame.powf(3.0);
    flame /= smoothstep(1.1, -0.1, unshifted_y).max(1e-4);

    let mut col = mix4([1.0, 1.0, 0.0, 0.0], [1.0, 1.0, 0.6, 0.0], flame);
    col = mix4([1.0, 0.0, 0.0, 0.0], col, smoothstep(0.0, 1.6, flame));

    let blue_color = mix4([0.0, 0.0, 1.0, 0.0], col, 0.95);
    col = mix4(col, blue_color, blueflame);

    col = [
        col[0] * flame,
        col[1] * flame,
        col[2] * flame,
        col[3] * flame,
    ];
    col[3] = flame;

    let halo_size = 0.5;
    let (hx, hy) = (center_uv_x, center_uv_y + 0.1);
    let center_l = 1.0 - (hx * hx + hy * hy).sqrt() / halo_size;
    let fbm_halo = noise.fbm(time * 0.035, time * 0.035);
    let halo = [
        0.8 * fbm_halo * center_l + 0.02,
        0.3 * fbm_halo * center_l + 0.02,
        0.3 * fbm_halo * center_l + 0.02,
        0.02,
    ];
    col = mix4(halo, col, col[3]);

    let grain = mix(rand(uv_x, uv_y) + rand(uv_x * 0.45, uv_y * 0.45), 1.0, 0.9);
    col = [
        (col[0] * grain).clamp(0.0, 1.0),
        (col[1] * grain).clamp(0.0, 1.0),
        (col[2] * grain).clamp(0.0, 1.0),
        (col[3] * grain).clamp(0.0, 1.0),
    ];

    col
}

// ── Public screensaver struct ─────────────────────────────────────────────────

/// Flame screensaver state.
///
/// Create one instance (e.g. as a field of your `eframe::App` struct) and
/// call [`FlameBackground::paint`] every frame from your `update` method.
pub struct FlameBackground {
    noise: NoiseTexture,
    time: f32,
    last_time: Option<f64>,
    buf_size: (usize, usize),
    pixels: Vec<Color32>,
    texture: Option<TextureHandle>,
}

impl Default for FlameBackground {
    fn default() -> Self {
        Self {
            noise: NoiseTexture::generate(),
            time: 0.0,
            last_time: None,
            buf_size: (0, 0),
            pixels: Vec::new(),
            texture: None,
        }
    }
}

impl FlameBackground {
    /// Paint the screensaver onto the egui background layer for this frame.
    ///
    /// Call this once per frame **before** drawing any UI panels or windows
    /// so the animation appears behind all other content.
    pub fn paint(&mut self, ctx: &Context) {
        ctx.request_repaint_after(Duration::from_millis(REPAINT_MS));

        let now = ctx.input(|i| i.time);
        let dt = match self.last_time {
            Some(last) => (now - last).clamp(0.0, 0.1) as f32,
            None => 0.0,
        };
        self.last_time = Some(now);
        self.time += dt * TIME_SPEED;

        let screen_rect = ctx.viewport_rect();
        let aspect = (screen_rect.width() / screen_rect.height().max(1.0)).max(0.01);
        let buf_h = BUFFER_HEIGHT;
        let buf_w = ((buf_h as f32) * aspect)
            .round()
            .clamp(MIN_BUFFER_WIDTH as f32, MAX_BUFFER_WIDTH as f32) as usize;

        if self.buf_size != (buf_w, buf_h) {
            self.buf_size = (buf_w, buf_h);
            self.pixels = vec![Color32::BLACK; buf_w * buf_h];
        }

        for py in 0..buf_h {
            for px in 0..buf_w {
                let c = shade_pixel(
                    &self.noise,
                    buf_w as f32,
                    buf_h as f32,
                    px as f32,
                    py as f32,
                    self.time,
                );
                self.pixels[py * buf_w + px] = Color32::from_rgba_premultiplied(
                    (c[0] * 255.0).round() as u8,
                    (c[1] * 255.0).round() as u8,
                    (c[2] * 255.0).round() as u8,
                    (c[3] * 255.0).round() as u8,
                );
            }
        }

        let image = ColorImage::new([buf_w, buf_h], self.pixels.clone());
        match &mut self.texture {
            Some(texture) => texture.set(image, TextureOptions::LINEAR),
            None => self.texture = Some(ctx.load_texture("flame", image, TextureOptions::LINEAR)),
        }

        let painter = Painter::new(ctx.clone(), LayerId::background(), screen_rect);
        painter.rect_filled(screen_rect, 0.0, Color32::BLACK);
        if let Some(texture) = &self.texture {
            painter.image(
                texture.id(),
                screen_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
    }
}
