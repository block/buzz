# syntax=docker/dockerfile:1.7
# Derived sprig image with the OpenCode runtime for remote agents.
#
# Spec context (docs/remote-agents.md, Image): the default
# `ghcr.io/block/buzz-sprig` stays lean (~15-25MB) and alternate-harness
# dependencies come via the `image` override field, not a fatter default.
# This file is the "buzz-sprig plus your tools" pattern: build it, push it
# to a registry your cluster can pull, and set it as the agent's `image`
# override when `agent_command` is `opencode`.
#
# Multi-arch: build on native amd64 and arm64 runners, like Dockerfile.sprig.
# The tag/digest below is an example — pin to the digest you actually built
# against (the spec requires digest pinning for default references and
# records overrides in the pod annotation).
FROM ghcr.io/block/buzz-sprig:latest

# OpenCode runtime (tier-1 harness, native ACP via `opencode acp`).
# Pinned release — bump deliberately. Musl asset naming:
# opencode-linux-<arch>-musl.tar.gz with arch arm64|x64, single root-level
# binary. Same URL shape as https://opencode.ai/install.
ARG OPENCODE_VERSION=1.18.21
RUN set -eux; \
      arch="$(uname -m)"; \
      case "$arch" in aarch64) arch="arm64" ;; x86_64) arch="x64" ;; *) echo "unsupported arch $arch" >&2; exit 1 ;; esac; \
      curl -fsSL "https://github.com/anomalyco/opencode/releases/download/v${OPENCODE_VERSION}/opencode-linux-${arch}-musl.tar.gz" \
        | tar -xz -C /usr/local/bin

# The sprig entrypoint, PATH (includes /usr/local/bin), user, and baked
# gitconfig are inherited unchanged — this image only adds the runtime.
