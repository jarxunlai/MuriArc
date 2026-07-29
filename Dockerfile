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
    cargo build --locked --release \
      -p muriarc-server \
      -p muriarc-upgrade-executor \
      -p muriarc-verifier \
      -p muriarc-release-fixture \
      -p muriarcctl \
      --features muriarc-server/postgres,muriarc-release-fixture/postgres \
    && cp target/release/muriarc-server /tmp/muriarc-server \
    && cp target/release/muriarc-upgrade-executor /tmp/muriarc-upgrade-executor \
    && cp target/release/muriarc-verifier /tmp/muriarc-verifier \
    && cp target/release/muriarc-release-fixture /tmp/muriarc-release-fixture \
    && cp target/release/muriarcctl /tmp/muriarcctl \
    && strip /tmp/muriarc-server \
      /tmp/muriarc-upgrade-executor \
      /tmp/muriarc-verifier \
      /tmp/muriarc-release-fixture \
      /tmp/muriarcctl

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 muriarc \
    && useradd --system --uid 10001 --gid muriarc --home-dir /nonexistent --shell /usr/sbin/nologin muriarc \
    && mkdir -p /opt/muriarc/ui /var/lib/muriarc/data /var/lib/muriarc/attachments \
    && chown -R muriarc:muriarc /opt/muriarc /var/lib/muriarc

COPY --from=server-build --chown=root:root /tmp/muriarc-server /usr/local/bin/muriarc-server
COPY --from=server-build --chown=root:root /tmp/muriarc-upgrade-executor /usr/local/bin/muriarc-upgrade-executor
COPY --from=server-build --chown=root:root /tmp/muriarc-verifier /usr/local/bin/muriarc-verifier
COPY --from=server-build --chown=root:root /tmp/muriarc-release-fixture /usr/local/bin/muriarc-release-fixture
COPY --from=server-build --chown=root:root /tmp/muriarcctl /usr/local/bin/muriarcctl
COPY --from=ui-build --chown=muriarc:muriarc /workspace/ui/dist/ /opt/muriarc/ui/

USER 10001:10001
ENV MURIARC_BIND_ADDR=0.0.0.0:8787 \
    MURIARC_UI_DIR=/opt/muriarc/ui \
    MURIARC_DATA_ROOT=/var/lib/muriarc/data \
    MURIARC_ATTACHMENT_ROOT=/var/lib/muriarc/attachments \
    RUST_LOG=muriarc_server=info,tower_http=info
EXPOSE 8787
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD ["curl", "--fail", "--silent", "--show-error", "http://127.0.0.1:8787/readyz"]
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/muriarc-server"]


# Build the public Tester executables as static musl binaries. This keeps the
# unsigned Tester runtime independent from the glibc-based signed runtime above
# and removes package-manager/tooling attack surface from the distributed image.
FROM server-build AS tester-build
RUN apt-get update \
    && apt-get install -y --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add x86_64-unknown-linux-musl
RUN --mount=type=cache,id=muriarc-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=muriarc-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,id=muriarc-server-tester-musl-target,target=/workspace/target,sharing=locked \
    cargo build --locked --release --target x86_64-unknown-linux-musl \
      -p muriarc-server \
      -p muriarc-standard-fixture \
      --features muriarc-server/postgres,muriarc-standard-fixture/postgres \
    && cp target/x86_64-unknown-linux-musl/release/muriarc-server /tmp/muriarc-server-tester \
    && cp target/x86_64-unknown-linux-musl/release/muriarc-standard-fixture /tmp/muriarc-standard-fixture-tester \
    && strip /tmp/muriarc-server-tester /tmp/muriarc-standard-fixture-tester

# The public Tester image is intentionally separate from the signed Server
# runtime. Formal consumers continue to build the `runtime` target above
# unchanged. Pin the minimal Alpine rootfs and add only CA roots; BusyBox wget
# provides the local readiness probe without shipping curl or a package manager
# from a general-purpose Debian runtime.
FROM alpine:3.22@sha256:14358309a308569c32bdc37e2e0e9694be33a9d99e68afb0f5ff33cc1f695dce AS tester-runtime
ARG MURIARC_IMAGE_SOURCE=https://github.com/jarxunlai/MuriArc
ARG MURIARC_IMAGE_REVISION
ARG MURIARC_IMAGE_VERSION=1.0.0-server-tester
LABEL org.opencontainers.image.title="MuriArc Server Docker Tester" \
      org.opencontainers.image.description="Unsigned linux/amd64 MuriArc Server Tester with optional synthetic standard-v1 seeder" \
      org.opencontainers.image.source="$MURIARC_IMAGE_SOURCE" \
      org.opencontainers.image.revision="$MURIARC_IMAGE_REVISION" \
      org.opencontainers.image.version="$MURIARC_IMAGE_VERSION" \
      org.opencontainers.image.licenses="Apache-2.0"
RUN apk upgrade --no-cache \
    && apk add --no-cache ca-certificates \
    && addgroup -S -g 10001 muriarc \
    && adduser -S -D -H -u 10001 -G muriarc -s /sbin/nologin muriarc \
    && mkdir -p /opt/muriarc/ui /opt/muriarc/fixtures/standard-v1 /var/lib/muriarc \
    && chown -R muriarc:muriarc /opt/muriarc /var/lib/muriarc
COPY --from=tester-build --chown=root:root /tmp/muriarc-server-tester /usr/local/bin/muriarc-server
COPY --from=tester-build --chown=root:root /tmp/muriarc-standard-fixture-tester /usr/local/bin/muriarc-standard-fixture
COPY --from=ui-build --chown=muriarc:muriarc /workspace/ui/dist/ /opt/muriarc/ui/
COPY --chown=root:root fixtures/standard-v1/ /opt/muriarc/fixtures/standard-v1/
USER 10001:10001
ENV MURIARC_BIND_ADDR=0.0.0.0:8787 \
    MURIARC_UI_DIR=/opt/muriarc/ui \
    MURIARC_DATA_ROOT=/var/lib/muriarc/data \
    MURIARC_ATTACHMENT_ROOT=/var/lib/muriarc/attachments \
    RUST_LOG=muriarc_server=info,tower_http=info
EXPOSE 8787
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD ["wget", "-q", "-T", "5", "-O", "/dev/null", "http://127.0.0.1:8787/readyz"]
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/muriarc-server"]
