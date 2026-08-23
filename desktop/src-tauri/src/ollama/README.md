# Desktop Ollama lifecycle

This module is deliberately separate from agent provider configuration. It
owns machine-level Ollama connection settings, native model operations, and the
optional Buzz-private process. Selecting an Ollama provider for an agent does
not grant Buzz process or model ownership.

Ownership modes:

- `connect_only`: probe and list only; the external daemon remains untouched.
- `external_managed_models`: pull/delete models through the native API, but
  never start or stop the external daemon.
- `managed`: start and stop only the child kept in this module's runtime slot,
  bound to `127.0.0.1:11434` with a private model directory.

## Managed artifact manifest

`artifacts.json` is intentionally empty until release engineering supplies a
tested version and official artifact metadata. Never populate it from an
unverified release listing. Every platform row must include:

- the exact official `github.com/ollama/ollama` HTTPS release URL;
- SHA-256 of the downloaded bytes;
- independent compressed-download and extracted-byte limits;
- archive kind and relative executable path.

Installation remains disabled when no matching verified row exists. The
installer checks both size limits, checksum, archive paths and entry types,
then performs a rollback-capable directory replacement.

## Intentional current limits

- Managed Ollama starts on an explicit command or when a local Ollama-backed
  agent needs it, and stops on Buzz exit. Persisted configuration alone does
  not start it when the app launches.
- Runtime/model-store uninstall is not implemented.
- Import adopts only endpoint, mode, and selected-model configuration. It never
  copies model weights or adopts an external model directory.
- Remote daemons can be connected to, but Buzz never manages their process.
