# Local account fixture

`scripts/user-deletion-account-fixture.py` adds account cases to an already
bootstrapped disposable PostgreSQL database. It needs Python 3 and `psql` on
PATH (or `PSQL=/path/to/psql`). `DATABASE_URL` must use a loopback host; the
script neither creates/drops databases nor starts/stops services.

```sh
python3 scripts/user-deletion-account-fixture.py seed \
  --target "$TARGET" --survivor "$SURVIVOR" \
  --community "$COMMUNITY" --other-community "$OTHER_COMMUNITY" \
  --manifest .local/account-fixture.json

# After every seeder and request preparation have finished:
python3 scripts/user-deletion-account-fixture.py snapshot \
  --manifest .local/account-fixture.json

# Run the workflow's print-only stage, then:
python3 scripts/user-deletion-account-fixture.py verify --phase preview \
  --manifest .local/account-fixture.json

# Run this community's account stage with --delete, then:
python3 scripts/user-deletion-account-fixture.py verify --phase after-account \
  --manifest .local/account-fixture.json
```

Seed inputs are lowercase hex public keys and distinct community UUIDs. Seed
adds a separate channel and 250 read-frontier rows for each user in each
community, revoked tokens, removed memberships, invites, policy acceptances,
allowlist entries, archive records and replay watermarks. Existing users and
memberships are reused without rewriting them. A new manifest is required;
partial seed failure rolls back all inserted rows. A process/filesystem failure
after database commit can leave fixture rows without a manifest; recreate the
disposable database instead of assuming such a seed is retryable.

Snapshot includes every row of the account tables, users, replay watermarks,
and deletion request/approval/stage tables. Preview requires exact equality.
After-account verifies the exact expected retained rows and fields; only
documented nullable attribution changes and their synchronization timestamps
may differ. Revoked tokens stay revoked, removed memberships stay removed,
profiles and archive visibility survive, and the other community stays intact.
The verifier deliberately does not constrain workflow control rows after delete,
because the real workflow must save progress. The integration harness verifies
those transitions separately, along with media/content tables.

The repeated target key in the other community deliberately tests tenant
scoping. Verify immediately after the first account stage. After processing
both communities, repeat `--completed-community` for both IDs:

```sh
python3 scripts/user-deletion-account-fixture.py verify --phase after-account \
  --completed-community "$COMMUNITY" --completed-community "$OTHER_COMMUNITY" \
  --manifest .local/account-fixture.json
```

Take the snapshot only once. This prevents accidentally replacing the baseline
after a destructive run. The manifest includes fixture credential hashes and
should stay local. Success covers these account operations only; it never
claims complete user erasure.
