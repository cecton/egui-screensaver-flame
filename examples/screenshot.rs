//! Dev tool: render the screensaver for a few frames, save an in-app GPU
//! screenshot to the path given as the first argument, then exit. Used to
//! check rendering where compositor capture is unavailable.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use egui_screensaver_flame::FlameBackground;

    struct App {
        flame: FlameBackground,
        path: String,
        frames: u32,
    }

    impl eframe::App for App {
        fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
            let ctx = ui.ctx().clone();
            self.flame.paint(&ctx, frame.wgpu_render_state());
            self.frames += 1;
            if self.frames == 30 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            ctx.request_repaint();
            let image = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(image) = image {
                let [w, h] = image.size;
                let mut out = image::RgbaImage::new(w as u32, h as u32);
                for (i, px) in image.pixels.iter().enumerate() {
                    let [r, g, b, a] = px.to_srgba_unmultiplied();
                    out.put_pixel((i % w) as u32, (i / w) as u32, image::Rgba([r, g, b, a]));
                }
                out.save(&self.path).expect("failed to save screenshot");
                println!("saved {}", self.path);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "screenshot.png".to_owned());
    eframe::run_native(
        "egui-screensaver-flame screenshot",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([960.0, 600.0]),
            ..Default::default()
        },
        Box::new(move |_cc| {
            Ok(Box::new(App {
                flame: FlameBackground::new(1.0, 1.0),
                path,
                frames: 0,
            }))
        }),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {}
