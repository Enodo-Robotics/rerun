# Mirrors the Linux x86_64 job in .github/workflows/build_wheels_on_tag.yml
#
# Two binaryen modes:
#   - BINARYEN=apt   → version 105 (buggy on -O2, reproduces the table-export bug)
#   - BINARYEN=121   → upstream tarball (candidate fix)
#
# Pass USER_UID / USER_GID to match the host user so bind-mounted artifacts
# stay writable on the host.

FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive

# CI's explicit apt list + clang (CI runners have it preinstalled; zstd-sys needs it
# for the wasm32 target) + curl/ca-certificates for rustup + git for `cargo metadata`.
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git sudo \
      build-essential clang pkg-config \
      libatk-bridge2.0-dev libatk1.0-dev libfontconfig-dev libfreetype-dev \
      libgl-dev libglib2.0-dev libgtk-3-dev libpango1.0-dev libssl-dev \
      libvulkan-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
      libxkbcommon-dev nasm \
 && rm -rf /var/lib/apt/lists/*

ARG BINARYEN=apt
RUN set -eux; \
    case "$BINARYEN" in \
      apt) \
        apt-get update && apt-get install -y --no-install-recommends binaryen && \
        rm -rf /var/lib/apt/lists/*; \
        ;; \
      *) \
        curl -fsSL \
          "https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN}/binaryen-version_${BINARYEN}-x86_64-linux.tar.gz" \
          | tar -xz -C /usr/local --strip-components=1; \
        ;; \
    esac; \
    wasm-opt --version

ARG USER_UID=1000
ARG USER_GID=1000
RUN groupadd -g "${USER_GID}" builder && \
    useradd -m -u "${USER_UID}" -g builder -s /bin/bash builder && \
    echo "builder ALL=(ALL) NOPASSWD: ALL" > /etc/sudoers.d/builder

USER builder
WORKDIR /workspace

ENV RUSTUP_HOME=/home/builder/.rustup \
    CARGO_HOME=/home/builder/.cargo \
    PATH=/home/builder/.cargo/bin:$PATH

# Must match rust-toolchain.toml at the repo root (currently 1.92.0).
ARG RUST_VERSION=1.92.0
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --default-toolchain "${RUST_VERSION}" --profile minimal \
                   --target wasm32-unknown-unknown \
 && rustc --version

# Pre-create cargo cache dirs so named volumes inherit `builder` ownership
# on first mount (Docker seeds a named volume from the image path it replaces).
RUN mkdir -p /home/builder/.cargo/registry /home/builder/.cargo/git

CMD ["bash"]
