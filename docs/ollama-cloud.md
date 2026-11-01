# Ollama Cloud in Buzz

[Ollama Cloud](https://docs.ollama.com/cloud) exposes an OpenAI-compatible API.
Buzz treats it as a **named provider** so you do not have to pick
“OpenAI-compatible” and type the base URL by hand ([#3874](https://github.com/block/buzz/issues/3874)).

## Desktop

1. Agent defaults / persona → provider **Ollama Cloud**
2. Paste your Ollama API key into the credential field (`OPENAI_COMPAT_API_KEY`)
3. Set a model id from your Ollama Cloud account (e.g. a hosted open model)

Buzz maps this to `BUZZ_AGENT_PROVIDER=ollama-cloud` and defaults
`OPENAI_COMPAT_BASE_URL` to `https://ollama.com/v1`.

## CLI / buzz-agent

```bash
BUZZ_AGENT_PROVIDER=ollama-cloud \
OPENAI_COMPAT_API_KEY=... \
OPENAI_COMPAT_MODEL=... \
  buzz-agent
```

Override the endpoint with `OPENAI_COMPAT_BASE_URL` if needed.

## Related

- Local Ollama (localhost): see open PR / docs for the `ollama` alias (#3145 / #3152)
- Generic OpenAI-compatible: provider `openai-compat` with any base URL
