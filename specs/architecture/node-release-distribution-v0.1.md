# Node release distribution v0.1

Status: active. First release: `node/v0.1.0`. The consumer-side update
command and `doctor` closure land with the managed context CLI
(handoff `075aa020…`); until then the consumer procedure below is manual.

## Purpose

One-way distribution of the node runtime — relay, CLIs, skills, signing
tooling, LaunchAgent templates — from a single development node to
downstream consumer nodes, over ordinary Git, with releases attributable to
the same identity that owns the development node's journal.

Roles are fixed by policy, not capability:

- **Development node** (`ted-laptop`): the only node where changes to the
  runtime happen. Builds, specs, and releases originate here.
- **Consumer node** (downstream hosts): verify, check out, rebuild, test,
  and generate feedback. Consumer nodes never edit installed executables;
  they configure profiles.

## Channel

- Repository: the Git fork both nodes already use.
- Release = an annotated tag in the `node/vX.Y.Z` namespace.
- Every release tag is signed with the development node's key
  (NIP-GS, BIP-340) — the same identity that signs the journal's owner
  declarations and is publicly attested through the hardware binding
  statement.

Release key (pinned):

```text
9c2fd8696a630bdf27c3d54394739bb3cbbf81b7cf7fa1e205a806eca33fa90e
```

## Consumer procedure

On each update check:

1. `git fetch origin --tags`
2. Select the newest `node/v*` tag.
3. Verify the signature and **pin the key explicitly** — `TRUST_FULLY` is
   advisory (config-match-only) and is never sufficient:

   ```bash
   git verify-tag --raw <tag> 2>&1 \
     | grep -q "GOODSIG 9c2fd8696a630bdf27c3d54394739bb3cbbf81b7cf7fa1e205a806eca33fa90e"
   ```

4. Check out the verified tag; never track a moving branch.
5. Rebuild from source (`cargo install --locked --path …` / the repo's
   install target). No prebuilt artifacts at two-node scale.
6. Run `doctor` once available; it must attest the installed executables
   match the checked-out release.
7. Append a signed session record to the shared context: tag, result, and
   any defect found. This residue is the feedback channel.

## Trust bootstrap

The verifier (`git-sign-nostr`) is built from the same repository it
verifies. First installation on a new consumer node is trust-on-first-use;
every subsequent update is verified by the previous installation's
verifier. TOFU happens once per node and should be recorded as a session
record when it happens.

## What never travels through this channel

Private keys, journals, replication cursors, profiles, artifact stores, and
all durable relay state. Git moves capability; the node keeps memory.
Distribution of context is the sovereign-sync layer's job, not this
channel's.

## Verification of the loop

The development node observes update health from the journal, not from the
consumer's machine: each consumer update produces a signed session record
that drains home through the existing shared streams. An update that leaves
no residue is indistinguishable from an update that never ran — silence is
a finding. Folding installed-version reporting into the steward's cycle is
the v0.2 candidate that makes version skew a witnessed condition rather
than a checkable one.

## Out of scope (v0.1)

- Automatic/unattended updates on consumer nodes.
- Prebuilt release artifacts and multi-platform binaries.
- Multi-key or threshold release signing.
- Revocation of a compromised release key (rotate by publishing a new
  binding statement and re-pinning consumers manually).
