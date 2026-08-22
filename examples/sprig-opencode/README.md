# sprig-opencode

A derived sprig image that adds the [OpenCode](https://opencode.ai) runtime,
for remote agents whose `agent_command` is `opencode` (native ACP mode,
`opencode acp`).

## Why a derived image

The default `ghcr.io/block/buzz-sprig` intentionally ships only Buzz's own
multicall binary (~15-25MB). Per the remote-agent spec
(`docs/remote-agents.md`, *Image*), alternate-harness dependencies come via
the agent's `image` override — "buzz-sprig plus your tools" — so OpenCode
(and its credentials path) is added here instead of fattening the default.

## Build and push

Build on the architectures your cluster runs (the base image and the musl
assets are multi-arch):

```sh
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  --tag <your-registry>/buzz-sprig-opencode:<tag> \
  --push \
  .
```

Bump the `OPENCODE_VERSION` build arg deliberately — it is pinned so image
builds are reproducible.

## Use it

Set the agent's image override to the pushed reference (digest form is the
most traceable) and the harness command to OpenCode:

- `agent_command`: `opencode` (resolved in-image; sprig's `opencode acp`
  default args apply)
- `image`: `<your-registry>/buzz-sprig-opencode@sha256:<digest>`

## Credentials

OpenCode resolves providers from its own config/auth store
(`~/.config/opencode/opencode.json`, `~/.local/share/opencode/auth.json`)
plus environment variables. For remote pods, provider credentials are
typically injected through the agent record's `env_vars` rather than baked
into the image — never bake secrets into a pushed layer.

## Conformance

An image override MUST contain the runtime ABI — the `buzz-acp` entrypoint
and everything the launch ABI requires. Building `FROM` the published sprig
image inherits all of it; only add tools on top.
