//! An externally-driven request to select entities and frame the camera on them.
//!
//! A driving application — a list UI, a review tool, anything outside the viewer — logs a command
//! to a reserved entity path, and the viewer selects the named entities and moves the camera to
//! fit them. The viewer has no idea what sent a command or why: all policy (what a list element
//! maps to, debounce, when to ask for a zoom-out) belongs to the driver.

use arrow::array::{Array as _, ArrayRef, BooleanArray, StringArray};

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

        // A latest-at query resolves each component independently, and static data is never
        // cleared — so a component omitted from a later command would keep its old value and
        // silently mix into a newer one. Treat the `paths` row as the command's identity, and
        // require every field to come from that same row, which makes a command atomic.
        //
        // Practical consequence: a command must always log `paths`, even when empty. The
        // `rerun.enodo.focus_entities` helper does so explicitly.
        let row_id = results.component_row_id(paths_component)?;

        let paths_array = results.component_batch_raw(paths_component)?;
        let Some(paths) = strings_from_arrow(&paths_array) else {
            // Present but unreadable. Rejecting is important: falling back to "no paths" would
            // turn a malformed command into "deselect everything and reframe the scene", which
            // is the most destructive thing this API can do.
            return None;
        };
        let entity_paths = paths.into_iter().map(EntityPath::from).collect();

        let flag = |component: ComponentIdentifier| -> Option<bool> {
            if results.component_row_id(component) != Some(row_id) {
                return None; // Left over from an earlier command.
            }
            let array = results.component_batch_raw(component)?;
            bool_from_arrow(&array)
        };

        Some((
            row_id,
            Self {
                entity_paths,
                do_pan: flag(do_pan_component).unwrap_or(true),
                do_rotate: flag(do_rotate_component).unwrap_or(false),
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
/// the utf8 variants. Returns `None` for anything that is not a string array at all: the caller
/// rejects such a command rather than treating it as an empty selection.
fn strings_from_arrow(array: &ArrayRef) -> Option<Vec<String>> {
    use arrow::datatypes::DataType;

    // An allowlist rather than a blind `cast`, because arrow will happily render numbers as
    // strings, and a numeric `paths` almost certainly means the driver has a bug.
    if !matches!(
        array.data_type(),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
    ) {
        re_log::warn_once!(
            "Ignoring focus command: `{COMPONENT_PATHS}` should be a string array, but it is {:?}.",
            array.data_type()
        );
        return None;
    }

    let array = if array.data_type() == &DataType::Utf8 {
        ArrayRef::clone(array)
    } else {
        arrow::compute::cast(array, &DataType::Utf8).ok()?
    };
    let array = array.as_any().downcast_ref::<StringArray>()?;

    Some(
        (0..array.len())
            .filter(|&i| array.is_valid(i))
            .map(|i| array.value(i).to_owned())
            .collect(),
    )
}

/// Pull a single flag out of an arrow array.
///
/// Accepts a boolean array, and any numeric one, since a client may well send `1`/`0` — or have
/// its booleans widened to some integer type on the way through `pyarrow`/`numpy`.
fn bool_from_arrow(array: &ArrayRef) -> Option<bool> {
    use arrow::datatypes::DataType;

    if !matches!(
        array.data_type(),
        DataType::Boolean
            | DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float32
            | DataType::Float64
    ) {
        re_log::warn_once!(
            "Focus command: expected a boolean flag, got {:?}. Using the default.",
            array.data_type()
        );
        return None;
    }

    let array = if array.data_type() == &DataType::Boolean {
        ArrayRef::clone(array)
    } else {
        arrow::compute::cast(array, &DataType::Boolean).ok()?
    };
    let array = array.as_any().downcast_ref::<BooleanArray>()?;

    (!array.is_empty() && array.is_valid(0)).then(|| array.value(0))
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

    fn empty_db() -> EntityDb {
        EntityDb::new(StoreId::random(StoreKind::Recording, "test_app".to_owned()))
    }

    /// Add one command row, logged statically the way the SDK helper does.
    fn add_command(db: &mut EntityDb, components: Vec<(&str, ArrayRef)>) -> RowId {
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
        row_id
    }

    fn db_with_command(components: Vec<(&str, ArrayRef)>) -> (EntityDb, RowId) {
        let mut db = empty_db();
        let row_id = add_command(&mut db, components);
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
        assert!(FocusCommand::latest_from_db(&empty_db()).is_none());
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
    fn a_command_without_paths_is_not_a_command() {
        // `paths` is the command's identity. Accepting a flags-only row would mean taking
        // `paths` from an older row — re-framing the previous subgroup instead of the new one.
        let (db, _) = db_with_command(vec![("do_pan", Arc::new(BooleanArray::from(vec![true])))]);
        assert!(FocusCommand::latest_from_db(&db).is_none());
    }

    #[test]
    fn stale_flags_from_an_earlier_row_are_not_mixed_in() {
        // Static data is never cleared, so an earlier command's `do_rotate` is still the latest
        // value of that component. It must not leak into a later command that omitted it.
        let mut db = empty_db();
        add_command(
            &mut db,
            vec![
                ("paths", strings(&["/first"])),
                ("do_rotate", Arc::new(BooleanArray::from(vec![true]))),
            ],
        );
        add_command(&mut db, vec![("paths", strings(&["/second"]))]);

        let (_, command) = FocusCommand::latest_from_db(&db).expect("should find a command");
        assert_eq!(command.entity_paths, vec![EntityPath::from("/second")]);
        assert!(
            !command.do_rotate,
            "do_rotate leaked from the earlier command"
        );
    }

    #[test]
    fn a_malformed_paths_component_is_rejected_not_treated_as_empty() {
        // Falling back to "no paths" would turn a driver bug into "deselect everything and
        // reframe the scene" — the most destructive thing this API can do.
        let (db, _) = db_with_command(vec![(
            "paths",
            Arc::new(Int64Array::from(vec![1_i64, 2])) as ArrayRef,
        )]);
        assert!(FocusCommand::latest_from_db(&db).is_none());
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
        // A client may well send 1/0 rather than true/false, or have its booleans widened.
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
        let mut db = empty_db();
        add_command(&mut db, vec![("paths", strings(&["/first"]))]);
        let second = add_command(&mut db, vec![("paths", strings(&["/second"]))]);

        let (row_id, command) = FocusCommand::latest_from_db(&db).expect("should find a command");
        assert_eq!(command.entity_paths, vec![EntityPath::from("/second")]);
        assert_eq!(
            row_id, second,
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
