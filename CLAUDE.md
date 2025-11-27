# Claude Instructions for Rerun Headless Mode

This document contains instructions for Claude on how to build and release custom Rerun wheels with headless mode features.

## Custom Features Added

This fork includes the following custom features for headless mode and file management:

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
RERUN_BUILDING_WHEEL=1 maturin build --release --manifest-path rerun_py/Cargo.toml --features web_viewer,nasm --out ./wheels/
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
RERUN_BUILDING_WHEEL=1 maturin build --release --manifest-path rerun_py/Cargo.toml --features web_viewer,nasm --out ./wheels/
```