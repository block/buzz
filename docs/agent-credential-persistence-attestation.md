# Agent Credential Persistence Attestation (v1)

Schema id: `buzz.desktop.exact_agent_credential_persistence.v1`

External controllers that assign work to a named Buzz Desktop agent often
need to prove — without any access to key material — that the agent's
credential is durably held by the OS keyring, bound to exactly that agent,
and not sitting in the inline `0o600` JSON fallback. This document defines
the public attestation object Buzz Desktop can issue for one managed agent.

## Invocation

Tauri command (desktop IPC):

```ts
import { getAgentPersistenceAttestation } from "@/shared/api/agentAttestation";

const attestation = await getAgentPersistenceAttestation(agentPubkey);
```

The command is strictly read-only: it observes the raw persisted agent store
and probes the keyring through the side-effect-free `load_all_readonly` path.
It never migrates keys, never writes, and no code path carries the nsec.

## Object shape

```json
{
  "schema_version": "buzz.desktop.exact_agent_credential_persistence.v1",
  "agent_pubkey": "<hex64>",
  "persistence_backend": "os_keyring",
  "inline_fallback": false,
  "parallelism": 1,
  "public_identity_hash": "<hex64>",
  "attestation_hash": "<hex64>",
  "stock_release_id": "buzz-desktop@0.5.7",
  "issued_at": "2026-08-08T12:00:00Z"
}
```

| Field | Meaning |
|---|---|
| `schema_version` | Exactly the schema id above. |
| `agent_pubkey` | The managed agent's identity pubkey (hex). |
| `persistence_backend` | `os_keyring` when the credential is in the OS keyring and absent from the JSON store; `inline_file` when it is serialized in the `0o600` fallback file. Consumers must treat unknown values as "not the backend I require" — future backends may be added. |
| `inline_fallback` | Explicit boolean mirror of `persistence_backend != os_keyring` (v1). |
| `parallelism` | The record's requested parallelism. Controllers requiring exact-agent binding gate on `1`. |
| `public_identity_hash` | SHA-256 (hex) of `agent_pubkey + "\n" + auth_tag`, where `auth_tag` is the agent's public NIP-OA auth-tag JSON, or the empty string for agents that predate NIP-OA. |
| `attestation_hash` | SHA-256 (hex) of this object serialized (serde struct-field order) with `attestation_hash` set to the empty string. |
| `stock_release_id` | `"<package-name>@<version>"` of the issuing desktop build. |
| `issued_at` | RFC 3339 issuance time. |

## Fail-closed semantics

The command errors — it never guesses — when persistence cannot be proven:

- `attestation_keyring_unreachable`: a keyring backend exists but was
  unreachable this boot and no inline key is present. The credential may
  exist; presence cannot be proven either way.
- `attestation_credential_missing`: no inline key, keyring reachable, no
  entry for this agent.

Builds compiled without the `system-keyring` feature keep agent keys inline
and attest `inline_file` / `inline_fallback: true` honestly.

## What this is not

- Not a trust or capability attestation (see NIP-TR discussions for that
  layer); this only describes *where the credential lives*.
- Not a replacement for NIP-OA: the auth tag remains the ownership proof.
  This object only hashes it as public identity material.
- Not a secret channel: no field ever contains key material, and the
  implementation's builder cannot receive the nsec by construction.
