//! RRL fork: labels and event markers drawn on the timeline.
//!
//! Every label added in the viewer's labelling panel, and every `rr.mark_event`, gets a coloured
//! flag on the time ruler and a faint line through the streams, so you can see at a glance how
//! far a recording has been labelled. Clicking a flag moves the time cursor to it.

use egui::{Rangef, Rect};
use re_chunk_store::{ChunkStoreGeneration, ChunkTrackingMode, RangeQuery};
use re_log_types::{AbsoluteTimeRange, StoreId, TimeInt, TimeReal, TimelineName};
use re_viewer_context::event_labels::{EventLabelKind, event_label_color, event_label_for_path};
use re_viewer_context::{StoreViewContext, TimeControlCommand};

use crate::time_ranges_ui::TimeRangesUi;

/// One label or event on the active timeline.
#[derive(Clone, Debug)]
struct Marker {
    time: TimeInt,
    label: String,
    kind: EventLabelKind,
    color: egui::Color32,
}

/// Markers found on the active timeline, recomputed only when the store changes.
#[derive(Default)]
pub struct EventMarkers {
    key: Option<(StoreId, ChunkStoreGeneration, TimelineName)>,
    markers: Vec<Marker>,
}

impl EventMarkers {
    fn update(&mut self, ctx: &StoreViewContext<'_>) {
        let timeline = ctx.time_ctrl.timeline_name();
        let key = (ctx.db.store_id().clone(), ctx.db.generation(), *timeline);
        if self.key.as_ref() == Some(&key) {
            return;
        }
        re_tracing::profile_function!();

        self.markers = collect_markers(ctx.db, timeline);
        self.key = Some(key);
    }

    /// Paint flags on the time ruler and lines through the streams; handle clicks on the flags.
    #[expect(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        ctx: &StoreViewContext<'_>,
        ui: &egui::Ui,
        time_ranges_ui: &TimeRangesUi,
        painter: &egui::Painter,
        timeline_rect: &Rect,
        streams_y_range: Rangef,
        time_commands: &mut Vec<TimeControlCommand>,
    ) {
        self.update(ctx);
        if self.markers.is_empty() {
            return;
        }

        let flag_height = (timeline_rect.height() * 0.5).min(10.0);
        let flag_width = flag_height * 0.8;
        let pole_bottom = timeline_rect.bottom();
        let pole_top = pole_bottom - flag_height * 1.4;

        let mut hovered: Vec<&Marker> = Vec::new();
        let pointer = ui.input(|i| i.pointer.hover_pos());

        for marker in &self.markers {
            let Some(x) = time_ranges_ui.x_from_time_f32(TimeReal::from(marker.time)) else {
                continue;
            };
            if !timeline_rect.x_range().contains(x) {
                continue;
            }

            painter.vline(
                x,
                streams_y_range,
                egui::Stroke::new(1.0, marker.color.gamma_multiply(0.35)),
            );
            painter.vline(
                x,
                Rangef::new(pole_top, pole_bottom),
                egui::Stroke::new(1.5, marker.color),
            );

            let flag = match marker.kind {
                // A pennant for labels…
                EventLabelKind::Label => vec![
                    egui::pos2(x, pole_top),
                    egui::pos2(x + flag_width, pole_top + flag_height * 0.35),
                    egui::pos2(x, pole_top + flag_height * 0.7),
                ],
                // …a diamond for events logged by a program.
                EventLabelKind::Event => vec![
                    egui::pos2(x, pole_top - flag_height * 0.35),
                    egui::pos2(x + flag_width * 0.5, pole_top),
                    egui::pos2(x, pole_top + flag_height * 0.35),
                    egui::pos2(x - flag_width * 0.5, pole_top),
                ],
            };
            painter.add(egui::Shape::convex_polygon(
                flag,
                marker.color,
                egui::Stroke::NONE,
            ));

            if let Some(pointer) = pointer
                && timeline_rect.contains(pointer)
                && (pointer.x - x).abs() <= flag_width.max(4.0)
            {
                hovered.push(marker);
            }
        }

        if hovered.is_empty() {
            return;
        }

        // The nearest marker wins a click; the tooltip lists all of them under the pointer.
        let rect = Rect::from_x_y_ranges(
            pointer.map_or_else(
                || timeline_rect.x_range(),
                |p| Rangef::new(p.x - flag_width, p.x + flag_width),
            ),
            timeline_rect.y_range(),
        );
        let response = ui
            .interact(rect, ui.id().with("event_marker"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_ui(|ui| {
                for marker in &hovered {
                    ui.horizontal(|ui| {
                        ui.colored_label(marker.color, "⏺");
                        ui.label(&marker.label);
                        if marker.kind == EventLabelKind::Event {
                            ui.weak("(event)");
                        }
                    });
                }
                ui.weak("Click to jump here");
            });

        if response.clicked()
            && let Some(pointer) = pointer
            && let Some(nearest) = hovered.iter().min_by(|a, b| {
                let dist = |m: &Marker| {
                    time_ranges_ui
                        .x_from_time_f32(TimeReal::from(m.time))
                        .map_or(f32::INFINITY, |x| (x - pointer.x).abs())
                };
                dist(a).total_cmp(&dist(b))
            })
        {
            time_commands.push(TimeControlCommand::SetTime(TimeReal::from(nearest.time)));
        }
    }
}

fn collect_markers(db: &re_entity_db::EntityDb, timeline: &TimelineName) -> Vec<Marker> {
    let engine = db.storage_engine();
    let store = engine.store();
    let query = RangeQuery::new(*timeline, AbsoluteTimeRange::EVERYTHING);

    let mut markers = Vec::new();
    for path in db.sorted_entity_paths() {
        let Some((label, kind)) = event_label_for_path(path) else {
            continue;
        };
        let color = event_label_color(&label);
        for chunk in store
            .range_relevant_chunks_for_all_components(
                ChunkTrackingMode::Ignore,
                &query,
                path,
                false,
            )
            .chunks
        {
            if let Some(column) = chunk.timelines().get(timeline) {
                markers.extend(column.times_raw().iter().map(|&t| Marker {
                    time: TimeInt::new_temporal(t),
                    label: label.clone(),
                    kind,
                    color,
                }));
            }
        }
    }

    markers.sort_by_key(|m| m.time);
    markers.dedup_by(|a, b| a.time == b.time && a.label == b.label);
    markers
}
