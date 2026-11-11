# Local media deletion fixture

`scripts/user-deletion-media-fixture.py` adds synthetic records and real versioned
objects to a disposable PostgreSQL database and S3-compatible bucket. The harness
owns service startup, schema bootstrap, bucket versioning and the two communities.
Python 3, `psql` and the AWS CLI must be on `PATH` (`PSQL` and `AWS` override those
executables). The script rejects non-loopback database and storage URLs.

Configure `DATABASE_URL`, `BUZZ_S3_ENDPOINT`, `BUZZ_S3_BUCKET`,
`BUZZ_S3_ACCESS_KEY`, `BUZZ_S3_SECRET_KEY`, and optionally `BUZZ_S3_REGION`.
Supply two different hex pubkeys and two existing community UUIDs:

```sh
python3 scripts/user-deletion-media-fixture.py seed \
  --target "$TARGET" --survivor "$SURVIVOR" \
  --community "$COMMUNITY" --other-community "$OTHER_COMMUNITY" \
  --manifest .scratch/media-fixture.json
python3 scripts/user-deletion-media-fixture.py verify \
  --manifest .scratch/media-fixture.json --phase before
```

The seed is additive and uses random content hashes/event IDs. It refuses to
overwrite a manifest. If interrupted, the manifest remains incomplete: use a
fresh disposable database/bucket and manifest rather than treating a partial
seed as complete. It never starts, stops, resets or deletes services.

Six cases cover exclusive media, same-community sharing, cross-community
sharing, preservation by other-community metadata, an unrelated survivor file,
and an unposted target upload attributed only by its storage upload record.
The exclusive file has two original versions, a thumbnail, metadata and an
upload record. Target and survivor uploader rows and sticky publication markers
are included. **Events are synthetic direct-SQL kind-9 records with dummy
signatures**, suitable for exercising inventory/deletion selectors; they do not
prove relay admission, upload authentication, client rendering, or signature
verification. The enclosing harness should exercise those separately.

After workflow print-only, run `verify --phase preview`: every fixture row and
object version must be identical to the seed baseline. After media deletion,
run `verify --phase after-media`: all exclusive/unposted versions must be absent,
shared/unrelated versions and all publication markers must match exactly,
captured target uploader proofs must be removed, and all source events must
still match. After the content stage, run `verify --phase after-content`: target
fixture events must additionally be absent and survivor events unchanged.

```sh
python3 scripts/user-deletion-media-fixture.py snapshot \
  --manifest .scratch/media-fixture.json > .scratch/media-current.json
python3 scripts/user-deletion-media-fixture.py verify \
  --manifest .scratch/media-fixture.json --phase preview
```

The manifest includes all exact version IDs and baseline rows. Verification
binds to the original database host/port/name and storage endpoint/bucket; it
does not store connection credentials. Verification
lists historical versions and delete markers, so hiding an object behind a
delete marker does not count as erasing it. Assertions cover this script's
fixture keys/rows; the enclosing harness should compare the rest of its seeded
database, workflow control state and storage separately. Snapshot/verify perform
only reads and never repair or rebaseline a mismatch.
