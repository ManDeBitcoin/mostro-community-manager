# A single immutable image serves the static UI, API and verified Mostro daemon for Umbrel.
FROM debian:bookworm-slim AS download
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl ncurses-base && rm -rf /var/lib/apt/lists/*
RUN set -eu; \
    case "$TARGETARCH" in \
      amd64) artifact=mostrod-x86_64-unknown-linux-musl; checksum=078c2f1a2f0d9a3e438e5d363480c79a19931664ceb36be0661155f73171bc86 ;; \
      arm64) artifact=mostrod-aarch64-unknown-linux-musl; checksum=f82fe9e7f09e727851a8fcbf107d4c266abdb1ee47f7092dba116f891ccce1f6 ;; \
      *) echo 'Unsupported architecture' >&2; exit 1 ;; \
    esac; \
    curl -fsSL "https://github.com/MostroP2P/mostro/releases/download/v0.20.0/$artifact" -o /mostrod; \
    echo "$checksum  /mostrod" | sha256sum -c -; \
    chmod 755 /mostrod; \
    TERM=xterm /mostrod --version
FROM node:22.13.0-bookworm-slim AS web
WORKDIR /src
COPY web/package*.json ./
RUN npm ci
COPY web ./
RUN npm run build
FROM rust:1.94.0-bookworm AS api
RUN rustup component add rustfmt clippy
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY api ./api
COPY config ./config
# Release tag reported by /api/health; "dev" outside the release workflow.
ARG MANAGER_VERSION=dev
ENV MANAGER_VERSION=$MANAGER_VERSION
RUN cargo fmt --all -- --check && cargo test --workspace --locked && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo build --locked --release -p mostro-community-api
FROM debian:bookworm-slim
LABEL org.opencontainers.image.source="https://github.com/ManDeBitcoin/mostro-community-manager"
LABEL org.opencontainers.image.title="Mostro Community Manager"
LABEL org.opencontainers.image.licenses="GPL-3.0-or-later"
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl ncurses-base procps && rm -rf /var/lib/apt/lists/*
COPY --from=api /src/target/release/mostro-community-api /usr/local/bin/mostro-community-api
COPY --from=download /mostrod /usr/local/bin/mostrod
# mostrod is GPL-3.0-or-later since v0.19.2: ship its license and source pointer.
COPY LICENSE /usr/share/licenses/mostro-community-manager/LICENSE
COPY config/upstream/LICENSE-GPL-3.0-or-later /usr/share/licenses/mostrod/LICENSE
COPY config/upstream/LICENSE-MIT /usr/share/licenses/mostrod/LICENSE-MIT
COPY config/upstream/NOTICE-mostrod.md /usr/share/licenses/mostrod/NOTICE.md
COPY docker/mostro-entrypoint.sh /usr/local/bin/mostro-entrypoint
RUN chmod 755 /usr/local/bin/mostro-entrypoint
COPY --from=web /src/dist /opt/manager/web
ENV API_BIND=0.0.0.0:3001 CONFIG_DIR=/data/config STATIC_DIR=/opt/manager/web TERM=xterm
# The mostrod release above; the entrypoint copies the database before another one starts.
ENV MOSTROD_VERSION=0.20.0
USER 1000:1000
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s CMD curl -fsS http://127.0.0.1:3001/api/health || exit 1
ENTRYPOINT ["mostro-community-api"]
