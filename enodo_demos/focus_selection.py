"""Externally-driven selection and camera framing (`rr.enodo.focus_entities`).

A row of boxes on a table, then focus commands that step from box to box.
The viewer applies the latest command it has, so opening the whole recording
selects and frames the last box; scrub the `step` timeline to replay the
others.
"""

from __future__ import annotations

import sys

import rerun as rr


def main(out: str) -> None:
    rr.init("enodo_demo_focus_selection")
    rr.save(out)

    rr.log("world/table", rr.Boxes3D(centers=[[0, 0, -0.05]], half_sizes=[[2.0, 0.6, 0.05]], colors=[[90, 90, 90]]), static=True)
    colors = [[230, 80, 60], [60, 160, 230], [90, 200, 110], [240, 190, 50]]
    for i, color in enumerate(colors):
        rr.log(
            f"world/box_{i}",
            rr.Boxes3D(centers=[[-1.5 + i, 0, 0.15 + 0.05 * i]], half_sizes=[[0.15, 0.15, 0.15 + 0.05 * i]], colors=[color], fill_mode="solid"),
            static=True,
        )

    for step, target in enumerate(["world/box_0", "world/box_1", "world/box_3", "world/box_2"]):
        rr.set_time("step", sequence=step)
        rr.enodo.focus_entities([target], do_pan=True, do_rotate=True)


if __name__ == "__main__":
    main(sys.argv[1])
