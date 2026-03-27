use std::sync::Arc;

use re_chunk::{Chunk, EntityPath, RowId, Timeline};
use re_log_types::{TimeInt, TimePoint};
use re_types::archetypes::TextLog;
use re_types_core::archetypes::Clear;
use re_viewer_context::{SystemCommand, SystemCommandSender as _, ViewerContext};

// ---------------------------------------------------------------------------

/// Default quick-tag labels, used to seed `custom_tags` on first run.
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
    /// Human-readable text of the annotation.
    text: String,
    /// Log level / tag.
    level: String,
    /// The timeline on which this annotation was placed.
    timeline: Timeline,
    /// The time value on the timeline.
    time: TimeInt,
    /// Entity sub-path (e.g. "annotations/anomaly" or "annotations/text").
    entity_path: EntityPath,
    /// The entity that was selected when this annotation was created.
    /// Used to restore selection when navigating to this annotation.
    source_entity: Option<EntityPath>,
    /// Color for display.
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
    /// Whether the side panel (editor) is currently shown.
    pub editor_visible: bool,

    /// Whether the horizontal tag bar is currently shown.
    pub tag_bar_visible: bool,

    /// User-defined quick tags, persisted across sessions.
    custom_tags: Vec<CustomTag>,

    /// Whether defaults have been seeded. Once true, deleting all tags won't re-seed.
    defaults_seeded: bool,

    /// Free-text input buffer.
    #[serde(skip)]
    text_input: String,

    /// Buffer for new tag label.
    #[serde(skip)]
    new_tag_label: String,

    /// Buffer for new tag level selection.
    #[serde(skip)]
    new_tag_level_idx: usize,

    /// In-memory list of annotations added this session.
    /// We keep this so we can display them without querying the store every frame.
    #[serde(skip)]
    annotations: Vec<AnnotationEntry>,

    /// Locked entity selection — captured when the user selects an entity while the panel
    /// is open. Persists across panel interactions so clicking buttons doesn't lose it.
    /// Click "Clear" or select a different entity to update.
    #[serde(skip)]
    locked_entity: Option<EntityPath>,

    /// Track which recording we've loaded tags from to avoid re-loading every frame.
    #[serde(skip)]
    tags_loaded_for: Option<re_log_types::StoreId>,
}

impl AnnotationPanel {
    /// Seed default tags on first run only. Once seeded, never re-seeds.
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

    /// Toggle visibility of the editor side panel.
    pub fn toggle_editor(&mut self) {
        self.editor_visible = !self.editor_visible;
    }

    /// Toggle visibility of the quick-tag bar.
    pub fn toggle_tag_bar(&mut self) {
        self.tag_bar_visible = !self.tag_bar_visible;
    }

    /// Update locked entity from the current viewport selection.
    /// Called from both the tag bar and the editor panel.
    fn update_locked_entity(&mut self, ctx: &ViewerContext<'_>) {
        let current_selection: Option<EntityPath> = ctx
            .selection_state
            .selected_items()
            .first_item()
            .and_then(|item| item.entity_path().cloned());

        if let Some(ref sel) = current_selection {
            if self.locked_entity.as_ref() != Some(sel) {
                self.locked_entity = Some(sel.clone());
            }
        }
    }

    /// Show the horizontal quick-tag bar above the time panel.
    pub fn show_tag_bar(&mut self, ctx: &ViewerContext<'_>, ui: &mut egui::Ui) {
        if !self.tag_bar_visible {
            return;
        }

        self.ensure_default_tags();
        self.update_locked_entity(ctx);

        let store_id = ctx.store_context.recording.store_id().clone();
        let time_ctrl = ctx.rec_cfg.time_ctrl.read();
        let current_timeline = time_ctrl.timeline().clone();
        let current_time = time_ctrl.time_int();
        drop(time_ctrl);

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
                    // Left side: tags (wrapping)
                    let tags_snapshot: Vec<_> = self
                        .custom_tags
                        .iter()
                        .map(|t| (t.label.clone(), t.level.clone(), t.egui_color()))
                        .collect();

                    let base_size = ui.style().text_styles[&egui::TextStyle::Body].size;
                    let tag_size = base_size * 1.25;

                    for (label, level, color) in &tags_snapshot {
                        let button = egui::Button::new(
                            egui::RichText::new(label).color(*color).size(tag_size),
                        );
                        if ui.add(button).clicked() {
                            if let Some(time) = current_time {
                                self.add_annotation(
                                    ctx,
                                    &store_id,
                                    &current_timeline,
                                    time,
                                    label.clone(),
                                    level.clone(),
                                    *color,
                                    selected_entity.as_ref(),
                                );
                            }
                        }
                    }

                    // Right side: entity path + export
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        #[cfg(not(target_arch = "wasm32"))]
                        if ui.button("Export").on_hover_text("Export annotations to .rrd").clicked() {
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

        // Load custom tags and existing annotations from recording on first open
        // (or when recording changes)
        let recording_id = ctx.store_context.recording.store_id();
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

        // Show current timeline + time
        let time_ctrl = ctx.rec_cfg.time_ctrl.read();
        let current_timeline = time_ctrl.timeline().clone();
        let current_time = time_ctrl.time_int();
        drop(time_ctrl);

        ui.label(format!(
            "Timeline: {}",
            current_timeline.name()
        ));
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
                ui.colored_label(
                    egui::Color32::from_rgb(130, 200, 130),
                    entity.to_string(),
                );
                if ui.small_button("Clear").clicked() {
                    clear_locked = true;
                }
            });
            ui.label(
                egui::RichText::new(
                    format!("Annotations → {}/_annotation", entity)
                )
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

        // Quick-tag buttons (unified: defaults + custom, all deletable)
        ui.strong("Quick tags");
        ui.add_space(4.0);

        let store_id = ctx.store_context.recording.store_id().clone();

        {
            let mut tag_to_remove: Option<usize> = None;
            let mut tag_to_annotate: Option<(String, String, egui::Color32)> = None;

            let tags_snapshot: Vec<_> = self
                .custom_tags
                .iter()
                .map(|t| (t.label.clone(), t.level.clone(), t.egui_color()))
                .collect();

            // Only protect tags that have annotations in the current recording
            let tags_in_use: std::collections::HashSet<&str> = self
                .annotations
                .iter()
                .map(|a| a.text.as_str())
                .collect();

            ui.horizontal_wrapped(|ui| {
                for (idx, (label, level, color)) in tags_snapshot.iter().enumerate() {
                    let in_use = tags_in_use.contains(label.as_str());
                    let button = egui::Button::new(
                        egui::RichText::new(label).color(*color),
                    );
                    let response = ui.add(button);
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

            if let Some((label, level, color)) = tag_to_annotate {
                if let Some(time) = current_time {
                    self.add_annotation(
                        ctx, &store_id, &current_timeline, time, label, level, color,
                        selected_entity.as_ref(),
                    );
                }
            }
            if let Some(idx) = tag_to_remove {
                self.custom_tags.remove(idx);
                self.save_tags_to_recording(ctx, &store_id);
            }
        }

        ui.add_space(4.0);

        // Add new custom tag
        const TAG_LEVELS: &[(&str, [u8; 3])] = &[
            ("INFO", [100, 180, 255]),
            ("WARN", [255, 165, 0]),
            ("ERROR", [255, 80, 80]),
            ("NOTE", [180, 180, 180]),
        ];

        egui::CollapsingHeader::new("Add custom tag")
            .default_open(false)
            .show(ui, |ui| {
                let label_response = ui.horizontal(|ui| {
                    ui.label("Label:");
                    ui.text_edit_singleline(&mut self.new_tag_label)
                }).inner;
                ui.horizontal(|ui| {
                    ui.label("Level:");
                    for (i, &(name, _)) in TAG_LEVELS.iter().enumerate() {
                        ui.selectable_value(&mut self.new_tag_level_idx, i, name);
                    }
                });
                let can_add = !self.new_tag_label.trim().is_empty();
                let enter_pressed = label_response.has_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (can_add && enter_pressed)
                    || ui.add_enabled(can_add, egui::Button::new("Add to Quick Tags")).clicked()
                {
                    let (level, color) = TAG_LEVELS[self.new_tag_level_idx];
                    self.custom_tags.push(CustomTag {
                        label: self.new_tag_label.trim().to_owned(),
                        level: level.to_owned(),
                        color,
                    });
                    self.new_tag_label.clear();
                    self.save_tags_to_recording(ctx, &store_id);
                }
            });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // Free-text annotation
        ui.strong("Free-text annotation");
        ui.add_space(4.0);

        let response = ui.add(
            egui::TextEdit::multiline(&mut self.text_input)
                .hint_text("Type annotation here…")
                .desired_width(ui.available_width())
                .desired_rows(3),
        );

        ui.add_space(4.0);

        let can_submit = !self.text_input.trim().is_empty() && current_time.is_some();

        // Submit on Ctrl+Enter while text has focus
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

                if let Some(time) = current_time {
                    self.add_annotation(
                        ctx,
                        &store_id,
                        &current_timeline,
                        time,
                        text_for_tag.clone(),
                        "INFO".to_owned(),
                        egui::Color32::from_rgb(180, 180, 180),
                        selected_entity.as_ref(),
                    );
                }
            }

            let can_add_tag = !text_for_tag.is_empty();
            if ui
                .add_enabled(can_add_tag, egui::Button::new("Add to Quick Tags"))
                .on_hover_text("Promote this text to a reusable quick tag")
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

        // List existing annotations
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

                            // Clickable time label — navigates to annotation time + selects source entity
                            let time_text = format!("[{}={}]", ann.timeline.name(), ann.time.as_i64());
                            let time_response = ui.small_button(&time_text);
                            if time_response.clicked() {
                                // Navigate to the annotation's time
                                ctx.command_sender().send_system(
                                    SystemCommand::SetActiveTime {
                                        rec_id: store_id.clone(),
                                        timeline: ann.timeline.clone(),
                                        time: Some(re_log_types::TimeReal::from(ann.time)),
                                    },
                                );
                                // Re-select the source entity if we have one
                                if let Some(ref source) = ann.source_entity {
                                    ctx.command_sender().send_system(
                                        SystemCommand::SetSelection(
                                            re_viewer_context::Item::InstancePath(
                                                re_entity_db::InstancePath::entity_all(
                                                    source.clone(),
                                                ),
                                            ),
                                        ),
                                    );
                                }
                            }
                            time_response.on_hover_text("Click to jump to this annotation's time");

                            // Show source entity if any
                            if let Some(ref source) = ann.source_entity {
                                ui.weak(format!("@ {source}"));
                            }
                        });

                        ui.label(&ann.text);

                        if ui
                            .small_button("Remove")
                            .clicked()
                        {
                            to_remove = Some(idx);
                        }

                        ui.add_space(4.0);
                        ui.separator();
                        ui.add_space(2.0);
                    }

                    if let Some(idx) = to_remove {
                        let ann = self.annotations.remove(idx);
                        // Send clear command
                        ctx.command_sender().send_system(SystemCommand::ClearAnnotation {
                            store_id: store_id.clone(),
                            entity_path: ann.entity_path.clone(),
                            timeline: ann.timeline.clone(),
                            time: ann.time,
                        });
                    }
                });
        }

        // Export / Save buttons
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // Check the actual store for annotation data, not just the in-memory session list.
        // Annotations persist in the EntityDb across sessions.
        let has_annotations_in_store = {
            let annotation_prefix = EntityPath::from(ANNOTATIONS_ENTITY_BASE);
            let suffix = "_annotation";
            ctx.store_context
                .recording
                .entity_paths()
                .iter()
                .any(|path| {
                    path.starts_with(&annotation_prefix)
                        || path.to_string().ends_with(suffix)
                })
        };

        let has_annotations = !self.annotations.is_empty() || has_annotations_in_store;

        #[cfg(not(target_arch = "wasm32"))]
        if ui
            .add_enabled(has_annotations, egui::Button::new("Export annotations to .rrd"))
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
        // Determine annotation entity path:
        // - If an entity is selected: {selected_entity}/_annotation
        // - Otherwise: annotations/{level}
        let entity_path = if let Some(selected) = selected_entity {
            EntityPath::from(format!("{}/_annotation", selected))
        } else {
            let sub = level.to_lowercase();
            EntityPath::from(format!("{ANNOTATIONS_ENTITY_BASE}/{sub}"))
        };

        // Snap time to the latest actual data point for the selected entity.
        // This avoids annotating at a time where the entity has no data,
        // which would show "nothing logged at that time" in the text log.
        let snapped_time = if let Some(selected) = selected_entity {
            let query = re_chunk_store::LatestAtQuery::new(*timeline.name(), time);
            let chunks = ctx.recording()
                .storage_engine()
                .store()
                .latest_at_relevant_chunks_for_all_components(&query, selected, true);
            // The first chunk's time column gives us the actual data time
            chunks
                .first()
                .and_then(|chunk| chunk.timelines().get(timeline.name()))
                .and_then(|col| {
                    // Find the latest time <= cursor time
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

        // Record locally
        self.annotations.push(AnnotationEntry {
            text: text.clone(),
            level: level.clone(),
            timeline: timeline.clone(),
            time: snapped_time,
            entity_path: entity_path.clone(),
            source_entity: selected_entity.cloned(),
            color,
        });

        // Send command to write into recording
        ctx.command_sender()
            .send_system(SystemCommand::AddAnnotation {
                store_id: store_id.clone(),
                entity_path,
                timeline: timeline.clone(),
                time: snapped_time,
                text,
                level,
            });
    }

    /// Save custom tags to the recording as a static TextDocument entity.
    /// This allows tags to be exported alongside annotations.
    fn save_tags_to_recording(
        &self,
        ctx: &ViewerContext<'_>,
        store_id: &re_log_types::StoreId,
    ) {
        let tags_json = serde_json::to_string_pretty(&self.custom_tags).unwrap_or_default();
        let entity_path = EntityPath::from(format!("{ANNOTATIONS_ENTITY_BASE}/_config/tags"));

        let text_doc = re_types::archetypes::TextDocument::new(tags_json)
            .with_media_type("text/plain");

        // Use a static (timeless) timepoint
        let timepoint = TimePoint::default();

        if let Ok(chunk) = Chunk::builder(entity_path)
            .with_archetype(RowId::new(), timepoint, &text_doc)
            .build()
        {
            ctx.command_sender()
                .send_system(SystemCommand::UpdateRecording(
                    store_id.clone(),
                    vec![chunk],
                ));
        }
    }

    /// Load existing annotations from the recording's EntityDb into the session list.
    /// This allows continuity after importing an annotations.rrd.
    fn load_annotations_from_store(&mut self, entity_db: &re_entity_db::EntityDb) {
        let annotation_prefix = EntityPath::from(ANNOTATIONS_ENTITY_BASE);
        let annotation_suffix = "_annotation";
        let config_prefix = EntityPath::from(format!("{ANNOTATIONS_ENTITY_BASE}/_config"));

        let engine = entity_db.storage_engine();

        for chunk in engine.store().iter_chunks() {
            let path = chunk.entity_path();

            // Skip config entities
            if path.starts_with(&config_prefix) {
                continue;
            }

            let is_annotation = path.starts_with(&annotation_prefix)
                || path.to_string().ends_with(annotation_suffix);

            if !is_annotation {
                continue;
            }

            // Extract text component for each row
            for row_idx in 0..chunk.num_rows() {
                let text: Option<re_types::components::Text> =
                    chunk.component_mono(row_idx).and_then(|r| r.ok());

                let level: Option<re_types::components::TextLogLevel> =
                    chunk.component_mono(row_idx).and_then(|r| r.ok());

                if let Some(text) = text {
                    let text_str = text.0 .0.as_str().to_owned();
                    let level_str = level
                        .map(|l| l.0 .0.as_str().to_owned())
                        .unwrap_or_else(|| "INFO".to_owned());

                    // Get time from the first timeline in this chunk
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

                    // Avoid duplicates
                    let already_exists = self.annotations.iter().any(|a| {
                        a.entity_path == *path
                            && a.time == time
                            && a.text == text_str
                    });

                    if !already_exists {
                        // Infer source entity from path:
                        // If path ends with /_annotation, the source is the parent
                        let path_str = path.to_string();
                        let source_entity = if path_str.ends_with("/_annotation") {
                            let parent = path_str.trim_end_matches("/_annotation");
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

    /// Try to load custom tags from the recording's tag config entity.
    pub fn load_tags_from_recording(&mut self, entity_db: &re_entity_db::EntityDb) {
        let entity_path = EntityPath::from(format!("{ANNOTATIONS_ENTITY_BASE}/_config/tags"));
        let query = re_chunk_store::LatestAtQuery::latest(re_chunk::TimelineName::log_time());

        if let Some((_idx, text)) =
            entity_db.latest_at_component::<re_types::components::Text>(&entity_path, &query)
        {
            // Text -> Utf8 -> ArrowString -> as_str()
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
    let text_log = TextLog::new(text).with_level(re_types::components::TextLogLevel(
        level.into(),
    ));

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

/// Prepare annotation export: collect annotation chunks, open file dialog,
/// and return a closure that can be passed to `BackgroundTasks::spawn_file_saver`.
///
/// Returns `Ok(None)` if the user cancels the file dialog or there are no annotations.
#[cfg(not(target_arch = "wasm32"))]
pub fn prepare_annotation_export(
    entity_db: &re_entity_db::EntityDb,
) -> anyhow::Result<Option<Box<dyn FnOnce() -> anyhow::Result<std::path::PathBuf> + Send>>> {
    let rrd_version = entity_db
        .store_info()
        .and_then(|info| info.store_version)
        .unwrap_or(re_build_info::CrateVersion::LOCAL);

    let store_id = entity_db.store_id();

    // Collect all chunks that belong to annotation entity paths:
    // - /annotations/** (generic annotations)
    // - **/_annotation (entity-specific annotations)
    let annotation_prefix = EntityPath::from(ANNOTATIONS_ENTITY_BASE);
    let annotation_suffix = "_annotation";

    let mut annotation_chunks: Vec<Arc<Chunk>> = entity_db
        .storage_engine()
        .store()
        .iter_chunks()
        .filter(|chunk| {
            let path = chunk.entity_path();
            path.starts_with(&annotation_prefix)
                || path.to_string().ends_with(annotation_suffix)
        })
        .cloned()
        .collect();

    if annotation_chunks.is_empty() {
        re_log::warn!("No annotation data to export.");
        return Ok(None);
    }

    // Sort by row ID for deterministic output
    annotation_chunks.sort_by_key(|chunk| chunk.row_id_range().map(|(min, _)| min));

    // Build log messages
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

    // Open file dialog
    let path = rfd::FileDialog::new()
        .set_file_name("annotations.rrd")
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
