# Production API image. The runtime stage contains neither the Rust toolchain
# nor the build cache; it runs as the unprivileged `rs-admin` user. The same
# image carries the one-shot migration binary and its grant script.

FROM rust:1.99.1-bookworm AS builder

ARG GIT_COMMIT=unknown

WORKDIR /build

# Copy the workspace manifests first so dependency compilation can be reused
# when only application sources change.
COPY Cargo.toml Cargo.lock ./
COPY backend ./backend

ENV GIT_COMMIT=${GIT_COMMIT}

RUN cargo build --locked --release \
    --package rs-admin-api \
    --bin rs-admin-api \
    --bin migrate

FROM debian:bookworm-slim

# curl is used by the Compose healthcheck; psql applies the role grants after
# the SQLx migrations. Neither needs the Rust toolchain.
RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        ca-certificates \
        curl \
        postgresql-client \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --system --gid 10001 rs-admin \
    && useradd --system --uid 10001 --gid rs-admin \
        --no-create-home --shell /usr/sbin/nologin rs-admin

COPY --from=builder /build/target/release/rs-admin-api /usr/local/bin/rs-admin-api
COPY --from=builder /build/target/release/migrate /usr/local/bin/rs-admin-migrate
COPY deploy/db/grants.sql /opt/rs-admin/grants.sql
COPY deploy/api/migrate.sh /usr/local/bin/rs-admin-migrate-and-grant

RUN chmod 0755 \
    /usr/local/bin/rs-admin-api \
    /usr/local/bin/rs-admin-migrate \
    /usr/local/bin/rs-admin-migrate-and-grant

USER 10001:10001

ENV TMPDIR=/tmp \
    RUST_LOG=info,sqlx=warn

EXPOSE 8080 9100

ENTRYPOINT ["/usr/local/bin/rs-admin-api"]
