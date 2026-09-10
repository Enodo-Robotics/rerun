//! An externally-driven request to select entities and frame the camera on them.
//!
//! A driving application — a list UI, a review tool, anything outside the viewer — logs a command
//! to a reserved entity path, and the viewer selects the named entities and moves the camera to
//! fit them. The viewer has no idea what sent a command or why: all policy (what a list element
//! maps to, debounce, when to ask for a zoom-out) belongs to the driver.

use arrow::array::{Array as _, ArrayRef, BooleanArray, LargeStringArray, StringArray};

use re_chunk_store::{LatestAtQuery, RowId};
use re_entity_db::EntityDb;
use re_log_types::{EntityPath, TimelineName};
use re_types_core::ComponentIdentifier;

/// The reserved entity path that framing commands are logged to.
///
/// Lives under [`re_log_types::VIEWER_COMMAND_ENTITY_PATH_PREFIX`], which the fork's rotating
/// file sink drops, so commands never reach a saved `.rrd`.
pub const FOCUS_COMMAND_ENTITY_PATH: &str = "viewer/_command/focus";

/// Component holding the entity paths to select, as strings.
const COMPONENT_PATHS: &str = "paths";

/// Component holding the `do_pan` flag.
const COMPONENT_DO_PAN: &str = "do_pan";

/// Component holding the `do_rotate` flag.
const COMPONENT_DO_ROTATE: &str = "do_rotate";

/// A request to select some entities and frame the camera on them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusCommand {
    /// Entities to select and frame.
    ///
    /// Each path contributes its whole subtree to the framing box, so naming `/robot` frames the
    /// meshes logged at `/robot/link_3`.
    ///
    /// Empty means "clear the selection", and — if `do_pan` is set — reframe the whole scene.
    pub entity_paths: Vec<EntityPath>,

    /// Move the camera to a distance that fits the entities.
    pub do_pan: bool,

    /// Re-aim the camera at the entities from wherever it currently stands.
    pub do_rotate: bool,
}

impl FocusCommand {
    /// Read the most recently logged command, along with the row it came from.
    ///
    /// The row id is the command's identity: the caller applies a command only when it differs
    /// from the last one applied, so scrubbing the time cursor never re-fires a command, while
    /// re-logging an identical payload does.
    ///
    /// Commands are expected to be logged as static data, which is why this queries at
    /// [`re_log_types::TimeInt::MAX`] rather than at the viewer's time cursor.
    pub fn latest_from_db(entity_db: &EntityDb) -> Option<(RowId, Self)> {
        let entity_path = EntityPath::from(FOCUS_COMMAND_ENTITY_PATH);

        let paths_component = ComponentIdentifier::from(COMPONENT_PATHS);
        let do_pan_component = ComponentIdentifier::from(COMPONENT_DO_PAN);
        let do_rotate_component = ComponentIdentifier::from(COMPONENT_DO_ROTATE);

        let query = LatestAtQuery::latest(TimelineName::log_time());
        let results = entity_db.latest_at(
            &query,
            &entity_path,
            [paths_component, do_pan_component, do_rotate_component],
        );

        if results.is_empty() {
            return None;
        }

        let (_time, row_id) = results.max_index();

        // A command that clears the selection may legitimately carry no paths at all, so a
        // missing or empty `paths` component is an empty selection rather than a parse failure.
        let entity_paths = results
            .component_batch_raw(paths_component)
            .map(|array| strings_from_arrow(&array))
            .unwrap_or_default()
            .into_iter()
            .map(EntityPath::from)
            .collect();

        let do_pan = results
            .component_batch_raw(do_pan_component)
            .and_then(|array| bool_from_arrow(&array))
            .unwrap_or(true);
        let do_rotate = results
            .component_batch_raw(do_rotate_component)
            .and_then(|array| bool_from_arrow(&array))
            .unwrap_or(false);

        Some((
            row_id,
            Self {
                entity_paths,
                do_pan,
                do_rotate,
            },
        ))
    }

    /// Does this command ask for the camera to move at all?
    pub fn moves_camera(&self) -> bool {
        self.do_pan || self.do_rotate
    }
}

/// Pull strings out of an arrow array, tolerating the several string layouts a client may send.
///
/// Commands arrive from arbitrary SDK code — `pyarrow` picks the layout — so this accepts each of
/// the utf8 variants rather than assuming one.
fn strings_from_arrow(array: &ArrayRef) -> Vec<String> {
    if let Some(array) = array.as_any().downcast_ref::<StringArray>() {
        return (0..array.len())
            .filter(|&i| array.is_valid(i))
            .map(|i| array.value(i).to_owned())
            .collect();
    }
    if let Some(array) = array.as_any().downcast_ref::<LargeStringArray>() {
        return (0..array.len())
            .filter(|&i| array.is_valid(i))
            .map(|i| array.value(i).to_owned())
            .collect();
    }
    if let Some(array) = array
        .as_any()
        .downcast_ref::<arrow::array::StringViewArray>()
    {
        return (0..array.len())
            .filter(|&i| array.is_valid(i))
            .map(|i| array.value(i).to_owned())
            .collect();
    }

    re_log::warn_once!(
        "Focus command: expected a string array for `{COMPONENT_PATHS}`, got {:?}",
        array.data_type()
    );
    Vec::new()
}

/// Pull a single flag out of an arrow array.
///
/// Accepts a boolean array, and also a numeric one, since a client may well send `1`/`0`.
fn bool_from_arrow(array: &ArrayRef) -> Option<bool> {
    if let Some(array) = array.as_any().downcast_ref::<BooleanArray>() {
        return (!array.is_empty() && array.is_valid(0)).then(|| array.value(0));
    }

    use arrow::array::AsArray as _;
    use arrow::datatypes::DataType;
    match array.data_type() {
        DataType::Int64 => {
            let array = array.as_primitive::<arrow::datatypes::Int64Type>();
            (!array.is_empty() && array.is_valid(0)).then(|| array.value(0) != 0)
        }
        DataType::UInt64 => {
            let array = array.as_primitive::<arrow::datatypes::UInt64Type>();
            (!array.is_empty() && array.is_valid(0)).then(|| array.value(0) != 0)
        }
        DataType::Int32 => {
            let array = array.as_primitive::<arrow::datatypes::Int32Type>();
            (!array.is_empty() && array.is_valid(0)).then(|| array.value(0) != 0)
        }
        other => {
            re_log::warn_once!("Focus command: expected a boolean flag, got {other:?}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{ArrayRef, BooleanArray, Int64Array, LargeStringArray, StringArray};

    use re_chunk::{Chunk, ChunkId, RowId, TimePoint};
    use re_entity_db::EntityDb;
    use re_log_types::{EntityPath, StoreId, StoreKind};
    use re_types_core::ComponentDescriptor;

    use super::{FOCUS_COMMAND_ENTITY_PATH, FocusCommand};

    /// Build a store holding one command, logged statically the way the SDK helper does.
    fn db_with_command(components: Vec<(&str, ArrayRef)>) -> (EntityDb, RowId) {
        let mut db = EntityDb::new(StoreId::random(StoreKind::Recording, "test_app".to_owned()));
        let row_id = RowId::new();
        let chunk =
            Chunk::builder_with_id(ChunkId::new(), EntityPath::from(FOCUS_COMMAND_ENTITY_PATH))
                .with_row(
                    row_id,
                    TimePoint::default(), // Static.
                    components
                        .into_iter()
                        .map(|(name, array)| (ComponentDescriptor::partial(name), array)),
                )
                .build()
                .expect("chunk should build");

        db.add_chunk(&Arc::new(chunk)).expect("chunk should ingest");
        (db, row_id)
    }

    fn strings(values: &[&str]) -> ArrayRef {
        Arc::new(StringArray::from(values.to_vec()))
    }

    #[test]
    fn reads_a_command_and_its_row_id() {
        let (db, expected_row_id) = db_with_command(vec![
            ("paths", strings(&["/world/robot", "/world/box"])),
            ("do_pan", Arc::new(BooleanArray::from(vec![true]))),
            ("do_rotate", Arc::new(BooleanArray::from(vec![true]))),
        ]);

        let (row_id, command) = FocusCommand::latest_from_db(&db).expect("should find a command");

        assert_eq!(row_id, expected_row_id, "row id is the command's identity");
        assert_eq!(
            command.entity_paths,
            vec![
                EntityPath::from("/world/robot"),
                EntityPath::from("/world/box")
            ]
        );
        assert!(command.do_pan);
        assert!(command.do_rotate);
    }

    #[test]
    fn an_empty_store_has_no_command() {
        let db = EntityDb::new(StoreId::random(StoreKind::Recording, "test_app".to_owned()));
        assert!(FocusCommand::latest_from_db(&db).is_none());
    }

    #[test]
    fn missing_flags_take_their_defaults() {
        // A driver that only sends paths should still get the hover behaviour: fit, don't rotate.
        let (db, _) = db_with_command(vec![("paths", strings(&["/world/robot"]))]);
        let (_, command) = FocusCommand::latest_from_db(&db).expect("should find a command");
        assert!(command.do_pan, "do_pan should default on");
        assert!(!command.do_rotate, "do_rotate should default off");
    }

    #[test]
    fn an_empty_path_list_is_a_clear_not_a_parse_failure() {
        // This is the unhover case, and it must not be mistaken for "no command".
        let (db, _) = db_with_command(vec![
            ("paths", strings(&[])),
            ("do_pan", Arc::new(BooleanArray::from(vec![true]))),
        ]);
        let (_, command) = FocusCommand::latest_from_db(&db).expect("should find a command");
        assert!(command.entity_paths.is_empty());
        assert!(command.do_pan, "an empty command still reframes the scene");
    }

    #[test]
    fn tolerates_the_string_layouts_a_client_may_send() {
        // pyarrow picks the layout, so we cannot assume plain utf8.
        let (db, _) = db_with_command(vec![(
            "paths",
            Arc::new(LargeStringArray::from(vec!["/world/robot"])) as ArrayRef,
        )]);
        let (_, command) = FocusCommand::latest_from_db(&db).expect("should find a command");
        assert_eq!(command.entity_paths, vec![EntityPath::from("/world/robot")]);
    }

    #[test]
    fn tolerates_numeric_flags() {
        // A client may well send 1/0 rather than true/false.
        let (db, _) = db_with_command(vec![
            ("paths", strings(&["/world/robot"])),
            ("do_pan", Arc::new(Int64Array::from(vec![0_i64]))),
            ("do_rotate", Arc::new(Int64Array::from(vec![1_i64]))),
        ]);
        let (_, command) = FocusCommand::latest_from_db(&db).expect("should find a command");
        assert!(!command.do_pan);
        assert!(command.do_rotate);
    }

    #[test]
    fn a_later_command_supersedes_an_earlier_one() {
        let mut db = EntityDb::new(StoreId::random(StoreKind::Recording, "test_app".to_owned()));

        let mut row_ids = Vec::new();
        for path in ["/first", "/second"] {
            let row_id = RowId::new();
            row_ids.push(row_id);
            let chunk =
                Chunk::builder_with_id(ChunkId::new(), EntityPath::from(FOCUS_COMMAND_ENTITY_PATH))
                    .with_row(
                        row_id,
                        TimePoint::default(),
                        [(
                            ComponentDescriptor::partial("paths"),
                            strings(&[path]) as ArrayRef,
                        )],
                    )
                    .build()
                    .expect("chunk should build");
            db.add_chunk(&Arc::new(chunk)).expect("chunk should ingest");
        }

        let (row_id, command) = FocusCommand::latest_from_db(&db).expect("should find a command");
        assert_eq!(command.entity_paths, vec![EntityPath::from("/second")]);
        assert_eq!(
            row_id, row_ids[1],
            "the row id must change, or the viewer would not re-fire"
        );
    }

    #[test]
    fn moves_camera_reflects_the_flags() {
        let command = FocusCommand {
            entity_paths: vec![],
            do_pan: false,
            do_rotate: false,
        };
        assert!(!command.moves_camera());
        assert!(
            FocusCommand {
                do_pan: true,
                ..command.clone()
            }
            .moves_camera()
        );
        assert!(
            FocusCommand {
                do_rotate: true,
                ..command
            }
            .moves_camera()
        );
    }
}
