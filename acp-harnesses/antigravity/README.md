# Antigravity ACP Harness for Buzz

This directory contains the Agent Client Protocol (ACP) harness for **Antigravity** (Google DeepMind's AI coding assistant powered by Gemini).

It allows Antigravity to operate as a first-class agent inside [Buzz](https://github.com/block/buzz) workspaces, responding to `@antigravity` mentions in channels, reviewing pull requests, and assisting with technical tasks.

## Protocol Architecture

```
Buzz Relay (Nostr) ──[WebSocket]──> buzz-acp ──[JSON-RPC stdio]──> buzz_acp_antigravity.py ──[HTTPS]──> Gemini API
```

## Quick Start

### 1. Requirements
- Python 3.10+
- `google-generativeai` SDK

```bash
pip install -r requirements.txt
```

### 2. Environment Variables
- `GEMINI_API_KEY`: Your Google Gemini API key (obtain from [Google AI Studio](https://aistudio.google.com/apikey)).
- `GEMINI_MODEL`: (Optional) Desired Gemini model ID (default: `gemini-2.0-flash`).

### 3. Launching via `buzz-acp`

```bash
export BUZZ_RELAY_URL="ws://your-relay-host:3001"
export BUZZ_PRIVATE_KEY="nsec1..."   # Agent's Nostr secret key
export GEMINI_API_KEY="AIza..."

buzz-acp \
  --relay-url "$BUZZ_RELAY_URL" \
  --private-key "$BUZZ_PRIVATE_KEY" \
  --agent-command python3 \
  --agent-args /path/to/buzz_acp_antigravity.py \
  --respond-to anyone
```

## Running Tests

```bash
pytest tests/ -v
```

All 7 unit tests (TC-01 through TC-07) verify JSON-RPC 2.0 protocol compliance, initialization, session management, cancellation, and error handling.
