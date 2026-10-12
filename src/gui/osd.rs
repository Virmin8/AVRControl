//! Volume overlay in the front-panel style, used on every platform. Runs as
//! `avrcontrol osd`, reads "<percent> <text>" lines from stdin and exits after
//! a short idle period. The parent stops writing to it before then (see
//! feedback.rs), so a value is never sent to a window that is closing.

use std::io::BufRead;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, CornerRadius, FontId, Pos2, Rect, Stroke, Vec2};

use crate::gui::theme;

pub const HIDE_AFTER: Duration = Duration::from_millis(2000);

/// Segments in the level bar.
const SEGMENTS: usize = 32;

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
            // Exit outright: a closed window whose process lingers would
            // swallow the next value.
            std::process::exit(0);
        }
        ui.ctx().request_repaint_after(Duration::from_millis(100));

        let rect = ui.max_rect().shrink(1.0);
        let p = ui.painter();
        let r = CornerRadius::same(14);
        p.rect_filled(rect, r, theme::VFD_BG);
        p.rect_stroke(
            rect,
            r,
            Stroke::new(1.0, theme::VFD_EDGE),
            egui::StrokeKind::Inside,
        );
        let inner = rect.shrink2(Vec2::new(20.0, 16.0));

        let label = theme::vfd_text(p, "VOLUME", FontId::new(11.0, theme::bold()), 1.6);
        theme::glow(
            p,
            inner.left_top(),
            Align2::LEFT_TOP,
            &label,
            theme::TEXT_DIM,
            false,
        );

        let muted = s.text == "muted";
        let (value, color) = if muted {
            ("MUTE".to_string(), theme::VFD.gamma_multiply(0.45))
        } else {
            (s.text.clone(), theme::VFD)
        };
        let big = theme::vfd_text(p, &value, FontId::new(34.0, theme::mono_bold()), 1.0);
        theme::glow(
            p,
            inner.right_top() + Vec2::new(0.0, -6.0),
            Align2::RIGHT_TOP,
            &big,
            color,
            !muted,
        );

        // Level bar: lit segments up to the volume, like a VFD meter.
        let bar_h = 12.0;
        let gap = 3.0;
        let top = inner.bottom() - bar_h;
        let seg_w = (inner.width() - gap * (SEGMENTS - 1) as f32) / SEGMENTS as f32;
        let lit = if muted {
            0
        } else {
            ((s.percent as f32 / 100.0) * SEGMENTS as f32).round() as usize
        };
        for i in 0..SEGMENTS {
            let x = inner.left() + i as f32 * (seg_w + gap);
            let seg = Rect::from_min_size(Pos2::new(x, top), Vec2::new(seg_w, bar_h));
            if i < lit {
                p.rect_filled(
                    seg.expand(1.5),
                    CornerRadius::same(2),
                    theme::VFD.gamma_multiply(0.15),
                );
                p.rect_filled(seg, CornerRadius::same(1), theme::VFD);
            } else {
                p.rect_filled(seg, CornerRadius::same(1), theme::VFD_OFF);
            }
        }
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
            .with_inner_size([340.0, 104.0])
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_taskbar(false)
            .with_always_on_top()
            .with_mouse_passthrough(true)
            .with_active(false),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "avrcontrol-osd",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            *ctx_slot.lock().unwrap() = Some(cc.egui_ctx.clone());
            Ok(Box::new(Osd { state }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
