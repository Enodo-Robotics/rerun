#!/usr/bin/env bash
# Registers an ephemeral runner with the `claude` label only, or RUNNER_LABELS
# (so jobs that ask for `self-hosted` stay on the host runner), runs one job,
# and exits.
set -euo pipefail

# Keeps the build cache under its limit (rerun's target dir grows quickly, as
# every change links new test binaries beside the old ones). First drop the
# linked executables, which are most of it and only cost a relink; wipe the
# compiled libraries too only if that is not enough.
limit_mb="${TARGET_LIMIT_MB:-80000}"
over() { [ "$(du -sm "$CARGO_TARGET_DIR" | cut -f1)" -gt "$limit_mb" ]; }
if over; then
    find "$CARGO_TARGET_DIR"/*/deps -maxdepth 1 -type f -executable ! -name '*.so' -delete 2> /dev/null || true
    if over; then
        find "$CARGO_TARGET_DIR" -mindepth 1 -delete
    fi
fi

cd /home/runner/actions-runner
./config.sh --unattended --ephemeral --replace --no-default-labels --labels "${RUNNER_LABELS:-claude}" \
    --url "$RUNNER_URL" --token "$RUNNER_TOKEN" --name "$RUNNER_NAME" --work /home/runner/work
unset RUNNER_TOKEN
exec ./run.sh
