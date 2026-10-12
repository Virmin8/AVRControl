//! Look and feel: "Front Panel". Near-black surfaces like the receiver's
//! chassis, an amber accent taken from its VFD display, Inter text, JetBrains
//! Mono for readouts and Lucide icons.

use std::sync::Arc;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId,
    Frame, Layout, Margin, Painter, Pos2, Rect, Response, RichText, Sense, Stroke, TextStyle, Ui,
    Vec2,
};

pub const BG: Color32 = Color32::from_rgb(11, 11, 12);
pub const SIDEBAR: Color32 = Color32::from_rgb(7, 7, 8);
pub const CARD: Color32 = Color32::from_rgb(20, 20, 22);
pub const CONTROL: Color32 = Color32::from_rgb(27, 27, 30);
pub const CONTROL_HOVER: Color32 = Color32::from_rgb(34, 34, 38);
pub const BORDER: Color32 = Color32::from_rgb(38, 38, 42);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(44, 44, 49);
pub const DIVIDER: Color32 = Color32::from_rgb(30, 30, 34);
/// The dark lower edge that makes keys and buttons look raised.
pub const KEY_SHADOW: Color32 = Color32::from_rgb(14, 14, 16);

pub const TEXT: Color32 = Color32::from_rgb(236, 236, 238);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(163, 163, 170);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(138, 138, 146);
pub const TEXT_DIM: Color32 = Color32::from_rgb(109, 109, 117);

pub const ACCENT: Color32 = Color32::from_rgb(255, 181, 71);
pub const ACCENT_STRONG: Color32 = Color32::from_rgb(255, 197, 107);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(38, 30, 17);
pub const ACCENT_BORDER: Color32 = Color32::from_rgb(107, 77, 30);
/// Text on an amber fill.
pub const ON_ACCENT: Color32 = Color32::from_rgb(29, 19, 0);

/// The front-panel display.
pub const VFD: Color32 = Color32::from_rgb(255, 181, 71);
pub const VFD_BG: Color32 = Color32::from_rgb(13, 10, 5);
pub const VFD_EDGE: Color32 = Color32::from_rgb(42, 33, 20);
/// An unlit segment or indicator.
pub const VFD_OFF: Color32 = Color32::from_rgb(56, 42, 20);
/// Lit format indicators (HDMI, 4K, ...).
pub const VFD_INDICATOR: Color32 = Color32::from_rgb(95, 212, 255);

pub const SUCCESS: Color32 = Color32::from_rgb(107, 227, 141);
pub const WARNING: Color32 = Color32::from_rgb(255, 207, 92);
pub const WARNING_BORDER: Color32 = Color32::from_rgb(110, 88, 40);
pub const DANGER: Color32 = Color32::from_rgb(255, 122, 138);

pub const RADIUS: u8 = 10;
pub const CONTROL_RADIUS: u8 = 7;

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

/// JetBrains Mono Medium, for values and readouts.
pub fn mono() -> FontFamily {
    FontFamily::Name("mono".into())
}

/// JetBrains Mono Bold, for the front-panel display.
pub fn mono_bold() -> FontFamily {
    FontFamily::Name("mono-bold".into())
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
        "jetbrains-mono",
        include_bytes!("../../assets/fonts/JetBrainsMono-Medium.ttf"),
    );
    add(
        &mut fonts,
        "jetbrains-mono-bold",
        include_bytes!("../../assets/fonts/JetBrainsMono-Bold.ttf"),
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
    fonts.families.insert(mono(), family("jetbrains-mono"));
    fonts
        .families
        .insert(mono_bold(), family("jetbrains-mono-bold"));
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
        s.icon_width = 16.0;
        s.icon_width_inner = 9.0;

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.panel_fill = BG;
        v.window_fill = CARD;
        v.window_stroke = Stroke::new(1.0, BORDER_STRONG);
        v.window_corner_radius = CornerRadius::same(RADIUS);
        v.menu_corner_radius = CornerRadius::same(RADIUS);
        v.extreme_bg_color = BG;
        v.faint_bg_color = Color32::from_rgb(16, 16, 18);
        v.code_bg_color = CONTROL;
        v.hyperlink_color = ACCENT;
        v.warn_fg_color = WARNING;
        v.error_fg_color = DANGER;
        // Soft amber, so selected text keeps its own colour readable.
        v.selection.bg_fill = Color32::from_rgb(92, 66, 26);
        v.selection.stroke = Stroke::new(1.0, ACCENT_STRONG);
        v.text_cursor.stroke = Stroke::new(2.0, ACCENT);

        let r = CornerRadius::same(CONTROL_RADIUS);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = CARD;
        w.noninteractive.weak_bg_fill = CARD;
        w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_SECONDARY);
        w.noninteractive.corner_radius = r;

        w.inactive.bg_fill = CONTROL;
        w.inactive.weak_bg_fill = CONTROL;
        // A step lighter than card borders so checkboxes and fields read.
        w.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(62, 62, 69));
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
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
        w.active.fg_stroke = Stroke::new(1.5, ON_ACCENT);
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
                    .font(FontId::new(21.0, bold()))
                    .color(TEXT),
            );
            ui.add_space(-3.0);
            ui.label(RichText::new(subtitle).size(13.0).color(TEXT_DIM));
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
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(11, 5))
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("●  {text}"))
                    .font(FontId::new(12.5, semibold()))
                    .color(color),
            );
        });
}

/// A bordered card with an icon, a title and optional subtitle.
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
        .inner_margin(Margin {
            left: 18,
            right: 18,
            top: 14,
            bottom: 16,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon).size(17.0).color(ACCENT));
                ui.add_space(2.0);
                let title = RichText::new(title)
                    .font(FontId::new(15.0, bold()))
                    .color(TEXT);
                match subtitle {
                    Some(sub) => {
                        ui.vertical(|ui| {
                            // Leave room for the header's right-hand widgets, so a
                            // long, wrapping subtitle can't push the card wider.
                            ui.set_max_width(ui.available_width() - 16.0);
                            ui.label(title);
                            ui.add_space(-5.0);
                            ui.label(RichText::new(sub).size(12.5).color(TEXT_DIM));
                        });
                    }
                    None => {
                        ui.label(title);
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), header_right);
            });
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(min_body);
                add(ui)
            })
            .inner
        })
        .inner
}

/// A titled column of readouts under a heavy rule, like a spec sheet.
pub fn spec<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.vertical(|ui| {
        ui.set_width(ui.available_width());
        let top = ui.cursor().top();
        ui.painter()
            .hline(ui.max_rect().x_range(), top, Stroke::new(2.0, BORDER));
        ui.add_space(10.0);
        section_label(ui, title);
        ui.add_space(2.0);
        add(ui)
    })
    .inner
}

/// "Key ......... value" row with a divider underneath. Values are set in
/// the mono face, like the receiver's own readouts.
pub fn kv(ui: &mut Ui, key: &str, value: impl Into<RichText>) {
    kv_colored(ui, key, value, TEXT);
}

pub fn kv_colored(ui: &mut Ui, key: &str, value: impl Into<RichText>, color: Color32) {
    ui.horizontal(|ui| {
        ui.set_min_height(22.0);
        ui.label(RichText::new(key).size(13.0).color(TEXT_DIM));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add(
                egui::Label::new(value.into().font(FontId::new(12.5, mono())).color(color))
                    .truncate(),
            );
        });
    });
    let y = ui.cursor().top() - 3.0;
    let x = ui.max_rect().x_range();
    ui.painter().hline(x, y, Stroke::new(1.0, DIVIDER));
}

/// Section heading inside a card or window: accent icon + bold title.
pub fn subheading(ui: &mut Ui, icon: &str, text: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon).size(15.0).color(ACCENT));
        ui.label(
            RichText::new(text)
                .font(FontId::new(14.5, bold()))
                .color(TEXT),
        );
    });
    ui.add_space(2.0);
}

/// Small uppercase label ("INPUT", "VIDEO").
pub fn section_label(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .font(FontId::new(11.0, bold()))
            .color(TEXT_DIM)
            .extra_letter_spacing(1.6),
    );
}

/// Icon-rail navigation button; the label shows as a tooltip.
pub fn rail_item(ui: &mut Ui, icon: &str, label: &str, active: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(42.0), Sense::click());
    let p = ui.painter();
    let r = CornerRadius::same(RADIUS);
    if active {
        p.rect_filled(rect, r, ACCENT_SOFT);
    } else if resp.hovered() {
        p.rect_filled(rect, r, CONTROL);
    }
    let color = if active {
        ACCENT
    } else if resp.hovered() {
        TEXT
    } else {
        TEXT_DIM
    };
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        FontId::new(19.0, FontFamily::Proportional),
        color,
    );
    resp.on_hover_text(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Two-line selectable list row (title + muted subtitle).
pub fn list_item(ui: &mut Ui, title: &str, subtitle: &str, selected: bool) -> Response {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::click());
    let p = ui.painter();
    let r = CornerRadius::same(CONTROL_RADIUS);
    if selected {
        p.rect_filled(rect, r, ACCENT_SOFT);
        let rail = Rect::from_min_size(rect.min, Vec2::new(2.0, rect.height()));
        p.rect_filled(rail, CornerRadius::ZERO, ACCENT);
    } else if resp.hovered() {
        p.rect_filled(rect, r, CONTROL);
    }
    let x = rect.left() + 12.0;
    p.text(
        egui::pos2(x, rect.top() + 15.0),
        Align2::LEFT_CENTER,
        title,
        FontId::new(13.5, semibold()),
        if selected { ACCENT } else { TEXT },
    );
    p.text(
        egui::pos2(x, rect.top() + 32.0),
        Align2::LEFT_CENTER,
        subtitle,
        FontId::new(11.5, mono()),
        TEXT_DIM,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A button that renders as selected (amber fill) when `selected`.
pub fn choice(ui: &mut Ui, label: impl Into<String>, selected: bool) -> Response {
    let label: String = label.into();
    let text = if selected {
        RichText::new(label).color(ON_ACCENT)
    } else {
        RichText::new(label)
    };
    let mut button = egui::Button::new(text).min_size(Vec2::new(0.0, 30.0));
    if selected {
        button = button.fill(ACCENT).stroke(Stroke::new(1.0, ACCENT_STRONG));
    }
    ui.add(button)
}

/// Amber main action button.
pub fn primary_button(ui: &mut Ui, label: impl Into<String>) -> Response {
    ui.add(
        egui::Button::new(RichText::new(label.into()).color(ON_ACCENT))
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
    ui.style_mut().spacing.item_spacing.x = 16.0;
    ui.columns(n, add)
}

// ------------------------------------------------------- front panel

/// Display text with letter spacing, laid out once and drawn by `glow`.
pub fn vfd_text(p: &Painter, text: &str, font: FontId, spacing: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: font,
            color: Color32::WHITE,
            extra_letter_spacing: spacing,
            ..Default::default()
        },
    );
    p.layout_job(job)
}

/// Draws `g` aligned at `pos`, with a faint bloom around it when `lit`,
/// like a VFD segment.
pub fn glow(
    p: &Painter,
    pos: Pos2,
    align: Align2,
    g: &Arc<egui::Galley>,
    color: Color32,
    lit: bool,
) {
    let at = align.anchor_size(pos, g.size()).min;
    if lit {
        let halo = color.gamma_multiply(0.12);
        for d in [Vec2::X, -Vec2::X, Vec2::Y, -Vec2::Y] {
            p.galley_with_override_text_color(at + d, g.clone(), halo);
        }
    }
    p.galley_with_override_text_color(at, g.clone(), color);
}

#[derive(Clone, Copy, PartialEq)]
pub enum VfdState {
    On,
    /// Dimmed: the receiver is in standby or still connecting.
    Dim,
    /// Red: the receiver can't be reached.
    Error,
}

/// What the front-panel display shows.
pub struct Vfd<'a> {
    pub state: VfdState,
    /// Big text on the left (the input).
    pub source: &'a str,
    /// Big text on the right (the volume), with a small unit after it.
    pub value: &'a str,
    pub unit: &'a str,
    pub left: &'a str,
    pub right: &'a str,
    /// Format indicators and whether each is lit.
    pub indicators: &'a [(&'a str, bool)],
}

/// The receiver's front-panel display, full width.
pub fn vfd(ui: &mut Ui, d: &Vfd) {
    let height = 124.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(RADIUS), VFD_BG);
    p.rect_stroke(
        rect,
        CornerRadius::same(RADIUS),
        Stroke::new(1.0, VFD_EDGE),
        egui::StrokeKind::Inside,
    );
    let color = match d.state {
        VfdState::On => VFD,
        VfdState::Dim => VFD.gamma_multiply(0.38),
        VfdState::Error => DANGER.gamma_multiply(0.85),
    };
    let lit = d.state == VfdState::On;
    let inner = rect.shrink2(Vec2::new(22.0, 16.0));
    let big = FontId::new(38.0, mono_bold());
    let base = inner.top() + 22.0;
    let source = vfd_text(p, d.source, big.clone(), 3.0);
    glow(
        p,
        Pos2::new(inner.left(), base),
        Align2::LEFT_CENTER,
        &source,
        color,
        lit,
    );
    let mut right = inner.right();
    if !d.unit.is_empty() {
        let unit = vfd_text(p, d.unit, FontId::new(15.0, mono_bold()), 0.0);
        glow(
            p,
            Pos2::new(right, base + 8.0),
            Align2::RIGHT_CENTER,
            &unit,
            color,
            lit,
        );
        right -= unit.size().x + 5.0;
    }
    let value = vfd_text(p, d.value, big, 0.5);
    glow(
        p,
        Pos2::new(right, base),
        Align2::RIGHT_CENTER,
        &value,
        color,
        lit,
    );

    let small = FontId::new(13.5, mono_bold());
    let row2 = base + 38.0;
    let left = vfd_text(p, &d.left.to_uppercase(), small.clone(), 2.0);
    glow(
        p,
        Pos2::new(inner.left(), row2),
        Align2::LEFT_CENTER,
        &left,
        color,
        lit,
    );
    let r2 = vfd_text(p, &d.right.to_uppercase(), small, 2.0);
    glow(
        p,
        Pos2::new(inner.right(), row2),
        Align2::RIGHT_CENTER,
        &r2,
        color,
        lit,
    );

    let mut x = inner.left();
    let ind = FontId::new(10.5, mono_bold());
    for (label, on) in d.indicators {
        let g = vfd_text(p, label, ind.clone(), 1.3);
        let pos = Pos2::new(x, inner.bottom() - 6.0);
        if *on && lit {
            glow(p, pos, Align2::LEFT_CENTER, &g, VFD_INDICATOR, true);
        } else {
            glow(p, pos, Align2::LEFT_CENTER, &g, VFD_OFF, false);
        }
        x += g.size().x + 16.0;
    }
}

/// A hardware-style input key with an LED bar that lights when `on`.
pub fn input_key(ui: &mut Ui, label: &str, on: bool, width: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 52.0), Sense::click());
    let p = ui.painter();
    let r = CornerRadius::same(CONTROL_RADIUS + 1);
    let fill = if resp.is_pointer_button_down_on() {
        CONTROL
    } else if resp.hovered() {
        CONTROL_HOVER
    } else {
        Color32::from_rgb(30, 30, 34)
    };
    // Raised look: a darker lower edge under the key face.
    p.rect_filled(rect, r, KEY_SHADOW);
    let face = Rect::from_min_max(rect.min, rect.max - Vec2::new(0.0, 2.0));
    p.rect_filled(face, r, fill);
    p.rect_stroke(
        face,
        r,
        Stroke::new(
            1.0,
            if resp.hovered() {
                ACCENT_BORDER
            } else {
                BORDER_STRONG
            },
        ),
        egui::StrokeKind::Inside,
    );
    let led = Rect::from_center_size(
        Pos2::new(rect.center().x, rect.top() + 12.0),
        Vec2::new(18.0, 3.0),
    );
    if on {
        p.rect_filled(
            led.expand(2.0),
            CornerRadius::same(3),
            VFD.gamma_multiply(0.18),
        );
        p.rect_filled(led, CornerRadius::same(2), VFD);
    } else {
        p.rect_filled(led, CornerRadius::same(2), BORDER_STRONG);
    }
    p.text(
        Pos2::new(rect.center().x, rect.top() + 32.0),
        Align2::CENTER_CENTER,
        label,
        FontId::new(12.5, semibold()),
        if on { TEXT } else { TEXT_SECONDARY },
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A volume slider drawn as a VFD level meter, filled to `fraction` (0..=1).
/// Click or drag to pick a level; the caller reads the response.
pub fn volume_slider(ui: &mut Ui, fraction: f32, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 24.0),
        Sense::click_and_drag(),
    );
    let p = ui.painter();
    let fraction = fraction.clamp(0.0, 1.0);
    let bar = Rect::from_min_max(
        Pos2::new(rect.left(), rect.center().y - 6.0),
        Pos2::new(rect.right(), rect.center().y + 6.0),
    );
    let gap = 3.0;
    let segments = ((bar.width() + gap) / 9.0).floor().max(8.0) as usize;
    let seg_w = (bar.width() - gap * (segments - 1) as f32) / segments as f32;
    let lit = (fraction * segments as f32).round() as usize;
    let on = if enabled { VFD } else { TEXT_DIM };
    for i in 0..segments {
        let x = bar.left() + i as f32 * (seg_w + gap);
        let seg = Rect::from_min_size(Pos2::new(x, bar.top()), Vec2::new(seg_w, bar.height()));
        if i < lit {
            if enabled {
                p.rect_filled(
                    seg.expand(1.5),
                    CornerRadius::same(2),
                    VFD.gamma_multiply(0.15),
                );
            }
            p.rect_filled(seg, CornerRadius::same(1), on);
        } else {
            p.rect_filled(seg, CornerRadius::same(1), VFD_OFF);
        }
    }
    // The thumb: a bright bar at the current level.
    if enabled {
        let x = bar.left() + fraction * bar.width();
        let thumb =
            Rect::from_center_size(Pos2::new(x, rect.center().y), Vec2::new(4.0, rect.height()));
        let color = if resp.hovered() || resp.dragged() {
            Color32::WHITE
        } else {
            TEXT
        };
        p.rect_filled(thumb.expand(1.0), CornerRadius::same(3), VFD_BG);
        p.rect_filled(thumb, CornerRadius::same(2), color);
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// A row of joined options, like a hardware selector switch. Returns the
/// index clicked, if any. `id` must be unique among siblings.
pub fn segmented(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    options: &[&str],
    selected: Option<usize>,
) -> Option<usize> {
    let font = FontId::new(12.5, semibold());
    let pad = 12.0;
    let h = 26.0;
    let widths: Vec<f32> = options
        .iter()
        .map(|o| {
            ui.painter()
                .layout_no_wrap(o.to_string(), font.clone(), TEXT)
                .size()
                .x
                + pad * 2.0
        })
        .collect();
    let total = widths.iter().sum::<f32>() + 4.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total, h + 4.0), Sense::hover());
    let enabled = ui.is_enabled();
    let p = ui.painter().clone();
    p.rect_filled(rect, CornerRadius::same(CONTROL_RADIUS), BG);
    p.rect_stroke(
        rect,
        CornerRadius::same(CONTROL_RADIUS),
        Stroke::new(1.0, BORDER_STRONG),
        egui::StrokeKind::Inside,
    );
    let base = ui.id().with(id);
    let mut clicked = None;
    let mut x = rect.left() + 2.0;
    for (i, (label, w)) in options.iter().zip(&widths).enumerate() {
        let r = Rect::from_min_size(Pos2::new(x, rect.top() + 2.0), Vec2::new(*w, h));
        let resp = ui.interact(r, base.with(i), Sense::click());
        let on = selected == Some(i);
        let inner = CornerRadius::same(CONTROL_RADIUS - 2);
        if on {
            p.rect_filled(r, inner, if enabled { ACCENT } else { CONTROL_HOVER });
        } else if resp.hovered() && enabled {
            p.rect_filled(r, inner, CONTROL_HOVER);
        }
        let color = match (on, enabled) {
            (true, true) => ON_ACCENT,
            (_, false) => TEXT_DIM,
            (false, true) if resp.hovered() => TEXT,
            _ => TEXT_SECONDARY,
        };
        p.text(
            r.center(),
            Align2::CENTER_CENTER,
            label,
            font.clone(),
            color,
        );
        if resp.clicked() && enabled && !on {
            clicked = Some(i);
        }
        if enabled {
            resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        x += w;
    }
    clicked
}
