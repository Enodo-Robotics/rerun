# Claude Instructions for Rerun Headless Mode

This document contains instructions for Claude on how to build and release custom Rerun wheels with headless mode features.

## Custom Features Added

This fork includes the following custom headless mode features:

1. **`--headless`**: Run without spawning GUI viewer
2. **`--continuous-download-interval <seconds>`**: Periodically save data at specified intervals
3. **`--rotate-files`**: Create timestamped files instead of appending to same file

These features are implemented in `/home/oscar/Documents/rerun/crates/top/rerun/src/commands/entrypoint.rs`.

## Building and Releasing Wheels

### Prerequisites

Ensure you're on the correct branch with headless features:
```bash
git checkout oscar-rrl
```

### Step 1: Build the CLI Binary

First, build the CLI binary with the headless features from the correct crate:
```bash
cargo build --release --bin rerun --manifest-path crates/top/rerun-cli/Cargo.toml
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

Extract and test the wheel to ensure it contains the headless features:
```bash
# Extract wheel for verification
python -m zipfile -e ./wheels/rerun_sdk-*.whl extracted_wheel
chmod +x extracted_wheel/rerun_sdk/rerun_cli/rerun

# Test the extracted binary
./extracted_wheel/rerun_sdk/rerun_cli/rerun --help | grep -A 3 headless

# Clean up
rm -rf extracted_wheel
```

### Step 5: Install and Test

Install the wheel and test the headless functionality:
```bash
pip install --force-reinstall ./wheels/rerun_sdk-*.whl
rerun --headless --help
```

## Testing Commands

Verify all features work correctly:

```bash
# Basic headless mode
rerun --headless

# Continuous download every 30 seconds
rerun --headless --save recording.rrd --continuous-download-interval 30

# Continuous download with file rotation every 60 seconds
rerun --headless --save recording.rrd --continuous-download-interval 60 --rotate-files
```

## Important Notes

1. **Always build CLI first**: The CLI binary must be built and copied to the Python package directory before building the wheel.

2. **Wheel naming**: Wheels are typically named like `rerun_sdk-0.24.0a1+dev-cp39-abi3-manylinux_2_35_x86_64.whl`

3. **Branch consistency**: Ensure you're on the `oscar-rrl` branch when building, as this contains the headless implementation.

4. **Feature validation**: The headless features require:
   - `--continuous-download-interval` requires both `--headless` and `--save`
   - `--rotate-files` requires `--continuous-download-interval`

## Troubleshooting

If the wheel doesn't contain headless features:

1. Verify the CLI binary has features: `./target/release/rerun --help | grep headless`
2. Ensure CLI binary was copied: `ls -la ./rerun_py/rerun_sdk/rerun_cli/rerun`
3. Check wheel contents: `python -m zipfile -l ./wheels/rerun_sdk-*.whl | grep rerun_cli`
4. Test extracted binary as shown in Step 4

## Build Commands Reference

```bash
# Full build sequence
git checkout oscar-rrl
cargo build --release --bin rerun --manifest-path crates/top/rerun-cli/Cargo.toml
cp ./target/release/rerun ./rerun_py/rerun_sdk/rerun_cli/rerun
RERUN_BUILDING_WHEEL=1 maturin build --release --manifest-path rerun_py/Cargo.toml --features web_viewer,nasm --out ./wheels/
```