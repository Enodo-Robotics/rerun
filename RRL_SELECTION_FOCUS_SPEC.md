# Build sheet: SDK-driven selection + camera framing

**Status:** Implemented on `oscar-v3.1.0`; verified in a live viewer · **Target:** `Enodo-Robotics/rerun` fork (at `RRL-v3.0.4`)
**Date:** 2026-09-10 · Rationale and rejected options are in the appendices.

## What we are building

An external application (not the viewer) sends a command naming entity paths. The viewer selects
them and glides the camera to fit them. Selection outlines get thicker so the result is obvious.

The viewer knows nothing about *why* a command arrived. All policy — what a list element maps to,
debounce, when to ask for a zoom-out — lives in the driving application.

## The command

Payload `(selection_list, do_pan, do_rotate)`:

| `selection_list` | `do_pan` | Effect |
|---|---|---|
| non-empty | `true` | Select the subgroup and fit it — the hover case. |
| non-empty | `false` | Select the subgroup, camera untouched. |
| empty | `true` | Clear selection, reframe the whole scene — the unhover case. |
| empty | `false` | Clear selection only. |

`do_rotate` composes with either: it re-aims the camera at the subgroup centre without moving it.

A selected path contributes its **whole subtree** to the framing box, so `/robot` frames the meshes
at `/robot/link_N`. If any requested path is absent from the store, the whole command is rejected
with a `warn_once!`.

## Work items

**1. Outline emphasis** — `crates/viewer/re_view/src/outlines.rs`
Scale `outline_radius_pixel` in `outline_config()` (`:13`) by a fork constant (~3×), and raise
`SIZE_BOOST_IN_POINTS_FOR_LINE_OUTLINES` (`:7`) and `..._POINT_OUTLINES` (`:10`) so the halo clears
the geometry. This one function feeds every 3D view (`ui_3d.rs:275`), 2D view (`ui_2d.rs:446`) and
map view (`map_view.rs:393`). Check the jump-flood step count in
`re_renderer/src/draw_phases/outlines.rs:526` tolerates the larger radius.

**2. `EyeState::frame_entities`** — `crates/viewer/re_view_spatial/src/eye.rs`, beside
`focus_entity` (`:925`), which it generalizes from one entity to a list. Must live in this file:
`EyeController` is `pub(crate)` with private fields (`:216-226`).

- Union `SceneBoundingBoxes::region_of_interest_per_entity` over the requested paths and their
  subtrees, falling back to `per_entity` where no ROI exists.
- `do_rotate`: hold `pos`, set `look_target` to the union centre. Under `Eye3DKind::Orbital` the
  orbit centre *is* `look_target`, so this is rotation about the camera — no new maths.
- `do_pan`: `pos = centre - fwd * (1.5 * union.centered_bounding_sphere_radius())`, matching
  `focus_entity` exactly so command framing and double-click framing look identical. Direction is
  the current forward when rotating, else the existing `pos → centre` direction. Clamp to
  `EyeController::MIN_ORBIT_DISTANCE` (`:233`); for a zero-extent box reuse the existing fallback of
  the scene ROI radius floored at `0.02` (`:952-962`). Note this ignores `fov_y`; if wide subgroups
  clip, switch to a true fit of `r / sin(fov_y/2)`.
- Commit with `did_interact = true`, `save_to_blueprint(...)` (`:311`), `start_interpolation()`
  (`:635`), and clear `EyeControls3D::tracking_entity` as `focus_entity` does (`:976`).
- Factor the arithmetic into a pure function for unit testing:
  `(union_bbox, fov_y, current_pos, do_pan, do_rotate) -> (pos, look_target)`.

**3. Transport** — reserved entity path, logged as `rr.AnyValues`
The driver logs to e.g. `/viewer/_command/focus`. Add a drop route for that path in `classify()`
(`crates/top/rerun/src/commands/entrypoint.rs`) so commands never reach a rotated `.rrd` — and
therefore never reach S3 — while the live viewer still receives them over gRPC.

**4. Trigger** — apply once per command
Each frame, `latest_at` the reserved path at the store's **latest** time (not the time cursor) and
compare the returned `RowId` with the last applied one. Time scrubbing then never re-fires; an
identical payload logged again does, because the `RowId` differs; opening an `.rrd` containing a
command applies it once at load.

**5. Selection** — build an `ItemCollection` (`item_collection.rs:70`) and send
`SystemCommand::SetSelection` (`command_sender.rs:123`, handled at `app.rs:1263`). Resolve each path
to `Item::DataResult` **per view**, not `Item::InstancePath` — `view_highlights.rs` grades the
former as `SelectionHighlight::Selection` (`:76`) and the latter only as `SiblingSelection` (`:70`),
so the naive version highlights more weakly than a plain click.

**6. Python helper** — `rerun_py/rerun_sdk/rerun/enodo.py`, pure Python, no codegen:
`focus_entities(paths, *, do_pan=True, do_rotate=False)`.

Order: 1 and 2 first — both are useful standalone and carry no design risk; drive 2 from a temporary
keyboard shortcut to see it before any transport exists. Then 3, 4, 5, 6.

## Decided parameters

| | |
|---|---|
| Motion curve | Existing `ease_out` (`eye.rs:627`) — fastest at `t=0`, so a sweep reads as pursuit. |
| Duration | Existing angle-scaled 0.2–0.7 s (`eye.rs:162`). |
| Retargeting | Each command re-bases the glide from the live pose; nothing is queued or dropped. |
| Views affected | Every 3D view containing at least one requested entity. |
| Fit slack | 1.5× bounding-sphere radius, identical to double-click focus. |
| Bad paths | Reject the whole command, `warn_once!`. |

**Out of scope:** isolation (hiding or dimming non-selected entities), 2D view framing,
occlusion-aware camera placement, time-cursor control, and blueprints for anything beyond the camera
components the viewer already writes on every drag frame.

## Testing

- Unit-test the pure fit function: rotate-only leaves `pos` bit-identical; pan-only preserves
  direction; zero-extent box clamps to the floor; `fov_y = None`; empty list is a no-op.
- Trigger semantics: applies once, survives a time scrub without re-firing, re-fires on a new
  command with an identical payload.
- Manual: a probe script logging entities plus commands, driven at cursor rate.
- `egui_kittest` snapshots would suit the outline change, but `git-lfs` is not installed here — CI
  only, or install it first.

## Implementation notes (2026-09-10)

All six work items are built. What testing changed:

**A timing bug the screenshot test caught.** A command is in the store the moment a recording
loads, but auto-layout has not created any view yet — so "no view shows this path" was true, the
command was marked applied, and it was lost forever. The first end-to-end run reproduced this
exactly, with the rejection warning explaining itself. Fixed by distinguishing the two cases:
absent from the *store* is a bad command (reject once, warn), while present-but-not-yet-in-a-view
is retried on later frames. Without this, any command arriving before its entities were logged, or
before layout settled, would silently do nothing.

**Framing is tighter than "everything visible".** With the agreed 1.5× bounding-sphere distance, a
2×2×2 box slightly overflowed the frame in the live test — its bounding sphere has radius √3, and
1.5√3 ≈ 2.6 is closer than a geometric fit needs. This is faithful to double-click focus, which
has the same behaviour, so it was left alone. If "maximize % coverage" should mean "nothing
clipped", the fix is the one already noted in work item 2: `r / sin(fov_y/2)`.

**Verified in a real viewer** (headless via `xvfb-run`, screenshot compared against a
no-command control): the camera framed the requested subtree, the box carried a thick white
selection outline — proving both the full-strength `Selection` highlight and the 3× emphasis — and
no rejection warning fired. Separately confirmed: a command logged through the fork's sink appears
in **no** rotated `.rrd`, while the geometry does.

**Tests:** 8 on the pure `framing_pose` arithmetic, 11 on command parsing. `framing_pose` ended up
with no `fov_y` parameter (the distance is 1.5x the bounding sphere, which ignores the field of
view), so the `fov_y = None` case listed under *Testing* above does not exist.
`re_view_spatial`'s two `test_help_view` snapshot tests fail here for lack of `git-lfs`, as
predicted, and are unrelated.

### Second round: independent review (2026-09-10)

An adversarial review of the committed code found several real defects, all now fixed:

- **A command was not atomic.** A latest-at query resolves each component independently and static
  data is never cleared, so a command that omitted a field would silently pick that field up from
  an *older* row. Worst case, `focus_entities([])` re-framed the previous subgroup instead of
  clearing — because `AnyValues` drops an empty list entirely when its type registry is cold
  (`any_batch_value.py:255-262`). Fixed on both sides: the Python helper now sends explicitly typed
  arrays, and the parser treats the `paths` row as the command's identity and ignores any field
  logged in a different row.
- **A malformed payload did the most destructive thing available.** An unreadable `paths` fell back
  to "no paths", i.e. deselect everything and reframe the scene. Now rejected.
- **The retry was unbounded**, re-walking every entity in the recording each frame via
  `all_entities()`, with no `request_repaint`. Now bounded to 3 seconds, with the store walked at
  most once per command, and a repaint requested while pending.
- **`last_applied` was global**, so switching between two recordings that both carried a command
  re-fired the stale one. Now keyed by store id.
- **Views that were not rendering consumed commands.** A view in a background tab satisfied the
  resolution check but never framed. The command now outlives its arrival frame and each view
  applies it at most once, by row id.
- **Undo was destroyed by hover-rate driving.** Camera writes are blueprint writes, and
  `BlueprintUndoState` only skips undo points while the *local* pointer is down, so ~1.6s of
  hovering filled the 100-entry history with camera micro-moves. `update` now takes an
  `is_externally_driven` flag.
- **Selection at cursor rate stole keyboard focus**, via the focus-sync path that scrolls panels to
  the selected item. Fixed with a new `SelectionSource::ExternalCommand`, which the spec's own
  appendix had prescribed and the first implementation ignored.
- **A NaN position poisoned the camera permanently.** `BoundingBox::is_nothing` is `max < min`,
  false for NaN, so a NaN box produced a NaN camera that was written to the blueprint. Now guarded
  in both the view and `framing_pose`.

The review also claimed selection used the exact path while framing used the subtree, so naming a
parent would outline nothing. That was **dismissed in error**: the test "disproving" it was
screenshotted headlessly, where the X server parks the pointer at screen centre — on the framed
box — so the white outline being read as *selection* was actually *hover*. Once the selection
colour became orange and distinguishable, the finding reproduced immediately. Selection now
resolves each requested path to the data results beneath it, matching the framing semantics.

Re-verified after the fixes, three-way: no command (two boxes, default framing, no outline), a
focus command (subgroup framed and outlined), and an empty command (framing restored, selection
cleared). The empty/unhover path was not verified in the first round.

**Incidental fix:** `re_test_context` had a non-exhaustive `SystemCommand` match — the fork's
annotation work added variants without updating it — which broke `--all-features` test builds for
every viewer crate.

## Open

Only look-and-feel questions, both answerable by using it: whether thicker outlines alone read as
obvious enough (fallback: A2 in Appendix 2), and whether a fast sweep feels like pursuit or lag
(levers in order: duration, a per-command duration field, an ease-in-out hybrid).

---

# Appendix 1 — existing machinery this builds on

- **The camera is already blueprint state.** `EyeControls3D`
  (`re_sdk_types/definitions/rerun/blueprint/archetypes/eye_controls3d.fbs`) carries `position`,
  `look_target`, `eye_up`, `kind`, `speed`, `tracking_entity`, `spin_speed`, and
  `save_blueprint_component` is the only way to move the eye. The viewer writes these on every drag
  frame, so command-rate writes are routine, not a hack.
- **Load-in framing is a fallback provider, not bespoke code.** `view_3d.rs:163-215` derives
  `look_target` from the region-of-interest centre and `position` from `1.5 × half_size().length()`.
  Matching it is the right target for "fit these entities".
- **Per-entity boxes are computed every frame.** `SceneBoundingBoxes` (`scene_bounding_boxes.rs`)
  keeps `per_entity` and `region_of_interest_per_entity` keyed by entity-path hash, plus a smoothed
  overall box.
- **Single-entity focus already exists** — `focus_entity` / `focus_point` (`eye.rs:925`, `:981`),
  driven from double-click in `ui_3d.rs:320-360`.
- **What did not exist:** any way for a log stream to trigger viewer behaviour. No UI-facing RPC in
  `re_protos/proto/rerun/v1alpha1/`, and selection is not in the blueprint. That gap is what work
  item 3 fills.

# Appendix 2 — rejected options

**A2 — genuinely thicker geometry.** `radius_boost_in_ui_points_for_outlines`
(`visualizers/lines3d.rs:187`) inflates only the outline pass, not the drawn line. Real thickening
means per-instance radius scaling inside each of `lines3d`, `lines2d`, `arrows3d`, `arrows2d`,
`points3d`, `points2d`, `boxes2d`, `boxes3d` — eight near-identical edits in code upstream actively
changes. Held as the fallback if outline emphasis proves too subtle, and then only for lines and
points.

**Dimming non-selected entities.** No per-entity opacity, fade or tint exists in the spatial render
path. It would need per-instance alpha across those same eight visualizers *plus* depth-sorted
transparency — a rendering change, not a colour tweak — or `Color` component overrides
(`view_query.rs:198`) that destroy per-instance colour while active.

**Hiding non-selected entities.** Blueprint visibility is *inherited*: `update_overrides_recursive`
(`re_viewport_blueprint/src/view_contents.rs:482-492`) seeds each node with `parent_visible` before
its own override applies, so hiding `/A` also hides a selected `/A/B`. An ephemeral isolation set at
the flat per-data-result gate (`re_viewport/src/system_execution.rs:88-91`) would have avoided that,
since that gate judges each result independently — but it forces a decision on whether a selected
path means the entity or its subtree, kept consistent with framing. Dropped for simplicity; it also
removes any `re_viewport` change from the build.

**Blueprint store as the transport.** `send_blueprint` (`rerun_py/rerun_sdk/rerun/sinks.py:390`)
takes `make_active`/`make_default`, i.e. sending a blueprint *activates* it, which implies replacing
the operator's layout on every command. Blueprint data does ride a `RecordingStream` internally
(`blueprint/api.py:742-753`), so arbitrary logging there is structurally possible, but whether the
viewer retains such entities without a `BlueprintActivationCommand` was never verified. Work item 3
sidesteps the question.

**Codegen'd blueprint archetype.** Best ergonomics and the only upstreamable shape, but it edits
`.fbs`, regenerates Rust/Python/C++, and starts generated-file drift this fork currently has none of.

**Prefix/glob expansion in the payload.** Rejected in favour of explicit path lists so the viewer
never guesses which entities a command meant. Subtree semantics for *framing* (above) is the one
deliberate exception.

**Instant camera cuts, and ease-in-out.** Instant was specified and then reversed — an eased glide,
up to 0.7 s, was wanted. Ease-in-out was rejected because its slow start applies on every retarget,
reading as lag exactly when the cursor moves fastest.

# Appendix 3 — caveats

**An independent review of this document was started and did not complete** (session rate limit),
so most claims here rest on a single pass. Two items were hand-checked and are recorded above: the
visibility-inheritance behaviour, and the `send_blueprint` activation semantics. Worth re-running a
review before or alongside implementation.

**Watch during implementation:** driving `SetSelection` at command rate re-renders the selection
panel on every command. If that churns, the panel can ignore command-sourced selections via the
existing `SelectionSource` enum (`command_sender.rs:219`), which exists for this kind of distinction
and needs no new plumbing.
