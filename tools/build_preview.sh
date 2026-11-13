#!/usr/bin/env bash
# Builds a preview site for this checkout into <out>: the web viewer, and a
# recording from every demo script in enodo_demos/ (made with this checkout's
# Python SDK) under <out>/recordings/. preview.yml runs it on the `claude`
# runner and hands the site to tools/preview.sh on the host to serve.
#
#   tools/build_preview.sh /tmp/site
#
# Needs the Python venv with the PyO3 config that .github/actions/claude-setup
# makes (or `pixi run ensure-pyo3-build-cfg` and an active venv).
set -euo pipefail
out="${1:?usage: build_preview.sh <out>}"
cd "$(dirname "${BASH_SOURCE[0]}")/.."

cargo run --locked --release -p re_dev_tools -- \
    build-web-viewer --no-default-features --features analytics,map_view --release
# A wheel rather than `maturin develop`: an editable install leaves out
# rerun_sdk.pth, which is what puts `rerun` on the path.
wheels="$(mktemp -d)"
RERUN_ALLOW_MISSING_BIN=1 maturin build --manifest-path rerun_py/Cargo.toml --out "$wheels"
pip install -q --force-reinstall --no-deps "$wheels"/rerun_sdk-*.whl
pip install -q "$wheels"/rerun_sdk-*.whl
rm -rf "$wheels"

rm -rf "$out"
mkdir -p "$out/recordings"
cp -a crates/viewer/re_web_viewer_server/web_viewer/. "$out/"
failed=0
for demo in enodo_demos/*.py; do
    name="$(basename "$demo" .py)"
    echo "demo: $demo"
    python "$demo" "$out/recordings/$name.rrd" || { echo "::error file=$demo::$demo failed"; failed=1; }
done
exit "$failed"
