//! Small widgets for the Share & Collaborate window, drawn from the theme tokens.
//!
//! Upstream moved its widget set into the `craft` framework, whose button types are built for
//! upstream's own dialogs. This window only needs a few flat controls, so they are drawn here
//! (Ubunye).

use egui::{Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Ui, vec2};

use crate::theme::{self, Tokens};

/// Selected and hover fill of a square icon button. Returns the icon tint.
pub fn button_chrome(ui: &Ui, rect: Rect, selected: bool, hovered: bool) -> Color32 {
    let t = Tokens::get(ui.ctx());
    let radius = CornerRadius::from(t.radius_sm);
    if selected {
        ui.painter().rect_filled(rect, radius, t.accent_soft);
        ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, t.accent_border), StrokeKind::Inside);
    } else if hovered {
        ui.painter().rect_filled(rect, radius, t.hover);
    }
    if selected {
        t.accent_text
    } else if hovered {
        t.text
    } else {
        t.icon
    }
}

/// A square close button (a cross), transparent until hovered.
pub fn close_button(ui: &mut Ui, box_size: f32, tooltip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(box_size, box_size), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tooltip));
    let tint = button_chrome(ui, rect, false, resp.hovered());
    let c = rect.center();
    let d = box_size * 0.2;
    let stroke = Stroke::new(1.4, tint);
    ui.painter().line_segment([c + vec2(-d, -d), c + vec2(d, d)], stroke);
    ui.painter().line_segment([c + vec2(-d, d), c + vec2(d, -d)], stroke);
    resp.on_hover_text(tooltip)
}

/// A one-pixel separator across the available width.
pub fn hairline(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment([r.left_center(), r.right_center()], Stroke::new(1.0, t.separator));
}

fn button(ui: &mut Ui, label: &str, min_width: f32, fill: Color32, text: Color32, border: Option<Color32>) -> Response {
    let t = Tokens::get(ui.ctx());
    let font = theme::medium(12.5);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, text);
    let size = vec2((galley.size().x + 24.0).max(min_width), 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let radius = CornerRadius::from(t.radius_sm);
    let fill = if resp.hovered() { fill.gamma_multiply(0.9) } else { fill };
    ui.painter().rect_filled(rect, radius, fill);
    if let Some(border) = border {
        ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, border), StrokeKind::Inside);
    }
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, text);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    resp
}

/// The main action: a filled button.
pub fn primary_button(ui: &mut Ui, label: &str, min_width: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    button(ui, label, min_width, t.primary_bg, t.primary_text, None)
}

/// A quieter action: a field-coloured button with a border.
pub fn secondary_button(ui: &mut Ui, label: &str, min_width: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    button(ui, label, min_width, t.field, t.text, Some(t.field_border))
}

/// A tab drawn as a pill; `label` is already translated.
pub fn pill_tab(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(label.to_owned(), theme::medium(12.5), t.text);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 20.0, 24.0), Sense::click());
    let radius = CornerRadius::from(t.radius_sm);
    if selected {
        ui.painter().rect_filled(rect, radius, t.hover);
        ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, radius, t.hover.gamma_multiply(0.6));
    }
    let color = if selected { t.text } else { t.text_dim };
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, color);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    resp
}
