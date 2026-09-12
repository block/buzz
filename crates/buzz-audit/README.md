# Audit hash encodings

New entries use `hash_version = 2`. The SHA-256 input starts with
`buzz:audit:v2\0`, then fields in this order: community UUID bytes, signed
64-bit big-endian sequence, storage-precision RFC3339 timestamp, action,
optional actor bytes, optional object ID, canonical JSON detail, optional
previous hash. Field tags are the single bytes 1 through 8. Each required
field carries its tag, unsigned 64-bit big-endian byte length, and bytes.
Optional fields carry their tag and a zero/one presence byte; present values
then carry their length and bytes. Public keys and previous hashes must be
32 bytes when present.

Apply migration 0045 (or the desired schema) and upgrade verifiers before
deploying the new writer. Older verifiers cannot validate version 2 rows.
The column defaults to 1 so existing rows and old writers retain their actual
encoding during a rolling deployment. New writers explicitly select 2 and
link to the previous row's unchanged hash. Do not backfill the version or
recompute historical hashes.

Legacy verification accepts absent object IDs, canonical hyphenated UUIDs,
and 64-character lowercase hex identifiers. It rejects other identifiers and
invalid cryptographic field widths because the old concatenation cannot
reliably separate those values. Historical event and media writers used these
accepted forms. A legacy entry with a nonconforming identifier needs operator
inspection; changing its identifier or version is not a repair of its proof.
Legacy genesis entries must have no previous hash, and subsequent entries
must have one.

These checks prevent alternate interpretations accepted by this verifier.
They cannot prove that a legacy row was never altered before this upgrade.
Preserve any independently trusted chain checkpoints when rolling forward.
