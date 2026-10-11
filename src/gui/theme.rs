//! Look and feel, matching the OpenDS5 companion app: dark navy surfaces,
//! thin borders, blue accent, Inter text and Lucide icons.

use std::sync::Arc;

use eframe::egui::{
    self, Align, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame,
    Layout, Margin, Response, RichText, Sense, Stroke, TextStyle, Ui, Vec2,
};

pub const BG: Color32 = Color32::from_rgb(7, 11, 18);
pub const SIDEBAR: Color32 = Color32::from_rgb(5, 11, 19);
pub const CARD: Color32 = Color32::from_rgb(6, 13, 22);
pub const CARD_HEADER: Color32 = Color32::from_rgb(5, 11, 19);
pub const CONTROL: Color32 = Color32::from_rgb(9, 18, 29);
pub const CONTROL_HOVER: Color32 = Color32::from_rgb(14, 26, 42);
pub const BORDER: Color32 = Color32::from_rgb(31, 40, 54);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(45, 56, 73);
pub const DIVIDER: Color32 = Color32::from_rgb(23, 31, 43);

pub const TEXT: Color32 = Color32::from_rgb(247, 249, 255);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(193, 202, 221);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(159, 177, 201);
pub const TEXT_DIM: Color32 = Color32::from_rgb(132, 146, 166);

pub const ACCENT: Color32 = Color32::from_rgb(40, 124, 255);
pub const ACCENT_STRONG: Color32 = Color32::from_rgb(61, 155, 255);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(13, 33, 64);
pub const ACCENT_BORDER: Color32 = Color32::from_rgb(28, 74, 150);
pub const NAV_ACTIVE: Color32 = Color32::from_rgb(17, 45, 92);

pub const SUCCESS: Color32 = Color32::from_rgb(85, 215, 108);
pub const SUCCESS_BORDER: Color32 = Color32::from_rgb(38, 110, 55);
pub const WARNING: Color32 = Color32::from_rgb(255, 211, 106);
pub const WARNING_BORDER: Color32 = Color32::from_rgb(120, 96, 44);
pub const DANGER: Color32 = Color32::from_rgb(255, 127, 147);
pub const DANGER_BORDER: Color32 = Color32::from_rgb(130, 50, 62);

pub const RADIUS: u8 = 4;
pub const CONTROL_RADIUS: u8 = 3;

/// Lucide icon glyphs (codepoints from lucide-static's codepoints.json).
#[allow(dead_code)] // a palette: not every icon is used on every platform
pub mod icon {
    pub const DASHBOARD: &str = "\u{e1c1}";
    pub const ACTIVITY: &str = "\u{e038}";
    pub const SPEAKER: &str = "\u{e166}";
    pub const VOLUME: &str = "\u{e1ab}";
    pub const MONITOR: &str = "\u{e11d}";
    pub const MONITOR_COG: &str = "\u{e603}";
    pub const KEYBOARD: &str = "\u{e284}";
    pub const SETTINGS: &str = "\u{e154}";
    pub const POWER: &str = "\u{e140}";
    pub const RADIO: &str = "\u{e142}";
    pub const AUDIO_LINES: &str = "\u{e55a}";
    pub const TV: &str = "\u{e195}";
    pub const CABLE: &str = "\u{e4e3}";
    pub const SLIDERS: &str = "\u{e29a}";
    pub const ZAP: &str = "\u{e1b4}";
    pub const CHECK: &str = "\u{e226}";
    pub const ALERT: &str = "\u{e077}";
    pub const WARN: &str = "\u{e193}";
    pub const REFRESH: &str = "\u{e145}";
    pub const SAVE: &str = "\u{e14d}";
    pub const UNDO: &str = "\u{e2a1}";
    pub const SEARCH: &str = "\u{e151}";
    pub const PLAY: &str = "\u{e13c}";
    pub const LAYERS: &str = "\u{e529}";
    pub const HOUSE: &str = "\u{e0f5}";
    pub const INFO: &str = "\u{e0f9}";
    pub const HDMI: &str = "\u{e4e7}";
    pub const WAVEFORM: &str = "\u{e55b}";
    pub const LINK: &str = "\u{e102}";
    pub const PLUG: &str = "\u{e37f}";
    pub const CLOCK: &str = "\u{e087}";
    pub const GAUGE: &str = "\u{e1bf}";
    pub const PLUS: &str = "\u{e13d}";
    pub const TRASH: &str = "\u{e18e}";
    pub const UP: &str = "\u{e04a}";
    pub const DOWN: &str = "\u{e042}";
    pub const CHEVRON_LEFT: &str = "\u{e06e}";
    pub const CHEVRON_RIGHT: &str = "\u{e06f}";
    pub const SUN: &str = "\u{e2b1}";
}

pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let add = |fonts: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        fonts
            .font_data
            .insert(name.to_string(), Arc::new(FontData::from_static(bytes)));
    };
    add(
        &mut fonts,
        "inter",
        include_bytes!("../../assets/fonts/Inter-Regular.ttf"),
    );
    add(
        &mut fonts,
        "inter-semibold",
        include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"),
    );
    add(
        &mut fonts,
        "inter-bold",
        include_bytes!("../../assets/fonts/Inter-Bold.ttf"),
    );
    add(
        &mut fonts,
        "lucide",
        include_bytes!("../../assets/fonts/lucide-subset.ttf"),
    );

    // Keep egui's own fonts as fallbacks (symbols, emoji).
    let fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let family = |primary: &str| {
        // Lucide first: Inter has its own glyphs in part of the private-use
        // area that would otherwise shadow the icons.
        let mut v = vec!["lucide".to_string(), primary.to_string()];
        v.extend(fallbacks.iter().cloned());
        v
    };
    fonts
        .families
        .insert(FontFamily::Proportional, family("inter"));
    fonts.families.insert(semibold(), family("inter-semibold"));
    fonts.families.insert(bold(), family("inter-bold"));
    if let Some(mono) = fonts.families.get_mut(&FontFamily::Monospace) {
        mono.push("lucide".into());
    }
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.text_styles = [
            (TextStyle::Heading, FontId::new(22.0, bold())),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(13.5, semibold())),
            (
                TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
        ]
        .into();
        let s = &mut style.spacing;
        s.item_spacing = Vec2::new(8.0, 8.0);
        s.button_padding = Vec2::new(12.0, 6.0);
        s.interact_size = Vec2::new(40.0, 30.0);
        s.combo_height = 320.0;

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.panel_fill = BG;
        v.window_fill = Color32::from_rgb(8, 17, 28);
        v.window_stroke = Stroke::new(1.0, BORDER_STRONG);
        v.window_corner_radius = CornerRadius::same(RADIUS);
        v.menu_corner_radius = CornerRadius::same(RADIUS);
        v.extreme_bg_color = Color32::from_rgb(4, 9, 15);
        v.faint_bg_color = Color32::from_rgb(9, 16, 26);
        v.code_bg_color = CONTROL;
        v.hyperlink_color = ACCENT_STRONG;
        v.warn_fg_color = WARNING;
        v.error_fg_color = DANGER;
        v.selection.bg_fill = ACCENT;
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        v.text_cursor.stroke = Stroke::new(2.0, ACCENT_STRONG);

        let r = CornerRadius::same(CONTROL_RADIUS);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = CARD;
        w.noninteractive.weak_bg_fill = CARD;
        w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_SECONDARY);
        w.noninteractive.corner_radius = r;

        w.inactive.bg_fill = CONTROL;
        w.inactive.weak_bg_fill = CONTROL;
        w.inactive.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
        w.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(238, 244, 255));
        w.inactive.corner_radius = r;

        w.hovered.bg_fill = CONTROL_HOVER;
        w.hovered.weak_bg_fill = CONTROL_HOVER;
        w.hovered.bg_stroke = Stroke::new(1.0, ACCENT_BORDER);
        w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        w.hovered.corner_radius = r;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = ACCENT;
        w.active.weak_bg_fill = ACCENT;
        w.active.bg_stroke = Stroke::new(1.0, ACCENT_STRONG);
        w.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        w.active.corner_radius = r;
        w.active.expansion = 0.0;

        w.open.bg_fill = CONTROL_HOVER;
        w.open.weak_bg_fill = CONTROL_HOVER;
        w.open.bg_stroke = Stroke::new(1.0, ACCENT_BORDER);
        w.open.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        w.open.corner_radius = r;
    });
}

// ---------------------------------------------------------------- widgets

/// Page title + one-line description, with optional widgets on the right.
pub fn page_header(ui: &mut Ui, title: &str, subtitle: &str, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(
                RichText::new(title)
                    .font(FontId::new(24.0, bold()))
                    .color(TEXT),
            );
            ui.add_space(-2.0);
            ui.label(RichText::new(subtitle).size(14.0).color(TEXT_SECONDARY));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), right);
    });
    ui.add_space(14.0);
}

/// Small coloured "● text" pill, like "All systems normal".
pub fn status_pill(ui: &mut Ui, text: &str, color: Color32, border: Color32) {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::symmetric(12, 7))
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("●  {text}"))
                    .font(FontId::new(13.5, semibold()))
                    .color(color),
            );
        });
}

/// A bordered card with an icon tile, a title and optional subtitle.
pub fn card<R>(
    ui: &mut Ui,
    icon: &str,
    title: &str,
    subtitle: Option<&str>,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    card_with(ui, icon, title, subtitle, |_| {}, add)
}

/// Like `card`, with extra widgets right-aligned in the header.
pub fn card_with<R>(
    ui: &mut Ui,
    icon: &str,
    title: &str,
    subtitle: Option<&str>,
    header_right: impl FnOnce(&mut Ui),
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    card_sized(ui, icon, title, subtitle, 0.0, header_right, add)
}

/// Height of one `kv` row, for sizing cards that should line up.
pub const ROW_HEIGHT: f32 = 38.0;

/// A card whose body is at least `min_body` tall, so cards side by side
/// line up even when one has fewer rows.
pub fn card_sized<R>(
    ui: &mut Ui,
    icon: &str,
    title: &str,
    subtitle: Option<&str>,
    min_body: f32,
    header_right: impl FnOnce(&mut Ui),
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(RADIUS))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            Frame::new()
                .fill(CARD_HEADER)
                .inner_margin(Margin::symmetric(14, 10))
                .corner_radius(CornerRadius {
                    nw: RADIUS,
                    ne: RADIUS,
                    sw: 0,
                    se: 0,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        icon_tile(ui, icon, ACCENT_STRONG);
                        ui.add_space(4.0);
                        ui.vertical(|ui| {
                            if subtitle.is_none() {
                                ui.add_space(8.0);
                            }
                            ui.label(
                                RichText::new(title)
                                    .font(FontId::new(16.0, bold()))
                                    .color(TEXT),
                            );
                            if let Some(sub) = subtitle {
                                ui.add_space(-4.0);
                                ui.label(RichText::new(sub).size(12.5).color(TEXT_SECONDARY));
                            }
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), header_right);
                    });
                });
            let rect = ui.cursor();
            ui.painter()
                .hline(rect.x_range(), rect.top(), Stroke::new(1.0, DIVIDER));
            Frame::new()
                .inner_margin(Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(min_body);
                    add(ui)
                })
                .inner
        })
        .inner
}

/// The 40px bordered square holding a card's icon.
pub fn icon_tile(ui: &mut Ui, icon: &str, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(RADIUS), CONTROL);
    p.rect_stroke(
        rect,
        CornerRadius::same(RADIUS),
        Stroke::new(1.0, BORDER_STRONG),
        egui::StrokeKind::Inside,
    );
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        FontId::new(19.0, FontFamily::Proportional),
        color,
    );
}

/// "Key ......... value" row with a divider underneath.
pub fn kv(ui: &mut Ui, key: &str, value: impl Into<RichText>) {
    kv_colored(ui, key, value, TEXT);
}

pub fn kv_colored(ui: &mut Ui, key: &str, value: impl Into<RichText>, color: Color32) {
    ui.horizontal(|ui| {
        ui.set_min_height(24.0);
        ui.label(RichText::new(key).size(14.0).color(TEXT_SECONDARY));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add(
                egui::Label::new(
                    value
                        .into()
                        .font(FontId::new(14.0, semibold()))
                        .color(color),
                )
                .truncate(),
            );
        });
    });
    let y = ui.cursor().top() - 3.0;
    let x = ui.max_rect().x_range();
    ui.painter().hline(x, y, Stroke::new(1.0, DIVIDER));
}

/// Section heading inside a card or window: accent icon + semibold title.
pub fn subheading(ui: &mut Ui, icon: &str, text: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon).size(16.0).color(ACCENT_STRONG));
        ui.label(
            RichText::new(text)
                .font(FontId::new(15.5, bold()))
                .color(TEXT),
        );
    });
    ui.add_space(2.0);
}

/// Uppercase sidebar section label ("DEVICE", "PAGES").
pub fn section_label(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .font(FontId::new(11.0, bold()))
            .color(TEXT_DIM)
            .extra_letter_spacing(1.6),
    );
}

/// Sidebar navigation entry; the active one gets the filled pill and rail.
pub fn nav_item(ui: &mut Ui, icon: &str, label: &str, active: bool) -> Response {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::click());
    let p = ui.painter();
    let r = CornerRadius::same(RADIUS);
    if active {
        p.rect_filled(rect, r, NAV_ACTIVE);
        p.rect_stroke(
            rect,
            r,
            Stroke::new(1.0, ACCENT_BORDER),
            egui::StrokeKind::Inside,
        );
        let rail = egui::Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height()));
        p.rect_filled(
            rail,
            CornerRadius {
                nw: RADIUS,
                sw: RADIUS,
                ne: 0,
                se: 0,
            },
            ACCENT,
        );
    } else if resp.hovered() {
        p.rect_filled(rect, r, ACCENT_SOFT);
    }
    let (icon_color, text_color, font) = if active {
        (ACCENT_STRONG, Color32::WHITE, bold())
    } else if resp.hovered() {
        (TEXT, TEXT, semibold())
    } else {
        (TEXT_SECONDARY, TEXT_SECONDARY, semibold())
    };
    p.text(
        rect.left_center() + Vec2::new(22.0, 0.0),
        egui::Align2::CENTER_CENTER,
        icon,
        FontId::new(18.0, FontFamily::Proportional),
        icon_color,
    );
    p.text(
        rect.left_center() + Vec2::new(44.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::new(14.5, font),
        text_color,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Two-line selectable list row (title + muted subtitle).
pub fn list_item(ui: &mut Ui, title: &str, subtitle: &str, selected: bool) -> Response {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 48.0), Sense::click());
    let p = ui.painter();
    let r = CornerRadius::same(RADIUS);
    if selected {
        p.rect_filled(rect, r, NAV_ACTIVE);
        p.rect_stroke(
            rect,
            r,
            Stroke::new(1.0, ACCENT_BORDER),
            egui::StrokeKind::Inside,
        );
    } else if resp.hovered() {
        p.rect_filled(rect, r, ACCENT_SOFT);
    }
    let x = rect.left() + 12.0;
    p.text(
        egui::pos2(x, rect.top() + 15.0),
        egui::Align2::LEFT_CENTER,
        title,
        FontId::new(14.0, semibold()),
        if selected { Color32::WHITE } else { TEXT },
    );
    p.text(
        egui::pos2(x, rect.top() + 33.0),
        egui::Align2::LEFT_CENTER,
        subtitle,
        FontId::new(12.5, FontFamily::Proportional),
        if selected { TEXT_SECONDARY } else { TEXT_DIM },
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A button that renders as selected (accent fill) when `selected`.
pub fn choice(ui: &mut Ui, label: impl Into<String>, selected: bool) -> Response {
    let label: String = label.into();
    let text = if selected {
        RichText::new(label).color(Color32::WHITE)
    } else {
        RichText::new(label)
    };
    let mut button = egui::Button::new(text).min_size(Vec2::new(0.0, 30.0));
    if selected {
        button = button.fill(ACCENT).stroke(Stroke::new(1.0, ACCENT_STRONG));
    }
    ui.add(button)
}

/// Accent-filled main action button.
pub fn primary_button(ui: &mut Ui, label: impl Into<String>) -> Response {
    ui.add(
        egui::Button::new(RichText::new(label.into()).color(Color32::WHITE))
            .fill(ACCENT)
            .stroke(Stroke::new(1.0, ACCENT_STRONG))
            .min_size(Vec2::new(0.0, 32.0)),
    )
}

/// Bordered secondary button.
pub fn button(ui: &mut Ui, label: impl Into<String>) -> Response {
    ui.add(egui::Button::new(label.into()).min_size(Vec2::new(0.0, 32.0)))
}

/// Lays out `n` equal-width columns with the standard card gap.
pub fn columns<R>(ui: &mut Ui, n: usize, add: impl FnOnce(&mut [Ui]) -> R) -> R {
    ui.style_mut().spacing.item_spacing.x = 12.0;
    ui.columns(n, add)
}
