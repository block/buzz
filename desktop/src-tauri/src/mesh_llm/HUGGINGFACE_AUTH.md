# Hugging Face authentication boundary

Hub browsing resolves `HF_TOKEN` through the Rust provider-secret catalog and
keeps the value native-side. Mesh model downloads are a separate boundary.

The currently pinned MeshLLM `v0.75.1` host-runtime download functions build
their Hugging Face client from process environment and do not accept an
in-memory token. Zorro therefore:

- permits public Hub selections;
- permits gated selections only when a non-empty `HF_TOKEN` was present in the
  environment that launched Zorro;
- does not copy a keyring value into process environment.

Before enabling keyring-only gated downloads, update the MeshLLM pin to an SDK
that accepts an in-memory token in its model-download/serve builder, then load
the provider secret immediately before startup and pass it through that API.
The token must remain absent from request serialization, IPC results, logs,
status payloads, and debug output.
