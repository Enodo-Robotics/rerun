"""
Enodo Robotics fork extensions.

These are not part of upstream Rerun. They let an application outside the viewer drive what the
viewer is looking at — see `RRL_SELECTION_FOCUS_SPEC.md` in the repository root.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from .any_value import AnyValues

if TYPE_CHECKING:
    from collections.abc import Sequence

    from .recording_stream import RecordingStream

__all__ = ["FOCUS_COMMAND_ENTITY_PATH", "focus_entities"]

# Kept in sync with `VIEWER_COMMAND_ENTITY_PATH_PREFIX` in `re_log_types` and
# `FOCUS_COMMAND_ENTITY_PATH` in `re_viewer_context`.
FOCUS_COMMAND_ENTITY_PATH = "viewer/_command/focus"


def focus_entities(
    paths: Sequence[str],
    *,
    do_pan: bool = True,
    do_rotate: bool = False,
    recording: RecordingStream | None = None,
) -> None:
    """
    Ask the viewer to select some entities and frame its camera on them.

    Intended to be driven by your own UI — hovering an element in a list, for example — so it is
    cheap enough to call at cursor rate. Each call supersedes the previous one, and the viewer
    glides rather than jumping, so a fast sweep reads as the camera pursuing the cursor.

    Commands are logged to a reserved entity path that the Enodo `rerun` binary drops from its
    rotated `.rrd` files, so driving the viewer does not pollute recordings.

    Every 3D view containing at least one of the named entities is reframed. If any path names
    something no view can show, the whole command is ignored and the viewer logs a warning —
    a partially applied framing would be misleading.

    Parameters
    ----------
    paths:
        Entity paths to select and frame. Each contributes its whole subtree, so `"/robot"` frames
        the meshes logged at `/robot/link_3`. Pass an empty sequence to clear the selection and —
        with `do_pan` — return to framing the whole scene.
    do_pan:
        Move the camera to a distance that fits the entities.
    do_rotate:
        Re-aim the camera at the entities from wherever it currently stands. The rotation happens
        about the camera position rather than about the orbit pivot, so with `do_pan=False` the
        camera turns in place and the entities may stay small in frame.
    recording:
        Specifies the [`rerun.RecordingStream`][] to use. If left unspecified, defaults to the
        current active data recording, if there is one.

    Examples
    --------
    ```python
    # Hovering a list element:
    rerun.enodo.focus_entities(["/world/robot/arm", "/world/robot/gripper"])

    # Pointer left the list — back to the whole scene:
    rerun.enodo.focus_entities([])
    ```

    """
    import pyarrow as pa

    from ._log import log

    # Explicitly typed arrays, rather than letting `AnyValues` infer. Inference drops an empty
    # list outright when it has not yet seen a value of that type, which would leave `paths`
    # unlogged — and because static data is never cleared, the viewer would then read the
    # *previous* command's paths and re-frame those instead of clearing the selection.
    log(
        FOCUS_COMMAND_ENTITY_PATH,
        AnyValues(
            paths=pa.array([str(path) for path in paths], type=pa.string()),
            do_pan=pa.array([bool(do_pan)], type=pa.bool_()),
            do_rotate=pa.array([bool(do_rotate)], type=pa.bool_()),
        ),
        # Static, so the command is timeline-independent: the viewer reads whichever was logged
        # last, no matter where the time cursor happens to sit.
        static=True,
        recording=recording,
    )
