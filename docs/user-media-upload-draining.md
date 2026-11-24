# Check media uploads before deleting a user

`wait_for_draining` checks the primary database once. It fails immediately if
the user or their agents have outstanding uploads. It does not poll, block
access, delete files, or advance the user-deletion workflow.

After access gating, run:

```sh
buzz-admin user-deletions wait-for-draining --pubkey <hex-or-npub> --uploads-blocked
```

The command prints JSON with `scope: "media_upload"`, `drained`, and counts by
community. It exits nonzero for active, unresolved, or unattributed uploads.
Keep the user blocked and rerun after their uploads finish. A database error or
standby connection is an error, never an empty successful result. A successful
result certifies only this media check; it does not certify other deletion work.

## Before running the check

`--uploads-blocked` is an operator assertion, not an action performed by this
command. It confirms both of these prerequisites:

- Every upload-serving replica runs the attributed-upload implementation. Apply
  the additive migration before upgrading the binaries. Resolve legacy in-flight
  work before using this check; a prior version may already have discarded its
  expired lease. A remaining legacy lease blocks the check because its signer
  is unknown. Do not roll upload writers back while relying on user draining.
- New uploads are blocked for the human and their agents on every replica. For
  NIP-FI, this requires Enforce mode, revocation applied on each serving replica,
  and continued denial throughout cleanup. An identity-provider receipt alone
  does not establish this. Off/Shadow mode and a membership change alone do not
  provide this prerequisite. Alternatively, stop upload ingress and serving
  tasks operationally and resolve their in-flight work.

The relay registers its lease before the final NIP-FI revocation check. A request
paused after its first authentication check therefore either appears in the
drain query or fails its final check before reading the body. Durable access
gating remains the caller's responsibility after the observation.

## Upload lifetime

The lease records the verified signer and the owner from a verified agent
assertion or the community profile. The observer also checks current community
ownership, so a transfer does not hide an upload from either affected account.
The existing `owner` column continues to identify the executing process.

During body reception and validation, the lease is renewable and reapable.
Canceling this preparatory work releases it. Immediately before each remote PUT,
including streaming video, thumbnails, upload records, and sidecars, a storage
callback validates the lease and durably marks `media_write_started`.

An armed upload uses `lease_until = 'infinity'` to keep older reapers and
community-deletion workers from discarding it or stepping past it. Its real
heartbeat deadline is `media_lease_until`; expiry prevents further writes and
renewal. Success waits for the configured ownership metadata before releasing the lease.
When audit is enabled, the upload audit entry is appended synchronously under
the admitted lease, with bounded database lock and statement waits. Audit failure
returns an upload error and retains the armed lease for investigation. Existing media-read modes still control the preview ledger;
the drain change grants no new preview access. Deployments that disable audit,
upload records and the preview ledger still lack upload ownership evidence for
later cleanup; draining does not replace that separate inventory prerequisite.

If a storage call is interrupted or its outcome is uncertain, the armed record
survives. Both user and community deletion remain blocked. The drain handler
does not turn an expired heartbeat into proof that a remote write stopped.

## Investigating an unresolved upload

Use the reported community ID and this read-only query on the primary to find
the relevant operation IDs. Include rows with no signer during upgrade checks.

```sql
SELECT id, community_id, owner, generation, fence_generation,
       encode(actor_pubkey, 'hex') AS actor,
       encode(actor_owner_pubkey, 'hex') AS admitted_owner,
       created_at, heartbeat_at, media_lease_until, media_write_started
FROM community_serving_write_leases
WHERE operation = 'media_upload'
ORDER BY community_id, created_at;
```

An unresolved row requires manual investigation of the serving process and
storage backend. Establish that the process cannot issue more writes and that
its remote requests have settled; a stopped heartbeat, a missing blob, or a
single object listing is insufficient. Preserve and reconcile any objects and
ownership evidence produced by the upload before retrying deletion. The lease
does not store an object manifest, so use request/storage logs to identify them.

Only after that independent confirmation may an operator release the exact
lease, checking its ID, executor, generation, and fence generation. There is no
automatic force-clear option in this handler. If completion cannot be proven,
leave the row and deletion blocked.

## Integration

`buzz_deletion::user::wait_for_draining::handle` is the callable handler;
`inspect` returns the report even when work remains. Both require the explicit
gating prerequisite. The separate real gating workflow can call `handle` and
save its stage result only on success. This PR does not change the legacy demo
workflow or treat its synthetic gate as proof of stopped uploads.
