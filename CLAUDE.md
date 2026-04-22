# Claude Instructions for Enodo Rerun Fork (RRL)

This document contains instructions for Claude on how to build, release, and develop the custom Rerun fork maintained by Enodo Robotics.

## Releasing Wheels (CI)

Wheels are built automatically by a self-hosted GitHub Actions runner when a tag is pushed:

```bash
git tag RRL-v0.2.0
git push origin RRL-v0.2.0
```

The workflow `.github/workflows/build_wheels_on_tag.yml` handles the full build, verification, and upload to the GitHub release. Tags follow the convention `RRL-v{major}.{minor}.{patch}`.

The self-hosted runner must be registered in the repo under Settings > Actions > Runners. The workflow can also be triggered manually via workflow_dispatch.

## Custom Features Added

### Headless Recording & File Management

1. **`--save <application_id>`**: Save recordings with an application ID
   - Files are named as `<application_id>_1.rrd`, `<application_id>_2.rrd`, etc.
   - Replaces file path-based saving with application ID-based naming

2. **`--save-dir <directory>`**: Specify directory for saving .rrd files
   - If not specified, files are saved to current working directory
   - Directory is created automatically if it doesn't exist

3. **`--save-interval <seconds>`**: Continuously save data at regular intervals
   - Automatically runs in headless mode (no GUI)
   - Creates rotating numbered files
   - Requires `--save` to be set

These features are implemented in `/home/oscar/Documents/rerun/crates/top/rerun/src/commands/entrypoint.rs`.

### Annotation System

4. **Annotation Panel** (`Ctrl+Shift+A` or "Ann." button in top bar):
   - Quick-tag buttons (Anomaly, Interesting, Bug, Note) for one-click annotation at current time
   - Custom user-defined tags (persisted across sessions, shareable via recording export)
   - Free-text annotations with Ctrl+Enter submit
   - Entity-aware: select an entity in the viewport, annotations log under `{entity}/_annotation`
   - Locked target: selection persists across panel interactions
   - Session annotation list with navigation (click time to jump) and removal

5. **Text Log View Enhancement**: Clicking a time value in the Text Log view selects the corresponding entity. For `_annotation` entities, the parent entity is selected instead.

6. **Export Annotations**: "Export annotations to .rrd" button saves only annotation entities to a separate file. Supports round-trip: export → close → re-import → continue annotating.

Key annotation files:
- Panel UI: `crates/viewer/re_viewer/src/ui/annotation_panel.rs`
- SystemCommands: `crates/viewer/re_viewer_context/src/global_context/command_sender.rs` (AddAnnotation, ClearAnnotation, UpdateRecording, ExportAnnotations)
- UICommand: `crates/viewer/re_ui/src/command.rs` (ToggleAnnotationPanel)
- Text Log view: `crates/viewer/re_view_text_log/src/view_class.rs` (entity selection on time click)
- App wiring: `crates/viewer/re_viewer/src/app.rs` and `app_state.rs`

### Other Changes

7. **Fallback file reader**: `.rrd` files load gracefully when inotify watches are exhausted (common on Linux with many file watchers). Falls back to non-streaming reader silently.
   - File: `crates/store/re_data_loader/src/loader_rrd.rs`

## Building and Releasing Wheels

### Prerequisites

Ensure you're on the correct branch with headless features:
```bash
git checkout oscar-rrl
```

### Step 1: Build the CLI Binary

First, build the CLI binary with the headless features:
```bash
cargo build --release --bin rerun
```

### Step 2: Replace CLI Binary in Python Package

The Python wheel includes a CLI binary that needs to be updated with our custom version:
```bash
cp ./target/release/rerun ./rerun_py/rerun_sdk/rerun_cli/rerun
```

### Step 3: Build the Python Wheel

Build the wheel with the correct CLI binary:
```bash
RERUN_BUILDING_WHEEL=1 maturin build --release --manifest-path rerun_py/Cargo.toml --features web_viewer,nasm,server --out ./wheels/
```

### Step 4: Verify the Wheel

Extract and test the wheel to ensure it contains the custom features:
```bash
# Extract wheel for verification
python -m zipfile -e ./wheels/rerun_sdk-*.whl extracted_wheel
chmod +x extracted_wheel/rerun_sdk/rerun_cli/rerun

# Test the extracted binary
./extracted_wheel/rerun_sdk/rerun_cli/rerun --help | grep -A 3 "save-dir"

# Clean up
rm -rf extracted_wheel
```

### Step 5: Install and Test

Install the wheel and test the custom functionality:
```bash
pip install --force-reinstall ./wheels/rerun_sdk-*.whl
rerun --help | grep -B 2 -A 2 "save-dir"
```

## Testing Commands

Verify all features work correctly:

```bash
# Save to current directory as my_app_1.rrd
rerun --save my_app

# Save to specific directory
rerun --save my_app --save-dir /data/recordings

# Continuous save every 30 seconds with rotating files (headless mode)
rerun --save my_app --save-dir ./recordings --save-interval 30
# Creates: ./recordings/my_app_1.rrd, ./recordings/my_app_2.rrd, etc.

# With gRPC server for remote connections
rerun --serve-grpc --save my_app --save-dir /data --save-interval 30
```

## Important Notes

1. **Always build CLI first**: The CLI binary must be built and copied to the Python package directory before building the wheel.

2. **Wheel naming**: Wheels are typically named like `rerun_sdk-0.24.0a1+dev-cp39-abi3-manylinux_2_35_x86_64.whl`

3. **Feature requirements**:
   - `--save-interval` requires `--save <application_id>` to be set
   - `--save-dir` is optional; defaults to current working directory if not specified
   - File rotation is automatic when using `--save-interval`
   - Headless mode is automatically enabled when using `--save-interval`

4. **File naming convention**:
   - Without `--save-interval`: Creates `<application_id>_1.rrd`
   - With `--save-interval`: Creates `<application_id>_1.rrd`, `<application_id>_2.rrd`, etc.

## Troubleshooting

If the wheel doesn't contain custom features:

1. Verify the CLI binary has features: `./target/release/rerun --help | grep "save-dir"`
2. Ensure CLI binary was copied: `ls -la ./rerun_py/rerun_sdk/rerun_cli/rerun`
3. Check wheel contents: `python -m zipfile -l ./wheels/rerun_sdk-*.whl | grep rerun_cli`
4. Test extracted binary as shown in Step 4
5. Verify directory creation: `mkdir -p /tmp/test && ./target/release/rerun --save test --save-dir /tmp/test --help`

## Build Commands Reference

```bash
# Full build sequence
git checkout oscar-rrl
cargo build --release --bin rerun
cp ./target/release/rerun ./rerun_py/rerun_sdk/rerun_cli/rerun
RERUN_BUILDING_WHEEL=1 maturin build --release --manifest-path rerun_py/Cargo.toml --features web_viewer,nasm,server --out ./wheels/
```