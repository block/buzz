# NIP-SW: Owner section-workspace import

## Status

Stage 1 is an opt-in migration tracer bullet. Set `BUZZ_SECTION_WORKSPACE_IMPORT_ENABLED=true` to accept imports. No client enables it yet. Viewer grants, revocation, moves, and section management are later protocol stages.

## Kinds and authority

- `9050`: owner-signed revision-zero import command.
- `30623`: relay-signed, reader-specific current projection.

The relay resolves the community from the request host. It never accepts a community UUID from the command. The command signer is the immutable workspace owner. Import requires `users:write`, is accepted at most once for `(community, owner)`, and references an existing owner-authored kind-30078 event in that community. Exact signed-event replay is idempotent; every different later import conflicts.

## Import

Kind 9050 has exactly these tags:

```json
[["p", "<owner-pubkey>"], ["action", "<action-uuid>"]]
```

The `p` value must equal the signer. Content is ordinary strict JSON (the signed Nostr event ID already binds its exact bytes):

```json
{
  "version": 1,
  "action_id": "<uuid>",
  "source_event_id": "<kind-30078-event-id>",
  "source_hash": "<sha256-lower-hex-of-decrypted-legacy-state>",
  "key_epoch": 1,
  "owner_key_envelope": "<nip44-wrapped-workspace-key>",
  "sections": [{"id":"<uuid>","rank":0,"encrypted_label":"<ciphertext>","encrypted_icon":null}],
  "assignments": [{"channel_id":"<uuid>","section_id":"<uuid>"}]
}
```

The relay rejects unknown fields, unsupported versions, nil or duplicate IDs, non-permutation ranks, duplicate channel assignments, unknown sections, missing/cross-community/deleted channels, a missing/cross-community/wrong-author legacy source, more than 100 sections or 1,000 assignments, metadata over 65,535 bytes, and key envelopes over 4,096 bytes.

Labels and icons remain client-encrypted. Stage 1 freezes neither a custom JSON canonicalization profile nor metadata encryption details; supported clients must agree on those before cutover.

## Owner projection

On the first accepted import, the relay atomically stores the verified command and a relay-signed kind-30623 projection at:

```text
d=<owner-pubkey>:<reader-pubkey>
p=<reader-pubkey>
```

There is exactly one `p` tag and it matches the reader component of `d`. The owner projection therefore uses `<owner>:<owner>`. Its content contains version 1, owner pubkey, workspace/layout/assignment revision 1, key epoch 1, the source migration marker, that reader's key envelope, sections, and assignments.

Kind 30623 is relay-only and parameterized replaceable. Every historical REQ, HTTP query, count, search hydration, known-event-ID lookup, and live fan-out applies result-level `p` authorization. Knowing an event ID is not authorization.
