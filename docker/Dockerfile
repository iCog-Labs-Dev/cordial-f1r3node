# syntax=docker/dockerfile:1.7

# Builds the local f1r3node binary with the sibling cordial-f1r3node checkout.
# Build context is the rho-calculus parent directory so the Dockerfile can copy:
#   ./f1r3node
#   ./cordial-f1r3node
#
# The companion Dockerfile.dockerignore keeps the context restricted to those
# source trees and excludes target/, node_modules/, and .git/ state.

ARG CORDIAL_RUST_BASE=rust:slim-bookworm
FROM ${CORDIAL_RUST_BASE} AS source

ENV PATH="/usr/local/cargo/bin:${PATH}"
ARG RUST_TOOLCHAIN=nightly-2026-02-09

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        clang \
        cmake \
        curl \
        libclang-dev \
        libprotobuf-dev \
        libssl-dev \
        pkg-config \
        protobuf-compiler; \
    rm -rf /var/lib/apt/lists/*

RUN --mount=type=cache,target=/usr/local/rustup/downloads \
    set -eux; \
    for attempt in 1 2 3; do \
        if rustup toolchain install "${RUST_TOOLCHAIN}" --profile minimal; then \
            break; \
        fi; \
        if [ "${attempt}" = "3" ]; then \
            exit 1; \
        fi; \
        sleep "$((attempt * 10))"; \
    done; \
    rustup default "${RUST_TOOLCHAIN}"

WORKDIR /build

COPY f1r3node/ /build/f1r3node/
COPY cordial-f1r3node/ /build/cordial-f1r3node/

# The source trees pin local developer toolchains. The demo image installs the
# f1r3node pinned nightly once in this shared source layer and removes local
# overrides so both checkouts compile through the same compiler inside Docker.
RUN rm -f /build/f1r3node/rust-toolchain.toml /build/cordial-f1r3node/rust-toolchain.toml

FROM source AS cordial-conformance

WORKDIR /build/cordial-f1r3node
CMD ["bash", "-lc", "/usr/local/cargo/bin/cargo test -p cordial-f1r3node-adapter --test conformance -- --nocapture"]

FROM source AS node-builder

WORKDIR /build/f1r3node

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/f1r3node/target \
    /usr/local/cargo/bin/cargo build --release -p node && \
    cp /build/f1r3node/target/release/node /build/node-binary

FROM debian:bookworm-slim AS runtime

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        jq \
        libc6 \
        libjemalloc2 \
        libssl3; \
    rm -rf /var/lib/apt/lists/*

WORKDIR /opt/docker

RUN mkdir -p /opt/docker/bin /var/lib/rnode && \
    chown -R daemon:daemon /opt/docker /var/lib/rnode

COPY --from=node-builder --chmod=755 /build/node-binary /opt/docker/bin/node
COPY --from=node-builder --chmod=755 /build/f1r3node/node/docker-entrypoint-rust.sh /opt/docker/bin/docker-entrypoint.sh
COPY --from=node-builder /build/f1r3node/node/src/main/resources /opt/docker/node/src/main/resources
COPY --from=node-builder /build/f1r3node/casper/src/main/resources /opt/docker/casper/src/main/resources
COPY --from=node-builder /build/f1r3node/rholang/examples /opt/docker/rholang/examples

EXPOSE 40400 40401 40402 40403 40404 40405

HEALTHCHECK --interval=30s --timeout=10s --start-period=90s --retries=5 \
    CMD curl -fsS http://127.0.0.1:40403/api/status >/dev/null || exit 1

ENTRYPOINT ["/opt/docker/bin/docker-entrypoint.sh"]
CMD ["run"]
