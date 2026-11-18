# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) and other LLMs when working with code in this repository.

## Project overview

Rerun is a time-aware multimodal data stack and visualizations tool used in robotics, spatial AI, computer vision, and similar domains. It provides SDKs (Python, Rust, C++) for logging rich data (images, point clouds, tensors, etc.) and a Viewer for visualization.

## Build system

We use `pixi` for task management and dependency installation. Check `pixi.toml` for a full list of tasks.

### Essential commands

**Building:**
- `pixi run py-build` - Build Python SDK into local .venv (uses uv)
- `pixi run rerun-build` - Build native viewer (without web viewer)
- `pixi run rerun-build-web` - Build web viewer (wasm)
- `pixi run cpp-build-all` - Build all C++ artifacts

**Running:**
- `pixi run rerun` - Run the viewer
- `pixi run uvpy script.py` - Run Python scripts with rerun SDK
- `cargo run -p <package_name>` - Run specific Rust example (e.g., `cargo run -p dna`)

**Code generation:**
- `pixi run codegen` - Generate Rust/Python/C++ code from .fbs type definitions

**Formatting:**
- `pixi run rs-fmt` - Format all Rust files. Always run this after making changes.
- `pixi run py-fmt` - Format Python files
- `pixi run cpp-fmt` - Format C++ files
- `pixi run toml-fmt` - Format TOML files

**Testing:**
- Use `cargo clippy -p <crate_name>` to run general rust checks before building things
- `cargo nextest run --all-features --no-fail-fast -p <crate_name>` - Run tests for a specific crate
  - Example: `cargo nextest run --all-features --no-fail-fast -p re_view_spatial`
- Use `cargo nextest` (not `cargo test`) for better output and parallelism
- Always use `--all-features` unless you have a specific reason not to
- Use `--no-fail-fast` to gather all test failures in a single run

## Code generation system

**Critical: Never edit generated files directly.** All generated files are marked "DO NOT EDIT" at the top.

### Type definition flow

```
.fbs files (definitions/) → pixi run codegen → Generated code (Rust/Python/C++) + docs (docs/content/reference/types/)
```

- Type definitions live in `crates/store/re_sdk_types/definitions/rerun/`
  - `datatypes/*.fbs` - Low-level types (Vec3D, Mat4x4, etc.)
  - `components/*.fbs` - Component types (Position3D, Color, etc.)
  - `archetypes/*.fbs` - Archetypes (Points3D, Image, etc.)
  - `blueprint/*.fbs` - Blueprint system types
- Codegen implementation is in `crates/build/re_types_builder/`
- After modifying .fbs files, run `pixi run codegen` to regenerate code

### Extension pattern

To add custom functionality to generated types, create `_ext` files:
- Rust: `filename_ext.rs` (automatically imported by codegen)
- Python: `filename_ext.py` (mixed in with generated class)
- C++: `filename_ext.cpp` (compiled and included automatically, parts of it may be marked for copy into the header by codegen)

## Code conventions

### General

- use `…` instead of `...` <!-- NOLINT -->
- validate various custom conventions via `pixi run lint-rerun <file>` (not passing any file will check everything)
- Use `format!("{x}")` over `format!("{}, x)` (same in log calls etc)
- Don't write trivial comments that add nothing new

## Architecture overview

### Crate organization

```
crates/
├── build/     # Code generation (re_types_builder)
├── store/     # Data types, storage, querying
├── top/       # User-facing SDKs and CLI
└── viewer/    # Viewer UI and rendering
```

For more details about the architecture see `ARCHITECTURE.md`.

### Type system hierarchy

The type system has three levels (generated from .fbs files):

1. **Datatypes** (`rerun.datatypes.*`) - Basic types like Vec3D, Color
2. **Components** (`rerun.components.*`) - Named semantic wrappers (Position3D, Radius)
3. **Archetypes** (`rerun.archetypes.*`) - Collections of components (Points3D, Image)

Each archetype specifies:
- Required components (must be provided)
- Recommended components (have good defaults)
- Optional components (purely optional)

Example: `Points3D` archetype requires `positions`, recommends `colors` and `radii`, allows optional `labels`.

### Data flow

```
SDK (log archetype)
    ↓ encode to Apache Arrow
LogMsg (encoded data)
    ↓ transport (gRPC/file/memory)
re_chunk_store (indexed time series DB)
    ↓ query
Viewer (immediate mode rendering)
```

### Blueprint system

The blueprint is the viewer's configuration layer:
- Stored as a separate store (`re_entity_db`) with "blueprint" timeline
- Defines: view layout, visibility, per-entity overrides, view properties
- Uses the same type system as logged data
- Basic blueprint path hierarchy: `/viewport/`, `/view/{uuid}/`, `/container/{uuid}/`

### Visualizers

Each view type (Spatial3D, TimeSeries, etc.) has registered visualizers:
- Determine which entities/archetypes can be visualized
- Execute per-frame: query data → process → generate render commands
- Examples: Points3DVisualizer, LineStripsVisualizer, MeshVisualizer

The viewer uses **immediate mode**: every frame, query the store and re-render from scratch.

## Python development workflow

Python uses a separate uv-managed .venv (not pixi's conda env):

```bash
pixi run py-build              # Build rerun-sdk into .venv
pixi run uvpy script.py        # Run Python scripts via uv
pixi run uv run script.py      # Explicit uv run
```

The `uv` wrapper script unsets `CONDA_PREFIX` to ensure isolation from pixi's environment.

## Important notes

- **PyO3 Configuration**: If you see PyO3 config errors, run `pixi run ensure-pyo3-build-cfg`
- **git-lfs**: Required for test snapshots. Install with your package manager and run `git lfs install`
- **Immediate Mode**: The entire viewer is rendered from scratch each frame (no state management callbacks)
- **Arrow Native**: Data is stored, transmitted, and queried as Apache Arrow arrays
- **Multi-language**: Changes to .fbs files affect Rust, Python, and C++ simultaneously

## Python docstring formatting

Python API docs are built with **MkDocs + mkdocstrings** (NOT Sphinx). Never use reStructuredText (rST) syntax in Python docstrings or documentation. Use markdown instead:

- **Cross-references:** Use `[`ClassName`][]` (mkdocstrings syntax), NOT `:class:`ClassName`` / `:func:` / `:meth:` (rST roles)
- **Warnings/notes:** Use MkDocs admonitions (`!!! warning` with indented body), NOT `.. warning::` (rST directives)
- **Deprecation notices:** Use the `@deprecated` decorator (mkdocstrings renders it automatically). Do NOT duplicate in the docstring with `.. deprecated::` or `**Deprecated:**`
- **Code blocks:** Use markdown fenced blocks (`` ``` ``), NOT `.. code-block::`
- **Parameter docs:** Use numpy-style sections (`Parameters`, `Returns` with `----------`), which is what the codebase already uses

## Documentation system

See [`docs/README.md`](docs/README.md) for the full documentation architecture.

The docs span multiple sites: the main docs at `rerun.io/docs` (built from `docs/content/`), plus API reference sites for Python (MkDocs), C++ (Doxygen), and JS (TypeDoc) at `ref.rerun.io/docs/{python,cpp,js}/`.

Key things to know:
- **`docs/content/reference/types/`** is auto-generated by `pixi run codegen` from `.fbs` files - do not edit directly
- **`docs/content/reference/cli.md`** is auto-generated by `pixi run man` - do not edit directly
- **Code snippets** live in `docs/snippets/all/` with implementations in Python, Rust, and C++
- `pixi run py-docs-serve` previews Python API docs locally
- `pixi run -e cpp cpp-docs` builds C++ docs

## Development references

- [`ARCHITECTURE.md`](ARCHITECTURE.md) - Detailed architecture documentation
- [`BUILD.md`](BUILD.md) - Full build instructions
- [`CODE_STYLE.md`](CODE_STYLE.md) - Code style guidelines
- [`CONTRIBUTING.md`](CONTRIBUTING.md) - Contribution guidelines
- [`DESIGN.md`](DESIGN.md) - Guidelines for UI design, covering GUI, CLI, documentation, log messages, etc
- [`docs/README.md`](docs/README.md) - Documentation system (sites, builds, deployment)
- [`rerun_py/README.md`](rerun_py/README.md) - Python SDK specific instructions

## Enodo Robotics fork (RRL) — custom features

This is a fork maintained by Enodo Robotics. Development happens in the private
`Enodo-Robotics/rerun-dev` (issues, PRs, previews, Claude); releases build on the public
fork `Enodo-Robotics/rerun`, whose release wheel URLs `enodo_motion` and `enodo_stream` pin.
A release is a tag matching `RRL-v*` on `main` here:

```
git tag -a RRL-v3.3.0 -m RRL-v3.3.0 origin/main && git push origin RRL-v3.3.0
```

`.github/workflows/publish.yml` then pushes the tagged commit to the public fork's `main`
together with the tag, and the tag runs `.github/workflows/build_wheels_on_tag.yml` there:
wheels for Linux, Windows and macOS on GitHub-hosted runners, uploaded to the public
release. (That workflow does nothing in `rerun-dev`.)

### Custom CLI flags added to the `rerun` binary

- **`--save-interval <seconds>`**: Continuously rotates the save file every N seconds. Requires `--save <path>` to be set; the path is then treated as a prefix, and each rotated file is named by its time span (see **Rotated file naming** below). Each rotated file gets a copy of every `SetStoreInfo` seen so far so it's loadable on its own.
- **`--save-static <path>`**: Streams static chunks (plus `BlueprintActivationCommand`) to a dedicated sidecar file instead of inlining them into the temporal output. Keeps rotated files small and avoids duplicating static data across every rotation. Valid on its own (without `--save`) to capture only static data. `SetStoreInfo` is written to both temporal and static files so each is independently loadable.
- **Rotated file naming**: With `--save-interval`, each rotated file is named `<path>_<start_ms>__<end_ms>.rrd` (zero-padded epoch milliseconds, UTC) rather than `<path>_N.rrd`. The window is the interval during which the sink held that segment open — not the span of the data inside it, so an idle segment still gets a full, correctly-named window. Consecutive segments tile exactly (`end_ms(N) == start_ms(N+1)`), so a gap between filenames is a real hole in coverage rather than a naming artifact. Both stamps come from the same clock and epoch as `log_time`, so a logged timestamp is directly comparable to a segment's window. This is unconditional: there is no flag, and no counter-based naming mode.

  Each segment is written under `<path>.partial_<start_ms>.rrd`, fsynced, and only then renamed into place (the directory is fsynced after). A consumer globbing `<path>_*.rrd` therefore never picks up a segment that is still being written, nor one whose contents a power loss truncated after its name was published. `rename` preserves mtime, so consumers using mtime as the segment end are unaffected. The partial name is keyed on the segment's start rather than a counter, because a counter restarts at 1 each run and would otherwise let a later run truncate a crashed run's leftover.

  SIGINT and SIGTERM are caught while a rotating sink is active, so a normal service stop finalizes the open segment instead of orphaning it. A hard crash (SIGKILL, power loss) still leaves a `.partial_` file that no glob will match; its start is readable from the name and its end is approximately its mtime, and the next start logs a warning listing them, but it is not recovered automatically.

- **`--application-id <id>`**: Overrides `application_id` on every incoming log message. Useful for centralized ingestion (`--serve-grpc --save …`) so all captured recordings are tagged with a consistent ID regardless of what the logging SDKs sent.

- **`--reconnect-interval <seconds>`**: Keeps `rerun+http://…/proxy` sources (from the command line, `--connect`, or the viewer's Open URL) alive. Without it the source ends with its connection; with it, a failed or closed connection is retried every N seconds, indefinitely, including a server that isn't up yet, so the viewer and the logging process can start in either order. Two things are filtered on later connections so a reconnect does not disturb what is shown: a chunk already delivered is dropped by `ChunkId` (a proxy that stayed up replays its whole buffer to every new client), and a blueprint for an application seen on an earlier connection is dropped, so a restarted process re-sending its default layout does not replace the one the user arranged. Recording data, static included, is kept: a restarted process usually starts a new recording, which needs its own static data. The connection uses HTTP/2 keep-alive at the same interval, so a link that silently drops is noticed. It is native only (the web viewer has no way to set it), and a source closed in the viewer while its server is down keeps polling until the next successful connection, when it notices and stops. Implementation: `crates/store/re_grpc_client/src/reconnect.rs` (`ReplayFilter`, `stream_reconnecting`); end-to-end test in `crates/store/re_grpc_client/tests/reconnect.rs`.

Upstream's `--save <path>` semantics are unchanged — without `--save-interval`, it writes a single file to the exact given path.

Example combining everything:

```
rerun --serve-grpc \
      --save /data/run --save-interval 30 \
      --save-static /data/static.rrd \
      --application-id my_robot
```

Implementation: `crates/top/rerun/src/commands/entrypoint.rs` (search for `stream_to_rrd_rotating`, `Sinks`, `classify`, `rewrite_application_id`, `span_path`, `partial_path`, `finalize_segment`). Unit tests for the path builders are in the `tests` module at the bottom of that file. Static vs temporal classification is done by parsing each `ArrowMsg` into a `re_chunk::Chunk` and calling `is_static()`.

### Externally-driven selection and camera framing

An application outside the viewer can ask it to select entities and frame the camera on them, by
logging a command to the reserved entity path `viewer/_command/focus`:

```python
rerun.enodo.focus_entities(["/world/robot/arm"], do_pan=True, do_rotate=False)
rerun.enodo.focus_entities([])  # clear the selection, reframe the whole scene
```

Cheap enough to drive from a hover handler. Every 3D view containing at least one named entity
reframes; each path contributes its whole subtree, so `/robot` frames the meshes at
`/robot/link_3`. `do_rotate` re-aims the camera from where it stands (rotation about the camera,
not the orbit pivot); `do_pan` moves it to fit. The move is an eased glide re-based from the live
pose on each command, so a rapid sweep reads as pursuit.

The viewer knows nothing about what sent a command — debounce and hover policy belong to the
driver. A command naming a path absent from the recording is rejected whole, with a warning; a path
that exists but is not yet shown by any view is retried on later frames, so a command that arrives
before layout settles is not lost.

Commands are kept out of the files this fork's binary writes: `classify` routes anything under
`re_log_types::VIEWER_COMMAND_ENTITY_PATH_PREFIX` to `MsgRoute::Drop`, so they never reach a
rotated `.rrd` and never reach S3. They *are* in the viewer's store, so "Save recording" from the
viewer UI, or `rr.save()` in a driver, would persist them — only the rotating sink filters.

Implementation: `FocusCommand` (`crates/viewer/re_viewer_context/src/focus_command.rs`) parses the
command; `take_new_focus_command` (`crates/viewer/re_viewer/src/app_state.rs`) applies the
selection once per command and hands it to views via `ViewerContext::focus_command`;
`EyeState::frame_bounding_box` and the pure `framing_pose` (`re_view_spatial/src/eye.rs`) do the
camera arithmetic. Selection uses `Item::DataResult`, not `Item::InstancePath`, because only the
former gets the full-strength highlight. Design notes and rejected alternatives:
`RRL_SELECTION_FOCUS_SPEC.md`.

Selection outlines are drawn `OUTLINE_EMPHASIS` (3x) thicker than the egui theme asks for
(`crates/viewer/re_view/src/outlines.rs`), since theme widths are tuned for 2D widget strokes and
are too subtle to pick an entity out of a dense 3D scene.

### Event markers

`rr.mark_event(name, duration=None)` earmarks the rotated `.rrd` files covering that moment, so a
bucket can be indexed by event without opening any file:

```
log_1789653004101__1789653007122_event_hard-stop.rrd
```

The span stays first in the name, so a consumer parsing `start`/`end` does not have to care whether
an event suffix is present; the suffix only appears when an event fired. Names are folded to
`[a-z0-9-]` and truncated (`"Hard Stop!"` → `hard-stop`), since `_` is the field separator. Several
events in one window are all named, joined with `-`, truncated only at the 255-byte filename limit.

A marker defines a time span — an instant unless `duration` is given — and earmarks every rotated
file it overlaps, so an incident longer than one rotation interval is not reduced to an arbitrary
slice of itself. The event's time is the marker's own `log_time`, so a driver whose detection lags
can `rr.set_time(…)` to mark a moment that has already passed. A span lying entirely before the
open file cannot be honoured (that file is named and possibly uploaded) and warns instead.

Markers are ordinary temporal data logged to `_event/<name>` and are **kept** in the recording —
unlike focus commands, which are dropped — so the Text Log view shows on the timeline why a file
was flagged. Implementation: `EventSpan`, `event_span_from_chunk`, `events_covering` and
`sanitize_event_name` in `entrypoint.rs`; `MsgRoute::EventMarker` routes a marker to the temporal
sink while the rotation loop collects it for the filename.

### Checks

These are what CI (`.github/workflows/ci.yml`) runs, so run them before you push. The first
build after a fresh checkout needs the PyO3 config (`pixi run ensure-pyo3-build-cfg`; on the
runner, `.github/actions/claude-setup` has made it).

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- --deny warnings
CLIPPY_CONF_DIR=scripts/clippy_wasm cargo clippy --locked --all-features --target wasm32-unknown-unknown --target-dir target_wasm -p re_viewer -- --deny warnings
cargo nextest run --locked --all-targets --all-features --no-fail-fast
cargo test --locked --all-features --doc
```

While you iterate, run clippy and nextest with `-p <crate>` for the crates you touch, then
the whole set once at the end. UI changes have egui_kittest snapshot tests (PNG files under
`snapshots/`, in Git LFS); if a snapshot changes on purpose, update it with
`UPDATE_SNAPSHOTS=1` and look at the new PNG before you commit it.

### Demo recordings

Every PR gets a live preview on the tailnet: its web viewer, with a recording from each
script in `enodo_demos/`, made with the PR's own Python SDK (`enodo_demos/README.md`). A
`Preview: enodo_demos/<name>.py` line in the PR description makes its link open that
recording.

When a feature is visible, add a small demo script there that shows it off, or extend the
one for that feature, and check that it runs (`tools/build_preview.sh /tmp/site` builds the SDK and runs
them all; after that `python enodo_demos/<name>.py /tmp/x.rrd` runs one). Keep demos tiny,
deterministic and generic: boxes, points, synthetic signals, no real robot data or internal
names.

## Working from GitHub

Claude also runs in GitHub Actions (`claude*.yml`), in a container on our self-hosted runner
(`docker/claude-runner`, label `claude`): Rust with the wasm target, binaryen, pixi,
cargo-nextest, Mesa's software Vulkan for the snapshot tests, and a Python venv on `PATH`
(`tools/build_preview.sh` builds the SDK into it, along with the web viewer and the demo
recordings). `CARGO_TARGET_DIR`
is a cache kept between jobs, so builds are incremental. `gh` is logged in as the Claude
GitHub App, and its pushes trigger CI and the preview. There is no one to ask while you run,
so decide sensibly, then say what you decided and why in your reply. Never force-push, rebase
a pushed branch, approve a PR or push to main, except for a release tag.

Keep fork changes easy to carry onto newer upstream releases: put new code in new files or
clearly marked blocks where you can, and don't reformat or restructure upstream code you
don't need to change.

**An issue for you** (mentioning @claude): implement it on a branch from main, following
everything above (tests, a demo recording when there is something to see, the checks). If
an open PR already closes the issue, continue on that one. Open the PR with `gh pr create`;
its description says what changed and why, what you decided that the issue left open, a
`Preview: enodo_demos/<name>.py` line when there is something to see, and
`Closes #<issue>`. Reply on the issue with the link. If the issue is a question, or is too
unclear to start, answer or ask on the issue instead.

**Feedback on a PR** (@claude in a comment or review): read all of it, including inline
comments and the earlier discussion, then push commits to the PR's branch and reply with
what changed. The push reruns CI and the preview; `gh pr checks` shows the result.

**Resolving conflicts with main** (claude-sync, or when asked): `git fetch origin` and
`git merge origin/main`. When both sides add to the same place, keep both. Check that
`git diff origin/main` only has the PR's own changes, run the checks, and fix what the merge
broke. Commit as `Merge main into <branch>` and push. If a conflict needs a decision only the
author can make, push nothing and explain on the PR.

**When CI fails** (claude-ci-fix): read `gh run view <id> --log-failed`, find the cause, fix
it, run the failing check yourself, and push. If it passes unchanged on a rerun and looks
flaky, `gh run rerun <id> --failed` and say so on the PR. From the third failed run in a row,
push nothing: comment on the PR with what fails and what you tried.

**Merging an approved PR** (claude-merge): the workflow runs once a person has approved the
PR's current head and CI has passed on it, and tells you how its checks ended. If the PR is
mergeable, run `gh pr merge <n> --squash --delete-branch` with the PR's title and a body
summing up the PR (keep `Closes #<issue>`). Otherwise don't merge: say on the PR what is
blocking.

**A release** ("@claude release", optionally with a version): pick the version from what
merged since the last tag (`git describe --tags --abbrev=0 --match 'RRL-v*'`): minor for
features, patch for fixes only. Tag main's head (`git tag -a RRL-v<x.y.z> -m RRL-v<x.y.z>
origin/main`) and push the tag; `publish.yml` pushes it to the public fork, where the wheels
build. Reply with the version, a short summary of what is in it, and links to the publish
run and to https://github.com/Enodo-Robotics/rerun/actions/workflows/build_wheels_on_tag.yml.
