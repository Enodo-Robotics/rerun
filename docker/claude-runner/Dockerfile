# A GitHub Actions runner for this repo's CI, Claude and preview-build jobs,
# with what CLAUDE.md needs already installed: the pinned Rust toolchain with
# the wasm target, the system libraries of the Linux wheel build
# (docker/Dockerfile), protoc, binaryen 121 (apt's 105 breaks the web viewer), pixi for
# the `pixi run` tasks, cargo-nextest, Mesa's software Vulkan for the snapshot
# tests, gh, and ssh (publish.yml). Started by tools/claude_runner.sh, one container per job.
FROM ubuntu:22.04

ARG RUNNER_VERSION=2.338.0
ARG RUST_VERSION=1.92.0
ARG BINARYEN_VERSION=121
ARG PIXI_VERSION=0.84.2
ARG NEXTEST_VERSION=0.9.151
# Matches pixi.toml's libprotobuf 6.* (protoc 32); lance needs its well-known types.
ARG PROTOC_VERSION=32.1
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl git git-lfs jq unzip xz-utils zstd build-essential clang pkg-config \
        libatk-bridge2.0-dev libatk1.0-dev libfontconfig-dev libfreetype-dev \
        libgl-dev libglib2.0-dev libgtk-3-dev libpango1.0-dev libssl-dev \
        libvulkan-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
        libxkbcommon-dev libwayland-dev mesa-vulkan-drivers nasm openssh-client \
        python3 python3-pip python3-venv python-is-python3 \
    && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
        -o /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
        > /etc/apt/sources.list.d/github-cli.list \
    && apt-get update && apt-get install -y --no-install-recommends gh \
    && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL "https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VERSION}/binaryen-version_${BINARYEN_VERSION}-x86_64-linux.tar.gz" \
        | tar -xz -C /usr/local --strip-components=1 \
    && wasm-opt --version \
    && curl -fsSL -o /tmp/protoc.zip "https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-linux-x86_64.zip" \
    && unzip -q -o /tmp/protoc.zip -d /usr/local bin/protoc 'include/*' && rm /tmp/protoc.zip \
    && chmod -R a+rX /usr/local/include/google && protoc --version \
    && curl -fsSL "https://github.com/prefix-dev/pixi/releases/download/v${PIXI_VERSION}/pixi-x86_64-unknown-linux-musl" \
        -o /usr/local/bin/pixi && chmod 755 /usr/local/bin/pixi \
    && curl -fsSL "https://get.nexte.st/${NEXTEST_VERSION}/linux" | tar -xz -C /usr/local/bin \
    && pip install --no-cache-dir maturin==1.9.6

RUN useradd --create-home --uid 1001 runner
WORKDIR /home/runner/actions-runner
RUN curl -fsSL "https://github.com/actions/runner/releases/download/v${RUNNER_VERSION}/actions-runner-linux-x64-${RUNNER_VERSION}.tar.gz" \
        | tar -xz \
    && ./bin/installdependencies.sh \
    && rm -rf /var/lib/apt/lists/* \
    && chown -R runner:runner .

# The cargo registry, the target dirs, pixi's package cache and environments,
# and the job checkouts are volumes, so builds are incremental from one job to
# the next. Pixi keeps its environments outside the checkout (actions/checkout
# cleans it). The directories exist here so new volumes start out owned by the
# runner user.
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo CARGO_TARGET_DIR=/home/runner/cache/target \
    PIXI_CACHE_DIR=/home/runner/cache/pixi-pkgs
# Cargo.toml gives every re_* crate debug info, and each test binary links all of it in: over
# 1 GB a binary, so the cache outgrew entrypoint.sh's limit and was wiped every job or two.
# The linker drops it from the binaries; panics keep their messages and backtraces their
# function names. (Cargo ignores `strip` for these, so it is a link arg.) Linux only, so the
# wasm build keeps .cargo/config.toml's rustflags. Set for every job, so they share one cache
# instead of rebuilding over each other's flags.
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS=-Clink-arg=-Wl,--strip-debug
ENV PATH=/opt/cargo/bin:$PATH
RUN mkdir -p /opt/rustup /opt/cargo/registry /opt/cargo/git /home/runner/cache/target \
        /home/runner/cache/pixi-pkgs /home/runner/cache/pixi-envs /home/runner/work /home/runner/.pixi \
    && chown -R runner:runner /opt/rustup /opt/cargo /home/runner/cache /home/runner/work /home/runner/.pixi
USER runner
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal \
        --default-toolchain "${RUST_VERSION}" -c clippy -c rustfmt -t wasm32-unknown-unknown \
    && pixi config set --global detached-environments /home/runner/cache/pixi-envs \
    && git lfs install
# ffmpeg for re_view_spatial's video snapshot tests, which find it on PATH. The version
# matches pixi.toml's pin (>=7.1,<8), since decoded frames differ between versions;
# Ubuntu's apt package is 4.4.
RUN pixi global install "ffmpeg>=7.1,<8" && ffmpeg -version | head -n1
ENV PATH=/home/runner/.pixi/bin:$PATH

COPY --chmod=755 entrypoint.sh /entrypoint.sh
ENTRYPOINT ["/entrypoint.sh"]
