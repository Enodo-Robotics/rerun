#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

use re_chunk::{Chunk, EntityPath, RowId, Timeline};
use re_log_types::{TimeInt, TimePoint, TimeReal};
use re_sdk_types::archetypes::TextLog;
use re_ui::annotation_chips::tag_chip;
use re_viewer_context::event_labels::{
    LABELS_CONFIG_PART, LABELS_ENTITY_BASE, event_label_color, event_label_for_path, label_slug,
    labelled_entity_for_path,
};
use re_viewer_context::{
    SystemCommand, SystemCommandSender as _, TimeControlCommand, ViewerContext,
};

// ---------------------------------------------------------------------------

/// Labels offered on first run. Once seeded, deletions persist across restarts.
const DEFAULT_LABELS: &[&str] = &["Anomaly", "Interesting", "Bug", "Note"];

/// Opens the picker for an existing label.
const PICK_LABEL_KEY: egui::Key = egui::Key::E;

/// Opens the picker for a new label.
const NEW_LABEL_KEY: egui::Key = egui::Key::N;

/// In-memory record of a label that has been placed on the timeline.
#[derive(Clone, Debug)]
struct AnnotationEntry {
    text: String,
    timeline: Timeline,
    time: TimeInt,
    entity_path: EntityPath,

    /// The entity that was selected when this label was placed.
    source_entity: Option<EntityPath>,
}

/// A label the user can place. Its colour follows from its name.
///
/// State saved by older versions also has `level` and `color`; serde ignores them.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
struct CustomTag {
    label: String,
}

/// What the label picker is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickerMode {
    /// `E`: pick a defined label by typing until only one matches.
    Existing,

    /// `N`: type the name of a new label.
    New,
}

#[derive(Clone, Debug)]
struct Picker {
    mode: PickerMode,
    text: String,

    /// False on the frame the shortcut was pressed, so the key's own text event is not typed
    /// into the field.
    shown: bool,
}

/// Result of matching typed text against the defined labels.
#[derive(Debug, PartialEq, Eq)]
enum LabelMatch<'a> {
    /// Exactly one label starts with the typed text.
    Unique(&'a str),

    /// Several labels start with it (or nothing is typed yet), in their original order.
    Several(Vec<&'a str>),

    /// No label starts with it.
    None,
}

/// Case-insensitive prefix match of `typed` against `labels`.
///
/// Nothing typed matches every label but never counts as unique, so pressing `E` with a single
/// label defined does not add it before you have typed anything.
fn match_label<'a>(labels: &[&'a str], typed: &str) -> LabelMatch<'a> {
    let typed = typed.trim().to_lowercase();
    let matches: Vec<&str> = labels
        .iter()
        .copied()
        .filter(|label| label.to_lowercase().starts_with(&typed))
        .collect();

    match matches.as_slice() {
        [] => LabelMatch::None,
        [only] if !typed.is_empty() => LabelMatch::Unique(only),
        _ => LabelMatch::Several(matches),
    }
}

/// How many leading characters of `label` set it apart from every other label (all of them, if
/// it is a prefix of another label).
fn unique_prefix_len(label: &str, labels: &[&str]) -> usize {
    let lower = label.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    (1..=chars.len())
        .find(|&n| {
            let prefix: String = chars[..n].iter().collect();
            labels
                .iter()
                .filter(|other| other.to_lowercase().starts_with(&prefix))
                .count()
                == 1
        })
        .unwrap_or(chars.len())
}

/// The annotation panel's colours that are not label colours.
#[expect(clippy::disallowed_methods)]
fn rgb(r: u8, g: u8, b: u8) -> egui::Color32 {
    egui::Color32::from_rgb(r, g, b)
}

/// The labelling panel state, persisted across frames.
#[derive(Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct AnnotationPanel {
    /// Whether the editor side panel is currently shown.
    pub editor_visible: bool,

    custom_tags: Vec<CustomTag>,

    /// True once `DEFAULT_LABELS` have been seeded into `custom_tags`. Prevents
    /// re-seeding after the user has deleted all labels.
    defaults_seeded: bool,

    #[serde(skip)]
    text_input: String,

    #[serde(skip)]
    new_tag_label: String,

    #[serde(skip)]
    annotations: Vec<AnnotationEntry>,

    /// Locked entity selection — persists so clicking panel buttons doesn't clear it.
    #[serde(skip)]
    locked_entity: Option<EntityPath>,

    #[serde(skip)]
    tags_loaded_for: Option<re_log_types::StoreId>,

    /// Fixed autosave path for this recording, computed once on first save.
    #[serde(skip)]
    autosave_path: Option<std::path::PathBuf>,

    /// The `E` / `N` label picker, while it is open.
    #[serde(skip)]
    picker: Option<Picker>,
}

impl AnnotationPanel {
    /// Seed default labels on first run. Once seeded, never re-seeds — so deleting
    /// all labels and restarting doesn't bring them back.
    fn ensure_default_tags(&mut self) {
        if !self.defaults_seeded {
            if self.custom_tags.is_empty() {
                self.custom_tags = DEFAULT_LABELS
                    .iter()
                    .map(|label| CustomTag {
                        label: (*label).to_owned(),
                    })
                    .collect();
            }
            self.defaults_seeded = true;
        }
    }

    fn has_tag(&self, label: &str) -> bool {
        let slug = label_slug(label);
        self.custom_tags
            .iter()
            .any(|t| label_slug(&t.label) == slug)
    }

    pub fn toggle_editor(&mut self) {
        self.editor_visible = !self.editor_visible;
    }

    /// Everything the labelling feature shows each frame: the `E` / `N` shortcuts and their
    /// picker, and the editor side panel when it is open.
    pub fn ui(&mut self, ctx: &ViewerContext<'_>, ui: &mut egui::Ui) {
        self.ensure_default_tags();

        let recording_id = ctx.store_context.recording.store_id().clone();
        if self.tags_loaded_for.as_ref() != Some(&recording_id) {
            self.annotations.clear();
            self.autosave_path = None;
            self.picker = None;
            self.load_tags_from_recording(ctx.store_context.recording);
            self.load_annotations_from_store(ctx.store_context.recording);
            self.tags_loaded_for = Some(recording_id);
        }

        self.update_locked_entity(ctx);
        self.handle_shortcuts(ui.ctx());
        self.picker_ui(ctx, ui.ctx());
        self.show_panel(ctx, ui);
    }

    /// Save annotations to disk now. Called after every add/remove — no data
    /// loss on crash. Silently no-ops on wasm or if there are no annotations.
    #[cfg(not(target_arch = "wasm32"))]
    fn save_now(&mut self, ctx: &ViewerContext<'_>) {
        if self.autosave_path.is_none() {
            self.autosave_path = Some(compute_autosave_path(ctx.store_context.recording));
        }
        if let Some(path) = &self.autosave_path {
            // Source from `self.annotations` (the panel's in-memory truth)
            // rather than scanning the store. Add/Clear go through
            // `command_sender.send_system`, which only processes on the next
            // frame, so reading the store here would lag one event behind.
            autosave_annotations_to(ctx.store_context.recording, &self.annotations, path);
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

        if let Some(ref sel) = current_selection
            && self.locked_entity.as_ref() != Some(sel)
        {
            self.locked_entity = Some(sel.clone());
        }
    }

    /// `E` opens the picker for an existing label, `N` for a new one. Ignored while a text
    /// field has focus, so typing an `e` anywhere else still types an `e`.
    ///
    /// Only a focused *text field* blocks them: clicking a 3D view, a list item or a button
    /// also gives that widget keyboard focus, and the keys must still work then.
    fn handle_shortcuts(&mut self, egui_ctx: &egui::Context) {
        if self.picker.is_some() || egui_ctx.text_edit_focused() {
            return;
        }

        let mode = egui_ctx.input(|i| {
            if !i.modifiers.is_none() {
                None
            } else if i.key_pressed(PICK_LABEL_KEY) {
                Some(PickerMode::Existing)
            } else if i.key_pressed(NEW_LABEL_KEY) {
                Some(PickerMode::New)
            } else {
                None
            }
        });

        if let Some(mode) = mode {
            self.picker = Some(Picker {
                mode,
                text: String::new(),
                shown: false,
            });
            egui_ctx.request_repaint();
        }
    }

    fn picker_ui(&mut self, ctx: &ViewerContext<'_>, egui_ctx: &egui::Context) {
        let Some(picker) = &mut self.picker else {
            return;
        };
        if !picker.shown {
            picker.shown = true;
            egui_ctx.request_repaint();
            return;
        }

        let labels: Vec<String> = self.custom_tags.iter().map(|t| t.label.clone()).collect();
        let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let timeline = ctx.time_ctrl.timeline().copied();
        let time = ctx.time_ctrl.time_int();
        let timestamp_format = ctx.app_options().timestamp_format;

        let mut chosen: Option<String> = None;
        let mut close = false;

        let modal = egui::Modal::new(egui::Id::new("label_picker")).show(egui_ctx, |ui| {
            ui.set_width(320.0);
            let mode = picker.mode;

            ui.strong(match mode {
                PickerMode::Existing => "Add label",
                PickerMode::New => "New label",
            });
            match (timeline, time) {
                (Some(timeline), Some(time)) => {
                    let at = timeline.typ().format(time, timestamp_format);
                    match &self.locked_entity {
                        Some(entity) => ui.weak(format!("at {at} on {entity}")),
                        None => ui.weak(format!("at {at}")),
                    };
                }
                _ => {
                    ui.weak("No time cursor: nothing will be added");
                }
            }
            ui.add_space(4.0);

            let response = ui.add(
                egui::TextEdit::singleline(&mut picker.text)
                    .hint_text(match mode {
                        PickerMode::Existing => "Type to pick a label…",
                        PickerMode::New => "Name of the new label…",
                    })
                    .desired_width(f32::INFINITY),
            );
            response.request_focus();
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));

            match mode {
                PickerMode::Existing => {
                    let found = match_label(&label_refs, &picker.text);
                    let candidates: Vec<&str> = match &found {
                        LabelMatch::Unique(label) => vec![label],
                        LabelMatch::Several(labels) => labels.clone(),
                        LabelMatch::None => Vec::new(),
                    };

                    // Enter takes the exact match if there is one, else the first candidate.
                    let typed = picker.text.trim().to_lowercase();
                    let enter_target = candidates
                        .iter()
                        .find(|l| l.to_lowercase() == typed)
                        .or_else(|| candidates.first())
                        .copied();

                    if let LabelMatch::Unique(label) = found {
                        chosen = Some(label.to_owned());
                    } else if enter {
                        if let Some(label) = enter_target {
                            chosen = Some(label.to_owned());
                        } else if !typed.is_empty() {
                            chosen = Some(picker.text.trim().to_owned());
                        }
                    }

                    ui.add_space(4.0);
                    if candidates.is_empty() {
                        if typed.is_empty() {
                            ui.weak("No labels yet: press Esc, then N to make one.");
                        } else {
                            ui.weak(format!(
                                "No label starts with {:?}: Enter makes it a new label.",
                                picker.text.trim()
                            ));
                        }
                    }
                    for label in &candidates {
                        let n = unique_prefix_len(label, &label_refs);
                        let split = label.char_indices().nth(n).map_or(label.len(), |(i, _)| i);
                        let (head, tail) = label.split_at(split);
                        let is_target = Some(*label) == enter_target;
                        let row = ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.colored_label(event_label_color(label), "⏺ ");
                            ui.label(egui::RichText::new(head).strong().underline());
                            ui.label(tail);
                            if is_target {
                                ui.weak("   ⏎");
                            }
                        });
                        if ui
                            .interact(
                                row.response.rect,
                                ui.id().with(("label_pick", *label)),
                                egui::Sense::click(),
                            )
                            .clicked()
                        {
                            chosen = Some((*label).to_owned());
                        }
                    }
                }
                PickerMode::New => {
                    let name = picker.text.trim();
                    ui.add_space(4.0);
                    if name.is_empty() {
                        ui.weak("Enter adds it at the time cursor.");
                    } else {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.colored_label(event_label_color(name), "⏺ ");
                            ui.label(name);
                        });
                        if enter {
                            chosen = Some(name.to_owned());
                        }
                    }
                }
            }

            ui.add_space(4.0);
            ui.weak("Esc to cancel");
        });

        if modal.should_close() {
            close = true;
        }

        if let Some(label) = chosen {
            close = true;
            if !self.has_tag(&label) {
                self.custom_tags.push(CustomTag {
                    label: label.clone(),
                });
                self.save_tags_to_recording(ctx, ctx.store_context.recording.store_id());
            }
            self.add_label_at_cursor(ctx, label);
        }

        if close {
            self.picker = None;
        }
    }

    /// Place `label` at the time cursor, on the locked entity if any.
    fn add_label_at_cursor(&mut self, ctx: &ViewerContext<'_>, label: String) {
        let (Some(timeline), Some(time)) =
            (ctx.time_ctrl.timeline().copied(), ctx.time_ctrl.time_int())
        else {
            return;
        };
        let store_id = ctx.store_context.recording.store_id().clone();
        let target = self.locked_entity.clone();
        self.add_annotation(ctx, &store_id, &timeline, time, label, target.as_ref());
    }

    /// The editor side panel.
    fn show_panel(&mut self, ctx: &ViewerContext<'_>, ui: &mut egui::Ui) {
        if !self.editor_visible {
            return;
        }

        let screen_width = ui.ctx().content_rect().width();

        let panel = egui::Panel::right("annotation_panel")
            .resizable(true)
            .min_size(200.0)
            .default_size(300.0)
            .max_size((0.45 * screen_width).round())
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
        ui.strong("Labels");
        ui.add_space(4.0);
        ui.weak(format!(
            "{} adds a label at the time cursor (type until one matches), {} adds a new one.",
            PICK_LABEL_KEY.name(),
            NEW_LABEL_KEY.name()
        ));
        ui.add_space(4.0);

        let current_timeline = ctx.time_ctrl.timeline().copied();
        let current_time = ctx.time_ctrl.time_int();
        let timestamp_format = ctx.app_options().timestamp_format;

        match (&current_timeline, current_time) {
            (Some(tl), Some(t)) => {
                ui.label(format!(
                    "{}: {}",
                    tl.name(),
                    tl.typ().format(t, timestamp_format)
                ));
            }
            _ => {
                ui.label("No time cursor");
            }
        }

        let selected_entity = self.locked_entity.clone();

        ui.add_space(4.0);
        let mut clear_locked = false;
        if let Some(ref entity) = selected_entity {
            ui.horizontal(|ui| {
                ui.label("Target:");
                ui.colored_label(rgb(130, 200, 130), entity.to_string());
                if ui.small_button("Clear").clicked() {
                    clear_locked = true;
                }
            });
        } else {
            ui.horizontal(|ui| {
                ui.label("Target:");
                ui.weak("(none)");
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

        let store_id = ctx.store_context.recording.store_id().clone();

        // Label chips: click to place, right-click to delete.
        {
            let mut tag_to_remove: Option<usize> = None;
            let mut tag_to_annotate: Option<String> = None;

            let labels: Vec<String> = self.custom_tags.iter().map(|t| t.label.clone()).collect();

            // Only protect labels that are placed in the current recording.
            let tags_in_use: std::collections::HashSet<String> = self
                .annotations
                .iter()
                .map(|a| label_slug(&a.text))
                .collect();

            ui.horizontal_wrapped(|ui| {
                for (idx, label) in labels.iter().enumerate() {
                    let in_use = tags_in_use.contains(&label_slug(label));
                    let response = tag_chip(ui, label, event_label_color(label), None);
                    if response.clicked() {
                        tag_to_annotate = Some(label.clone());
                    }
                    if response.secondary_clicked() && !in_use {
                        tag_to_remove = Some(idx);
                    }
                    if in_use {
                        response.on_hover_text(
                            "Click to place at the time cursor. In use — remove its labels first to delete it.",
                        );
                    } else {
                        response.on_hover_text(
                            "Click to place at the time cursor. Right-click to delete.",
                        );
                    }
                }
            });

            if let Some(label) = tag_to_annotate {
                self.add_label_at_cursor(ctx, label);
            }
            if let Some(idx) = tag_to_remove {
                self.custom_tags.remove(idx);
                self.save_tags_to_recording(ctx, &store_id);
            }
        }

        ui.add_space(4.0);

        let label_response = ui
            .horizontal(|ui| {
                ui.label("New label:");
                ui.text_edit_singleline(&mut self.new_tag_label)
            })
            .inner;
        let trimmed = self.new_tag_label.trim().to_owned();
        let duplicate = self.has_tag(&trimmed);
        let can_add = !trimmed.is_empty() && !duplicate;
        let enter_pressed =
            label_response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (can_add && enter_pressed)
            || ui
                .add_enabled(can_add, egui::Button::new("Add label"))
                .clicked()
        {
            self.custom_tags.push(CustomTag { label: trimmed });
            self.new_tag_label.clear();
            self.save_tags_to_recording(ctx, &store_id);
        }
        if duplicate && !self.new_tag_label.trim().is_empty() {
            ui.weak("Label already exists");
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        ui.strong("Free-text note");
        ui.add_space(4.0);

        let response = ui.add(
            egui::TextEdit::multiline(&mut self.text_input)
                .hint_text("Type a note here…")
                .desired_width(ui.available_width())
                .desired_rows(3),
        );

        ui.add_space(4.0);

        let can_submit = !self.text_input.trim().is_empty()
            && current_time.is_some()
            && current_timeline.is_some();

        let ctrl_enter = response.has_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);

        if ui
            .add_enabled(can_submit, egui::Button::new("Add note"))
            .clicked()
            || (can_submit && ctrl_enter)
        {
            let text = self.text_input.trim().to_owned();
            self.text_input.clear();
            self.add_label_at_cursor(ctx, text);
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        ui.strong(format!("In this recording ({})", self.annotations.len()));
        ui.add_space(4.0);

        if self.annotations.is_empty() {
            ui.weak("No labels yet.");
        } else {
            let mut to_remove: Option<usize> = None;

            for (idx, ann) in self.annotations.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.colored_label(event_label_color(&ann.text), "⏺");

                    let time_text = ann.timeline.typ().format(ann.time, timestamp_format);
                    let time_response = ui.small_button(&time_text);
                    if time_response.clicked() {
                        ctx.command_sender()
                            .send_system(SystemCommand::TimeControlCommands {
                                store_id: store_id.clone(),
                                time_commands: vec![
                                    TimeControlCommand::SetActiveTimeline(*ann.timeline.name()),
                                    TimeControlCommand::SetTime(TimeReal::from(ann.time)),
                                ],
                            });
                        if let Some(ref source) = ann.source_entity {
                            ctx.command_sender()
                                .send_system(SystemCommand::set_selection(
                                    re_viewer_context::Item::InstancePath(
                                        re_entity_db::InstancePath::entity_all(source.clone()),
                                    ),
                                ));
                        }
                    }
                    time_response.on_hover_text(format!(
                        "Jump to this label ({} timeline)",
                        ann.timeline.name()
                    ));

                    ui.label(&ann.text);
                    if let Some(ref source) = ann.source_entity {
                        ui.weak(format!("@ {source}"));
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Remove").clicked() {
                            to_remove = Some(idx);
                        }
                    });
                });
            }

            if let Some(idx) = to_remove {
                let ann = self.annotations.remove(idx);
                ctx.command_sender()
                    .send_system(SystemCommand::ClearAnnotation {
                        store_id: store_id.clone(),
                        entity_path: ann.entity_path.clone(),
                        timeline: ann.timeline,
                        time: ann.time,
                    });
                #[cfg(not(target_arch = "wasm32"))]
                self.save_now(ctx);
            }
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        #[cfg(not(target_arch = "wasm32"))]
        {
            let has_annotations = !self.annotations.is_empty()
                || ctx
                    .store_context
                    .recording
                    .sorted_entity_paths()
                    .any(is_label_entity);

            if ui
                .add_enabled(has_annotations, egui::Button::new("Export labels to .rrd"))
                .on_hover_text(
                    "Save only the labels to a separate .rrd file.\n\
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
    }

    fn add_annotation(
        &mut self,
        ctx: &ViewerContext<'_>,
        store_id: &re_log_types::StoreId,
        timeline: &Timeline,
        time: TimeInt,
        text: String,
        selected_entity: Option<&EntityPath>,
    ) {
        // Snap the label time to the latest actual data point for the
        // selected entity. Avoids attaching the label at a cursor position
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

        // Per-row entity path: `<entity>/_annotation/<label>/<time>`. Each label
        // gets its own unique path so it can be removed in isolation via
        // `drop_entity_path` without disturbing siblings with the same name.
        let tag_slug = label_slug(&text);
        let time_segment = snapped_time.as_i64().to_string();
        let entity_path = if let Some(selected) = selected_entity {
            EntityPath::from(format!("{selected}/_annotation/{tag_slug}/{time_segment}"))
        } else {
            EntityPath::from(format!("{LABELS_ENTITY_BASE}/{tag_slug}/{time_segment}"))
        };

        self.annotations.push(AnnotationEntry {
            text: text.clone(),
            timeline: *timeline,
            time: snapped_time,
            entity_path: entity_path.clone(),
            source_entity: selected_entity.cloned(),
        });
        self.annotations.sort_by_key(|a| a.time);

        ctx.command_sender()
            .send_system(SystemCommand::AddAnnotation {
                store_id: store_id.clone(),
                entity_path,
                timeline: *timeline,
                time: snapped_time,
                text,
            });

        #[cfg(not(target_arch = "wasm32"))]
        self.save_now(ctx);
    }

    fn save_tags_to_recording(&self, ctx: &ViewerContext<'_>, store_id: &re_log_types::StoreId) {
        let tags_json = serde_json::to_string_pretty(&self.custom_tags).unwrap_or_default();
        let entity_path =
            EntityPath::from(format!("{LABELS_ENTITY_BASE}/{LABELS_CONFIG_PART}/tags"));

        let text_doc =
            re_sdk_types::archetypes::TextDocument::new(tags_json).with_media_type("text/plain");

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

    fn load_annotations_from_store(&mut self, entity_db: &re_entity_db::EntityDb) {
        let engine = entity_db.storage_engine();

        for chunk in engine.store().iter_physical_chunks() {
            let path = chunk.entity_path();

            // `rr.mark_event` markers are drawn on the timeline too, but they are not the
            // user's labels to edit here.
            if !is_label_entity(path) {
                continue;
            }

            for row_idx in 0..chunk.num_rows() {
                let unit = chunk.row_sliced_unit_shallow(row_idx);
                let text: Option<re_sdk_types::components::Text> = unit
                    .component_mono(re_sdk_types::archetypes::TextLog::descriptor_text().component)
                    .and_then(|r| r.ok());

                let Some(text) = text else {
                    continue;
                };
                let text_str = text.0.0.as_str().to_owned();

                let (timeline, time) = chunk
                    .timelines()
                    .values()
                    .next()
                    .and_then(|col| {
                        let times = col.times_raw();
                        if row_idx < times.len() {
                            Some((*col.timeline(), TimeInt::new_temporal(times[row_idx])))
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

                let already_exists = self
                    .annotations
                    .iter()
                    .any(|a| a.entity_path == *path && a.time == time && a.text == text_str);

                if !already_exists {
                    // Labels found in the recording can be picked with `E` too.
                    if !self.has_tag(&text_str) && !text_str.contains('\n') {
                        self.custom_tags.push(CustomTag {
                            label: text_str.clone(),
                        });
                    }
                    self.annotations.push(AnnotationEntry {
                        text: text_str,
                        timeline,
                        time,
                        entity_path: path.clone(),
                        source_entity: labelled_entity_for_path(path),
                    });
                }
            }
        }

        self.annotations.sort_by_key(|a| a.time);
    }

    /// Adds the labels defined in the recording to the ones known to the panel.
    pub fn load_tags_from_recording(&mut self, entity_db: &re_entity_db::EntityDb) {
        let entity_path =
            EntityPath::from(format!("{LABELS_ENTITY_BASE}/{LABELS_CONFIG_PART}/tags"));
        let query = re_chunk_store::LatestAtQuery::latest(re_chunk::TimelineName::log_time());

        if let Some((_idx, text)) = entity_db.latest_at_component::<re_sdk_types::components::Text>(
            &entity_path,
            &query,
            re_sdk_types::archetypes::TextDocument::descriptor_text().component,
        ) && let Ok(tags) = serde_json::from_str::<Vec<CustomTag>>(text.0.0.as_str())
        {
            for tag in tags {
                if !self.has_tag(&tag.label) {
                    self.custom_tags.push(tag);
                }
            }
        }
    }
}

/// Whether `path` holds a label placed in the viewer (not an `rr.mark_event` marker, nor the
/// panel's configuration).
fn is_label_entity(path: &EntityPath) -> bool {
    matches!(
        event_label_for_path(path),
        Some((_, re_viewer_context::event_labels::EventLabelKind::Label))
    )
}

/// Build a [`Chunk`] containing a single [`TextLog`] annotation row.
pub fn build_annotation_chunk(
    entity_path: &EntityPath,
    timeline: &Timeline,
    time: TimeInt,
    text: &str,
) -> anyhow::Result<Chunk> {
    let text_log = TextLog::new(text);

    let timepoint = TimePoint::from_iter([(
        *timeline.name(),
        re_log_types::TimeCell::new(timeline.typ(), time.as_i64()),
    )]);

    let chunk = Chunk::builder(entity_path.clone())
        .with_archetype(RowId::new(), timepoint, &text_log)
        .build()?;

    Ok(chunk)
}

/// Writes the exported annotations, returning the path written to.
#[cfg(not(target_arch = "wasm32"))]
pub type AnnotationSaver = Box<dyn FnOnce() -> anyhow::Result<std::path::PathBuf> + Send>;

/// Collect annotation chunks, open a save dialog, and return a closure for the file saver.
///
/// Returns `None` if the user cancels or there are no annotations.
#[cfg(not(target_arch = "wasm32"))]
pub fn prepare_annotation_export(entity_db: &re_entity_db::EntityDb) -> Option<AnnotationSaver> {
    let rrd_version = entity_db
        .store_info()
        .and_then(|info| info.store_version)
        .unwrap_or(re_build_info::CrateVersion::LOCAL);

    let store_id = entity_db.store_id();

    let annotation_prefix = EntityPath::from(LABELS_ENTITY_BASE);
    let mut annotation_chunks: Vec<Arc<Chunk>> = entity_db
        .storage_engine()
        .store()
        .iter_physical_chunks()
        .filter(|chunk| {
            let path = chunk.entity_path();
            path.starts_with(&annotation_prefix) || is_label_entity(path)
        })
        .cloned()
        .collect();

    if annotation_chunks.is_empty() {
        re_log::warn!("No annotation data to export.");
        return None;
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

    let path = path?;

    Some(Box::new(move || {
        crate::saving::encode_to_file(rrd_version, &path, messages.into_iter())?;
        Ok(path)
    }))
}

/// Compute the autosave path once per session.
/// Returns `{recording_dir}/{recording_name}_{session_id}_annotations.rrd`.
/// Session ID is derived from the wall clock + process id so reopening the
/// same recording in a new viewer session doesn't clobber earlier autosaves.
#[cfg(not(target_arch = "wasm32"))]
fn compute_autosave_path(entity_db: &re_entity_db::EntityDb) -> std::path::PathBuf {
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

    save_dir.join(format!("{file_stem}_{session_id}_annotations.rrd"))
}

/// Write all current annotation chunks to `save_path`. Called after every
/// add/remove so an unexpected crash doesn't lose work.
///
/// Chunks are built from the panel's in-memory `annotations` list, not by
/// scanning the recording — `AddAnnotation` / `ClearAnnotation` are queued
/// system commands that don't take effect until the next frame, so the live
/// store would lag one event behind.
#[cfg(not(target_arch = "wasm32"))]
fn autosave_annotations_to(
    entity_db: &re_entity_db::EntityDb,
    annotations: &[AnnotationEntry],
    save_path: &std::path::Path,
) {
    let rrd_version = entity_db
        .store_info()
        .and_then(|info| info.store_version)
        .unwrap_or(re_build_info::CrateVersion::LOCAL);

    let store_id = entity_db.store_id();

    if annotations.is_empty() {
        // Truncate the file so removing the last annotation isn't silently
        // dropped (otherwise a stale save would survive on disk).
        if save_path.exists()
            && let Err(err) = std::fs::remove_file(save_path)
        {
            re_log::warn!(
                "Failed to remove stale annotation autosave {}: {err}",
                save_path.display()
            );
        }
        return;
    }

    let chunks: Vec<Arc<Chunk>> = annotations
        .iter()
        .filter_map(|ann| {
            match build_annotation_chunk(&ann.entity_path, &ann.timeline, ann.time, &ann.text) {
                Ok(chunk) => Some(Arc::new(chunk)),
                Err(err) => {
                    re_log::warn_once!("Failed to build annotation chunk for autosave: {err}");
                    None
                }
            }
        })
        .collect();

    if chunks.is_empty() {
        return;
    }

    let store_info_msg = entity_db
        .store_info_msg()
        .map(|msg| Ok(re_log_types::LogMsg::SetStoreInfo(msg.clone())));

    let data_messages: Vec<re_chunk::ChunkResult<re_log_types::LogMsg>> = chunks
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

#[cfg(test)]
mod tests {
    use super::*;

    const LABELS: &[&str] = &["correct", "comma", "Bug"];

    /// Focuses the widget `show` adds, then presses `key`, and returns the picker mode the
    /// shortcut opened, if any.
    fn press_with_focus(key: egui::Key, show: impl Fn(&mut egui::Ui)) -> Option<PickerMode> {
        let ctx = egui::Context::default();
        let mut panel = AnnotationPanel::default();

        // Focus requests take effect on the next frame.
        let _ignored = ctx.run_ui(egui::RawInput::default(), &show);

        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let _ignored = ctx.run_ui(input, |ui| {
            show(ui);
            panel.handle_shortcuts(ui.ctx());
        });
        panel.picker.map(|p| p.mode)
    }

    #[test]
    fn shortcuts_work_while_a_non_text_widget_has_focus() {
        let button = |ui: &mut egui::Ui| {
            ui.button("view").request_focus();
        };
        assert_eq!(
            press_with_focus(PICK_LABEL_KEY, button),
            Some(PickerMode::Existing)
        );
        assert_eq!(
            press_with_focus(NEW_LABEL_KEY, button),
            Some(PickerMode::New)
        );
    }

    #[test]
    fn shortcuts_are_ignored_while_a_text_field_has_focus() {
        let text_field = |ui: &mut egui::Ui| {
            let mut text = String::new();
            ui.text_edit_singleline(&mut text).request_focus();
        };
        assert_eq!(press_with_focus(PICK_LABEL_KEY, text_field), None);
    }

    #[test]
    fn typing_disambiguates() {
        assert_eq!(
            match_label(LABELS, ""),
            LabelMatch::Several(LABELS.to_vec())
        );
        assert_eq!(
            match_label(LABELS, "c"),
            LabelMatch::Several(vec!["correct", "comma"])
        );
        assert_eq!(
            match_label(LABELS, "co"),
            LabelMatch::Several(vec!["correct", "comma"])
        );
        assert_eq!(match_label(LABELS, "cor"), LabelMatch::Unique("correct"));
        assert_eq!(match_label(LABELS, "com"), LabelMatch::Unique("comma"));
        assert_eq!(match_label(LABELS, "b"), LabelMatch::Unique("Bug"));
        assert_eq!(match_label(LABELS, "x"), LabelMatch::None);
    }

    #[test]
    fn nothing_typed_is_never_unique() {
        assert_eq!(
            match_label(&["only"], ""),
            LabelMatch::Several(vec!["only"])
        );
    }

    #[test]
    fn a_label_that_prefixes_another_stays_ambiguous() {
        assert_eq!(
            match_label(&["cor", "correct"], "cor"),
            LabelMatch::Several(vec!["cor", "correct"])
        );
        assert_eq!(
            match_label(&["cor", "correct"], "corr"),
            LabelMatch::Unique("correct")
        );
    }

    #[test]
    fn unique_prefix() {
        assert_eq!(unique_prefix_len("correct", LABELS), 3);
        assert_eq!(unique_prefix_len("comma", LABELS), 3);
        assert_eq!(unique_prefix_len("Bug", LABELS), 1);
        assert_eq!(unique_prefix_len("cor", &["cor", "correct"]), 3);
    }

    #[test]
    fn old_saved_tags_still_load() {
        let json = r#"[{"label": "Bug", "level": "ERROR", "color": [255, 80, 80]}]"#;
        let tags: Vec<CustomTag> = serde_json::from_str(json).unwrap();
        assert_eq!(tags[0].label, "Bug");
    }
}
