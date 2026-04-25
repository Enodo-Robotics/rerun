use std::sync::Arc;

use re_chunk::{Chunk, EntityPath, RowId, Timeline};
use re_log_types::{TimeInt, TimePoint, TimeReal};
use re_sdk_types::archetypes::TextLog;
use re_types_core::archetypes::Clear;
use re_ui::annotation_chips::tag_chip;
use re_viewer_context::{SystemCommand, SystemCommandSender as _, TimeControlCommand, ViewerContext};

// ---------------------------------------------------------------------------

/// Default quick-tag labels, used to seed `custom_tags` on first run.
/// Once seeded, deletions persist across restarts.
const DEFAULT_QUICK_TAGS: &[(&str, &str, [u8; 3])] = &[
    ("Anomaly", "WARN", [255, 165, 0]),
    ("Interesting", "INFO", [100, 180, 255]),
    ("Bug", "ERROR", [255, 80, 80]),
    ("Note", "INFO", [180, 180, 180]),
];

/// Base entity path under which all annotations are stored.
const ANNOTATIONS_ENTITY_BASE: &str = "annotations";

/// In-memory record of an annotation that has been logged.
#[derive(Clone, Debug)]
struct AnnotationEntry {
    text: String,
    level: String,
    timeline: Timeline,
    time: TimeInt,
    entity_path: EntityPath,
    /// The entity that was selected when this annotation was created.
    source_entity: Option<EntityPath>,
    color: egui::Color32,
}

/// A user-defined quick tag.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
struct CustomTag {
    label: String,
    level: String,
    color: [u8; 3],
}

impl CustomTag {
    fn egui_color(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.color[0], self.color[1], self.color[2])
    }
}

/// The annotation panel state, persisted across frames.
#[derive(Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct AnnotationPanel {
    /// Whether the editor side panel is currently shown.
    pub editor_visible: bool,

    /// Whether the quick-tag bar above the time panel is currently shown.
    pub tag_bar_visible: bool,

    custom_tags: Vec<CustomTag>,

    /// True once `DEFAULT_QUICK_TAGS` have been seeded into `custom_tags`. Prevents
    /// re-seeding after the user has deleted all tags.
    defaults_seeded: bool,

    #[serde(skip)]
    text_input: String,

    #[serde(skip)]
    new_tag_label: String,

    #[serde(skip)]
    new_tag_level_idx: usize,

    #[serde(skip)]
    annotations: Vec<AnnotationEntry>,

    /// Locked entity selection — persists so clicking panel buttons doesn't clear it.
    #[serde(skip)]
    locked_entity: Option<EntityPath>,

    #[serde(skip)]
    tags_loaded_for: Option<re_log_types::StoreId>,

    /// Fixed autosave path for this session, computed once on first save.
    #[serde(skip)]
    autosave_path: Option<std::path::PathBuf>,
}

impl AnnotationPanel {
    /// Seed default tags on first run. Once seeded, never re-seeds — so deleting
    /// all tags and restarting doesn't bring them back.
    fn ensure_default_tags(&mut self) {
        if !self.defaults_seeded {
            if self.custom_tags.is_empty() {
                self.custom_tags = DEFAULT_QUICK_TAGS
                    .iter()
                    .map(|&(label, level, color)| CustomTag {
                        label: label.to_owned(),
                        level: level.to_owned(),
                        color,
                    })
                    .collect();
            }
            self.defaults_seeded = true;
        }
    }

    pub fn toggle_editor(&mut self) {
        self.editor_visible = !self.editor_visible;
    }

    pub fn toggle_tag_bar(&mut self) {
        self.tag_bar_visible = !self.tag_bar_visible;
    }

    /// Save annotations to disk now. Called after every add/remove — no data
    /// loss on crash. Silently no-ops on wasm or if there are no annotations.
    #[cfg(not(target_arch = "wasm32"))]
    fn save_now(&mut self, ctx: &ViewerContext<'_>) {
        if self.autosave_path.is_none() {
            self.autosave_path = compute_autosave_path(ctx.store_context.recording);
        }
        if let Some(path) = &self.autosave_path {
            autosave_annotations_to(ctx.store_context.recording, path);
        }
    }

    /// Lock onto whatever the user has selected in the viewport so clicking
    /// panel buttons doesn't clear the target.
    fn update_locked_entity(&mut self, ctx: &ViewerContext<'_>) {
        let current_selection: Option<EntityPath> = ctx
            .selection_state()
            .selected_items()
            .first_item()
            .and_then(|item| item.entity_path().cloned());

        if let Some(ref sel) = current_selection {
            if self.locked_entity.as_ref() != Some(sel) {
                self.locked_entity = Some(sel.clone());
            }
        }
    }

    /// Horizontal quick-tag bar, rendered above the time panel.
    pub fn show_tag_bar(&mut self, ctx: &ViewerContext<'_>, ui: &mut egui::Ui) {
        if !self.tag_bar_visible {
            return;
        }

        self.ensure_default_tags();
        self.update_locked_entity(ctx);

        let store_id = ctx.store_context.recording.store_id().clone();
        let current_timeline = ctx.time_ctrl.timeline().cloned();
        let current_time = ctx.time_ctrl.time_int();
        let selected_entity = self.locked_entity.clone();

        egui::TopBottomPanel::bottom("annotation_tag_bar")
            .resizable(false)
            .frame(egui::Frame {
                fill: ui.style().visuals.panel_fill,
                inner_margin: egui::Margin::symmetric(8, 6),
                ..Default::default()
            })
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    let tags_snapshot: Vec<_> = self
                        .custom_tags
                        .iter()
                        .map(|t| (t.label.clone(), t.level.clone(), t.egui_color()))
                        .collect();

                    let base_size = ui.style().text_styles[&egui::TextStyle::Body].size;
                    let tag_size = base_size * 1.25;

                    for (label, level, color) in &tags_snapshot {
                        if tag_chip(ui, label, *color, Some(tag_size)).clicked() {
                            if let (Some(time), Some(tl)) = (current_time, &current_timeline) {
                                self.add_annotation(
                                    ctx,
                                    &store_id,
                                    tl,
                                    time,
                                    label.clone(),
                                    level.clone(),
                                    *color,
                                    selected_entity.as_ref(),
                                );
                            }
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        #[cfg(not(target_arch = "wasm32"))]
                        if ui
                            .button("Export")
                            .on_hover_text("Export annotations to .rrd")
                            .clicked()
                        {
                            ctx.command_sender()
                                .send_system(SystemCommand::ExportAnnotations {
                                    store_id: store_id.clone(),
                                });
                        }

                        if let Some(ref entity) = selected_entity {
                            ui.weak(format!("@ {entity}"));
                        }
                    });
                });
            });
    }

    /// Main UI entry point for the editor side panel.
    pub fn show_panel(&mut self, ctx: &ViewerContext<'_>, ui: &mut egui::Ui) {
        if !self.editor_visible {
            return;
        }

        self.ensure_default_tags();

        let recording_id = ctx.store_context.recording.store_id().clone();
        if self.tags_loaded_for.as_ref() != Some(&recording_id) {
            self.load_tags_from_recording(ctx.store_context.recording);
            self.load_annotations_from_store(ctx.store_context.recording);
            self.tags_loaded_for = Some(recording_id);
        }

        let screen_width = ui.ctx().screen_rect().width();

        let panel = egui::SidePanel::right("annotation_panel")
            .resizable(true)
            .min_width(200.0)
            .default_width(300.0)
            .max_width((0.45 * screen_width).round())
            .frame(egui::Frame {
                fill: ui.style().visuals.panel_fill,
                ..Default::default()
            });

        panel.show_animated_inside(ui, true, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;

            egui::ScrollArea::both()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    egui::Frame {
                        inner_margin: egui::Margin::same(8),
                        ..Default::default()
                    }
                    .show(ui, |ui| {
                        self.panel_contents(ctx, ui);
                    });
                });
        });
    }

    fn panel_contents(&mut self, ctx: &ViewerContext<'_>, ui: &mut egui::Ui) {
        ui.strong("Annotations");
        ui.add_space(4.0);

        let current_timeline = ctx.time_ctrl.timeline().cloned();
        let current_time = ctx.time_ctrl.time_int();

        if let Some(tl) = &current_timeline {
            ui.label(format!("Timeline: {}", tl.name()));
        } else {
            ui.label("Timeline: (none)");
        }
        if let Some(t) = current_time {
            ui.label(format!("Time: {}", t.as_i64()));
        } else {
            ui.label("Time: (none)");
        }

        self.update_locked_entity(ctx);
        let selected_entity = self.locked_entity.clone();

        ui.add_space(4.0);
        let mut clear_locked = false;
        if let Some(ref entity) = selected_entity {
            ui.horizontal(|ui| {
                ui.label("Target:");
                ui.colored_label(egui::Color32::from_rgb(130, 200, 130), entity.to_string());
                if ui.small_button("Clear").clicked() {
                    clear_locked = true;
                }
            });
            ui.label(
                egui::RichText::new(format!("Annotations → {}/_annotation/{{tag}}", entity))
                    .small()
                    .weak(),
            );
        } else {
            ui.horizontal(|ui| {
                ui.label("Target:");
                ui.weak("(none — annotations go to /annotations/)");
            });
            ui.label(
                egui::RichText::new("Select an entity in the viewport to target it")
                    .small()
                    .weak(),
            );
        }
        if clear_locked {
            self.locked_entity = None;
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        ui.strong("Quick tags");
        ui.add_space(4.0);

        let store_id = ctx.store_context.recording.store_id().clone();

        // Unified quick-tag buttons: defaults + custom, all deletable.
        {
            let mut tag_to_remove: Option<usize> = None;
            let mut tag_to_annotate: Option<(String, String, egui::Color32)> = None;

            let tags_snapshot: Vec<_> = self
                .custom_tags
                .iter()
                .map(|t| (t.label.clone(), t.level.clone(), t.egui_color()))
                .collect();

            // Only protect tags that have annotations in the current recording.
            let tags_in_use: std::collections::HashSet<&str> =
                self.annotations.iter().map(|a| a.text.as_str()).collect();

            ui.horizontal_wrapped(|ui| {
                for (idx, (label, level, color)) in tags_snapshot.iter().enumerate() {
                    let in_use = tags_in_use.contains(label.as_str());
                    let response = tag_chip(ui, label, *color, None);
                    if response.clicked() {
                        tag_to_annotate = Some((label.clone(), level.clone(), *color));
                    }
                    if response.secondary_clicked() && !in_use {
                        tag_to_remove = Some(idx);
                    }
                    if in_use {
                        response.on_hover_text(
                            "Click to annotate. In use — remove annotations first to delete.",
                        );
                    } else {
                        response.on_hover_text("Click to annotate. Right-click to remove.");
                    }
                }
            });

            if let (Some((label, level, color)), Some(time), Some(tl)) =
                (tag_to_annotate, current_time, &current_timeline)
            {
                self.add_annotation(
                    ctx,
                    &store_id,
                    tl,
                    time,
                    label,
                    level,
                    color,
                    selected_entity.as_ref(),
                );
            }
            if let Some(idx) = tag_to_remove {
                self.custom_tags.remove(idx);
                self.save_tags_to_recording(ctx, &store_id);
            }
        }

        ui.add_space(4.0);

        const TAG_LEVELS: &[(&str, [u8; 3])] = &[
            ("INFO", [100, 180, 255]),
            ("WARN", [255, 165, 0]),
            ("ERROR", [255, 80, 80]),
            ("NOTE", [180, 180, 180]),
        ];

        egui::CollapsingHeader::new("Add custom tag")
            .default_open(false)
            .show(ui, |ui| {
                let label_response = ui
                    .horizontal(|ui| {
                        ui.label("Label:");
                        ui.text_edit_singleline(&mut self.new_tag_label)
                    })
                    .inner;
                ui.horizontal(|ui| {
                    ui.label("Level:");
                    for (i, &(name, _)) in TAG_LEVELS.iter().enumerate() {
                        ui.selectable_value(&mut self.new_tag_level_idx, i, name);
                    }
                });
                let trimmed = self.new_tag_label.trim().to_owned();
                let duplicate = self.custom_tags.iter().any(|t| t.label == trimmed);
                let can_add = !trimmed.is_empty() && !duplicate;
                let enter_pressed = label_response.has_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (can_add && enter_pressed)
                    || ui
                        .add_enabled(can_add, egui::Button::new("Add to Quick Tags"))
                        .clicked()
                {
                    let (level, color) = TAG_LEVELS[self.new_tag_level_idx];
                    self.custom_tags.push(CustomTag {
                        label: trimmed,
                        level: level.to_owned(),
                        color,
                    });
                    self.new_tag_label.clear();
                    self.save_tags_to_recording(ctx, &store_id);
                }
                if duplicate && !self.new_tag_label.trim().is_empty() {
                    ui.weak("Tag already exists");
                }
            });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        ui.strong("Free-text annotation");
        ui.add_space(4.0);

        let response = ui.add(
            egui::TextEdit::multiline(&mut self.text_input)
                .hint_text("Type annotation here…")
                .desired_width(ui.available_width())
                .desired_rows(3),
        );

        ui.add_space(4.0);

        let can_submit =
            !self.text_input.trim().is_empty() && current_time.is_some() && current_timeline.is_some();

        let ctrl_enter = response.has_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);

        let text_for_tag = self.text_input.trim().to_owned();

        ui.horizontal(|ui| {
            if ui
                .add_enabled(can_submit, egui::Button::new("Add annotation"))
                .clicked()
                || (can_submit && ctrl_enter)
            {
                self.text_input.clear();
                if let (Some(time), Some(tl)) = (current_time, &current_timeline) {
                    self.add_annotation(
                        ctx,
                        &store_id,
                        tl,
                        time,
                        text_for_tag.clone(),
                        "INFO".to_owned(),
                        egui::Color32::from_rgb(180, 180, 180),
                        selected_entity.as_ref(),
                    );
                }
            }

            let tag_duplicate = self.custom_tags.iter().any(|t| t.label == text_for_tag);
            let can_add_tag = !text_for_tag.is_empty() && !tag_duplicate;
            if ui
                .add_enabled(can_add_tag, egui::Button::new("Add to Quick Tags"))
                .on_hover_text(if tag_duplicate {
                    "Tag already exists"
                } else {
                    "Promote this text to a reusable quick tag"
                })
                .clicked()
            {
                self.custom_tags.push(CustomTag {
                    label: text_for_tag,
                    level: "INFO".to_owned(),
                    color: [100, 180, 255],
                });
                self.text_input.clear();
                self.save_tags_to_recording(ctx, &store_id);
            }
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        ui.strong("Session annotations");
        ui.add_space(4.0);

        if self.annotations.is_empty() {
            ui.weak("No annotations yet.");
        } else {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    let mut to_remove: Option<usize> = None;

                    for (idx, ann) in self.annotations.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.colored_label(ann.color, &ann.level);

                            let time_text =
                                format!("[{}={}]", ann.timeline.name(), ann.time.as_i64());
                            let time_response = ui.small_button(&time_text);
                            if time_response.clicked() {
                                // Set active timeline then jump to time
                                ctx.command_sender().send_system(
                                    SystemCommand::TimeControlCommands {
                                        store_id: store_id.clone(),
                                        time_commands: vec![
                                            TimeControlCommand::SetActiveTimeline(
                                                *ann.timeline.name(),
                                            ),
                                            TimeControlCommand::SetTime(TimeReal::from(ann.time)),
                                        ],
                                    },
                                );
                                if let Some(ref source) = ann.source_entity {
                                    ctx.command_sender().send_system(
                                        SystemCommand::set_selection(
                                            re_viewer_context::Item::InstancePath(
                                                re_entity_db::InstancePath::entity_all(
                                                    source.clone(),
                                                ),
                                            ),
                                        ),
                                    );
                                }
                            }
                            time_response
                                .on_hover_text("Click to jump to this annotation's time");

                            if let Some(ref source) = ann.source_entity {
                                ui.weak(format!("@ {source}"));
                            }
                        });

                        ui.label(&ann.text);

                        if ui.small_button("Remove").clicked() {
                            to_remove = Some(idx);
                        }

                        ui.add_space(4.0);
                        ui.separator();
                        ui.add_space(2.0);
                    }

                    if let Some(idx) = to_remove {
                        let ann = self.annotations.remove(idx);
                        ctx.command_sender()
                            .send_system(SystemCommand::ClearAnnotation {
                                store_id: store_id.clone(),
                                entity_path: ann.entity_path.clone(),
                                timeline: ann.timeline.clone(),
                                time: ann.time,
                            });
                        #[cfg(not(target_arch = "wasm32"))]
                        self.save_now(ctx);
                    }
                });
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        let has_annotations_in_store = {
            let annotation_prefix = EntityPath::from(ANNOTATIONS_ENTITY_BASE);
            ctx.store_context.recording.sorted_entity_paths().any(|path| {
                path.starts_with(&annotation_prefix)
                    || path.to_string().contains("/_annotation")
            })
        };

        let has_annotations = !self.annotations.is_empty() || has_annotations_in_store;

        #[cfg(not(target_arch = "wasm32"))]
        if ui
            .add_enabled(
                has_annotations,
                egui::Button::new("Export annotations to .rrd"),
            )
            .on_hover_text(
                "Save only the annotation entities to a separate .rrd file.\n\
                 Opens a file dialog to choose where to save.",
            )
            .clicked()
        {
            ctx.command_sender()
                .send_system(SystemCommand::ExportAnnotations {
                    store_id: store_id.clone(),
                });
        }
    }

    fn add_annotation(
        &mut self,
        ctx: &ViewerContext<'_>,
        store_id: &re_log_types::StoreId,
        timeline: &Timeline,
        time: TimeInt,
        text: String,
        level: String,
        color: egui::Color32,
        selected_entity: Option<&EntityPath>,
    ) {
        // Each tag gets its own sub-entity so multiple tags can coexist under the same parent.
        let tag_slug = text.to_lowercase().replace(' ', "_");
        let entity_path = if let Some(selected) = selected_entity {
            EntityPath::from(format!("{}/_annotation/{tag_slug}", selected))
        } else {
            EntityPath::from(format!("{ANNOTATIONS_ENTITY_BASE}/{tag_slug}"))
        };

        // Snap the annotation time to the latest actual data point for the
        // selected entity. Avoids attaching the annotation at a cursor position
        // where the entity has no data (which shows as empty in the text log).
        let snapped_time = if let Some(selected) = selected_entity {
            let query = re_chunk_store::LatestAtQuery::new(*timeline.name(), time);
            let results = ctx
                .recording()
                .storage_engine()
                .store()
                .latest_at_relevant_chunks_for_all_components(
                    re_chunk_store::ChunkTrackingMode::Ignore,
                    &query,
                    selected,
                    true,
                );
            results
                .chunks
                .first()
                .and_then(|chunk| chunk.timelines().get(timeline.name()))
                .and_then(|col| {
                    col.times_raw()
                        .iter()
                        .map(|&t| TimeInt::new_temporal(t))
                        .filter(|&t| t <= time)
                        .max()
                })
                .unwrap_or(time)
        } else {
            time
        };

        self.annotations.push(AnnotationEntry {
            text: text.clone(),
            level: level.clone(),
            timeline: timeline.clone(),
            time: snapped_time,
            entity_path: entity_path.clone(),
            source_entity: selected_entity.cloned(),
            color,
        });

        ctx.command_sender()
            .send_system(SystemCommand::AddAnnotation {
                store_id: store_id.clone(),
                entity_path,
                timeline: timeline.clone(),
                time: snapped_time,
                text,
                level,
            });

        #[cfg(not(target_arch = "wasm32"))]
        self.save_now(ctx);
    }

    fn save_tags_to_recording(
        &self,
        ctx: &ViewerContext<'_>,
        store_id: &re_log_types::StoreId,
    ) {
        let tags_json = serde_json::to_string_pretty(&self.custom_tags).unwrap_or_default();
        let entity_path = EntityPath::from(format!("{ANNOTATIONS_ENTITY_BASE}/_config/tags"));

        let text_doc =
            re_sdk_types::archetypes::TextDocument::new(tags_json).with_media_type("text/plain");

        let timepoint = TimePoint::default();

        if let Ok(chunk) = Chunk::builder(entity_path)
            .with_archetype(RowId::new(), timepoint, &text_doc)
            .build()
        {
            ctx.command_sender()
                .send_system(SystemCommand::UpdateRecording(store_id.clone(), vec![chunk]));
        }
    }

    fn load_annotations_from_store(&mut self, entity_db: &re_entity_db::EntityDb) {
        let annotation_prefix = EntityPath::from(ANNOTATIONS_ENTITY_BASE);
        let config_prefix = EntityPath::from(format!("{ANNOTATIONS_ENTITY_BASE}/_config"));

        let engine = entity_db.storage_engine();

        for chunk in engine.store().iter_physical_chunks() {
            let path = chunk.entity_path();

            if path.starts_with(&config_prefix) {
                continue;
            }

            let is_annotation = path.starts_with(&annotation_prefix)
                || path.to_string().contains("/_annotation");

            if !is_annotation {
                continue;
            }

            for row_idx in 0..chunk.num_rows() {
                let unit = chunk.row_sliced_unit_shallow(row_idx);
                let text: Option<re_sdk_types::components::Text> = unit
                    .component_mono(
                        re_sdk_types::archetypes::TextLog::descriptor_text().component,
                    )
                    .and_then(|r| r.ok());

                let level: Option<re_sdk_types::components::TextLogLevel> = unit
                    .component_mono(
                        re_sdk_types::archetypes::TextLog::descriptor_level().component,
                    )
                    .and_then(|r| r.ok());

                if let Some(text) = text {
                    let text_str = text.0 .0.as_str().to_owned();
                    let level_str = level
                        .map(|l| l.0 .0.as_str().to_owned())
                        .unwrap_or_else(|| "INFO".to_owned());

                    let (timeline, time) = chunk
                        .timelines()
                        .values()
                        .next()
                        .and_then(|col| {
                            let times = col.times_raw();
                            if row_idx < times.len() {
                                Some((
                                    col.timeline().clone(),
                                    TimeInt::new_temporal(times[row_idx]),
                                ))
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| {
                            (
                                Timeline::new(
                                    re_chunk::TimelineName::log_time(),
                                    re_log_types::TimeType::TimestampNs,
                                ),
                                TimeInt::ZERO,
                            )
                        });

                    let color = match level_str.as_str() {
                        "WARN" => egui::Color32::from_rgb(255, 165, 0),
                        "ERROR" => egui::Color32::from_rgb(255, 80, 80),
                        _ => egui::Color32::from_rgb(180, 180, 180),
                    };

                    let already_exists = self.annotations.iter().any(|a| {
                        a.entity_path == *path && a.time == time && a.text == text_str
                    });

                    if !already_exists {
                        let path_str = path.to_string();
                        let source_entity = if let Some(pos) = path_str.find("/_annotation") {
                            let parent = &path_str[..pos];
                            if !parent.is_empty() {
                                Some(EntityPath::from(parent))
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        self.annotations.push(AnnotationEntry {
                            text: text_str,
                            level: level_str,
                            timeline,
                            time,
                            entity_path: path.clone(),
                            source_entity,
                            color,
                        });
                    }
                }
            }
        }
    }

    pub fn load_tags_from_recording(&mut self, entity_db: &re_entity_db::EntityDb) {
        let entity_path = EntityPath::from(format!("{ANNOTATIONS_ENTITY_BASE}/_config/tags"));
        let query = re_chunk_store::LatestAtQuery::latest(re_chunk::TimelineName::log_time());

        if let Some((_idx, text)) = entity_db.latest_at_component::<re_sdk_types::components::Text>(
            &entity_path,
            &query,
            re_sdk_types::archetypes::TextDocument::descriptor_text().component,
        ) {
            let text_str = text.0 .0.as_str();
            if let Ok(tags) = serde_json::from_str::<Vec<CustomTag>>(text_str) {
                if self.custom_tags.is_empty() {
                    self.custom_tags = tags;
                }
            }
        }
    }
}

/// Build a [`Chunk`] containing a single [`TextLog`] annotation row.
pub fn build_annotation_chunk(
    entity_path: &EntityPath,
    timeline: &Timeline,
    time: TimeInt,
    text: &str,
    level: &str,
) -> anyhow::Result<Chunk> {
    let text_log = TextLog::new(text)
        .with_level(re_sdk_types::components::TextLogLevel(level.into()));

    let timepoint = TimePoint::from_iter([(
        *timeline.name(),
        re_log_types::TimeCell::new(timeline.typ(), time.as_i64()),
    )]);

    let chunk = Chunk::builder(entity_path.clone())
        .with_archetype(RowId::new(), timepoint, &text_log)
        .build()?;

    Ok(chunk)
}

/// Build a [`Chunk`] containing a [`Clear`] to remove annotation data at a specific time.
pub fn build_clear_chunk(
    entity_path: &EntityPath,
    timeline: &Timeline,
    time: TimeInt,
) -> anyhow::Result<Chunk> {
    let clear = Clear::new(false);

    let timepoint = TimePoint::from_iter([(
        *timeline.name(),
        re_log_types::TimeCell::new(timeline.typ(), time.as_i64()),
    )]);

    let chunk = Chunk::builder(entity_path.clone())
        .with_archetype(RowId::new(), timepoint, &clear)
        .build()?;

    Ok(chunk)
}

/// Collect annotation chunks, open a save dialog, and return a closure for the file saver.
///
/// Returns `Ok(None)` if the user cancels or there are no annotations.
#[cfg(not(target_arch = "wasm32"))]
pub fn prepare_annotation_export(
    entity_db: &re_entity_db::EntityDb,
) -> anyhow::Result<Option<Box<dyn FnOnce() -> anyhow::Result<std::path::PathBuf> + Send>>> {
    let rrd_version = entity_db
        .store_info()
        .and_then(|info| info.store_version)
        .unwrap_or(re_build_info::CrateVersion::LOCAL);

    let store_id = entity_db.store_id();

    let annotation_prefix = EntityPath::from(ANNOTATIONS_ENTITY_BASE);
    let mut annotation_chunks: Vec<Arc<Chunk>> = entity_db
        .storage_engine()
        .store()
        .iter_physical_chunks()
        .filter(|chunk| {
            let path = chunk.entity_path();
            path.starts_with(&annotation_prefix) || path.to_string().contains("/_annotation")
        })
        .cloned()
        .collect();

    if annotation_chunks.is_empty() {
        re_log::warn!("No annotation data to export.");
        return Ok(None);
    }

    annotation_chunks.sort_by_key(|chunk| chunk.row_id_range().map(|(min, _)| min));

    let store_info_msg = entity_db
        .store_info_msg()
        .map(|msg| Ok(re_log_types::LogMsg::SetStoreInfo(msg.clone())));

    let data_messages: Vec<re_chunk::ChunkResult<re_log_types::LogMsg>> = annotation_chunks
        .into_iter()
        .map(|chunk| {
            chunk
                .to_arrow_msg()
                .map(|msg| re_log_types::LogMsg::ArrowMsg(store_id.clone(), msg))
        })
        .collect();

    let messages: Vec<_> = store_info_msg.into_iter().chain(data_messages).collect();

    // Derive default filename from the recording's data source or application ID.
    let default_name = entity_db
        .data_source
        .as_ref()
        .and_then(|src| {
            if let re_log_channel::LogSource::File { path, .. } = src {
                path.file_stem()
                    .map(|s| format!("{}_annotations.rrd", s.to_string_lossy()))
            } else {
                None
            }
        })
        .unwrap_or_else(|| {
            let app_id = entity_db
                .store_info()
                .map(|info| crate::saving::sanitize_app_id(info.store_id.application_id()))
                .unwrap_or_else(|| "recording".to_owned());
            format!("{app_id}_annotations.rrd")
        });

    let path = rfd::FileDialog::new()
        .set_file_name(&default_name)
        .set_title("Export annotations")
        .save_file();

    let Some(path) = path else {
        return Ok(None);
    };

    let path_for_closure = path.clone();
    Ok(Some(Box::new(move || {
        crate::saving::encode_to_file(rrd_version, &path_for_closure, messages.into_iter())?;
        Ok(path_for_closure)
    })))
}

/// Compute the autosave path once per session.
/// Returns `{recording_dir}/{recording_name}_{session_id}_annotations.rrd`.
/// Session ID is derived from the wall clock + process id so reopening the
/// same recording in a new viewer session doesn't clobber earlier autosaves.
#[cfg(not(target_arch = "wasm32"))]
fn compute_autosave_path(entity_db: &re_entity_db::EntityDb) -> Option<std::path::PathBuf> {
    let (save_dir, file_stem) = entity_db
        .data_source
        .as_ref()
        .and_then(|src| {
            if let re_log_channel::LogSource::File { path, .. } = src {
                let dir = path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| ".".into());
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "recording".to_owned());
                Some((dir, stem))
            } else {
                None
            }
        })
        .unwrap_or_else(|| (".".into(), "recording".to_owned()));

    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let session_id = format!(
        "{:016x}",
        secs.wrapping_mul(2654435761)
            .wrapping_add(std::process::id() as u64 * 6364136223846793005)
    );

    Some(save_dir.join(format!("{file_stem}_{session_id}_annotations.rrd")))
}

/// Write all current annotation chunks to `save_path`. Called after every
/// add/remove so an unexpected crash doesn't lose work.
#[cfg(not(target_arch = "wasm32"))]
fn autosave_annotations_to(entity_db: &re_entity_db::EntityDb, save_path: &std::path::Path) {
    let rrd_version = entity_db
        .store_info()
        .and_then(|info| info.store_version)
        .unwrap_or(re_build_info::CrateVersion::LOCAL);

    let store_id = entity_db.store_id();
    let annotation_prefix = EntityPath::from(ANNOTATIONS_ENTITY_BASE);

    let mut annotation_chunks: Vec<Arc<Chunk>> = entity_db
        .storage_engine()
        .store()
        .iter_physical_chunks()
        .filter(|chunk| {
            let path_str = chunk.entity_path().to_string();
            chunk.entity_path().starts_with(&annotation_prefix)
                || path_str.contains("/_annotation")
        })
        .cloned()
        .collect();

    if annotation_chunks.is_empty() {
        return;
    }

    annotation_chunks.sort_by_key(|chunk| chunk.row_id_range().map(|(min, _)| min));

    let store_info_msg = entity_db
        .store_info_msg()
        .map(|msg| Ok(re_log_types::LogMsg::SetStoreInfo(msg.clone())));

    let data_messages: Vec<re_chunk::ChunkResult<re_log_types::LogMsg>> = annotation_chunks
        .into_iter()
        .map(|chunk| {
            chunk
                .to_arrow_msg()
                .map(|msg| re_log_types::LogMsg::ArrowMsg(store_id.clone(), msg))
        })
        .collect();

    let messages: Vec<_> = store_info_msg.into_iter().chain(data_messages).collect();

    match crate::saving::encode_to_file(rrd_version, save_path, messages.into_iter()) {
        Ok(()) => re_log::debug!("Auto-saved annotations to {}", save_path.display()),
        Err(err) => re_log::warn!("Failed to auto-save annotations: {err}"),
    }
}
