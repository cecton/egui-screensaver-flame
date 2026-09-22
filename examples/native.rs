#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use egui_screensaver_flame::FlameBackground;

    struct App {
        flame: FlameBackground,
    }

    impl Default for App {
        fn default() -> Self {
            Self {
                flame: FlameBackground::new(1.0, 1.0),
            }
        }
    }

    impl eframe::App for App {
        fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
            let ctx = ui.ctx().clone();
            self.flame.paint(&ctx, frame.gl());
        }
    }

    eframe::run_native(
        "egui-screensaver-flame demo",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {}
