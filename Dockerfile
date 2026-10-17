# syntax=docker/dockerfile:1.7
#
# parcel-server image: the Rust API + content server, with the built SPA
# served from /app/static. Built with --no-default-features so Swagger UI is
# compiled out (it is also runtime-gated off in production).

# ---- frontend -------------------------------------------------------------
FROM oven/bun:1.4.0 AS frontend
WORKDIR /app/frontend
COPY frontend/package.json frontend/bun.lock ./
RUN bun install --frozen-lockfile
COPY frontend/ ./
RUN bun run build

# ---- backend --------------------------------------------------------------
FROM rust:1.97.1-slim-bookworm AS backend
WORKDIR /app
# aws-lc-sys (rustls's and jsonwebtoken's crypto) compiles AWS-LC with the gcc
# this image ships, through its `cc` builder: no cmake, clang or Go needed.
RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY .cargo ./.cargo
COPY crates ./crates
# sqlx::migrate! embeds crates/parcel-server/migrations at compile time.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked -p parcel-server --no-default-features \
    && cp target/release/parcel-server /usr/local/bin/parcel-server

# ---- runtime --------------------------------------------------------------
# Same pinned distroless base as slauth-rs: glibc + libgcc, no shell, nonroot,
# and the CA bundle (/etc/ssl/certs) that reqwest's platform verifier trusts.
FROM gcr.io/distroless/cc-debian12@sha256:fccdbb0a547c14e23fcf4ce8ad62ca5d43b4faae8d22cd292f490fef9946c96e AS runtime
WORKDIR /app
COPY --from=backend --chown=65532:65532 /usr/local/bin/parcel-server /usr/local/bin/parcel-server
COPY --from=frontend --chown=65532:65532 /app/frontend/dist /app/static
USER 65532:65532
ENV APP_ENV=production \
    PARCEL_STATIC_DIR=/app/static \
    PORT=8080
EXPOSE 8080
CMD ["parcel-server", "serve"]
