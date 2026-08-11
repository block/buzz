# Chunk 09 — membership

**What it does**

Reads the relay's roster with `./run.sh list-members` and approves every pubkey in `buzz_members`
that is not already on it, one at a time with a pause between adds. It then re-reads the roster and
asserts it matches the intended state, so a converged host runs no add at all and reports zero
changes.

**SOP steps covered:** Step 11.

**Preconditions**

- Chunk 07 has run and the stack is up: the relay must have **started successfully at least once**,
  because `buzz-admin` resolves the community the relay seeded at startup.
- `.env` from chunk 06 is in place with `BUZZ_RELAY_PRIVATE_KEY` set — `add-member` refuses to run
  without it, since it has to sign the kind:13534 roster event.
- `buzz_members` is supplied at run time as a list of `{pubkey, role}`. **64-character hex only, no
  `npub1…`.** Keep it out of the repository (AGENTS.md rule 2) — a file outside the repo, passed
  with `-e @…`:

  ```yaml
  # ~/buzz-roster.yml  — not in git
  buzz_members:
    - pubkey: "<64-char lowercase hex public key>"
      role: member
    - pubkey: "<64-char lowercase hex public key>"
      role: admin
  ```

**Run**

```bash
cd /Users/jeff/group-build-project/buzz/launchpad/deploy/ansible
ansible-playbook playbooks/09-members.yml --limit dev-vm -e @"$HOME"/buzz-roster.yml
```

`./deploy run 09` runs the same playbook, but passes no extra vars — it only sees a roster that
Ansible loads by itself, so use the command above whenever the list lives outside the repo.
`./deploy check 09` is a safe dry run: it reads the roster and reports what is missing without
adding anything.

**Verify**

```bash
# 1. Reruns converge — this is the check, not a nicety.
cd /Users/jeff/group-build-project/buzz/launchpad/deploy/ansible
ansible-playbook playbooks/09-members.yml --limit dev-vm -e @"$HOME"/buzz-roster.yml
# expect: PLAY RECAP ... changed=0    failed=0

# 2. The roster itself, read on the VM.
ssh -p 2222 dev@127.0.0.1 'cd /opt/buzz/compose && sudo ./run.sh list-members'
```

Expected shape — the owner (seeded from `RELAY_OWNER_PUBKEY`, role `owner`) plus one line per
approved key:

```
pubkey                                                             role     added_by                                                           created_at
----------------------------------------------------------------------------------------------------------------------------------------------------------
<64 hex>                                                           owner    -                                                                  2026-08-12T09:14:02Z
<64 hex>                                                           member   -                                                                  2026-08-12T09:31:20Z
```

The play's own `success_msg` states the same thing as a count, and its final task prints these
lines, so evidence for hardening-spec Part F assertion 19 ("roster matches intent") needs no
separate session.

**Rollback**

The roster lives in Postgres, so it survives a container restart — undo it explicitly, and with the
same discipline as adding:

```bash
ssh -p 2222 dev@127.0.0.1
cd /opt/buzz/compose
sudo ./run.sh remove-member <64-hex pubkey> --role member
sleep 1   # same timestamp rule as adding
sudo ./run.sh list-members
```

For a wholesale undo, restore the VM snapshot taken before this chunk
(`VBoxManage snapshot buzz-dev restore <name>`), which reverts the database with everything else. If
the run failed part-way, do **not** re-run it blind: read `list-members` first, fix `buzz_members` to
match what you actually want, and re-run — the role adds only what is missing.

**Traps**

- Adds are serialised on purpose — `throttle: 1`, a `pause` of ≥1s and a `flock`; two adds in the
  same second can corrupt the kind:13534 roster event, and that roster is the access control list
  (SOP Step 11; hardening-spec sec-B13). Never `async`, never a parallel fan-out.
- **Whether an unapproved key is actually refused has never been tested** — SOP Step 11's own
  honesty flag. Refusal happens during NIP-42 auth *after* the WebSocket `101`, so a successful
  upgrade proves the `Host` matched a community, not that membership was enforced. Rehearse it here
  before trusting `BUZZ_REQUIRE_RELAY_MEMBERSHIP` on the VPS (sec-B13).
- `RELAY_URL host '…' is not mapped to a community` means the relay never started properly, not that
  the command is wrong — there is no community-seeding command, the relay seeds from `RELAY_URL` at
  startup (SOP Step 11 note; runbooks/relay-build-list.md). The role detects this and says so
  instead of surfacing a bare non-zero exit.
- `add-member` can never change an existing member's role: the insert is `ON CONFLICT DO NOTHING`
  and still reports success (`crates/buzz-db/src/relay_members.rs`). The role fails early on a role
  mismatch; fix it with `remove-member` then re-run.
- Do not list the owner's key in `buzz_members`. It is approved at startup with role `owner`, and
  `buzz-admin` refuses to set that role from the CLI (SOP Step 11; `validate_role`).
- `npub1…` is rejected by design: `list-members` prints hex, and comparing hex is what keeps the
  role idempotent. Convert before adding it to the roster file.
- Missing `BUZZ_RELAY_PRIVATE_KEY` in the relay container's environment makes `add-member` fail
  closed — that is chunk 06's `.env`, not a fault in this chunk (SOP Step 7).
- Only the **first** column of `list-members` is a member; `added_by` is a pubkey too. A parser that
  scans the whole line for hex silently treats an adder as approved and skips someone real.
