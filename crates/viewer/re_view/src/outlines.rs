use egui::NumExt as _;
use re_ui::ContextExt as _;

// TODO(andreas): It would be nice if these wouldn't need to be set on every single line/point builder.

/// Gap between lines and their outline.
pub const SIZE_BOOST_IN_POINTS_FOR_LINE_OUTLINES: f32 = 2.5;

/// Gap between points and their outline.
pub const SIZE_BOOST_IN_POINTS_FOR_POINT_OUTLINES: f32 = 4.0;

/// How much thicker selection & hover outlines are drawn than the egui theme asks for.
///
/// The theme widths are tuned for 2D widget strokes, which are far too subtle to pick a
/// selected entity out of a dense 3D scene at a glance.
pub const OUTLINE_EMPHASIS: f32 = 3.0;

/// Colour of the selection outline in spatial views.
///
/// Deliberately *not* the theme's `selection_stroke_color` (`#f0f2ff`, a near-white): that value is
/// picked to read as text on a primary-coloured background, and against a bright or washed-out 3D
/// scene it has almost no contrast. High-vis orange is rare in sensor data, reads as "attention"
/// rather than as part of the scene, and stays distinguishable from green geometry for red-green
/// colourblind viewers.
// Deliberately hard-coded rather than declared as a design token: the point of this colour is
// that it does *not* follow the theme. The theme's `selection_stroke_color` is what proved too
// faint in spatial views, and a token would have to be defined per theme anyway, for a value that
// should be identical in light and dark.
#[expect(clippy::disallowed_methods)]
pub const SELECTION_OUTLINE_COLOR: egui::Color32 = egui::Color32::from_rgb(0xff, 0x8c, 0x00);

/// Produce an [`re_renderer::OutlineConfig`] based on the [`egui::Style`] of the provided [`egui::Context`].
pub fn outline_config(gui_ctx: &egui::Context) -> re_renderer::OutlineConfig {
    // Use the exact same colors we have in the ui!
    let hover_outline = gui_ctx.hover_stroke();
    let selection_outline = gui_ctx.selection_stroke();

    // See also: SIZE_BOOST_IN_POINTS_FOR_LINE_OUTLINES

    let outline_radius_ui_pts = 0.5 * f32::max(hover_outline.width, selection_outline.width);
    let outline_radius_pixel =
        (gui_ctx.pixels_per_point() * outline_radius_ui_pts * OUTLINE_EMPHASIS).at_least(0.5);

    re_renderer::OutlineConfig {
        outline_radius_pixel,
        color_layer_a: re_renderer::Rgba::from(hover_outline.color),
        color_layer_b: re_renderer::Rgba::from(SELECTION_OUTLINE_COLOR),
    }
}

/// As [`outline_config`], but with the selection outline in a caller-chosen colour.
///
/// Used to let an externally-driven framing command pick its own highlight colour, so a driver can
/// colour-code what it is pointing at. `None` keeps [`SELECTION_OUTLINE_COLOR`].
///
/// The colour is per view, not per entity — [`re_renderer::OutlineConfig`] has one colour per
/// outline layer — so everything selected in a view shares it.
pub fn outline_config_with_selection_color(
    gui_ctx: &egui::Context,
    selection_color: Option<egui::Color32>,
) -> re_renderer::OutlineConfig {
    let mut config = outline_config(gui_ctx);
    if let Some(color) = selection_color {
        config.color_layer_b = re_renderer::Rgba::from(color);
    }
    config
}
