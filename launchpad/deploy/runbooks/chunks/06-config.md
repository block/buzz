# Chunk 06 — config

**What it does.** Generates every secret and both Nostr identities **on the target**, renders
`.env` at mode 0600 root, and renders the two cohort-owned overrides — `compose.cohort.yml` and the
`tls internal` Caddyfile. Nothing secret is templated from the control node and nothing lands in a
tracked file.

**SOP steps covered:** 7 (7.1–7.5), corrected for TLS per `hardening-spec.md` §A.3. Rationale lives
there, not here.

## Preconditions

- Chunk 05 has run and recorded a pin in `/opt/buzz/compose/UPSTREAM_COMMIT`.
- You have the same resolved image reference to hand — the key generator runs from that image.

## Run

`./deploy run 06` cannot pass `-e`, so drive this one directly with the pin chunk 05 used:

```bash
cd /Users/jeff/group-build-project/buzz/launchpad/deploy/ansible
ansible-playbook playbooks/06-config.yml --limit dev-vm \
  -e buzz_image=ghcr.io/block/buzz:sha-96ae141
```

## Verify

```bash
ssh -p 2222 dev@127.0.0.1 'sudo stat -c "%a %U" /opt/buzz/compose/.env'
# expect: 600 root

# No unfilled placeholder survives (replicates upstream run.sh's require_env,
# which we lose by invoking docker compose directly).
ssh -p 2222 dev@127.0.0.1 \
  "sudo grep -Ec '^[[:space:]]*[A-Za-z_][A-Za-z0-9_]*=.*CHANGE_ME' /opt/buzz/compose/.env"
# expect: 0

# The addressing family must agree, or every client is refused.
ssh -p 2222 dev@127.0.0.1 \
  "sudo grep -E '^(RELAY_URL|BUZZ_DOMAIN|BUZZ_ADMIN_HOST)=' /opt/buzz/compose/.env"
# expect: RELAY_URL=wss://buzz-vm.test:8443
#         BUZZ_DOMAIN=buzz-vm.test
#         BUZZ_ADMIN_HOST=admin.buzz-vm.test:8443
```

The owner's secret key is left at `/root/owner-key.txt` (mode 0600). **Move it to a password manager
and delete it** — chunk 08 needs it to launch the desktop app as the owner.

## Rollback

Re-running is safe and changes nothing. To start the configuration over — accepting that it mints
**new identities** and therefore a different owner and a different relay key:

```bash
ssh -p 2222 dev@127.0.0.1 'sudo rm /opt/buzz/compose/.env /root/owner-key.txt'
```

Do not do this on a stack that has published anything. The relay key cannot be rotated; every event
signed with the old one becomes unverifiable (`hardening-spec.md` §B8).

## Traps

- **Secrets are generate-once by design.** `.env` is written only when absent; later runs converge
  only the non-secret lines. A role that re-rendered the whole file every run would rotate
  `BUZZ_RELAY_PRIVATE_KEY` and silently break signature verification for the whole community.
- **Two separate keypairs, and the generator's advice is wrong for both.** `buzz-admin generate-key`
  prints *"Set BUZZ_PRIVATE_KEY to the secret key"*. There is no `BUZZ_PRIVATE_KEY` here: the owner
  contributes its **public** half to `RELAY_OWNER_PUBKEY`, and a **second** keypair contributes its
  **secret** half to `BUZZ_RELAY_PRIVATE_KEY`. Reusing one keypair for both conflates two roles.
- **`RELAY_URL` is not `BUZZ_`-prefixed**, and the relay's own fatal-error message calls it
  `BUZZ_RELAY_URL`, which is wrong — setting that name has no effect and leaves the `ws://localhost:3000`
  default in place (`relay-build-list.md`).
- **`BUZZ_ADMIN_HOST` is empty in production on purpose.** With it unset the relay never mounts
  `/api/admin/v1` at all. Its only credential is a matching `Host` header — no token, no Nostr auth,
  no membership check — so on a public host setting it is an unauthenticated disclosure of moderation
  reports and feedback attachments (`hardening-spec.md` §B1).
- **`BUZZ_AUTO_MIGRATE=true` is correct here and wrong for production.** Upstream's `compose.yml`
  defaults it to `false`, which is what upstream considers the production value (§B6).
- **The Caddyfile is validated before it is installed**, via `caddy validate` in a throwaway
  container. A malformed one would otherwise take the reverse proxy — and therefore the whole
  deployment — offline on the next restart.
