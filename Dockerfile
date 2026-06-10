# ── Build stage (Rust) ──────────────────────────────────────
# Hard-code --platform to prevent exec format error on ARM Macs.
FROM --platform=linux/amd64 rust:1.95-bookworm AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p buzz-relay \
    && strip target/release/buzz-relay

# ── Web build stage (Node/pnpm) ────────────────────────────
FROM --platform=linux/amd64 node:24-bookworm-slim AS web-builder
WORKDIR /build
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY web/ web/
RUN corepack enable && pnpm install --frozen-lockfile --filter buzz-web
RUN pnpm -C web build

# ── Runtime stage ───────────────────────────────────────────
FROM --platform=linux/amd64 debian:bookworm-slim

# CAKE: non-root UID 1000 (numeric, not username)
RUN groupadd -g 1000 buzz && useradd -u 1000 -g buzz -m buzz

# CAKE: writable dirs
RUN mkdir -p /cache /tmp && chown buzz:buzz /cache /tmp

# git: relay shells out to `git` for hydrate/receive-pack/upload-pack (S3-backed repos)
# socat: Istio abstract→file socket bridge
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates git socat && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/buzz-relay /code/buzz-relay
COPY --from=web-builder /build/web/dist /code/web
COPY script/start /code/start
RUN chmod +x /code/start

ENV BUZZ_WEB_DIR="/code/web"

# CAKE: required Envoy env vars (overridden at runtime by CAKE).
ENV ENVOY_ADMIN_SOCKET_PATH="@envoy-admin.sock" \
    ENVOY_INGRESS_PORT="20001" \
    ENVOY_HTTP_EGRESS_SOCKET_PATH="@egress.sock" \
    ENVOY_DATADOG_PORT="3030" \
    CASH_FRAMEWORK="rust"

USER 1000
ENTRYPOINT ["/code/start"]
