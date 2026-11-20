//! RRL fork: labels and event markers on the timeline.

#![cfg(feature = "testing")]
#![expect(clippy::unwrap_used)] // Fine for tests

use re_chunk::Chunk;
use re_chunk_store::{LatestAtQuery, RowId};
use re_log_types::example_components::{MyPoint, MyPoints};
use re_log_types::{EntityPath, build_frame_nr};
use re_sdk_types::archetypes::TextLog;
use re_test_context::TestContext;
use re_time_panel::TimePanel;
use re_viewer_context::blueprint_timeline;
use re_viewport_blueprint::ViewportBlueprint;

fn text_log_chunk(path: &str, frame: i64, text: &str) -> Chunk {
    Chunk::builder(EntityPath::from(path))
        .with_archetype(RowId::new(), [build_frame_nr(frame)], &TextLog::new(text))
        .build()
        .unwrap()
}

#[test]
pub fn time_panel_labels_and_events() {
    TimePanel::ensure_registered_subscribers();

    let mut test_context = TestContext::new();

    let points = MyPoint::from_iter(0..1);
    let mut chunks: Vec<Chunk> = (0..100)
        .map(|frame| {
            Chunk::builder(EntityPath::from("/robot/signal"))
                .with_sparse_component_batches(
                    RowId::new(),
                    [build_frame_nr(frame)],
                    [(MyPoints::descriptor_points(), Some(&points as _))],
                )
                .build()
                .unwrap()
        })
        .collect();

    // Labels as the labelling panel writes them, with and without a target entity…
    chunks.push(text_log_chunk("annotations/correct/10", 10, "correct"));
    chunks.push(text_log_chunk("annotations/comma/30", 30, "comma"));
    chunks.push(text_log_chunk(
        "robot/signal/_annotation/correct/50",
        50,
        "correct",
    ));
    // …and an event from `rr.mark_event`.
    chunks.push(text_log_chunk("_event/hard-stop", 75, "hard-stop"));

    test_context.add_chunks(chunks.into_iter());
    test_context.set_active_timeline("frame_nr");

    let mut time_panel = TimePanel::default();
    let mut harness = test_context
        .setup_kittest_for_rendering_ui([700.0, 250.0])
        .build_ui(|ui| {
            test_context.run(&ui.ctx().clone(), |viewer_ctx| {
                let blueprint = ViewportBlueprint::from_db(
                    viewer_ctx.store_context.blueprint,
                    &LatestAtQuery::latest(blueprint_timeline()),
                );

                let mut time_commands = Vec::new();
                let store_ctx = viewer_ctx.active_recording_store_view_context();
                time_panel.show_expanded_with_header(
                    viewer_ctx,
                    &store_ctx,
                    &blueprint,
                    ui,
                    &mut time_commands,
                );

                test_context.send_time_commands(viewer_ctx.store_id().clone(), time_commands);
            });

            test_context.handle_system_commands(ui.ctx());
        });

    harness.run();
    harness.snapshot("time_panel_labels_and_events");
}
