# NIP-FI Community Issuer Bindings

A deployment that provisions communities under one host suffix, for example
`<name>.communities.buzz.xyz`, stores the NIP-FI issuers allowed to vouch for
each community in `community_nip_fi_issuers`. The relay writes these rows when
it creates a community; `buzz-admin nip-fi` backfills and edits them.

A host may hold a binding only when it is exactly one DNS label under the
suffix, has no port, and is not *reserved*. A host is reserved when
`https://<host>` is a `command_audiences` entry of any issuer in
`BUZZ_NIP_FI_ISSUERS`, so a command audience can never become a community
assertion audience. The relay refuses to start while a reserved host holds a
binding, and the operator lookup route reports no issuers for a host that is not
bindable or for an issuer that is not configured.

## Configuration

`BUZZ_NIP_FI_PROVISION_BINDING` is read in every NIP-FI mode. Unset or empty
binds nothing.

```json
{
  "host_suffix": "communities.buzz.xyz",
  "authorized_issuers": ["https://app.builderlab.xyz/api/goose"]
}
```

- `host_suffix` is a lowercase domain name without a port or trailing dot.
- `authorized_issuers` is non-empty, and every entry must be an `issuer` in
  `BUZZ_NIP_FI_ISSUERS`. Each issuer is 1 to 2048 bytes.
- The suffix must not cover any `BUZZ_NIP_FI_COMMUNITIES` host, and every
  `canonical_uri` there must be readable. The relay and `buzz-admin` both
  refuse to start otherwise.

`buzz-admin` parses the same variables with the relay's parser. Run it with the
relay's environment: `BUZZ_NIP_FI_PROVISION_BINDING`, `BUZZ_NIP_FI_ISSUERS`,
`BUZZ_NIP_FI_COMMUNITIES`, and `DATABASE_URL`.

## Commands

```sh
buzz-admin nip-fi backfill
buzz-admin nip-fi bind --host acme.communities.buzz.xyz --issuer https://issuer.example
buzz-admin nip-fi unbind --host acme.communities.buzz.xyz --issuer https://issuer.example
```

`backfill` binds the authorized issuers to every active, unbound, bindable
community, a page of 1000 hosts at a time. Archived communities are bound on
purpose, so unarchiving leaves a community that can log in. A community whose
deletion has started is skipped. It always prints its counts and exits 1 while
any bindable community remains unbound:

```json
{"rows_bound": 12, "unbound": 0, "unbound_hosts": [], "truncated": false}
```

`unbound_hosts` lists at most 100 hosts; `truncated` is true when there are
more.

`bind` refuses a reserved host, a host that is not one label under the suffix,
and an issuer not in `BUZZ_NIP_FI_ISSUERS`.

`unbind` works without `BUZZ_NIP_FI_PROVISION_BINDING`. It refuses to remove a
community's last issuer, because an empty community looks never-provisioned and
the next `backfill` would bind it again. To swap issuers, bind the new one and
then unbind the old one. To stop a community logging in, archive or delete it.
When the provisioning config is present and shows the host is not bindable (a
reserved host or one outside the suffix), `unbind` removes the last issuer too;
this is how to clear a reserved-host binding that blocks relay startup.

`backfill` never adds the default issuers beside an operator `bind`: both lock
the community row, and the backfill only inserts while the community still has
no binding.

## Lookup route

`GET /operator/communities/nip-fi-binding?host=<host>` is operator-signed and
returns `{"host": ..., "active": ..., "issuers": [...]}`. kgoose relies on this
contract:

- `host` is the host stored on the community row.
- Every lifecycle state answers 200. `active` is `false` once the community is
  archived or its deletion has started quiescing, including a tombstoned
  community; it stays `true` while a deletion is only requested.
- `issuers` is `[]` when `BUZZ_NIP_FI_PROVISION_BINDING` is unset, the host is
  not bindable, or nothing is bound. Only issuers in `BUZZ_NIP_FI_ISSUERS` are
  reported, in byte order.
- 404 means no community has this host; 400 means the host is malformed.

## Rollout

1. Deploy the relay with `BUZZ_NIP_FI_PROVISION_BINDING` set. New communities
   are bound at creation from then on.
2. Run `buzz-admin nip-fi backfill` with the relay's environment. Repeat it until
   it exits 0.
3. Do not turn on the database binding source in enforce mode while any
   bindable community is unbound: an unbound community fails closed.
