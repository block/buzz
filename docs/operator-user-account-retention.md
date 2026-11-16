# Account cleanup: retained records and boundaries

Account cleanup removes attributable credentials, access and personal account
records. It does not mean every reference to an identity disappears. Where
ownership or treatment is uncertain, preserve the record and report the gap.
Do not infer permission to delete another person's data from a matching key.

| Records | Treatment |
| --- | --- |
| Shared channel creator attribution | Keep `channels.created_by`. Channel content belongs to content cleanup; account cleanup removes the target's membership. |
| Repositories, Git history and routing | Keep repositories, history, name mappings and required owner references. Do not automatically transfer repositories or make them read-only for other users. Retaining a routing reference is not proof that the target's access has been revoked. |
| Other people's role provenance | Keep non-null `relay_operators.added_by` on surviving roles. Removing the target's own role is separate. |
| Moderation and enforcement | Keep bans, reports, moderation actions, admin action history and rate-limit history, including identifying keys and notes. Pending actions and copied payloads still require review before claiming complete access revocation or erasure. |
| Audit history | Keep operator audit and hash-chained audit history intact. Record new role revocations in the normal audit trail. |
| Shared device registrations | Keep registrations used by other accounts. Delete only with proven exclusive ownership; preserve and report uncertain ownership. Relay identity keys and client installation strings do not establish gateway ownership. |
| Shared signed membership lists | Rebuild current member/admin/metadata lists using the existing relay signer, atomically with each channel membership removal. Preserve superseded historical events and creator attribution. Never edit a signed payload in place. |
| Multi-subject commands and membership notices | Preserve; attribution and historical-message removal remain separate. |
| Deletion evidence and replay protection | Keep deletion requests, approvals, inventories, stage history, fences and replay watermarks. Watermark removal is not unfinished cleanup. |

These are explicit retention exceptions, not assertions of anonymous data.
Other people's signed references can continue to identify the deleted key.
Retaining a record must never be used to claim the deleted identity has lost
authority: repository owner checks, config-backed operator roles and live
sessions need separate enforcement and verification.

Profile rows, profile events, managed-agent definitions and archived identities
belong to profile cleanup. Independent leftover-data verification and final
coordinator wiring belong to the verifier change. The existing account command
must keep reporting `account_erasure_complete: false` while those prerequisites
and the remaining account work are incomplete.

For supported selectors and runnable commands, see [offline account cleanup](operator-user-account-cleanup.md).

All deletion commands run offline with relay and external writers stopped.
Preview is read-only. A per-community invocation must not silently mutate
deployment-global privileges or registrations.
