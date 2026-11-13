# Demo recordings

Each `*.py` here writes one small recording that shows off a feature of this
fork. The PR preview (`.github/workflows/preview.yml`) builds the PR's web
viewer and Python SDK, runs every script, and serves the recordings on the
tailnet; a `Preview: enodo_demos/<name>.py` line in the PR description makes
the preview link open that script's recording.

A script takes the output path as its only argument:

```sh
python enodo_demos/event_markers.py /tmp/event_markers.rrd
```

Keep them tiny, deterministic and generic: boxes, points, synthetic signals.
No real robot data; recordings that must stay out of git go in the preview
host's private folder instead (see `tools/preview.sh`).
