# egui-screensaver-flame

Flame screensaver for [egui](https://github.com/emilk/egui).

A windswept fire rendered from a fractal-noise fragment shader, ported from
[Blake Bowen's "Flame in the wind" CodePen](https://codepen.io/osublake/pen/pqNXoq),
itself adapting a shader by Shadertoy user
[@kuvkar](https://www.shadertoy.com/view/4tXXRn).

The original runs the shader on the GPU as a full-screen PixiJS filter. Here
the same per-pixel math is evaluated on the CPU into a small downsampled
buffer (sized to match the window's aspect ratio), then scaled up to fill the
screen.

## Usage

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
egui-screensaver-flame = "0.1"
```

Then call `paint` every frame from your `eframe::App::update` implementation,
before drawing any UI windows:

```rust
use egui_screensaver_flame::FlameBackground;

struct MyApp {
    flame: FlameBackground,
}

impl eframe::App for MyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.flame.paint(&ctx);
        // … rest of your UI …
    }
}
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
