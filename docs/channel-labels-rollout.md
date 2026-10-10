# Channel labels: offline writer cutover

NIP-CL is **off by default**. This release does not implement a distributed
binary-version fence. `BUZZ_NIP_CL_WRITER_CUTOVER=offline-v1` is an operator
attestation, not proof that old writers have stopped. Do not enable the feature
unless the procedure below is enforced by the deployment/database operator.

## Boundary

- `BUZZ_NIP_CL_ENABLED=true` enables labeled creation and label mutation and
  advertises `nip-cl` on mapped community hosts. A configured
  `BUZZ_RELAY_PRIVATE_KEY` and the exact cutover declaration are required.
- Startup checks every undeleted channel, in bounded pages across communities,
  against the configured signer's complete canonical metadata projection. It
  fails before listeners/background publishers if repair is needed. It neither
  repairs nor establishes fleet exclusion. A community undergoing deletion can
  fail admission during this audit; finish its lifecycle before activation.
- Ordinary publishers and full operator reconciliation preserve labels even
  with the feature disabled. Unlabeled creation retains its legacy admission
  when disabled; clients must not assume NIP-CL receipts without capability.
- All replicas and repair tools must use the same stable relay key. Key rotation,
  import of draft `label`/extra-`t` formats, and arbitrary owner SQL are not
  automatic migrations. Resolve those states explicitly before activation.

## Cutover checklist

1. Back up the database and relay key. Record source/image identities for every
   command writer, metadata publisher, bootstrap/repair job, and maintenance
   tool. Suspend autoscalers, restart policies and scheduled old repair jobs.
2. Close routing to **all** community hosts. Stop all old relay and operator
   processes, drain transactions and prove they cannot restart with database
   write access. Closing ingress alone does not stop background publishers.
3. Revoke the old deployment's database login (or replace its credential and
   revoke all other write paths). Terminate its established sessions as well:
   revocation does not terminate existing transactions. Use a distinct current
   deployment credential. Verify old credentials cannot reconnect and
   `pg_stat_activity` has no old writer sessions. If identities are shared,
   fence the entire shared population. Record the observed exclusion, not just
   a configuration change. Never resume an old binary with the new credential.
4. Apply migrations with the new operator binary (`buzz-admin migrate`) while
   routing remains closed. Inventory community hosts from the operator control
   plane. For **each** host run the new binary with the existing stable key:

   ```bash
   # DATABASE_URL and BUZZ_RELAY_PRIVATE_KEY come from your secret manager.
   RELAY_URL=wss://community.example buzz-admin reconcile-channels
   ```

   Full repair pages through all undeleted channels, including archived ones.
   `--channel` is intentionally roster-only and does **not** repair metadata.
   A successful run for one host is not evidence for another host. Resolve any
   invalid stored label or oversized snapshot instead of clearing labels.
5. Start only compatible replicas, still behind closed routing, with:

   ```text
   BUZZ_NIP_CL_ENABLED=true
   BUZZ_NIP_CL_WRITER_CUTOVER=offline-v1
   BUZZ_RELAY_PRIVATE_KEY=<unchanged stable secret>
   ```

   Require the canonical metadata activation audit and readiness to pass on
   each replica. A config parse pass is not readiness. Verify mapped-host
   NIP-11 `self` equals the intended relay identity and `supported_extensions`
   includes `nip-cl`; an unknown host must not advertise it.
6. Exercise creation, mutation, exact retry, label discovery and ordinary
   metadata updates through each replica. Check signed snapshots and durable
   application evidence. Recheck old-writer exclusion, then reopen routing.

For a fresh isolated database, a zero-channel audit is valid. For an existing
fleet, an empty audit is not proof that the intended database was selected.
Record database identity and expected channel counts before activation.

## Disable and rollback

Set `BUZZ_NIP_CL_ENABLED=false` on every compatible replica to stop new label
commands and remove the advertisement. Keep the same key and label-preserving
metadata writers. A retained command with an uncertain outcome remains uncertain
if admission is now denied; preserve its exact signed journal.

**Do not roll back to a pre-label binary while label state remains.** Such a
binary can erase metadata labels even if commands are disabled. Any rollback
requiring old code must keep routing closed and all old writers fenced pending a
separately reviewed state migration. Do not delete application evidence merely
to reapply a command. Soft deletion is not permission to replay it.

## CLI recovery and human smoke test

Use the built candidate CLI, not an older installed `buzz`. Over HTTPS the CLI
uses the authenticated NIP-11 `self`; over HTTP, supply `--trusted-relay <hex>`
from an out-of-band source. Every new command needs a fresh local journal path;
parent directories must exist. Keep it until the outcome is resolved.

```bash
buzz channels create --name label-smoke --type stream --visibility open \
  --label team:infra --command-file ./create.jsonl
# Save the returned channel_id as CHANNEL.
buzz channels labels get --channel "$CHANNEL"
buzz channels labels update --channel "$CHANNEL" --add-label status:active \
  --command-file ./add.jsonl
buzz channels labels find --label team:infra --limit 1
buzz channels labels retry --command-file ./add.jsonl
buzz channels topic --channel "$CHANNEL" --topic ordinary-metadata
buzz channels labels get --channel "$CHANNEL"
```

Expected: command outcome `committed`; reads contain both labels after the topic
update. Retry reuses the original event and must not restore superseded labels.
`unknown` is not rejection. Use only `retry --command-file` for uncertain
commands; never generate a new ID or infer a receipt from current labels.
Committed journals return their known outcome without resending. Journals lock
out concurrent local attempts; do not copy a journal to send it concurrently.

Agent evidence does not complete the repository's human-testing requirement.
Run this smoke test and report the candidate revision and observed results.
