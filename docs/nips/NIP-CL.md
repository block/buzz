NIP-CL
======

Channel Labels
--------------

`draft` `optional` `relay`

## Abstract

This specification defines bounded, shared string labels for Buzz stream and forum channels. Clients create initial labels with kind `9007`, add or remove labels with kind `9002`, and read the authoritative label set from relay-signed kind `39000` channel metadata.

This NIP also defines atomic application and retry behavior for **all kind `9007` creation commands that include `h`, including unlabeled creation**. It does not change the contract for unlabeled creation without `h`.

These tags extend [NIP-29](https://github.com/nostr-protocol/nips/blob/master/29.md); they do not claim standard NIP-29 label semantics. They deliberately use `label`, not [NIP-32](https://github.com/nostr-protocol/nips/blob/master/32.md) `l`/`L` tags: this is a shared channel-metadata set, without NIP-32 namespace semantics or a `#l` query contract.

The key words MUST, MUST NOT, SHOULD, SHOULD NOT and MAY are to be interpreted as normative requirements.

## Scope

Labels are channel metadata, not per-user organization. They MUST NOT change channel identity, history, Canvas, membership or permissions. Label values and namespace-like prefixes confer no authority. This specification reserves no values or prefixes and assigns no feature-specific behavior to them.

Version 1 applies only to stream and forum channels. Relays MUST reject initial labels on direct-message channels and label mutations targeting direct-message channels.

Archived channels retain readable labels but MUST reject new label mutations. A client must unarchive the channel separately before submitting a new mutation. Deleted channels MUST NOT accept label mutations or receive new metadata snapshots. A deleted channel UUID MUST NOT be reused for creation.

Replace-all operations, relay-wide label search and consumer interfaces are outside this specification.

## Capability discovery

A supporting relay MUST advertise `nip-cl` in its NIP-11 `supported_extensions` string array, not in `supported_nips`:

```json
{
  "supported_extensions": ["nip-cl"]
}
```

Clients MUST check this capability before submitting labeled creation or label mutation commands, and before relying on this NIP's acknowledgement guarantees for unlabeled `h`-tagged creation. Relays MUST independently enforce whether these operations are enabled. Advertisement alone is not an enforcement mechanism.

A relay MUST NOT advertise support until all serving command and metadata writers preserve the guarantees in this specification.

## Label values and limits

A label value MUST be a nonempty ASCII string matching the complete pattern:

```text
^[a-z0-9][a-z0-9._:/-]*$
```

Each value MUST be at most 64 bytes. Relays MUST reject noncanonical values rather than trimming, lowercasing, normalizing or otherwise rewriting them. Clients MUST preserve valid values they do not recognize.

The following limits apply:

| Limit | Maximum |
|---|---:|
| Labels stored on one channel | 32 |
| Bytes in one label value | 64 |
| Raw label tags in one creation or mutation command, before deduplication | 64 |
| Bytes in the compact-JSON array of snapshot label tags | 4,096 |

For creation, the raw count covers `label` tags. For mutation, it covers `add-label` and `remove-label` tags combined. Every label or label-operation tag MUST contain exactly two elements: its tag name and its value.

The snapshot-array limit applies to the array containing only the `label` tags, not to the whole snapshot. Existing whole-event and transport-frame limits also apply. A relay MUST reject a command whose resulting snapshot would exceed an applicable limit before applying the command.

## Events

| Purpose | Kind | Channel identifier | Label tags |
|---|---:|---|---|
| Create a channel with initial labels | `9007` | `h` | `label` |
| Add or remove labels | `9002` | `h` | `add-label`, `remove-label` |
| Read authoritative channel metadata | `39000` | `d` | `label` |

Creation and mutation commands are signed by the requesting author. Metadata snapshots are signed by the relay. Existing envelope, authentication and channel-creation requirements continue to apply.

An `h`-tagged creation or label mutation command MUST contain exactly one `h` tag with exactly two elements: `h` and a valid, non-nil channel UUID. Duplicate or malformed channel identifiers, and missing identifiers where required, MUST be rejected before command application. Labeled creation without `h` MUST be rejected.

The examples below are event fragments. Normal event fields, signatures and any required authentication tags are omitted.

### Initial labels: kind 9007

Initial labels are expressed as repeated `label` tags alongside the ordinary creation fields:

```json
{
  "kind": 9007,
  "tags": [
    ["h", "42fdbd94-13e6-4f82-b065-5812e4a02a8b"],
    ["channel_type", "stream"],
    ["name", "Build Systems"],
    ["label", "team:infra"],
    ["label", "workflow/build"]
  ],
  "content": ""
}
```

Repeated values MUST be deduplicated in the applied set, without rewriting the signed command. Omitting initial labels creates an empty label set. Initial fields, labels, owner membership and the authoritative metadata snapshot MUST be committed together for the `h`-tagged creation path, whether or not labels are present. Rejection MUST leave no newly created channel or owner membership.

After the retry admission checks below, an exact retry of a committed creation command MUST be recognized before treating an existing channel as a creation conflict. A different creation event targeting an existing or deleted UUID MUST be rejected as a conflict. It MUST NOT disclose the channel's metadata.

### Add and remove labels: kind 9002

A label mutation MUST contain at least one `add-label` or `remove-label` tag:

```json
{
  "kind": 9002,
  "tags": [
    ["h", "42fdbd94-13e6-4f82-b065-5812e4a02a8b"],
    ["add-label", "status:active"],
    ["remove-label", "status:queued"]
  ],
  "content": ""
}
```

A label mutation MUST be label-only. It MUST NOT also change name, about, visibility, archive state, topic, purpose or TTL. Required envelope and authentication tags remain permitted. Malformed or unknown-tag handling MUST NOT allow a label operation to bypass privileged metadata authorization.

Let `L` be the current label set, `A` the distinct values of all `add-label` tags, and `R` the distinct values of all `remove-label` tags. The relay MUST reject the command if `A` and `R` intersect. Otherwise, the resulting set is:

```text
L' = (L ∪ A) ∖ R
```

Adding an existing value or removing an absent value succeeds as a no-op, subject to the same validation and authorization as a state-changing command. A new no-op command MUST record committed application but reuse the existing authoritative snapshot without changing its ID or timestamp. It MUST NOT publish a new snapshot solely for the no-op. A missing or stale snapshot MUST be repaired under the publication guarantees below before acknowledging success.

The resulting set, not the intermediate order of tags, determines the stored-label limit. Operations MUST preserve labels not named by the command. Ordinary metadata commands that omit label operations MUST preserve the current label set. `label` tags on kind `9002` MUST NOT be interpreted as replacement or mutation operations.

### Metadata snapshot: kind 39000

The relay publishes the complete current label set as `label` tags on the channel metadata snapshot:

```json
{
  "kind": 39000,
  "tags": [
    ["d", "42fdbd94-13e6-4f82-b065-5812e4a02a8b"],
    ["t", "stream"],
    ["name", "Build Systems"],
    ["label", "status:active"],
    ["label", "team:infra"],
    ["label", "workflow/build"]
  ],
  "content": ""
}
```

Label tags MUST be deduplicated and sorted lexicographically by ASCII value. No `label` tags means an empty label set. The existing `channel_type` creation tag and `t` metadata tag retain their existing meanings; labels do not replace channel type.

Clients MUST read labels from authorized, relay-signed channel metadata, not infer authoritative state from stored command events. Before accepting a snapshot, a client MUST verify its NIP-01 event ID and signature, and require its signer to match the relay identity obtained from the configured relay's authenticated NIP-11 `self` field or an explicitly trusted out-of-band identity. A valid signature from an arbitrary key is insufficient. The snapshot MUST identify the expected channel with exactly one two-element `d` tag and contain a complete canonical label set within the limits above; otherwise the client MUST discard it, not interpret it as an empty set.

Clients MUST scope cached snapshots to the community and relay identity, and select replacements using NIP-01 ordering: greatest `created_at`, then lowest lexicographic event ID on a tie. An older snapshot arriving later MUST NOT roll labels back.

Existing ACL-filtered discovery and exact channel reads include labels. This extension does not define a `#label` query filter or guarantee a global live metadata feed. Filtering a locally loaded catalog does not establish relay-wide completeness.

Label commands and metadata snapshots are state events, not conversation messages. Receiving them MUST NOT increase chat or reply counts, add chat unreads, mark a conversation read, or generate mention or push notifications from their tags.

## Authorization and visibility

Creation and label-only mutation require `channels:write` and the existing identity and community admission checks.

New label mutations require channel owner/admin authority or the existing exception for the owning human of an active owner agent. Under that exception, the human need not independently be a channel member. Ordinary membership alone is insufficient. Authorization and channel lifecycle state MUST be checked against the state with which the command is serialized, not against an earlier cached role or channel read.

Any authorized label writer can add or remove any valid value; a prefix does not give its creator exclusive control. An agent that labels a channel therefore needs the same authority as any other label writer.

Labels have exactly the same visibility as channel metadata. Open-channel labels are visible to community members permitted to discover that channel. Private-channel labels remain subject to the channel ACL. Labels MUST NOT weaken tenant isolation or bypass discovery restrictions.

Label state, application evidence, serialization, caches and delivery MUST be scoped to the community resolved from the connection's host, as described in [the multi-tenant relay contract](../multi-tenant-relay.md). The same signed event submitted in another community MUST NOT recover evidence from the first community.

## Application and publication guarantees

A successful acknowledgement for a new command means committed application, not merely storage of the signed request.

For `h`-tagged creation and state-changing label mutation, the relay MUST atomically commit the signed command, its application evidence, channel changes and authoritative signed `39000` snapshot. If application fails, none of those changes may commit. A new no-op command commits the signed command and its application evidence while preserving the authoritative snapshot as specified above. This guarantee does not require atomic publication of kinds `39001` and `39002` or successful delivery of every live notification.

A transactionally stored command MAY itself serve as application evidence if its provenance and retention satisfy this NIP. No separate receipt table is required. Mere presence of a stored command without proof of atomic application MUST NOT be treated as success or permission to replay its effects.

Concurrent mutations MUST serialize against authoritative channel state. Independent additions or removals MUST preserve unrelated values. State limits, authorization, archive state and deletion MUST be checked within the same serialization boundary.

Every metadata publisher, including ordinary metadata updates and repair, MUST preserve the canonical labels. A delayed publisher MUST NOT restore stale labels, including after the final label has been removed. A newly published replacement MUST use freshly read state and a timestamp greater than the previous snapshot timestamp; it MUST NOT give stale captured tags a newer timestamp.

Deletion and metadata publication MUST serialize so that a deleted channel cannot regain a metadata snapshot. Repair MUST compare label content, not merely snapshot existence, and MUST NOT republish missing or deleted channels.

A successful command proves its application at a point in time. A later authorized command may supersede its effects.

## Acknowledgements, retries and recovery

An exact retry resubmits the same signed event, without changing its tags, timestamp, ID or signature. This NIP introduces no separate receipt event kind or HTTP API.

### Retry admission

Version 1 does not provide historical receipt recovery that bypasses freshness or current access checks. Every submission, including an exact retry, MUST pass signature, authenticated-author, transport, community, token-scope and the existing ±900-second timestamp checks before a positive acknowledgement. A positive retry acknowledgement also requires the channel to exist, be unarchived and undeleted, and the author to currently satisfy the label-mutation authority rules above. This applies to retries of creation commands as well as mutations. Channel authority and lifecycle checks MUST share the command's serialization boundary.

After those checks, a retained committed command MUST succeed without reapplication or republication, even if a later command has superseded its effects. The relay MUST perform duplicate detection before a new creation's UUID-conflict check and repeat it within the serialization boundary before any new application.

Failure of current admission MUST reject this submission without applying anything. It MUST NOT turn an earlier committed command into an unapplied command. Opposing edits, archive, channel deletion, channel-role revocation and soft deletion of the command event MUST NOT make the command eligible for reapplication. An expired command MUST NOT be newly applied.

### Responses and client outcomes

For commands covered by this NIP received as a parseable `EVENT` with an event ID, relays MUST return a NIP-01 `OK` response using the following outcomes. Prefixes in this table are fixed, case-sensitive strings; human-readable details MAY follow them after a space. Clients MUST NOT depend on those details.

| Relay result | `OK` accepted | Message or required prefix | Meaning |
| --- | --- | --- | --- |
| New application committed, including a no-op | `true` | Empty string | This exact command committed. |
| Retained committed application, after retry admission | `true` | `duplicate: nip-cl-committed` | This exact command committed earlier; nothing was reapplied or republished. |
| Submission rejected before application | `false` | `invalid: nip-cl-rejected`, `auth-required: nip-cl-rejected`, `restricted: nip-cl-rejected`, `duplicate: nip-cl-rejected`, `rate-limited: nip-cl-rejected` or `error: nip-cl-rejected`, according to cause | This submission did not apply. It says nothing about an earlier uncertain attempt. |
| Application outcome cannot be established | `false` | `error: nip-cl-unknown` | The relay cannot prove commit or rejection. The original outcome remains unknown. |

Malformed commands and timestamp admission failures use `invalid:`, authentication failures use `auth-required:`, permission or lifecycle denials use `restricted:`, and a different creation event targeting a used UUID uses `duplicate:`. A failure known to have rolled back MAY use `error: nip-cl-rejected`; an uncertain commit MUST use `error: nip-cl-unknown`. A stored command without proof of application MUST produce the unknown outcome if admission permits examining it; it MUST NOT be applied again or inferred successful from current labels. Errors MUST NOT reveal inaccessible channel metadata or evidence from another community.

Examples (event IDs abbreviated):

```json
["OK", "<event-id>", true, ""]
```

```json
["OK", "<event-id>", true, "duplicate: nip-cl-committed"]
```

```json
["OK", "<event-id>", false, "invalid: nip-cl-rejected timestamp outside admission window"]
```

```json
["OK", "<event-id>", false, "error: nip-cl-unknown application evidence unavailable"]
```

Clients MUST persist and reuse the same signed event while delivery is uncertain. They MUST distinguish these states:

| Observation | Client outcome |
| --- | --- |
| Positive `OK` for the exact event ID from the target relay | **Committed**, even if an earlier attempt was uncertain. |
| Explicit `nip-cl-rejected` response, with no earlier uncertain or concurrent attempt for that ID | **Rejected new command**. |
| `nip-cl-unknown`, timeout, disconnect or an unrecognized response | **Unknown** unless committed application is already known. |
| Later rejection, expiry, missing evidence or failed/inaccessible read after an uncertain attempt | **Unknown**; none proves that the earlier attempt did not commit. |

A later negative response MUST NOT downgrade known committed application. Clients MUST NOT automatically generate a new event ID to resolve an uncertain outcome. Current-label readback is an independent, ACL-filtered observation of current state, not receipt evidence.

### Retention

Application evidence MUST survive soft deletion of the command event and MUST be retained until the signed command can no longer pass new-application admission. Physical retention or community deletion MAY remove evidence after that point. Community deletion MUST fence subsequent writes. Without retained evidence, an expired command MUST NOT apply and an uncertain original outcome remains unknown. Indefinite receipt recovery is not promised.

## Compatibility and security considerations

Clients that do not use labels can continue ordinary channel operations. Such operations MUST preserve existing labels. Clients MAY ignore label presentation without affecting navigation or channel identity.

Relays MUST enforce the canonical value, count, size, command-shape and authorization rules before committing an operation. A label prefix is not a permission, ownership claim or trusted application identity.

Implementations MUST keep every metadata-writing path label-preserving while labels exist. Disabling new label writes is not sufficient to make an incompatible metadata writer safe.

Labels are shared metadata and are not suitable for secrets or private per-user notes. Exact retries remain subject to current admission; they do not restore access. Current channel metadata remains protected by its current ACL.
