# Buzz Admin

`buzz-admin` provides operator commands for a Buzz relay.

## Community host aliases

`buzz-admin community add-host --host <host>` registers another authority that
routes to the configured community. Every registered alias is returned in that
community's unauthenticated NIP-11 document and is readable by anyone who can
reach the relay. The complete list is intentional: Buzz Desktop uses it to
resolve historical media URLs after a relay address changes.

Adding a loopback, private-range, link-local, or `.local` host requires an
interactive confirmation. For reviewed automation, pass `--yes` (or `-y`) to
confirm the disclosure non-interactively.
