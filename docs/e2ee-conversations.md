# Buzz End-to-End Encrypted Conversations

`draft` `specification`

## Abstract

This document specifies Buzz-native end-to-end encrypted conversations as an optional relay/community capability. Encrypted conversations use Buzz's existing Nostr event log, channel membership model, and community boundary, while moving message plaintext and conversation keys out of relay custody.

The primary cryptographic protocol is **Messaging Layer Security (MLS)** for Buzz-native DMs and channels. NIP-17/NIP-59/NIP-44 gift-wrapped direct messages remain the Nostr interoperability path and are not replaced by this specification.

## Status

Draft. This is the target shape to iterate toward; exact kind numbers, wire payload schemas, and storage tables may change as implementation hardens.

## Design Goals

1. **Content confidentiality from the relay.** The relay stores and forwards ciphertext for encrypted conversations, but cannot decrypt message bodies.
2. **Community-scoped optionality.** A community may disable, allow, or require encrypted conversations. In single-community deployments this is relay configuration; in multi-community deployments it is community configuration resolved from the request host.
3. **Uniform encrypted conversation model.** Buzz-native 1:1 DMs, group DMs, and private channels use the same MLS-based model.
4. **Preserve Buzz's relay value.** The relay continues to authenticate, authorize, store, fan out, audit, and enforce visible membership policy.
5. **Agents are explicit cryptographic members.** An agent that can read an encrypted conversation is admitted as a member with key material and a revocation path.
6. **No silent weakening.** When a feature depends on plaintext server access — search, workflows, summaries, media inspection — encrypted conversations must degrade explicitly or move execution to an admitted decrypting client/agent.
7. **No false retroactive revocation.** Removing a member prevents future access after an MLS epoch change. It does not erase plaintext already delivered to that member.

## Non-Goals

- Hiding all metadata from the relay in v1. Channel id, event kind, author pubkey, timing, approximate size, and any public tags remain relay-visible.
- Replacing NIP-17 private DM interoperability.
- Server-side plaintext search over encrypted bodies.
- Cryptographically erasing messages already delivered to a removed member.
- Encrypting huddle media frames. MLS may later distribute media keys, but message E2EE and real-time audio E2EE are distinct protocols.
- Defining a global cross-community device identity system. This spec treats device/key material as community-scoped by default, but the wire model reserves an explicit credential-binding extension point so a later global device registry can be introduced without changing encrypted-conversation semantics.

## Terms

| Term | Meaning |
|---|---|
| **Community** | Buzz tenant/security boundary selected by request host. In single-community mode, the relay's one implicit community. |
| **Conversation** | A Buzz DM or channel with message history. |
| **Encrypted conversation** | A conversation whose message application payloads are encrypted with MLS. |
| **MLS group** | The MLS cryptographic group corresponding to one encrypted Buzz conversation. |
| **MLS client** | A cryptographic participant in an MLS group. In Buzz this is usually a device or agent instance, not merely a user pubkey. |
| **User identity** | A Nostr pubkey used for Buzz authentication and visible authorship. |
| **Device credential** | A per-device or per-agent credential used as an MLS leaf identity and bound to a user identity by Buzz client policy. The binding is versioned so it can be community-local in v1 or point at a future global device registry. |
| **Epoch** | MLS group state version. Membership changes advance the epoch. |
| **Welcome** | MLS message that gives a newly added client access to the current epoch. |
| **KeyPackage** | MLS pre-published join material used to add an MLS client asynchronously. |

## Policy Model

Each community has an E2EE policy:

```text
disabled | optional | required
```

### `disabled`

- The relay MUST reject Buzz-native encrypted conversation control and application events.
- The relay MUST NOT advertise Buzz-native MLS support for that community.
- Clients SHOULD hide encrypted conversation creation and encrypted send affordances.
- Existing NIP-17 gift-wrap support MAY remain available if the relay separately supports NIP-17.

### `optional`

- The relay MUST accept valid Buzz-native encrypted conversation events.
- Plaintext and encrypted conversations MAY coexist in the same community.
- Clients SHOULD let users choose encryption at conversation creation.

### `required`

- The relay MUST require encrypted operation for newly created DMs and private channels.
- The relay MUST reject plaintext channel-message events into encrypted conversations.
- The relay SHOULD reject creation of new plaintext DMs and private channels, while allowing public plaintext channels if the community policy defines public rooms as out of scope.

The initial single-community configuration surface SHOULD be an environment variable such as:

```text
BUZZ_E2EE_POLICY=disabled|optional|required
```

In multi-community mode the equivalent value belongs to the host-resolved community record. A client-supplied tag MUST NOT select or override E2EE policy.

## Discovery

A relay/community that supports Buzz-native encrypted conversations MUST advertise a Buzz extension identifier through NIP-11, for example:

```json
{
  "supported_extensions": ["buzz-e2ee-mls-v1"]
}
```

A relay/community whose policy is `disabled` MUST NOT advertise `buzz-e2ee-mls-v1`.

In a multi-community deployment, NIP-11 responses MUST be derived from the request host's resolved community or relay-static configuration only. They MUST NOT incorporate state from any other community.

## Relationship to Existing Buzz Protocol

Buzz already uses Nostr events as the event log and `h` tags as the channel/group routing key. This specification preserves that shape:

- encrypted application messages are still signed Nostr events;
- encrypted channel-scoped events still carry `h=<channel_id>`;
- relay membership, channel membership, token scoping, archival checks, and fan-out remain relay-enforced;
- encrypted content is opaque to the relay and MUST NOT be indexed as plaintext.

NIP-17 gift wraps (`kind:1059`) remain the standard Nostr-private DM mechanism. Buzz-native MLS events are a separate encrypted-conversation protocol optimized for Buzz DMs, channels, agents, and community policy.

## Channel and Conversation State

An encrypted conversation has a server-visible encryption mode:

```text
plaintext | mls_v1
```

The encryption mode is immutable after conversation creation unless a future migration spec defines a safe transition. A client MUST NOT silently downgrade an encrypted conversation to plaintext.

For DMs and private channels in a `required` community, new conversations MUST use `mls_v1`.

## MLS Identity Model

Each encrypted conversation maps to one MLS group.

Buzz SHOULD model each device and each agent runtime as a distinct MLS client/leaf. A user with three devices therefore participates through three MLS leaves, all associated with the same visible Buzz user identity. An agent that reads a room participates through its own MLS leaf, associated with the agent's Buzz identity and, where applicable, owner attestation.

A device credential MUST carry a versioned binding to the visible Buzz user or agent identity. The v1 binding is community-scoped: the same Nostr pubkey may join multiple communities, but KeyPackages, group membership, and MLS state are community-local.

The binding format MUST leave room for a later global device registry. A future spec may define a portable device subject, such as a globally signed device record, but encrypted-conversation events MUST continue to resolve authorization through the host-derived community. In other words: a global device registry may help identify or revoke a device across communities, but it must not become a way to select a community or bypass community membership.

## KeyPackage Publication

A client that wants to be addable to encrypted conversations publishes one or more community-scoped KeyPackages.

A KeyPackage publication event MUST be:

- signed by, or otherwise verifiably bound to, the user/agent identity it represents;
- scoped to the community selected by the relay URL/host;
- associated with a versioned device credential binding;
- replaceable or otherwise expirable;
- consumed at most once by a successful add operation;
- excluded from plaintext search indexing.

The relay MAY materialize KeyPackage metadata for lookup and consumption, but it MUST NOT require access to private key material.

## MLS Group Lifecycle Events

Buzz-native MLS requires event types for:

1. group initialization / descriptor;
2. KeyPackage publication;
3. MLS proposals;
4. MLS commits;
5. MLS welcomes;
6. encrypted application messages.

Candidate kind allocation is the `42xxx` range:

| Candidate kind | Purpose |
|---|---|
| `42000` | MLS group descriptor / latest public group state pointer. |
| `42001` | MLS KeyPackage publication. |
| `42002` | MLS proposal. |
| `42003` | MLS commit. |
| `42004` | MLS welcome, p-gated to one recipient client/user. |
| `42010` | MLS encrypted application message. |
| `42011` | MLS encrypted application control message, such as encrypted edit/delete metadata, if not represented inside `42010`. |

Exact kind numbers are provisional. Final allocation MUST be added to `buzz-core/src/kind.rs` and documented in `NOSTR.md` or a dedicated NIP-style document.

## Public Envelope Requirements

All channel-scoped encrypted events MUST include:

- `h=<channel_id>`;
- a valid Nostr signature;
- a kind allowed by the community E2EE policy;
- content whose size fits relay frame/content limits;
- enough public metadata for the relay to enforce channel membership and fan-out.

Encrypted application events MAY include public `e` tags for thread/reply/reaction targets. If present, the relay MAY validate that referenced channel-scoped target events belong to the same channel.

Encrypted application events MAY include public `p` tags for notification routing. Public `p` tags leak mention/recipient metadata and are a product choice. Clients that require hidden mentions MUST place mention data only inside the encrypted payload and accept reduced relay-side notification quality.

## Relay Validation

For Buzz-native encrypted conversation events, the relay MUST validate:

- event signature and timestamp under normal Buzz ingest rules;
- authenticated publisher and scope;
- community policy (`disabled`, `optional`, `required`);
- channel existence, membership, token-channel restrictions, and archived/deleted state;
- required `h` tag for channel-scoped encrypted events;
- p-gated read/write rules for welcome or recipient-specific key material;
- syntactic envelope shape for encrypted kinds;
- that plaintext message kinds are not accepted into `mls_v1` conversations.

The relay MUST NOT decrypt MLS application payloads.

The relay MUST exclude MLS ciphertext-bearing events from plaintext full-text indexing. It SHOULD also exclude them from workflow triggers that assume plaintext content.

## Read Authorization

Historical and live reads of encrypted channel-scoped events use the same channel-membership authorization model as plaintext channel events. A member may receive ciphertext even if its local device cannot decrypt a particular epoch; decryption failure is a client state problem, not a relay authorization failure.

Recipient-specific global events, such as MLS welcomes, MUST be p-gated like existing gift wraps and membership notifications. A subscription that can match p-gated encrypted key material MUST include `#p` values matching the authenticated pubkey, unless the event kind has a separately justified ids-only exemption.

## Membership Changes and Revocation

Buzz channel membership and MLS membership must move together.

When a member is added to an encrypted conversation:

1. the relay-visible channel membership change is authorized under normal Buzz rules;
2. an MLS add/commit sequence admits one or more device/agent leaves for that member;
3. welcome material is delivered only to the added client(s);
4. the new member can decrypt from the admitted epoch forward.

When a member is removed:

1. the relay-visible channel membership removal is authorized under normal Buzz rules;
2. an MLS remove/commit sequence advances the epoch;
3. removed leaves do not receive future epoch secrets;
4. the relay stops authorizing future reads/writes according to channel membership policy.

Removing a member does not cryptographically erase plaintext already delivered to that member. Clients MAY implement best-effort delete-for-everyone or disappearing-message UX, but that is not a cryptographic revocation guarantee.

## History Semantics

New members MUST NOT receive old epoch secrets by default. Encrypted history sharing, if supported, MUST be an explicit action by existing authorized members and SHOULD be represented as auditable encrypted export/re-share events.

A client opening an encrypted conversation may encounter:

- ciphertext from epochs it can decrypt;
- ciphertext from earlier epochs it cannot decrypt;
- missing local MLS state;
- stale KeyPackages;
- concurrent commits that require recovery.

Clients MUST present undecryptable history honestly rather than falling back to server plaintext.

## Search, Feed, and Notifications

Encrypted message bodies MUST NOT be sent to server-side plaintext search.

Feed and notification behavior depends on public envelope metadata:

- public `p` tags can preserve mention notifications while leaking mention metadata;
- hidden mentions protect metadata but require client-side or agent-side notification logic after decryption;
- channel activity feeds can continue to show encrypted envelope events without body previews.

Search over encrypted bodies is a client-side or admitted-agent feature unless a future encrypted-search design is specified.

## Workflows

Relay-side workflows MUST NOT inspect encrypted message bodies.

A workflow MAY trigger on public envelope metadata, such as channel, kind, author, reaction target, or explicit public tags. A workflow that needs plaintext content MUST run through an admitted cryptographic member, typically a client-side workflow runner or an agent that is visibly present in the encrypted conversation.

## Agents

An agent that reads or acts on encrypted message content MUST be an MLS member of the conversation.

The UI and APIs SHOULD distinguish:

- agents that are channel members but cannot decrypt the encrypted conversation;
- agents that are cryptographic members and can read plaintext;
- hosted agents whose runtime is outside the user's local device boundary.

Removing an agent from an encrypted conversation MUST trigger the same future-secrecy behavior as removing a human/device leaf.

## Media and Attachments

Message E2EE does not automatically encrypt media blobs.

For encrypted conversations, clients SHOULD encrypt attachment bytes or attachment content keys before upload. The encrypted message payload SHOULD carry the attachment decryption key and authenticated metadata. Blossom/S3 object hashes, sizes, MIME types, and URLs may remain relay-visible unless a future media privacy spec hides them.

The relay SHOULD NOT inspect or thumbnail encrypted attachment plaintext unless an admitted decrypting agent/client explicitly performs that work and republishes derived artifacts under the conversation's policy.

## Audit

The relay audit log records envelope and control-plane facts, not plaintext message bodies.

Auditable facts include:

- encrypted event acceptance;
- channel encryption mode;
- membership changes;
- MLS proposal/commit/welcome envelope ids;
- policy changes;
- rejected plaintext downgrades.

Audit entries MUST NOT include decrypted message content.

## Multi-Community Requirements

In multi-community deployments:

- E2EE policy is resolved from the request host's community.
- Encrypted conversation rows, materialized MLS state, KeyPackages, audit entries, search exclusions, and workflow decisions are community-scoped.
- Device credentials are community-scoped in v1, but their binding format may reference a future global device subject.
- A global device subject, if later defined, MUST NOT authorize access by itself; community membership and host-derived policy remain authoritative.
- A channel id in one community MUST NOT authorize an encrypted event in another community.
- A KeyPackage published in one community MUST NOT be consumed by another unless a future cross-community identity spec explicitly permits it and the host-derived community still authorizes the operation.
- NIP-11 advertisement for E2EE MUST reflect the host-resolved community.

This follows the existing multi-community rule: URL/host selects the community; client-supplied tags do not.

## Compatibility

Plaintext Buzz clients that do not implement `buzz-e2ee-mls-v1` will see unknown encrypted kinds or placeholder envelopes. They MUST NOT render ciphertext as plaintext chat content. Encrypted-capable clients SHOULD display an upgrade/unsupported notice when entering encrypted conversations from unsupported clients.

NIP-17 clients interoperate through NIP-17 gift wraps, not through Buzz-native MLS channels.

## Security Considerations

- **Relay compromise:** A compromised relay can deny service, withhold events, reorder delivery within protocol limits, or leak metadata it sees. It should not learn encrypted message plaintext without compromising clients/agents.
- **Client compromise:** A compromised member device can read messages available to that device and can exfiltrate plaintext. MLS future secrecy limits damage after device update/removal, but cannot undo exfiltration.
- **Agent runtime trust:** An admitted agent can read plaintext. Hosted agents shift trust to the agent runtime even when the relay cannot decrypt.
- **Metadata leakage:** Public tags, timing, event sizes, channel ids, authorship, and membership operations remain visible to the relay in v1.
- **Downgrade:** Clients must not silently send plaintext into encrypted conversations. The relay must reject plaintext message kinds for `mls_v1` conversations.
- **State loss:** Loss of MLS state can make history undecryptable. Recovery and backup require explicit design; server plaintext fallback is forbidden.

## Open Questions

1. What is the final device credential binding format, and how does v1 community-scoped binding leave room for a future global device subject?
2. Are encrypted public channels supported, or only DMs/private channels?
3. Does `required` apply to all conversations or only DMs/private channels?
4. Which metadata is public in v1: mentions, reactions, thread graph, attachment metadata?
5. How are concurrent MLS commits serialized or recovered in Buzz's event log?
6. What is the first supported key backup/recovery mechanism?
7. How should NIP-17 and Buzz-native MLS coexist in the desktop/mobile DM UI?
