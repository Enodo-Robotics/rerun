#!/usr/bin/env bash
# Registers an ephemeral runner with the `claude` label only (so jobs that ask
# for `self-hosted` stay on the host runner), runs one job, and exits.
set -euo pipefail

# Keeps the build cache under its limit (rerun's target dir grows quickly).
limit_mb="${TARGET_LIMIT_MB:-80000}"
if [ "$(du -sm "$CARGO_TARGET_DIR" | cut -f1)" -gt "$limit_mb" ]; then
    find "$CARGO_TARGET_DIR" -mindepth 1 -delete
fi

cd /home/runner/actions-runner
./config.sh --unattended --ephemeral --replace --no-default-labels --labels claude \
    --url "$RUNNER_URL" --token "$RUNNER_TOKEN" --name "$RUNNER_NAME" --work /home/runner/work
unset RUNNER_TOKEN
exec ./run.sh
