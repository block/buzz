# Manage a community's NIP-FI requirement

Use `buzz-admin nip-fi-policy` to enable, disable or inspect a community's
stored requirement through the deployment operator API. The command needs the
configured `RELAY_OPERATOR_API_ORIGIN` and an allowlisted operator signing key
in a protected file. It sends one signed request and never retries automatically.

Before rolling, close `BUZZ_NIP_FI_UPGRADES_ENABLED` on every old relay and
keep `BUZZ_NIP_FI_CHANGES=false` on new relays. Pause callers and upgrade them
for the [supported API](operator-nip-fi-policy-client.md) before deploying this
relay version. Apply the schema and roll compatible management writers across
the whole fleet. Then explicitly
enable that gate, which controls both enable and disable. Enable additionally
requires the deployment-selected issuer to be trusted and already bound to the
community. The active community must have exactly one owner; the authenticated
actor mediated by the operator must be that owner. See the
[policy contract](community-auth-upgrades.md) and [binding guide](operator-nip-fi-bindings.md).

Read current preconditions first:

```sh
buzz-admin nip-fi-policy --origin https://operator.example.org --key-file operator.key \
  status --community-id c9acf9a6-37d5-4af1-8df9-92a165637b01 --host team.communities.example.org
```

Save a new request file with a fresh operation UUID and the exact
`expected_policy_generation` and `expected_requires_nip_fi` from that response:

```json
{
  "operation_id": "c2ffcc20-9505-48fd-b6c7-446aa833bcd1",
  "community_id": "c9acf9a6-37d5-4af1-8df9-92a165637b01",
  "expected_host": "team.communities.example.org",
  "actor_pubkey": "<owner's registered lowercase hex public key>",
  "expected_owner_pubkey": "<same current owner's public key>",
  "expected_issuer": "https://identity.example.org",
  "requires_nip_fi": true,
  "expected_policy_generation": 0,
  "expected_requires_nip_fi": false
}
```

```sh
buzz-admin nip-fi-policy --origin https://operator.example.org --key-file operator.key \
  enable --request policy.json
buzz-admin nip-fi-policy --origin https://operator.example.org --key-file operator.key \
  status --operation-id c2ffcc20-9505-48fd-b6c7-446aa833bcd1
```

To disable, save a **new** file/UUID with `requires_nip_fi:false`, omit
`expected_issuer`, and copy freshly read preconditions. Run `disable --request`
with that file. The subcommand checks the saved action before reading the key
or sending anything. A same-state operation also advances the generation,
creating a barrier against earlier uncommitted requests.

After `policy_conflict`, re-read current policy and use a new UUID for the revised
intent. After a timeout or lost response, keep the complete original file and UUID:
read its status or retry that exact file. A committed replay never reapplies policy.

| Exit | Result |
|---|---|
| 0 | Current-state read, or applied stored disable |
| 10 | Enable accepted; connection closure remains unverified (`pending`) |
| 11 | Historical acceptance superseded by a later generation |
| 12 | Receipt target or required trust unavailable |
| 5 | Invalid input, API refusal or uncertain transport/response |

Inspect `mode` and `effective_required` separately from the stored flag.
`enforce` keeps FI required after disable; `off` suspends FI after enable.
`shadow` follows the stored flag. An enable receipt remains pending even after
local sweeps, since it cannot certify fleet-wide connection closure. Required
host caches can over-restrict for their remaining one-second lease after disable.
A racing sweep may close one last non-FI socket; it can reconnect under optional
policy. FI-admitted sockets retain their finite deadline. Re-enable repeats the
optional-lease drain and existing periodic revalidation.

The old `nip-fi-upgrade` command and endpoint are removed. Requests to that
route now return 404. Recover historical receipts with `nip-fi-policy status
--operation-id`; their original null CAS preconditions remain visible. Never
reuse a historical operation UUID for a new intent.
