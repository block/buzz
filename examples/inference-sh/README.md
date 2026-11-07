# inference.sh

A persona pack that gives Buzz agents access to AI apps on [inference.sh](https://inference.sh) — image generation, video, audio, search, LLMs, and more.

| Agent | Role |
|-------|------|
| **Shelly** | Creative agent — generates images, video, audio, and other media on demand |

## Prerequisites

Install the [inference-mcp-bridge](https://github.com/inference-sh/inference-mcp-bridge) binary:

```bash
# Download from releases
curl -L https://github.com/inference-sh/inference-mcp-bridge/releases/latest/download/inference-mcp-bridge_$(uname -s)_$(uname -m | sed 's/aarch64/arm64/').tar.gz | tar xz

# Or build from source
cargo install --git https://github.com/inference-sh/inference-mcp-bridge --path bridge
```

Get an API key at [inference.sh/settings/keys](https://app.inference.sh/settings/keys).

## Usage

```bash
export INFERENCE_API_KEY="your-api-key"

# Validate the pack
buzz pack validate ./examples/inference-sh

# Inspect resolved config
buzz pack inspect ./examples/inference-sh
```

## Structure

```
inference-sh/
├── .plugin/
│   └── plugin.json       # Pack manifest (OPS-compatible)
├── agents/
│   └── shelly.persona.md  # Creative agent
├── .mcp.json             # inference-mcp-bridge config
└── README.md
```

## How it works

The pack configures [inference-mcp-bridge](https://github.com/inference-sh/inference-mcp-bridge) as an MCP server. The bridge translates between Buzz's stdio MCP transport and inference.sh's HTTP API, giving agents access to tools like `app_run`, `app_list`, and `app_get`.

## Customizing

Edit `agents/shelly.persona.md` to change the agent's behavior, or add more personas to the pack. Each agent in the pack shares the inference.sh MCP server defined in `.mcp.json`.

See `crates/buzz-persona/PERSONA_PACK_SPEC.md` for the full format reference.
