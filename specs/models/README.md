# Domain Model Glossary

| Term | Meaning |
| --- | --- |
| Local relay | A single-process, one-community Buzz node intended for local experimentation. |
| Signed event | A Nostr event whose ID and Schnorr signature verify. |
| Durable event | A signed event outside the NIP-01 ephemeral kind range. |
| Effective event | The event currently visible after replaceable-event rules are applied. |
| Event log | An append-only newline-delimited JSON record of accepted durable events. |
| Subscription | A client-selected set of Nostr filters receiving historical and live matching events. |
| Stable node | A node whose acknowledged durable history survives restart and remains attributable. |
| Portable relay core | Deterministic verification, classification, reduction, and filter behavior shared by relay adapters. |
| Port | A required relay effect whose implementation varies by runtime. |
| Adapter | A runtime-specific implementation of transport, storage, subscription, policy, or effects. |
| Event journal | The source of accepted durable history, independent of its storage representation. |
| Relay decision | The normative result of submission: stored, duplicate, superseded, ephemeral, or rejected. |
| Conformance profile | A named set of observable relay guarantees implemented and tested by an adapter. |
| Replication source | A durable-history port that exports exact signed events in journal order. |
| Replication cursor | An opaque position interpreted only by the source stream that issued it. |
| Replication sink | A destination port that independently applies source policy, verification, and normal ingest. |
| Checkpoint-safe receipt | A terminal destination outcome after which an orchestrator may persist the source cursor. |
| Event author | The Nostr public key whose signature covers an event's exact envelope. |
| Principal | A person, agent, relay node, or system identity recognized by local security policy. |
| Authenticated principal | An ephemeral, audience-bound result proving current control of an authorized verification method. |
| Append context | The declared origin of an append: direct, replication, or system. |
| Relay node principal | A stable node identifier, potentially a DID, whose active verification keys may rotate. |
| Peer binding | Destination-controlled configuration binding one replication source to an authenticated relay node principal. |
| Delegation | A cryptographically verified, scoped grant allowing a principal to act under explicitly stated conditions. |
| Read authorization | Request-level and per-event policy applied consistently to query, count, historical, and live delivery. |
