#!/usr/bin/env bash
# Runs this repo's CI, Claude and preview-build jobs in containers on this
# machine (docker/claude-runner), one fresh container per job, in a few slots
# so CI can run while Claude works. Each slot keeps its own build cache
# (rerun's target dir is big, and cargo locks it per build). The containers
# only connect out to GitHub, so no ports are opened. They get no host files
# and no Docker socket: only volumes for the caches.
#
# They reach the internet but nothing on our side: a firewall rule drops what
# their network (br-claude, shared with UsdEditor's Claude runner) sends to
# private addresses (the LAN, the tailnet, this machine). The rule is a system
# service, so it is back after a reboot, and each slot checks that it is in
# place before every job.
#
#   tools/claude_runner.sh install   build the image, add the firewall if missing (sudo), start the
#                                    slots, and register the preview runner on the host
#   tools/claude_runner.sh build     rebuild the image (after a Rust or runner upgrade)
#   tools/claude_runner.sh run <n>   one slot's loop (the service runs it)
#
# Run install from the main checkout, which the services point to. Needs Docker
# and `gh` logged in as a repo admin (for registration tokens). The workflows
# use these runners through the `claude` label, and the short jobs through
# `claude-light` (one more slot, `light`, that builds nothing).
#
# install also registers a plain runner on the host, with the label
# rerun-preview, for the one job that needs the host: tools/preview.sh, which
# serves the previews on the tailnet with Docker. It runs nothing else: the
# preview's build, from the PR's code, runs on the `claude` runners. It lives in
# ~/actions-runner-rerun-dev (PREVIEW_RUNNER_DIR), next to UsdEditor's
# ~/actions-runner, and reuses that runner's download.
set -euo pipefail
cd "$(dirname "$0")/.."

repo=Enodo-Robotics/rerun-dev
image=rerun_claude_runner
slots="${CLAUDE_RUNNER_SLOTS:-2}"
# Per slot. UsdEditor's runner (up to 24g) shares this 31 GB machine; a build
# that runs out of memory fails inside its container, not on the host.
memory="${CLAUDE_RUNNER_MEMORY:-14g}"
jobs="${CLAUDE_RUNNER_CARGO_JOBS:-6}"
network=claude-runner
subnet=172.31.250.0/24
gateway=172.31.250.1
private="10.0.0.0/8 172.16.0.0/12 192.168.0.0/16 100.64.0.0/10 169.254.0.0/16 224.0.0.0/4"

build() {
    rust=$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain)
    docker build -t "$image" --build-arg RUST_VERSION="$rust" docker/claude-runner
}

make_network() {
    docker network inspect "$network" > /dev/null 2>&1 ||
        docker network create --subnet "$subnet" -o com.docker.network.bridge.name=br-claude \
            -o com.docker.network.bridge.enable_icc=false "$network" > /dev/null
}

# Exits 0 if a container on the network can reach this machine (sshd listens
# on all addresses), which means the firewall rule is missing.
reachable() {
    docker run --rm --network "$network" --entrypoint bash "$image" \
        -c "timeout 3 bash -c '</dev/tcp/$gateway/22'" 2> /dev/null
}

online() {
    docker run --rm --network "$network" --dns 1.1.1.1 --dns 8.8.8.8 --entrypoint curl "$image" \
        -fsS -o /dev/null https://api.github.com
}

run() {
    slot="${1:?usage: $0 run <slot>}"
    name="rerun-claude-runner-$slot"
    # The `light` slot takes the short jobs (claude-light: 👀, merge checks,
    # reviews' setup, publishing), so they don't wait behind a build. It
    # builds nothing, so it needs little memory and no caches.
    if [[ "$slot" == light ]]; then
        opts=(--memory 4g -e RUNNER_LABELS=claude-light -v rerun-claude-light-work:/home/runner/work)
    else
        opts=(--memory "$memory" -e CARGO_BUILD_JOBS="$jobs"
            -v rerun-claude-cargo-registry:/opt/cargo/registry
            -v rerun-claude-cargo-git:/opt/cargo/git
            -v "rerun-claude-$slot-cache:/home/runner/cache"
            -v "rerun-claude-$slot-work:/home/runner/work")
    fi
    make_network
    while true; do
        if reachable; then
            echo "The container can reach this machine: is claude-runner-firewall.service running?" >&2
            sleep 60
            continue
        fi
        # A registration token lasts an hour and only registers a runner.
        if ! token=$(gh api -X POST "repos/$repo/actions/runners/registration-token" --jq .token); then
            sleep 60
            continue
        fi
        docker run --rm --init --name "$name" --network "$network" \
            --dns 1.1.1.1 --dns 8.8.8.8 \
            --pids-limit 4096 --cap-drop ALL --security-opt no-new-privileges "${opts[@]}" \
            -e RUNNER_URL="https://github.com/$repo" -e RUNNER_TOKEN="$token" \
            -e RUNNER_NAME="$(hostname)-rerun-$slot" \
            "$image" || sleep 30
    done
}

# The firewall unit, the same as UsdEditor's tools/claude_runner.sh writes.
# Rules in raw PREROUTING see the container's packets before Docker's and
# Tailscale's forwarding rules, whatever order those are in.
firewall_unit() {
    printf '%s\n' "[Unit]" \
        "Description=Keeps the Claude runner's containers off the LAN, the tailnet and this machine" \
        "Before=docker.service" "" "[Service]" "Type=oneshot" "RemainAfterExit=yes"
    for net in $private; do
        echo "ExecStart=/bin/sh -c 'iptables -t raw -C PREROUTING -i br-claude -d $net -j DROP 2>/dev/null || iptables -t raw -I PREROUTING -i br-claude -d $net -j DROP'"
    done
    echo "ExecStart=/bin/sh -c 'ip6tables -t raw -C PREROUTING -i br-claude -j DROP 2>/dev/null || ip6tables -t raw -I PREROUTING -i br-claude -j DROP'"
    printf '%s\n' "" "[Install]" "WantedBy=multi-user.target"
}

runner_unit() {
    printf '%s\n' "[Unit]" "Description=GitHub Actions runner slot %i for $repo" \
        "After=network-online.target" "" "[Service]" "ExecStart=$PWD/tools/claude_runner.sh run %i" \
        "ExecStop=docker stop rerun-claude-runner-%i" "Restart=always" "" "[Install]" "WantedBy=default.target"
}

preview_runner() {
    dir="${PREVIEW_RUNNER_DIR:-$HOME/actions-runner-rerun-dev}"
    if [[ ! -f "$dir/.runner" ]]; then
        mkdir -p "$dir"
        tarball=$(ls ~/actions-runner/actions-runner-linux-x64-*.tar.gz 2> /dev/null | tail -n1)
        if [[ -n "$tarball" ]]; then
            tar -xzf "$tarball" -C "$dir"
        else
            curl -fsSL "https://github.com/actions/runner/releases/download/v2.338.0/actions-runner-linux-x64-2.338.0.tar.gz" | tar -xz -C "$dir"
        fi
        token=$(gh api -X POST "repos/$repo/actions/runners/registration-token" --jq .token)
        (cd "$dir" && ./config.sh --unattended --replace --url "https://github.com/$repo" --token "$token" \
            --name "$(hostname)-preview" --labels rerun-preview --work _work)
    fi
    printf '%s\n' "[Unit]" "Description=GitHub Actions runner for $repo's previews (label rerun-preview)" \
        "After=network-online.target" "" "[Service]" "WorkingDirectory=$dir" "ExecStart=$dir/run.sh" \
        "KillSignal=SIGINT" "Restart=always" "" "[Install]" "WantedBy=default.target" \
        > ~/.config/systemd/user/rerun-preview-runner.service
    systemctl --user daemon-reload
    systemctl --user enable --now rerun-preview-runner.service
}

install() {
    build
    if ! systemctl is-active --quiet claude-runner-firewall.service; then
        firewall_unit | sudo tee /etc/systemd/system/claude-runner-firewall.service > /dev/null
        sudo systemctl daemon-reload
        sudo systemctl enable --now claude-runner-firewall.service
    fi
    make_network
    if reachable; then
        echo "The firewall rule is not working: the container can reach this machine." >&2
        exit 1
    fi
    if ! online; then
        echo "The container cannot reach GitHub." >&2
        exit 1
    fi
    mkdir -p ~/.config/systemd/user
    runner_unit > ~/.config/systemd/user/rerun-claude-runner@.service
    systemctl --user daemon-reload
    for slot in $(seq "$slots") light; do
        systemctl --user enable --now "rerun-claude-runner@$slot.service"
    done
    preview_runner
    echo "Started $slots slots, the light slot and the preview runner. Logs: journalctl --user -u 'rerun-*' -f"
}

"${1:?usage: $0 install|build|run <slot>}" "${@:2}"
