# syntax=docker/dockerfile:1.7

FROM node:22-bookworm-slim AS ui-build
WORKDIR /workspace/ui
RUN corepack enable \
    && corepack prepare pnpm@11.5.0 --activate
COPY ui/package.json ui/pnpm-lock.yaml ui/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile
COPY ui/ ./
COPY branding/ /workspace/branding/
ENV VITE_MURIARC_GATEWAY=remote
RUN pnpm run build

FROM rust:1.88-bookworm AS server-build
RUN apt-get update \
    && apt-get install -y --no-install-recommends clang cmake pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /workspace
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ ./crates/
COPY src-tauri/ ./src-tauri/
COPY migrations/ ./migrations/
RUN --mount=type=cache,id=muriarc-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=muriarc-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=muriarc-server-target,target=/workspace/target,sharing=locked \
    cargo build --locked --release -p muriarc-server --features postgres \
    && cp target/release/muriarc-server /tmp/muriarc-server \
    && strip /tmp/muriarc-server

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 muriarc \
    && useradd --system --uid 10001 --gid muriarc --home-dir /nonexistent --shell /usr/sbin/nologin muriarc \
    && mkdir -p /opt/muriarc/ui /var/lib/muriarc/data /var/lib/muriarc/attachments \
    && chown -R muriarc:muriarc /opt/muriarc /var/lib/muriarc

COPY --from=server-build --chown=root:root /tmp/muriarc-server /usr/local/bin/muriarc-server
COPY --from=ui-build --chown=muriarc:muriarc /workspace/ui/dist/ /opt/muriarc/ui/

USER 10001:10001
ENV MURIARC_BIND_ADDR=0.0.0.0:8787 \
    MURIARC_UI_DIR=/opt/muriarc/ui \
    MURIARC_DATA_ROOT=/var/lib/muriarc/data \
    MURIARC_ATTACHMENT_ROOT=/var/lib/muriarc/attachments \
    RUST_LOG=muriarc_server=info,tower_http=info
EXPOSE 8787
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD ["curl", "--fail", "--silent", "--show-error", "http://127.0.0.1:8787/api/v1/health"]
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/muriarc-server"]
