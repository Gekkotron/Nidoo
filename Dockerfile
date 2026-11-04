# syntax=docker/dockerfile:1.6
#
# Multi-stage build of the Nidoo hub.
# Uses cargo-chef to cache the dependency graph in its own layer so
# incremental rebuilds only recompile the workspace crates (nidoo-*),
# not the hundreds of crates they pull in from crates.io.
#
# Stage layout:
#   base     — toolchain + system deps + cargo-chef + trunk (rarely rebuilt)
#   planner  — emits recipe.json from the current Cargo.toml/Cargo.lock
#   builder  — cooks deps (cached layer), then compiles web + hub
#   sound-builder — compiles nidoo-sound alone (own cooked deps)
#   sound    — the nidoo-sound image: `docker build --target sound`
#   runtime  — ships only the hub binary on debian:bookworm-slim (the
#              default target: keep it the last stage)

ARG RUST_IMAGE=rust:1-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim

# ---------- base ----------
FROM ${RUST_IMAGE} AS base

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        ca-certificates \
        cmake \
        clang \
        pkg-config \
        libssl-dev \
        curl \
 && rm -rf /var/lib/apt/lists/*

RUN rustup target add wasm32-unknown-unknown
RUN cargo install cargo-chef --locked --version "^0.1"
RUN cargo install trunk --locked --version "^0.21"

WORKDIR /src

# ---------- planner ----------
FROM base AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ---------- builder ----------
FROM base AS builder

# Cook native (release) deps — this layer is reused unless Cargo.lock
# or any Cargo.toml changes. Scoped to the hub so the feature set
# matches the final `cargo build -p nidoo`.
COPY --from=planner /src/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo chef cook --release -p nidoo --recipe-path recipe.json

# Cook wasm deps for the Leptos UI compiled by trunk. Scoped to
# nidoo-web: the hub crates pull tokio's `net` (mio), which does not
# build for wasm32-unknown-unknown.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo chef cook --release --target wasm32-unknown-unknown -p nidoo-web --recipe-path recipe.json

# brotli for the web assets' pre-compressed copies (crates/nidoo-web/compress.sh). Its own
# layer after the cooked deps, so adding it did not invalidate them.
RUN apt-get update \
 && apt-get install -y --no-install-recommends brotli \
 && rm -rf /var/lib/apt/lists/*

# Bring the real source in and build the workspace crates against the
# cooked target/ directory.
COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cd crates/nidoo-web && trunk build --release

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release -p nidoo

# ---------- sound-builder ----------
# nidoo-sound, the Sendspin-compatible sound service, has its own build so a
# hub rebuild never touches the sound image (and the other way round).
FROM base AS sound-builder
COPY --from=planner /src/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo chef cook --release -p nidoo-sound --recipe-path recipe.json
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release -p nidoo-sound

# ---------- sound ----------
FROM ${RUNTIME_IMAGE} AS sound

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        ca-certificates \
        tzdata \
 && rm -rf /var/lib/apt/lists/*

COPY --from=sound-builder /src/target/release/nidoo-sound /usr/local/bin/nidoo-sound

# ./data is /data from here (sound/identity.key, sound/players.json).
WORKDIR /
EXPOSE 4953
ENTRYPOINT ["/usr/local/bin/nidoo-sound"]
CMD ["run"]

# ---------- runtime ----------
FROM ${RUNTIME_IMAGE} AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        ca-certificates \
        tzdata \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/nidoo /usr/local/bin/nidoo

# No ENV defaults: the hub's own (0.0.0.0:8081, ./data = /data from here) fit,
# and an image NIDOO_* value would beat a user's override in .env.
WORKDIR /
EXPOSE 8081
ENTRYPOINT ["/usr/local/bin/nidoo"]
CMD ["run"]
