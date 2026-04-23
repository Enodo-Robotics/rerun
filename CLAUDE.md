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

5. **WASM linker flags (web viewer)**:
   The web viewer requires specific linker flags to work correctly. These are configured in two places:
   - `.cargo/config.toml` — under `[target.wasm32-unknown-unknown]` rustflags
   - `crates/build/re_dev_tools/src/build_web_viewer/lib.rs` — in the `RUSTFLAGS`/`CARGO_ENCODED_RUSTFLAGS` env vars (this script overrides `.cargo/config.toml`, so flags must be set in both places)

   Required flags:
   - `--growable-table`: Without this, `wasm-ld` emits `table[0]` with `max == min`, preventing the function table from growing. The viewer crashes at load time.
   - `--export-table`: Needed alongside `--growable-table` for wasm-bindgen compatibility.
   - `+bulk-memory,+simd128`: Target features for native memcpy/memset and SIMD. Stock Rerun enables these; without them the fork is slower than upstream.

   Note: `CARGO_ENCODED_RUSTFLAGS` uses `\x1f` (unit separator) to delimit flags, not spaces like `RUSTFLAGS`.

6. **wasm-opt is disabled in the build**:
   binaryen 105 (shipped by Ubuntu 22.04's `apt install binaryen`) has a bug where `-O2` rewrites `__wbindgen_export_1` (externref table) to point at table[0] (funcref), crashing the viewer at load time. We skip wasm-opt entirely in `build_web_viewer/lib.rs`. This adds ~8 MiB to the WASM size but avoids the bug.

   To re-enable wasm-opt in the future: bump the CI binaryen to >= 121 on all platforms (Linux needs to switch from `apt` to the release tarball, as Windows already does), then restore the wasm-opt block in `build_web_viewer/lib.rs`. Verify the built wasm afterwards by checking the table exports — `__wbindgen_export_1` must point at the externref table (index 1), `__wbindgen_export_6` must point at the funcref table (index 0).

## Troubleshooting

If the wheel doesn't contain custom features:

1. Verify the CLI binary has features: `./target/release/rerun --help | grep "save-dir"`
2. Ensure CLI binary was copied: `ls -la ./rerun_py/rerun_sdk/rerun_cli/rerun`
3. Check wheel contents: `python -m zipfile -l ./wheels/rerun_sdk-*.whl | grep rerun_cli`
4. Test extracted binary as shown in Step 4
5. Verify directory creation: `mkdir -p /tmp/test && ./target/release/rerun --save test --save-dir /tmp/test --help`

If the web viewer fails to load (table growth error, RuntimeError, or "table.set called with wrong type"):

1. Check that `--growable-table` is in the RUSTFLAGS: inspect `.cargo/config.toml` and `build_web_viewer/lib.rs`
2. Verify the built wasm table exports — `__wbindgen_export_1` must point at the externref table (index 1) and `__wbindgen_export_6` at the funcref table (index 0). Use this inspector against `web_viewer/re_viewer_bg.wasm` or any wheel's bundled wasm:
   ```bash
   python3 -c "
   with open('re_viewer_bg.wasm','rb') as f: d=f.read()
   def leb(b,o):
       r=0;s=0
       while True:
           x=b[o]; o+=1; r|=(x&0x7f)<<s
           if not (x&0x80): break
           s+=7
       return r,o
   off=8
   KIND={0:'func',1:'table',2:'memory',3:'global'}
   while off<len(d):
       sid=d[off]; off+=1; size,off=leb(d,off)
       if sid==7:
           n,p=leb(d,off)
           for _ in range(n):
               nl,p=leb(d,p); nm=bytes(d[p:p+nl]).decode(); p+=nl
               k=d[p]; p+=1; ix,p=leb(d,p)
               if KIND.get(k)=='table':
                   print(f'{nm} -> table[{ix}]')
           break
       off+=size"
   ```
   If both exports point at `table[0]`, wasm-opt has scrambled them — verify it's disabled in `build_web_viewer/lib.rs`.
3. Build the WASM locally (`cargo run --release -p re_dev_tools -- build-web-viewer --no-default-features --features analytics,map_view --release -g`) and re-run the inspector — faster than waiting for CI.

## Build Commands Reference

```bash
# Full build sequence
git checkout oscar-rrl
cargo build --release --bin rerun
cp ./target/release/rerun ./rerun_py/rerun_sdk/rerun_cli/rerun
RERUN_BUILDING_WHEEL=1 maturin build --release --manifest-path rerun_py/Cargo.toml --features web_viewer,nasm,server --out ./wheels/
```