# syntax=docker/dockerfile:1.7
#
# Public Buzz relay image — published as ghcr.io/block/buzz:<tag>.
#
# Builds the `buzz-relay` binary (Rust 1.95) and the `buzz-web` static bundle
# (pnpm + vite), then assembles them into a small debian-slim runtime with
# `git` available (the relay shells out to git for repo hydrate / receive-pack
# / upload-pack — see crates/buzz-relay/src/api/git).
#
# Multi-arch is handled by running this same Dockerfile on native amd64 and
# native arm64 runners (see .github/workflows/docker.yml). The Dockerfile
# itself is platform-agnostic; do not add --platform pins.

ARG RUST_VERSION=1.95
ARG NODE_VERSION=24
ARG DEBIAN_VERSION=bookworm

# ─── Stage 1: cargo-chef base ───────────────────────────────────────────────
FROM rust:${RUST_VERSION}-${DEBIAN_VERSION} AS chef
RUN cargo install cargo-chef --locked --version 0.1.71
WORKDIR /build

# ─── Stage 2: plan dependency graph ─────────────────────────────────────────
# Only the manifests are needed to compute the recipe; this layer rebuilds
# only when Cargo.{toml,lock} or crate manifests change, not on every source
# edit.
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ─── Stage 3: cook dependencies, then build the binary ──────────────────────
FROM chef AS builder
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        pkg-config \
        libssl-dev \
        ca-certificates \
        git \
    && rm -rf /var/lib/apt/lists/*
COPY --from=planner /build/recipe.json recipe.json
# Cook the full workspace recipe — relay deps include workspace siblings, so
# scoping to -p buzz-relay misses transitive deps and re-builds them later.
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked -p buzz-relay --bin buzz-relay \
    && strip target/release/buzz-relay

# ─── Stage 4: web bundle (pnpm + vite) ──────────────────────────────────────
# Independent of the Rust layers so a CSS change doesn't bust Rust cache and
# vice versa.
FROM node:${NODE_VERSION}-${DEBIAN_VERSION}-slim AS web-builder
WORKDIR /build
RUN corepack enable
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY patches/ patches/
COPY web/package.json web/
RUN pnpm install --frozen-lockfile --filter buzz-web
COPY web/ web/
RUN pnpm -C web build

# ─── Stage 5: runtime ───────────────────────────────────────────────────────
FROM debian:${DEBIAN_VERSION}-slim AS runtime

# OCI annotations: required for GHCR to auto-link the image to this repo and
# inherit its visibility. org.opencontainers.image.source is the load-bearing
# one — without it GHCR keeps the image private even when the repo is public.
LABEL org.opencontainers.image.title="Buzz" \
      org.opencontainers.image.description="WebSocket relay server for the Buzz communications platform" \
      org.opencontainers.image.source="https://github.com/block/buzz" \
      org.opencontainers.image.url="https://github.com/block/buzz" \
      org.opencontainers.image.documentation="https://github.com/block/buzz#readme" \
      org.opencontainers.image.licenses="Apache-2.0"

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        git \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 1000 buzz \
    && useradd  --system --uid 1000 --gid 1000 --home-dir /var/lib/buzz \
                --create-home --shell /usr/sbin/nologin buzz

COPY --from=builder    /build/target/release/buzz-relay /usr/local/bin/buzz-relay
COPY --from=web-builder /build/web/dist                 /srv/buzz/web

ENV BUZZ_WEB_DIR=/srv/buzz/web

# 3000: app (WS + REST + web UI)  ·  8080: /_liveness, /_readiness  ·  9102: /metrics
EXPOSE 3000 8080 9102

USER buzz:buzz
WORKDIR /var/lib/buzz

ENTRYPOINT ["/usr/local/bin/buzz-relay"]
