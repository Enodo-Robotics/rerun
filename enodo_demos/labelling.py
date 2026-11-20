"""Labelling: labels and events on the timeline, `E` / `N` to add more.

A synthetic signal that has been labelled part of the way through, the way the
viewer's labelling panel stores labels, plus one `rr.enodo.mark_event` event.
Each label is a pennant on the time ruler and each event a diamond, so you can
see how far the labelling has got.

Move the time cursor past the last label and press `E`, then type `c`, `cor`
or `com` to place one of the labels below; `N` makes a new label.
"""

from __future__ import annotations

import json
import math
import sys

import rerun as rr

START = 1_789_653_000.0  # A fixed epoch time, so the recording is the same on every run.
LABELS = ["correct", "comma", "bug"]


def main(out: str) -> None:
    rr.init("enodo_demo_labelling")
    rr.save(out)

    # The labels the panel offers, as it saves them in the recording.
    rr.log(
        "annotations/_config/tags",
        rr.TextDocument(json.dumps([{"label": label} for label in LABELS])),
        static=True,
    )

    labelled = {20: "correct", 45: "comma", 70: "correct", 95: "bug", 120: "correct"}

    for i in range(300):
        t = START + i * 0.05
        # Its own timeline: the SDK stamps `log_time` with the wall clock.
        rr.set_time("time", timestamp=t)
        rr.log("robot/signal", rr.Scalars(math.sin(i / 8) + 0.3 * math.sin(i / 3)))

        # Labels, at the path the viewer's labelling panel writes: `annotations/<label>/<time>`.
        if label := labelled.get(i):
            time_ns = round(t * 1e9)
            rr.log(f"annotations/{label}/{time_ns}", rr.TextLog(label))

        if i == 200:
            rr.enodo.mark_event("hard stop")


if __name__ == "__main__":
    main(sys.argv[1])
