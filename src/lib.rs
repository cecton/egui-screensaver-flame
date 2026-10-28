//! Flame screensaver for [egui](https://github.com/emilk/egui).
//!
//! A windswept fire rendered from a fractal-noise fragment shader, ported from
//! [Blake Bowen's "Flame in the wind" CodePen](https://codepen.io/osublake/pen/pqNXoq),
//! itself adapting a shader by Shadertoy user
//! [@kuvkar](https://www.shadertoy.com/view/4tXXRn).
//!
//! Like the original, this runs the shader on the GPU: a fullscreen triangle
//! is drawn through an [`egui::PaintCallback`] using [`egui_wgpu`], so the
//! per-pixel math runs in parallel across the GPU's shader cores instead of
//! serially on the CPU. **This requires the host `eframe::App` to use the
//! `wgpu` rendering backend** (not `glow`) — [`FlameBackground::paint`]
//! simply skips drawing if no [`egui_wgpu::RenderState`] is available.
//!
//! The pipeline targets the render pass eframe sets up by default: no depth
//! buffer and no multisampling.
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
//!         self.flame.paint(&ctx, frame.wgpu_render_state());
//!     }
//! }
//! ```

use std::sync::Arc;
use std::time::Duration;

use egui::{Color32, Context, LayerId, Painter, Shape};
use egui_wgpu::wgpu::util::DeviceExt as _;
use egui_wgpu::{CallbackTrait, RenderState, ScreenDescriptor, wgpu};

/// Side length of the tileable value-noise texture used for the fractal
/// Brownian motion, in texels.
const NOISE_SIZE: usize = 64;

/// Matches the original's `time += 0.1 * delta` running on a ~60 fps Pixi
/// ticker (`delta` ~= 1 at 60 fps), i.e. roughly 6 time-units per second.
const TIME_SPEED: f32 = 6.0;

const REPAINT_MS: u64 = 33;

const SHADER_SRC: &str = include_str!("shader/flame.wgsl");

/// Size in bytes of the shader's `Params` uniform: `time`, `scale`, then
/// `dimensions` (a `vec2`, 8-byte aligned, so it lands at offset 8).
const PARAMS_SIZE: u64 = 16;

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

/// Compiled GPU resources for the flame shader: pipeline, uniform buffer and
/// the one-time noise texture. Built once, lazily, on the first frame a
/// [`RenderState`] is available (see [`FlameBackground::paint`]).
struct GpuResources {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl GpuResources {
    fn new(render_state: &RenderState) -> Self {
        let device = &render_state.device;

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("egui-screensaver-flame shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("egui-screensaver-flame params"),
            size: PARAMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let noise_texture = device.create_texture_with_data(
            &render_state.queue,
            &wgpu::TextureDescriptor {
                label: Some("egui-screensaver-flame noise"),
                size: wgpu::Extent3d {
                    width: NOISE_SIZE as u32,
                    height: NOISE_SIZE as u32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &generate_noise_texels(NOISE_SIZE),
        );
        let noise_view = noise_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("egui-screensaver-flame noise sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("egui-screensaver-flame layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(PARAMS_SIZE),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("egui-screensaver-flame bind group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&noise_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("egui-screensaver-flame pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("egui-screensaver-flame pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: render_state.target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            uniforms,
            bind_group,
        }
    }
}

/// The per-frame [`CallbackTrait`] handed to egui: uploads this frame's
/// uniforms in `prepare` and draws the fullscreen triangle in `paint`.
struct FlameCallback {
    gpu: Arc<GpuResources>,
    time: f32,
    scale: f32,
}

impl CallbackTrait for FlameCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        _callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let [width, height] = screen_descriptor.size_in_pixels;
        let params = [self.time, self.scale, width as f32, height as f32];
        let mut bytes = [0u8; PARAMS_SIZE as usize];
        for (dst, value) in bytes
            .iter_mut()
            .zip(params.iter().flat_map(|v| v.to_ne_bytes()))
        {
            *dst = value;
        }
        queue.write_buffer(&self.gpu.uniforms, 0, &bytes);
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        _callback_resources: &egui_wgpu::CallbackResources,
    ) {
        render_pass.set_pipeline(&self.gpu.pipeline);
        render_pass.set_bind_group(0, &self.gpu.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

/// Flame screensaver state.
///
/// Create one instance (e.g. as a field of your `eframe::App` struct) and
/// call [`FlameBackground::paint`] every frame from your `ui` method.
pub struct FlameBackground {
    time: f32,
    last_time: Option<f64>,
    gpu: Option<Arc<GpuResources>>,
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
            gpu: None,
            scale,
            speed,
        }
    }

    /// Paint the screensaver onto the egui background layer for this frame.
    ///
    /// Call this once per frame **before** drawing any UI panels or windows
    /// so the animation appears behind all other content. `render_state`
    /// should come from `eframe::Frame::wgpu_render_state()` — if it's
    /// `None` (e.g. the host app uses the `glow` backend instead of `wgpu`),
    /// this draws nothing but a black background.
    pub fn paint(&mut self, ctx: &Context, render_state: Option<&RenderState>) {
        ctx.request_repaint_after(Duration::from_millis(REPAINT_MS));

        let now = ctx.input(|i| i.time);
        let dt = match self.last_time {
            Some(last) => (now - last).clamp(0.0, 0.1) as f32,
            None => 0.0,
        };
        self.last_time = Some(now);
        self.time += dt * TIME_SPEED * self.speed;

        if self.gpu.is_none()
            && let Some(render_state) = render_state
        {
            self.gpu = Some(Arc::new(GpuResources::new(render_state)));
        }

        let screen_rect = ctx.viewport_rect();
        let painter = Painter::new(ctx.clone(), LayerId::background(), screen_rect);
        painter.rect_filled(screen_rect, 0.0, Color32::BLACK);

        if let Some(gpu) = self.gpu.clone() {
            painter.add(Shape::Callback(egui_wgpu::Callback::new_paint_callback(
                screen_rect,
                FlameCallback {
                    gpu,
                    time: self.time,
                    scale: self.scale,
                },
            )));
        }
    }
}
