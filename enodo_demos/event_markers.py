"""Event markers (`rr.enodo.mark_event`).

A synthetic speed signal with two incidents marked on the timeline; the
markers show in the Text Log next to the plot.
"""

from __future__ import annotations

import math
import sys

import rerun as rr

START = 1_789_653_000.0  # A fixed epoch time, so the recording is the same on every run.


def main(out: str) -> None:
    rr.init("enodo_demo_event_markers")
    rr.save(out)

    for i in range(200):
        t = START + i * 0.05
        # Its own timeline: the SDK stamps `log_time` with the wall clock.
        rr.set_time("time", timestamp=t)
        speed = 1.0 + 0.3 * math.sin(i / 10)
        if 80 <= i < 95:
            speed = max(0.0, speed - (i - 79) * 0.08)  # A hard stop.
        rr.log("robot/speed", rr.Scalars(speed))
        if i == 80:
            rr.enodo.mark_event("Hard Stop!")
        if i == 150:
            rr.enodo.mark_event("gripper slip", duration=1.5)


if __name__ == "__main__":
    main(sys.argv[1])
