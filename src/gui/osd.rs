//! Fallback volume overlay (used where the desktop has no OSD service we can
//! drive, e.g. Windows). Runs as `avrcontrol osd`, reads "<percent> <text>"
//! lines from stdin and exits after a short idle period.

use std::io::BufRead;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;

const HIDE_AFTER: Duration = Duration::from_millis(2000);

#[derive(Clone)]
struct State {
    percent: u8,
    text: String,
    updated: Instant,
}

struct Osd {
    state: Arc<Mutex<State>>,
}

impl eframe::App for Osd {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let s = self.state.lock().unwrap().clone();
        if s.updated.elapsed() > HIDE_AFTER {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        ui.ctx().request_repaint_after(Duration::from_millis(100));

        egui::Frame::new()
            .fill(ui.visuals().window_fill.gamma_multiply(0.95))
            .corner_radius(16.0)
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("Onkyo volume").size(15.0));
                    let fraction = s.percent as f32 / 100.0;
                    ui.add(egui::ProgressBar::new(fraction).desired_width(250.0));
                    let text = s.text.clone();
                    ui.label(egui::RichText::new(text).size(20.0).strong());
                });
            });
    }
}

pub fn run() -> anyhow::Result<()> {
    let state = Arc::new(Mutex::new(State {
        percent: 0,
        text: String::new(),
        updated: Instant::now(),
    }));

    let reader_state = state.clone();
    let ctx_slot: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));
    let reader_ctx = ctx_slot.clone();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let Some((percent, text)) = line.split_once(' ') else {
                continue;
            };
            if let Ok(percent) = percent.parse::<u8>() {
                *reader_state.lock().unwrap() = State {
                    percent: percent.min(100),
                    text: text.trim().to_string(),
                    updated: Instant::now(),
                };
                if let Some(ctx) = reader_ctx.lock().unwrap().as_ref() {
                    ctx.request_repaint();
                }
            }
        }
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(format!("{}.osd", crate::config::APP_ID))
            .with_title("AVR Control volume")
            .with_inner_size([320.0, 110.0])
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_taskbar(false)
            .with_always_on_top()
            .with_active(false),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "avrcontrol-osd",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            *ctx_slot.lock().unwrap() = Some(cc.egui_ctx.clone());
            Ok(Box::new(Osd { state }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
