# ── Build stage (Rust) ──────────────────────────────────────
# Hard-code --platform to prevent exec format error on ARM Macs.
FROM --platform=linux/amd64 rust:1.95-bookworm AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p sprout-relay \
    && strip target/release/sprout-relay

# ── Web build stage (Node/pnpm) ────────────────────────────
FROM --platform=linux/amd64 node:22-bookworm-slim AS web-builder
WORKDIR /build
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY web/ web/
RUN corepack enable && pnpm install --frozen-lockfile --filter sprout-web
RUN pnpm -C web build

# ── Runtime stage ───────────────────────────────────────────
FROM --platform=linux/amd64 debian:bookworm-slim

# CAKE: non-root UID 1000 (numeric, not username)
RUN groupadd -g 1000 sprout && useradd -u 1000 -g sprout -m sprout

# CAKE: writable dirs
RUN mkdir -p /cache /tmp && chown sprout:sprout /cache /tmp

# socat for Istio abstract→file socket bridge
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates socat && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/sprout-relay /code/sprout-relay
COPY --from=web-builder /build/web/dist /code/web
COPY script/start /code/start
RUN chmod +x /code/start

ENV SPROUT_WEB_DIR="/code/web"

# CAKE: required Envoy env vars (overridden at runtime by CAKE).
ENV ENVOY_ADMIN_SOCKET_PATH="@envoy-admin.sock" \
    ENVOY_INGRESS_PORT="20001" \
    ENVOY_HTTP_EGRESS_SOCKET_PATH="@egress.sock" \
    ENVOY_DATADOG_PORT="3030" \
    CASH_FRAMEWORK="rust"

USER 1000
ENTRYPOINT ["/code/start"]
