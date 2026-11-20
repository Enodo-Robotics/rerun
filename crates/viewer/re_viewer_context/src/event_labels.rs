//! RRL fork: the entity paths that hold labels and event markers, shared by the labelling panel
//! (which writes them) and the time panel (which draws them on the timeline).

use re_log_types::EntityPath;

/// Entity path under which labels without a target entity are stored:
/// `annotations/<label>/<time>`.
pub const LABELS_ENTITY_BASE: &str = "annotations";

/// Path part under a target entity that holds its labels: `<entity>/_annotation/<label>/<time>`.
pub const LABELS_ENTITY_PART: &str = "_annotation";

/// Path part, directly under [`LABELS_ENTITY_BASE`], for the labelling panel's own configuration.
pub const LABELS_CONFIG_PART: &str = "_config";

/// What a timeline marker stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventLabelKind {
    /// Added by hand in the viewer's labelling panel.
    Label,

    /// Logged by a program with `rr.mark_event`.
    Event,
}

/// If `path` holds a label or an event marker, its name and kind.
///
/// * `annotations/<label>/<time>` and `<entity>/_annotation/<label>/<time>` are labels;
/// * `_event/<name>` is an event marker (see [`re_log_types::EVENT_MARKER_ENTITY_PATH_PREFIX`]).
pub fn event_label_for_path(path: &EntityPath) -> Option<(String, EventLabelKind)> {
    let parts = path.as_slice();

    if let [first, name, ..] = parts
        && first.unescaped_str() == re_log_types::EVENT_MARKER_ENTITY_PATH_PREFIX
    {
        return Some((name.unescaped_str().to_owned(), EventLabelKind::Event));
    }

    if let [first, label, _time] = parts
        && first.unescaped_str() == LABELS_ENTITY_BASE
        && label.unescaped_str() != LABELS_CONFIG_PART
    {
        return Some((label.unescaped_str().to_owned(), EventLabelKind::Label));
    }

    let n = parts.len();
    if n >= 3 && parts[n - 3].unescaped_str() == LABELS_ENTITY_PART {
        return Some((
            parts[n - 2].unescaped_str().to_owned(),
            EventLabelKind::Label,
        ));
    }

    None
}

/// The entity a label is attached to, for `<entity>/_annotation/<label>/<time>`.
pub fn labelled_entity_for_path(path: &EntityPath) -> Option<EntityPath> {
    let parts = path.as_slice();
    let n = parts.len();
    (n > 3 && parts[n - 3].unescaped_str() == LABELS_ENTITY_PART)
        .then(|| EntityPath::new(parts[..n - 3].to_vec()))
}

/// Path part a label name is stored under: lower-case, spaces as `_`.
pub fn label_slug(label: &str) -> String {
    label.trim().to_lowercase().replace(' ', "_")
}

/// A colour for a label, the same in every panel and every session.
pub fn event_label_color(label: &str) -> egui::Color32 {
    // FNV-1a: stable across runs and platforms, unlike the std hasher.
    let mut hash: u32 = 0x811c_9dc5;
    for byte in label_slug(label).bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    let hue = (hash >> 8) as f32 / (1 << 24) as f32;
    egui::Color32::from(egui::ecolor::Hsva::new(hue, 0.75, 0.9, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_paths() {
        let label = |s: &str| event_label_for_path(&EntityPath::from(s));

        assert_eq!(
            label("annotations/correct/123"),
            Some(("correct".to_owned(), EventLabelKind::Label))
        );
        assert_eq!(
            label("world/robot/_annotation/comma/7"),
            Some(("comma".to_owned(), EventLabelKind::Label))
        );
        assert_eq!(
            label("_event/hard_stop"),
            Some(("hard_stop".to_owned(), EventLabelKind::Event))
        );
        assert_eq!(label("annotations/_config/tags"), None);
        assert_eq!(label("world/robot"), None);
        assert_eq!(label("annotations"), None);

        assert_eq!(
            labelled_entity_for_path(&EntityPath::from("world/robot/_annotation/comma/7")),
            Some(EntityPath::from("world/robot"))
        );
        assert_eq!(
            labelled_entity_for_path(&EntityPath::from("annotations/comma/7")),
            None
        );
    }

    #[test]
    fn color_ignores_case() {
        assert_eq!(
            event_label_color("Hard stop"),
            event_label_color("hard_stop")
        );
    }
}
