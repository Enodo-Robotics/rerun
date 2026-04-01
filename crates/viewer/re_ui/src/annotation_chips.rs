/// Render a tag chip button: colored rounded background with white text.
/// Returns the response so callers can check for clicks.
pub fn tag_chip(
    ui: &mut egui::Ui,
    label: &str,
    bg_color: egui::Color32,
    font_size: Option<f32>,
) -> egui::Response {
    let text = if let Some(size) = font_size {
        egui::RichText::new(label)
            .color(egui::Color32::WHITE)
            .size(size)
    } else {
        egui::RichText::new(label)
            .color(egui::Color32::WHITE)
            .small()
    };

    let button = egui::Button::new(text)
        .fill(bg_color)
        .rounding(egui::Rounding::same(4))
        .stroke(egui::Stroke::NONE);

    ui.add(button)
}

/// Render a non-interactive tag chip label (colored rounded rect with white text).
pub fn tag_chip_label(ui: &mut egui::Ui, label: &str, bg_color: egui::Color32) {
    let font_id = egui::TextStyle::Small.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font_id, egui::Color32::WHITE);

    let padding = egui::vec2(6.0, 2.0);
    let desired_size = galley.size() + padding * 2.0;
    let (rect, _response) = ui.allocate_exact_size(desired_size, egui::Sense::hover());

    if ui.is_rect_visible(rect) {
        ui.painter()
            .rect_filled(rect, egui::Rounding::same(4), bg_color);
        ui.painter()
            .galley(rect.min + padding, galley, egui::Color32::WHITE);
    }
}
