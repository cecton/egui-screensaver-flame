[![crates.io](https://img.shields.io/crates/v/egui-screensaver-flame.svg)](https://crates.io/crates/egui-screensaver-flame)
[![docs.rs](https://docs.rs/egui-screensaver-flame/badge.svg)](https://docs.rs/egui-screensaver-flame)
[![Rust version](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![dependency status](https://deps.rs/repo/github/cecton/egui-screensaver-flame/status.svg)](https://deps.rs/repo/github/cecton/egui-screensaver-flame)
[![CI](https://github.com/cecton/egui-screensaver-flame/actions/workflows/ci.yml/badge.svg)](https://github.com/cecton/egui-screensaver-flame/actions/workflows/ci.yml)
[![demo](https://img.shields.io/badge/demo-live-blue)](https://cecton.github.io/egui-screensaver-flame/)

# egui-screensaver-flame

Flame screensaver for [egui](https://github.com/emilk/egui).

A windswept fire rendered from a fractal-noise fragment shader, ported from
[Blake Bowen's "Flame in the wind" CodePen](https://codepen.io/osublake/pen/pqNXoq),
itself adapting a shader by Shadertoy user
[@kuvkar](https://www.shadertoy.com/view/4tXXRn).

Like the original, this runs the shader on the GPU — via an
[`egui::PaintCallback`] through [`egui_wgpu`] — so the per-pixel math runs in
parallel across the GPU's shader cores instead of serially on the CPU. This
means the host `eframe::App` must use the `wgpu` rendering backend (not
`glow`).

## Usage

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
egui-screensaver-flame = "0.5"
```

Then call `paint` every frame from your `eframe::App::ui` implementation,
before drawing any UI windows:

```rust
use egui_screensaver_flame::FlameBackground;

struct MyApp {
    flame: FlameBackground,
}

impl eframe::App for MyApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.flame.paint(&ctx, frame.wgpu_render_state());
        // … rest of your UI …
    }
}
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
